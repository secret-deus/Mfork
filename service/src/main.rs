//! Mfork elevated service.
//!
//! Runs with root/SYSTEM privilege and starts the mihomo core so TUN mode
//! can create its virtual adapter without the GUI ever being elevated.
//! The GUI talks to it over a JSON-line protocol on 127.0.0.1:33217,
//! authenticated by a token file that only administrators can write.
//!
//! Platform backends:
//!   Windows — SCM service running as SYSTEM (`win` module)
//!   macOS   — launchd daemon in /Library/LaunchDaemons (`unix` module)
//!   Linux   — systemd unit in /etc/systemd/system (`unix` module)
//!
//! Security model: the core binary path is pinned at install time (stored
//! next to the service binary, admin-writable only). Requests can only
//! choose the config file and data dir, never the executable.
//!
//! Subcommands (run elevated):
//!   mfork-service install <core-exe-path>
//!   mfork-service uninstall
//!   mfork-service serve            (foreground; what launchd/systemd run)
//! No args on Windows: service entry, invoked by the SCM.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub const PORT: u16 = 33217;
pub const SERVICE_NAME: &str = "MforkService";

#[derive(Deserialize)]
struct Req {
    token: String,
    cmd: String,
    config: Option<String>,
    dir: Option<String>,
}

#[derive(Serialize, Default)]
struct Resp {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    msg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    core_running: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn token_path() -> PathBuf {
    exe_dir().join("service.token")
}

fn core_path_file() -> PathBuf {
    exe_dir().join("core-path.txt")
}

static CHILD: Mutex<Option<Child>> = Mutex::new(None);
static STOPPING: AtomicBool = AtomicBool::new(false);

fn stop_core() {
    if let Some(mut c) = CHILD.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

fn handle(req: &Req, token: &str, core_bin: &str) -> Resp {
    if req.token != token {
        return Resp { ok: false, msg: Some("bad token".into()), ..Default::default() };
    }
    match req.cmd.as_str() {
        "status" => {
            let mut guard = CHILD.lock().unwrap();
            let (running, pid) = match guard.as_mut() {
                Some(c) => match c.try_wait() {
                    Ok(None) => (true, Some(c.id())),
                    _ => {
                        *guard = None;
                        (false, None)
                    }
                },
                None => (false, None),
            };
            Resp { ok: true, core_running: Some(running), pid, ..Default::default() }
        }
        "start" => {
            let (Some(config), Some(dir)) = (&req.config, &req.dir) else {
                return Resp { ok: false, msg: Some("missing config/dir".into()), ..Default::default() };
            };
            stop_core();
            let out = |name: &str| {
                std::fs::File::create(PathBuf::from(dir).join(name))
                    .map(std::process::Stdio::from)
                    .unwrap_or_else(|_| std::process::Stdio::null())
            };
            match Command::new(core_bin)
                .arg("-d")
                .arg(dir)
                .arg("-f")
                .arg(config)
                .stdout(out("core.log"))
                .stderr(out("core.err.log"))
                .spawn()
            {
                Ok(child) => {
                    let pid = child.id();
                    *CHILD.lock().unwrap() = Some(child);
                    Resp { ok: true, pid: Some(pid), ..Default::default() }
                }
                Err(e) => Resp { ok: false, msg: Some(format!("spawn failed: {e}")), ..Default::default() },
            }
        }
        "stop" => {
            stop_core();
            Resp { ok: true, ..Default::default() }
        }
        "version" => Resp { ok: true, msg: Some(env!("CARGO_PKG_VERSION").into()), ..Default::default() },
        _ => Resp { ok: false, msg: Some("unknown cmd".into()), ..Default::default() },
    }
}

/// JSON-line server loop; exits when STOPPING is set.
fn serve() {
    let token = std::fs::read_to_string(token_path()).unwrap_or_default().trim().to_string();
    let core_bin = std::fs::read_to_string(core_path_file()).unwrap_or_default().trim().to_string();
    let listener = match TcpListener::bind(("127.0.0.1", PORT)) {
        Ok(l) => l,
        Err(_) => return,
    };
    let _ = listener.set_nonblocking(true);
    loop {
        if STOPPING.load(Ordering::SeqCst) {
            break;
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                let mut line = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                if reader.read_line(&mut line).is_ok() {
                    let resp = match serde_json::from_str::<Req>(&line) {
                        Ok(req) => handle(&req, &token, &core_bin),
                        Err(_) => Resp { ok: false, msg: Some("bad request".into()), ..Default::default() },
                    };
                    let _ = writeln!(stream, "{}", serde_json::to_string(&resp).unwrap());
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(120));
            }
            Err(_) => break,
        }
    }
    stop_core();
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::ffi::OsString;
    use windows_service::service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    use windows_service::{define_windows_service, service_dispatcher};

    define_windows_service!(ffi_service_main, service_main);

    fn service_main(_args: Vec<OsString>) {
        let handler = move |control: ServiceControl| -> ServiceControlHandlerResult {
            match control {
                ServiceControl::Stop | ServiceControl::Shutdown => {
                    STOPPING.store(true, Ordering::SeqCst);
                    ServiceControlHandlerResult::NoError
                }
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                _ => ServiceControlHandlerResult::NotImplemented,
            }
        };
        let Ok(status_handle) = service_control_handler::register(SERVICE_NAME, handler) else {
            return;
        };
        let set = |state: ServiceState, accept: ServiceControlAccept| {
            let _ = status_handle.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: state,
                controls_accepted: accept,
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: std::time::Duration::from_secs(5),
                process_id: None,
            });
        };
        set(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN);
        serve();
        set(ServiceState::Stopped, ServiceControlAccept::empty());
    }

    pub fn run() -> Result<(), String> {
        service_dispatcher::start(SERVICE_NAME, ffi_service_main).map_err(|e| e.to_string())
    }

    pub fn install(core_exe: &str) -> Result<(), String> {
        let core = PathBuf::from(core_exe);
        if !core.exists() {
            return Err(format!("core binary not found: {core_exe}"));
        }
        // pin the core path and mint a fresh token (dir is admin-writable)
        std::fs::write(core_path_file(), core_exe).map_err(|e| format!("write core path: {e}"))?;
        let token: String = (0..32)
            .map(|_| char::from_digit(fastrand::u32(0..16), 16).unwrap())
            .collect();
        std::fs::write(token_path(), &token).map_err(|e| format!("write token: {e}"))?;

        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE)
            .map_err(|e| format!("open SCM (需要管理员权限): {e}"))?;
        let info = ServiceInfo {
            name: OsString::from(SERVICE_NAME),
            display_name: OsString::from("Mfork Service"),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: std::env::current_exe().map_err(|e| e.to_string())?,
            launch_arguments: vec![],
            dependencies: vec![],
            account_name: None, // LocalSystem
            account_password: None,
        };
        let service = match manager.create_service(&info, ServiceAccess::START) {
            Ok(s) => s,
            Err(windows_service::Error::Winapi(e))
                if e.raw_os_error() == Some(1073) => // ERROR_SERVICE_EXISTS
            {
                // re-install (e.g. after a core upgrade): the running
                // instance holds the old token + core path, restart it
                let s = manager
                    .open_service(
                        SERVICE_NAME,
                        ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
                    )
                    .map_err(|e| e.to_string())?;
                let _ = s.stop();
                for _ in 0..50 {
                    match s.query_status() {
                        Ok(st) if st.current_state == ServiceState::Stopped => break,
                        _ => std::thread::sleep(std::time::Duration::from_millis(100)),
                    }
                }
                s
            }
            Err(e) => return Err(format!("create service: {e}")),
        };
        service
            .start(&[] as &[&std::ffi::OsStr])
            .map_err(|e| format!("start service: {e}"))?;
        println!("installed and started");
        Ok(())
    }

    pub fn uninstall() -> Result<(), String> {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .map_err(|e| format!("open SCM (需要管理员权限): {e}"))?;
        let service = manager
            .open_service(
                SERVICE_NAME,
                ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
            )
            .map_err(|e| format!("open service: {e}"))?;
        let _ = service.stop();
        service.delete().map_err(|e| format!("delete service: {e}"))?;
        let _ = std::fs::remove_file(token_path());
        let _ = std::fs::remove_file(core_path_file());
        println!("uninstalled");
        Ok(())
    }
}

#[cfg(windows)]
fn main() {
    let arg = std::env::args().nth(1);
    let result = match arg.as_deref() {
        Some("install") => {
            let core = std::env::args().nth(2).unwrap_or_default();
            win::install(&core)
        }
        Some("uninstall") => win::uninstall(),
        Some("serve") => {
            // console mode for debugging (no SCM)
            serve();
            Ok(())
        }
        _ => win::run(),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(unix)]
mod unix {
    use super::*;

    /// Where the daemon binary, token and pinned core path live
    /// (root-owned, world-readable so the GUI can read the token).
    pub fn install_dir() -> PathBuf {
        if cfg!(target_os = "macos") {
            PathBuf::from("/Library/Application Support/Mfork")
        } else {
            PathBuf::from("/usr/local/lib/mfork")
        }
    }

    const LABEL: &str = "app.mfork.service";

    fn plist_path() -> PathBuf {
        PathBuf::from(format!("/Library/LaunchDaemons/{LABEL}.plist"))
    }

    fn unit_path() -> PathBuf {
        PathBuf::from("/etc/systemd/system/mfork-service.service")
    }

    fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
        let out = Command::new(cmd)
            .args(args)
            .output()
            .map_err(|e| format!("{cmd}: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "{cmd} {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }

    fn write_mode(path: &PathBuf, data: &str, mode: u32) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, data).map_err(|e| format!("write {}: {e}", path.display()))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|e| e.to_string())
    }

    pub fn install(core_exe: &str) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        let core = PathBuf::from(core_exe);
        if !core.exists() {
            return Err(format!("core binary not found: {core_exe}"));
        }
        let dir = install_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;

        // copy ourselves into the root-owned dir so the daemon does not
        // depend on where the app bundle lives
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        let target = dir.join("mfork-service");
        // stop a previous instance first: the binary may be in use
        stop_daemon();
        if me != target {
            std::fs::copy(&me, &target).map_err(|e| format!("copy service: {e}"))?;
        }
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;

        // pin the core path and mint a fresh token
        write_mode(&dir.join("core-path.txt"), core_exe, 0o644)?;
        let token: String = (0..32)
            .map(|_| char::from_digit(fastrand::u32(0..16), 16).unwrap())
            .collect();
        write_mode(&dir.join("service.token"), &token, 0o644)?;

        let exe = target.to_string_lossy().to_string();
        if cfg!(target_os = "macos") {
            let plist = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array><string>{exe}</string><string>serve</string></array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
    <key>StandardOutPath</key><string>/tmp/mfork-service.log</string>
    <key>StandardErrorPath</key><string>/tmp/mfork-service.log</string>
</dict>
</plist>
"#
            );
            write_mode(&plist_path(), &plist, 0o644)?;
            run(
                "launchctl",
                &["bootstrap", "system", &plist_path().to_string_lossy()],
            )?;
        } else {
            let unit = format!(
                "[Unit]\nDescription=Mfork elevated service (mihomo core for TUN)\nAfter=network.target\n\n\
                 [Service]\nExecStart={exe} serve\nRestart=on-failure\nRestartSec=2\n\n\
                 [Install]\nWantedBy=multi-user.target\n"
            );
            write_mode(&unit_path(), &unit, 0o644)?;
            run("systemctl", &["daemon-reload"])?;
            run("systemctl", &["enable", "--now", "mfork-service.service"])?;
        }
        println!("installed and started");
        Ok(())
    }

    fn stop_daemon() {
        if cfg!(target_os = "macos") {
            let _ = run("launchctl", &["bootout", &format!("system/{LABEL}")]);
        } else {
            let _ = run("systemctl", &["disable", "--now", "mfork-service.service"]);
        }
        // give the old process a moment to release the port
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    pub fn uninstall() -> Result<(), String> {
        stop_daemon();
        if cfg!(target_os = "macos") {
            let _ = std::fs::remove_file(plist_path());
        } else {
            let _ = std::fs::remove_file(unit_path());
            let _ = run("systemctl", &["daemon-reload"]);
        }
        let _ = std::fs::remove_dir_all(install_dir());
        println!("uninstalled");
        Ok(())
    }

    /// launchd/systemd stop the daemon with SIGTERM: turn that into a
    /// clean shutdown so the core child is killed too.
    pub fn install_signal_handlers() {
        extern "C" fn on_term(_: libc::c_int) {
            STOPPING.store(true, Ordering::SeqCst);
        }
        let h = on_term as extern "C" fn(libc::c_int) as libc::sighandler_t;
        unsafe {
            libc::signal(libc::SIGTERM, h);
            libc::signal(libc::SIGINT, h);
        }
    }
}

#[cfg(unix)]
fn main() {
    let arg = std::env::args().nth(1);
    let result = match arg.as_deref() {
        Some("install") => {
            let core = std::env::args().nth(2).unwrap_or_default();
            unix::install(&core)
        }
        Some("uninstall") => unix::uninstall(),
        _ => {
            unix::install_signal_handlers();
            serve();
            Ok(())
        }
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
