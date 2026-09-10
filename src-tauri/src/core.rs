//! mihomo core process management: spawn, supervise, stop.

use serde::Serialize;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Clone, Serialize)]
pub struct ApiInfo {
    pub port: u16,
    pub secret: String,
    pub mixed_port: u16,
    pub started_at_ms: u128,
}

#[derive(Default)]
pub struct CoreState {
    pub child: Mutex<Option<Child>>,
    pub api: Mutex<Option<ApiInfo>>,
    /// core was started through the elevated service (TUN mode)
    pub via_service: Mutex<bool>,
}

#[derive(Clone, Serialize)]
pub struct CoreStatus {
    pub running: bool,
    pub api: Option<ApiInfo>,
}

fn free_port() -> Result<u16, String> {
    TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("no free port: {e}"))
}

fn random_secret() -> String {
    (0..32)
        .map(|_| char::from_digit(fastrand::u32(0..16), 16).unwrap())
        .collect()
}

pub const CORE_BIN: &str = if cfg!(windows) { "mihomo.exe" } else { "mihomo" };

/// The binary the core actually runs from: a self-upgraded copy in the app
/// data dir wins over the bundled one.
pub fn core_binary(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(p) = crate::updater::upgraded_binary(app) {
        return Ok(p);
    }
    bundled_binary(app)
}

/// Android 10+ only executes files inside the app's nativeLibraryDir, so
/// the core ships as `libmihomo.so` in jniLibs (docs/ANDROID.md). Our own
/// Rust library is loaded from that same directory: ask the dynamic
/// linker where it came from and look next to it.
#[cfg(target_os = "android")]
fn native_lib_dir() -> Result<PathBuf, String> {
    use std::ffi::CStr;
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    let probe = native_lib_dir as *const () as *const libc::c_void;
    let ok = unsafe { libc::dladdr(probe, &mut info) };
    if ok == 0 || info.dli_fname.is_null() {
        return Err("dladdr: cannot locate our own library".into());
    }
    let me = unsafe { CStr::from_ptr(info.dli_fname) }.to_string_lossy().to_string();
    PathBuf::from(&me)
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| format!("no parent dir for {me}"))
}

/// Locate the bundled mihomo binary: bundle resources first, then the
/// source tree (dev builds where resources are not yet copied).
pub fn bundled_binary(app: &AppHandle) -> Result<PathBuf, String> {
    #[cfg(target_os = "android")]
    {
        let _ = app;
        let p = native_lib_dir()?.join("libmihomo.so");
        return if p.exists() {
            Ok(p)
        } else {
            Err(format!(
                "libmihomo.so not packaged ({}) — run `node scripts/fetch-core.mjs --android` before building",
                p.display()
            ))
        };
    }
    #[allow(unreachable_code)]
    if let Ok(dir) = app.path().resource_dir() {
        let p = dir.join("resources").join("mihomo").join(CORE_BIN);
        if p.exists() {
            return Ok(p);
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("mihomo")
        .join(CORE_BIN);
    if dev.exists() {
        return Ok(dev);
    }
    Err(format!(
        "{CORE_BIN} not found in resources/mihomo/ — run `node scripts/fetch-core.mjs` to download it"
    ))
}

fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("core");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

#[tauri::command]
pub fn core_start(app: AppHandle, state: State<'_, CoreState>) -> Result<ApiInfo, String> {
    let use_service = crate::settings::load(&app).tun.enable;
    if use_service && !crate::service::ping(&app) {
        return Err("TUN 模式需要系统服务：请先在设置页安装 Mfork 服务".into());
    }

    // hold the lock through spawn so two concurrent calls cannot both start
    let mut child_guard = state.child.lock().unwrap();
    let already = if *state.via_service.lock().unwrap() {
        crate::service::core_running(&app)
    } else if let Some(child) = child_guard.as_mut() {
        matches!(child.try_wait(), Ok(None))
    } else {
        false
    };
    if already {
        if let Some(info) = state.api.lock().unwrap().clone() {
            return Ok(info);
        }
    }

    let bin = core_binary(&app)?;
    let dir = data_dir(&app)?;
    let api_port = free_port()?;
    let mut settings = crate::settings::load(&app);
    let mixed_port = if settings.mixed_port > 0 {
        let fixed = settings.mixed_port;
        if TcpListener::bind(("127.0.0.1", fixed)).is_err() {
            return Err(format!("端口 {fixed} 已被占用，请在设置中更换混合端口"));
        }
        fixed
    } else {
        // automatic: keep the port from last time so tools that do not read
        // the OS proxy (CLIs, containers, IDEs) keep working across restarts
        let last = settings.last_mixed_port;
        let port = if last > 0 && TcpListener::bind(("127.0.0.1", last)).is_ok() {
            last
        } else {
            free_port()?
        };
        if port != last {
            settings.last_mixed_port = port;
            let _ = crate::settings::save(&app, &settings);
        }
        port
    };
    let secret = random_secret();

    let config_path = crate::profile::generate_config(&app, api_port, &secret, mixed_port)?;

    // clean up whatever ran before, regardless of mode
    if let Some(mut c) = child_guard.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    if crate::service::ping(&app) && crate::service::core_running(&app) {
        crate::service::stop_core(&app);
    }

    let pid;
    // where core.err.log ends up: our data dir, or the service's own
    let mut log_dir = dir.clone();
    if use_service {
        let (p, svc_dir) = crate::service::start_core(
            &app,
            &config_path.to_string_lossy(),
            &dir.to_string_lossy(),
        )?;
        pid = p;
        if let Some(d) = svc_dir {
            log_dir = PathBuf::from(d);
        }
        *state.via_service.lock().unwrap() = true;
    } else {
        let mut cmd = Command::new(&bin);
        cmd.arg("-d")
            .arg(&dir)
            .arg("-f")
            .arg(&config_path)
            .stdout(std::process::Stdio::from(
                std::fs::File::create(dir.join("core.log")).map_err(|e| e.to_string())?,
            ))
            .stderr(std::process::Stdio::from(
                std::fs::File::create(dir.join("core.err.log")).map_err(|e| e.to_string())?,
            ));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = cmd.spawn().map_err(|e| format!("spawn mihomo failed: {e}"))?;
        pid = child.id();
        *child_guard = Some(child);
        *state.via_service.lock().unwrap() = false;
    }
    drop(child_guard);

    // wait until the external controller answers (max ~8s)
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if TcpStream::connect_timeout(
            &format!("127.0.0.1:{api_port}").parse().unwrap(),
            Duration::from_millis(300),
        )
        .is_ok()
        {
            break;
        }
        if Instant::now() > deadline {
            let _ = core_stop_inner(&app, &state);
            return Err(format!(
                "mihomo (pid {pid}) did not open the API port within 8s — see core.err.log in {}",
                log_dir.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(150));
    }

    let info = ApiInfo {
        port: api_port,
        secret,
        mixed_port,
        started_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    };
    *state.api.lock().unwrap() = Some(info.clone());

    // the mixed port may have changed: re-point the OS proxy at it
    if let Err(e) = crate::system::sync_system_proxy(&app) {
        let _ = app.emit("sys-proxy-error", e);
    }

    // watcher: notify the frontend if the core dies
    let app2 = app.clone();
    if use_service {
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(2));
            let state = app2.state::<CoreState>();
            if !*state.via_service.lock().unwrap() {
                break; // switched mode or stopped deliberately
            }
            if state.api.lock().unwrap().is_none() {
                break;
            }
            if !crate::service::core_running(&app2) {
                *state.api.lock().unwrap() = None;
                *state.via_service.lock().unwrap() = false;
                crate::system::clear_if_enabled(&app2);
                let _ = app2.emit("core-exited", None::<i32>);
                break;
            }
        });
    } else {
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(800));
            let state = app2.state::<CoreState>();
            let mut guard = state.child.lock().unwrap();
            match guard.as_mut() {
                Some(c) => match c.try_wait() {
                    Ok(Some(status)) => {
                        *guard = None;
                        *state.api.lock().unwrap() = None;
                        drop(guard);
                        crate::system::clear_if_enabled(&app2);
                        let _ = app2.emit("core-exited", status.code());
                        break;
                    }
                    Ok(None) => {}
                    Err(_) => break,
                },
                None => break, // stopped deliberately
            }
        });
    }

    Ok(info)
}

fn core_stop_inner(app: &AppHandle, state: &State<'_, CoreState>) -> bool {
    let via = {
        let mut v = state.via_service.lock().unwrap();
        std::mem::take(&mut *v)
    };
    let mut stopped = false;
    if via {
        crate::service::stop_core(app);
        stopped = true;
    }
    let mut guard = state.child.lock().unwrap();
    if let Some(mut child) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
        stopped = true;
    }
    *state.api.lock().unwrap() = None;
    stopped
}

/// Stop the core so its executable can be replaced; the OS proxy is left
/// alone because the frontend restarts the core right away.
pub fn stop_for_upgrade(app: &AppHandle, state: &State<'_, CoreState>) {
    core_stop_inner(app, state);
    // Windows keeps the image locked for a moment after the process ends
    std::thread::sleep(Duration::from_millis(300));
}

#[tauri::command]
pub fn core_stop(app: AppHandle, state: State<'_, CoreState>) -> bool {
    let stopped = core_stop_inner(&app, &state);
    // no core to point at any more — turn the OS proxy off (setting kept,
    // so the next start re-applies it)
    crate::system::clear_if_enabled(&app);
    stopped
}

#[tauri::command]
pub fn core_status(state: State<'_, CoreState>) -> CoreStatus {
    let running = {
        let mut guard = state.child.lock().unwrap();
        match guard.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    };
    CoreStatus {
        running,
        api: if running {
            state.api.lock().unwrap().clone()
        } else {
            None
        },
    }
}

/// Kill the core when the app exits so no orphan process is left behind.
/// A service-managed core is stopped too: with the GUI gone nobody would
/// manage the TUN takeover.
pub fn kill_on_exit(app: &AppHandle) {
    let state = app.state::<CoreState>();
    if std::mem::take(&mut *state.via_service.lock().unwrap()) {
        crate::service::stop_core(app);
    }
    let mut guard = state.child.lock().unwrap();
    if let Some(mut child) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}
