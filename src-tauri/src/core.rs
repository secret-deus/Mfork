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
    /// Android: fd of the VPN service's TUN interface while the core uses it
    pub tun_fd: Mutex<Option<i32>>,
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

/// Android: copy the geo databases packaged in the APK into the core's
/// working dir when they are missing (or a new app version shipped bigger
/// ones). Elsewhere the core downloads them itself.
fn seed_geodata(dir: &PathBuf) {
    #[cfg(target_os = "android")]
    {
        let Ok(lib) = native_lib_dir() else { return };
        for (packaged, name) in [("libgeoip_metadb.so", "geoip.metadb"), ("libgeosite_dat.so", "geosite.dat")] {
            let src = lib.join(packaged);
            let dst = dir.join(name);
            let Ok(sm) = std::fs::metadata(&src) else { continue };
            let same = std::fs::metadata(&dst).map(|d| d.len() == sm.len()).unwrap_or(false);
            if !same {
                let _ = std::fs::copy(&src, &dst);
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = dir;
    }
}

/// Tail of the core's stdout / stderr files, for the UI's error state.
#[tauri::command]
pub fn core_log_tail(app: AppHandle, lines: usize) -> Result<String, String> {
    let dir = data_dir(&app)?;
    let mut out = String::new();
    for name in ["core.err.log", "core.log"] {
        let text = std::fs::read_to_string(dir.join(name)).unwrap_or_default();
        let tail: Vec<&str> = text.lines().rev().take(lines).collect();
        if tail.is_empty() {
            continue;
        }
        out.push_str(&format!("── {name}\n"));
        for l in tail.iter().rev() {
            out.push_str(l);
            out.push('\n');
        }
    }
    Ok(out)
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

/// Android: system VPN consent dialog (no-op elsewhere). Blocking work
/// runs off the main thread so the Kotlin side can drive the activity.
#[tauri::command]
pub async fn vpn_prepare(app: AppHandle) -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        tauri::async_runtime::spawn_blocking(move || crate::vpn::prepare(&app))
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(true)
    }
}

#[derive(Serialize)]
pub struct DiagItem {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

fn timed(name: &str, f: impl FnOnce() -> Result<String, String>) -> DiagItem {
    let t0 = Instant::now();
    match f() {
        Ok(d) => DiagItem {
            name: name.into(),
            ok: true,
            detail: format!("{d} · {} ms", t0.elapsed().as_millis()),
        },
        Err(e) => DiagItem { name: name.into(), ok: false, detail: e },
    }
}

/// Probe the network from both sides of the tunnel: the app process (which
/// Android keeps outside the VPN) and the core. Tells "connected but no
/// traffic" cases apart: app cannot reach the net at all (OEM data
/// restriction), app sockets looping back into our own TUN, core DNS dead,
/// or only the proxied path failing.
fn net_diagnose_inner(app: &AppHandle) -> Vec<DiagItem> {
    const PROBE_URL: &str = "http://connect.rom.miui.com/generate_204";
    let api = app.state::<CoreState>().api.lock().unwrap().clone();
    let client = |proxy: Option<String>| -> Result<reqwest::blocking::Client, String> {
        let mut b = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(6))
            .no_proxy();
        if let Some(p) = proxy {
            b = b.proxy(reqwest::Proxy::all(p).map_err(|e| e.to_string())?);
        }
        b.build().map_err(|e| e.to_string())
    };
    let mut out = Vec::new();
    out.push(timed("应用 → 223.5.5.5:443（TCP，不经 DNS）", || {
        let s = TcpStream::connect_timeout(
            &"223.5.5.5:443".parse().unwrap(),
            Duration::from_secs(4),
        )
        .map_err(|e| e.to_string())?;
        let local = s.local_addr().map(|a| a.ip().to_string()).unwrap_or_default();
        if local.starts_with("172.19.0.") || local.starts_with("fdfe:dcba:9876") {
            return Err(format!("本地地址 {local}：连接被路由进了自己的 VPN（回环）"));
        }
        Ok(format!("可达，本地地址 {local}"))
    }));
    out.push(timed("应用系统 DNS → www.baidu.com", || {
        use std::net::ToSocketAddrs;
        let addrs: Vec<_> = ("www.baidu.com", 80)
            .to_socket_addrs()
            .map_err(|e| e.to_string())?
            .collect();
        if addrs.is_empty() {
            return Err("无结果".into());
        }
        Ok(addrs.iter().take(2).map(|a| a.ip().to_string()).collect::<Vec<_>>().join(", "))
    }));
    out.push(timed("应用 HTTP → generate_204", || {
        let r = client(None)?.get(PROBE_URL).send().map_err(|e| e.to_string())?;
        Ok(format!("HTTP {}", r.status().as_u16()))
    }));
    match api {
        None => out.push(DiagItem {
            name: "内核".into(),
            ok: false,
            detail: "内核未运行，跳过内核侧探测".into(),
        }),
        Some(api) => {
            out.push(timed("内核 DNS → www.baidu.com", || {
                let r = client(None)?
                    .get(format!(
                        "http://127.0.0.1:{}/dns/query?name=www.baidu.com&type=A",
                        api.port
                    ))
                    .bearer_auth(&api.secret)
                    .send()
                    .map_err(|e| e.to_string())?;
                let v: serde_json::Value = r.json().map_err(|e| e.to_string())?;
                let n = v.get("Answer").and_then(|a| a.as_array()).map(|a| a.len()).unwrap_or(0);
                if n == 0 {
                    Err(format!("无应答：{v}"))
                } else {
                    Ok(format!("{n} 条记录"))
                }
            }));
            out.push(timed("经内核混合端口 HTTP → generate_204", || {
                let r = client(Some(format!("http://127.0.0.1:{}", api.mixed_port)))?
                    .get(PROBE_URL)
                    .send()
                    .map_err(|e| e.to_string())?;
                Ok(format!("HTTP {}", r.status().as_u16()))
            }));
        }
    }
    out
}

#[tauri::command]
pub async fn net_diagnose(app: AppHandle) -> Result<Vec<DiagItem>, String> {
    tauri::async_runtime::spawn_blocking(move || net_diagnose_inner(&app))
        .await
        .map_err(|e| e.to_string())
}

/// Start (or return the already running) core. The body blocks for up to
/// a minute waiting for the API port, and on Android talks to the VPN
/// plugin, so it runs on the blocking pool rather than the main thread.
#[tauri::command]
pub async fn core_start(app: AppHandle) -> Result<ApiInfo, String> {
    tauri::async_runtime::spawn_blocking(move || core_start_inner(&app))
        .await
        .map_err(|e| e.to_string())?
}

fn core_start_inner(app: &AppHandle) -> Result<ApiInfo, String> {
    let state = app.state::<CoreState>();
    let mut settings = crate::settings::load(app);
    // desktop: TUN needs the elevated service; Android: the VPN service
    let use_service = cfg!(desktop) && settings.tun.enable;
    if use_service && !crate::service::ping(app) {
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

    // Android TUN: the VPN service brings the interface up first and the
    // config points the core at its fd
    #[cfg(target_os = "android")]
    let tun_fd: Option<i32> = if settings.tun.enable {
        Some(crate::vpn::establish(app, settings.tun.mtu, settings.ipv6, "Mfork")?)
    } else {
        None
    };
    #[cfg(not(target_os = "android"))]
    let tun_fd: Option<i32> = None;
    *state.tun_fd.lock().unwrap() = tun_fd;

    let config_path = match crate::profile::generate_config(app, api_port, &secret, mixed_port) {
        Ok(p) => p,
        Err(e) => {
            release_tun(app, &state);
            return Err(e);
        }
    };
    seed_geodata(&dir);
    // first start may still have to fetch a geo file the profile needs
    let first_start = !dir.join("geoip.metadb").exists();

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
        let log_files = (|| -> std::io::Result<_> {
            Ok((std::fs::File::create(dir.join("core.log"))?,
                std::fs::File::create(dir.join("core.err.log"))?))
        })();
        let (stdout, stderr) = match log_files {
            Ok(files) => files,
            Err(e) => {
                release_tun(app, &state);
                return Err(format!("open core logs failed: {e}"));
            }
        };
        let mut cmd = Command::new(&bin);
        cmd.arg("-d")
            .arg(&dir)
            .arg("-f")
            .arg(&config_path)
            .stdout(std::process::Stdio::from(
                stdout,
            ))
            .stderr(std::process::Stdio::from(
                stderr,
            ));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        // Only the core child should inherit the VPN descriptor. Keep the
        // parent's CLOEXEC flag intact and report handoff errors via spawn.
        #[cfg(target_os = "android")]
        if let Some(fd) = tun_fd {
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(move || {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                release_tun(app, &state);
                return Err(format!("spawn mihomo failed: {e}"));
            }
        };
        pid = child.id();
        *child_guard = Some(child);
        *state.via_service.lock().unwrap() = false;
    }
    drop(child_guard);

    // wait until the external controller answers (max ~8s)
    let wait = if first_start { 60 } else { 8 };
    let deadline = Instant::now() + Duration::from_secs(wait);
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
                "mihomo (pid {pid}) did not open the API port within {wait}s — see core.err.log in {}",
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
    #[cfg(target_os = "android")]
    if tun_fd.is_some() {
        crate::vpn::attach_core(app, api_port, &info.secret);
    }

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
                release_tun(&app2, &state);
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
                        release_tun(&app2, &state);
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

/// Android: tear the VPN down once the core no longer uses its fd.
fn release_tun(app: &AppHandle, state: &State<'_, CoreState>) {
    let had = state.tun_fd.lock().unwrap().take().is_some();
    #[cfg(target_os = "android")]
    if had {
        crate::vpn::stop(app);
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, had);
    }
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
    release_tun(app, state);
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
pub async fn core_stop(app: AppHandle) -> bool {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CoreState>();
        let stopped = core_stop_inner(&app, &state);
        // no core to point at any more — turn the OS proxy off (setting kept,
        // so the next start re-applies it)
        crate::system::clear_if_enabled(&app);
        stopped
    })
    .await
    .unwrap_or(false)
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
