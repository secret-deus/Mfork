//! mihomo core process management: spawn, supervise, stop.

use serde::Serialize;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

#[path = "../../shared/core_geodata.rs"]
mod geodata;
#[path = "core_logs.rs"]
mod logs;

#[derive(Clone, Serialize)]
pub struct ApiInfo {
    pub port: u16,
    #[serde(skip)]
    pub secret: String,
    pub id: String,
    pub transport: &'static str,
    #[serde(skip)]
    pub controller: crate::controller::Controller,
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
    /// Keep the last attempted core's actual log directory even after failure.
    pub log_dir: Mutex<Option<PathBuf>>,
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

fn random_secret() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("无法生成安全 API 密钥: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
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

/// Seed from the installed app, even when the executable was self-upgraded.
fn seed_geodata(app: &AppHandle, dir: &PathBuf) -> Result<geodata::Preparation, String> {
    #[cfg(target_os = "android")]
    {
        let _ = app;
        let lib = native_lib_dir()?;
        geodata::prepare_with_bundled(dir, &[
            ("geoip.metadb", lib.join("libgeoip_metadb.so")),
            ("geosite.dat", lib.join("libgeosite_dat.so")),
        ])
    }
    #[cfg(not(target_os = "android"))]
    {
        let bundled = bundled_binary(app)?;
        geodata::prepare_with_bundled(dir, &geodata::bundled_sources(bundled.parent().ok_or("missing resource directory")?))
    }
}

/// Tail of the core's stdout / stderr files, for the UI's error state.
#[tauri::command]
pub fn core_log_tail(app: AppHandle, lines: usize) -> Result<String, String> {
    let last_dir = app.state::<CoreState>().log_dir.lock().unwrap().clone();
    let dir = match last_dir {
        Some(dir) => dir,
        None => data_dir(&app)?,
    };
    Ok(logs::tail(&dir, lines))
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
                let v = api.controller.request("GET", "/dns/query?name=www.baidu.com&type=A", None, Duration::from_secs(6))?;
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
/// two minutes waiting for first-run geodata, and on Android talks to the VPN
/// plugin, so it runs on the blocking pool rather than the main thread.
#[tauri::command]
pub async fn core_start(app: AppHandle) -> Result<ApiInfo, String> {
    tauri::async_runtime::spawn_blocking(move || core_start_inner(&app))
        .await
        .map_err(|e| e.to_string())?
}

fn core_start_inner(app: &AppHandle) -> Result<ApiInfo, String> {
    let state = app.state::<CoreState>();
    let data_guard = crate::settings::data_lock();
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
    let api_port = if crate::controller::USE_UNIX_SOCKET { 0 } else { free_port()? };
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
    let secret = random_secret()?;
    let controller = crate::controller::Controller::new(api_port, &secret)?;
    let info = ApiInfo {
        port: api_port,
        secret: secret.clone(),
        id: random_secret()?,
        transport: controller.transport(),
        controller,
        mixed_port,
        started_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),
    };

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

    let config_path = match crate::profile::generate_config(app, api_port, &secret, mixed_port, &info.controller) {
        Ok(p) => p,
        Err(e) => {
            release_tun(app, &state);
            return Err(e);
        }
    };
    drop(data_guard);

    // clean up whatever ran before, regardless of mode
    if let Some(mut c) = child_guard.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    if crate::service::ping(&app) && crate::service::core_running(&app) {
        crate::service::stop_core(&app);
    }

    let pid;
    let cold_start;
    // where core.err.log ends up: our data dir, or the service's own
    let mut log_dir = dir.clone();
    if use_service {
        let (p, svc_dir, svc_cold_start) = crate::service::start_core(
            &app,
            &config_path.to_string_lossy(),
            &dir.to_string_lossy(),
        )?;
        pid = p;
        cold_start = svc_cold_start;
        if let Some(d) = svc_dir {
            log_dir = PathBuf::from(d);
        }
        *state.via_service.lock().unwrap() = true;
    } else {
        // Prepare only the directory this child will actually use, after any
        // previous child has stopped. Service-owned files are prepared there.
        let preparation = match seed_geodata(&app, &dir) {
            Ok(preparation) => preparation,
            Err(e) => {
                release_tun(app, &state);
                return Err(format!("prepare core geodata failed: {e}"));
            }
        };
        cold_start = preparation.cold_start;
        for path in preparation.quarantined {
            eprintln!("[Mfork] preserved incomplete geodata at {}", path.display());
        }
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
    *state.log_dir.lock().unwrap() = Some(log_dir.clone());

    // Android's system VPN can be up even when the core's TUN listener fails.
    // Retain startup evidence independently of the later controller WebSocket.
    #[cfg(target_os = "android")]
    if let Some(fd) = tun_fd {
        let startup_dir = log_dir.clone();
        let stack = settings.tun.stack.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(2));
            let inherited = std::fs::read_link(format!("/proc/{pid}/fd/{fd}"));
            eprintln!("[MforkTun] pid={pid} fd={fd} stack={stack} inherited={inherited:?}");
            for name in ["core.log", "core.err.log"] {
                if let Ok(text) = std::fs::read_to_string(startup_dir.join(name)) {
                    // Only TUN startup diagnostics: do not publish subscription
                    // contents, API secrets or unrelated connection history.
                    for line in text.lines().filter(|line| line.to_ascii_lowercase().contains("tun" )).take(30) {
                        eprintln!("[MforkTun] {line}");
                    }
                }
            }
        });
    }

    // The service reports its own cache state BEFORE spawning. The GUI's
    // populated cache says nothing about a service's first download.
    let wait = geodata::startup_timeout(cold_start);
    let deadline = Instant::now() + wait;
    let mut next_liveness_check = Instant::now();
    loop {
        let readiness_error = match info.controller.ready() {
            Ok(()) => break,
            Err(error) => error,
        };
        if Instant::now() >= next_liveness_check {
            let running = if use_service {
                crate::service::core_running_status(app)
            } else {
                Some(state.child.lock().unwrap().as_mut()
                    .map(|child| matches!(child.try_wait(), Ok(None)))
                    .unwrap_or(false))
            };
            // Failed IPC is unknown, not proof that a downloading core exited.
            // Keep retrying the API/status probes until the existing deadline.
            if running == Some(false) {
                let _ = core_stop_inner(app, &state);
                return Err(logs::startup_error(pid, "exited before the controller became ready", &log_dir));
            }
            next_liveness_check = Instant::now() + Duration::from_secs(1);
        }
        if Instant::now() > deadline {
            let _ = core_stop_inner(app, &state);
            return Err(logs::startup_error(
                pid,
                &format!("controller not ready within {}s: {readiness_error}", wait.as_secs()),
                &log_dir,
            ));
        }
        std::thread::sleep(Duration::from_millis(150));
    }

    // Controller readiness does not imply TUN readiness. The upstream core
    // keeps serving its API after a listener initialization failure.
    #[cfg(target_os = "android")]
    if tun_fd.is_some() {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let config = info.controller.request("GET", "/configs", None, Duration::from_secs(1));
            if config.ok().and_then(|c| c["tun"]["enable"].as_bool()) == Some(true) {
                break;
            }
            let startup = std::fs::read_to_string(log_dir.join("core.log")).unwrap_or_default();
            let failure = startup.lines().find(|l| l.contains("Start TUN listening error"));
            if failure.is_some() || Instant::now() >= deadline {
                let reason = failure.unwrap_or("TUN listener did not become ready within 10s").to_owned();
                let _ = core_stop_inner(app, &state);
                return Err(format!("VPN startup failed: {reason}"));
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    {
        let mut api = state.api.lock().unwrap();
        app.state::<crate::controller::Streams>().stop_all();
        *api = Some(info.clone());
    }
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
    let session_id = info.id.clone();
    if use_service {
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(2));
            let state = app2.state::<CoreState>();
            if !*state.via_service.lock().unwrap() {
                break; // switched mode or stopped deliberately
            }
            if state.api.lock().unwrap().as_ref().map(|api| &api.id) != Some(&session_id) {
                break;
            }
            if crate::service::core_running_status(&app2) == Some(false) {
                if state.api.lock().unwrap().as_ref().map(|api| &api.id) != Some(&session_id) { break; }
                clear_api(&app2, &state);
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
            if state.api.lock().unwrap().as_ref().map(|api| &api.id) != Some(&session_id) { break; }
            match guard.as_mut() {
                Some(c) => match c.try_wait() {
                    Ok(Some(status)) => {
                        *guard = None;
                        clear_api(&app2, &state);
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

fn clear_api(app: &AppHandle, state: &State<'_, CoreState>) {
    let mut api = state.api.lock().unwrap();
    app.state::<crate::controller::Streams>().stop_all();
    *api = None;
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
    clear_api(app, state);
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

/// Service-managed cores have no Child handle in the GUI. Do not mistake a
/// transient helper IPC failure for an exit and discard a live session.
#[tauri::command]
pub async fn core_status(app: AppHandle) -> Result<CoreStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CoreState>();
        let via_service = *state.via_service.lock().unwrap();
        let running = if via_service {
            crate::service::core_running_status(&app)
                .unwrap_or_else(|| state.api.lock().unwrap().is_some())
        } else {
            let mut guard = state.child.lock().unwrap();
            guard.as_mut().map(|child| matches!(child.try_wait(), Ok(None))).unwrap_or(false)
        };
        CoreStatus {
            running,
            api: if running { state.api.lock().unwrap().clone() } else { None },
        }
    }).await.map_err(|e| e.to_string())
}

/// Kill the core when the app exits so no orphan process is left behind.
/// A service-managed core is stopped too: with the GUI gone nobody would
/// manage the TUN takeover.
pub fn kill_on_exit(app: &AppHandle) {
    let state = app.state::<CoreState>();
    clear_api(app, &state);
    if std::mem::take(&mut *state.via_service.lock().unwrap()) {
        crate::service::stop_core(app);
    }
    let mut guard = state.child.lock().unwrap();
    if let Some(mut child) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}
