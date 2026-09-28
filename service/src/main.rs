//! Mfork elevated service.
//!
//! Runs with root/SYSTEM privilege and starts the mihomo core so TUN mode
//! can create its virtual adapter without the GUI ever being elevated.
//! The GUI uses bounded, mutually authenticated v2 frames on 127.0.0.1:33217.
//! Legacy plaintext-token requests are never accepted.
//!
//! Platform backends:
//!   Windows — SCM service running as SYSTEM (`win` module)
//!   macOS   — launchd daemon in /Library/LaunchDaemons (`unix` module)
//!   Linux   — systemd unit in /etc/systemd/system (`unix` module)
//!
//! Security model: at install time the core binary is *copied* into the
//! service's own directory (admin/root-writable only) and that copy is
//! pinned; the service never executes anything from a user-writable path.
//! Requests can only choose the config file, never the executable; the
//! data dir (and the core's log files) live under the service's own
//! directory, so nothing is ever created with root/SYSTEM privilege at a
//! caller-chosen path. On Unix the token file is owned by the installing user with
//! mode 0600 so other local users cannot drive the daemon.
//!
//! Subcommands (run elevated):
//!   mfork-service install <core-exe-path> [--owner <uid>]
//!   mfork-service uninstall
//!   mfork-service serve            (foreground; what launchd/systemd run)
//! No args on Windows: service entry, invoked by the SCM.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

/// Shared geodata readiness check: quarantine a truncated MMDB, decide cold vs
/// warm start. Only used by the "start" flow below.
#[path = "../../shared/core_geodata.rs"]
#[allow(dead_code)]
mod geodata;
#[path = "../../shared/service_protocol.rs"]
#[allow(dead_code)]
mod protocol;
#[cfg(windows)]
#[path = "../../shared/service_windows.rs"]
mod security;

pub const PORT: u16 = 33217;
pub const SERVICE_NAME: &str = "MforkService";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Req {
    cmd: String,
    config: Option<String>,
    /// accepted for protocol compatibility, ignored (see "start")
    #[allow(dead_code)]
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
    /// data dir the core was started with (holds core.log / core.err.log)
    #[serde(skip_serializing_if = "Option::is_none")]
    dir: Option<String>,
    /// geodata readiness observed before spawning; `Some(true)` means the core
    /// still has to fetch/download a geo database and needs the long budget
    #[serde(skip_serializing_if = "Option::is_none")]
    cold_start: Option<bool>,
}

fn install_dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        security::install_root()
    }
    #[cfg(unix)]
    {
        Ok(unix::install_dir())
    }
}
fn reject_link(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        security::reject_reparse(path)
    }
    #[cfg(unix)]
    {
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.file_type().is_symlink() => {
                Err(format!("refusing symlink: {}", path.display()))
            }
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}
fn read_small(path: &Path, limit: u64) -> Result<String, String> {
    reject_link(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("service metadata too large".into());
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}
fn data_dir() -> Result<PathBuf, String> {
    let root = install_dir()?;
    #[cfg(windows)]
    {
        security::prepare_subdir(&root, "data")
    }
    #[cfg(unix)]
    {
        Ok(root.join("data"))
    }
}

fn install_geodata_sources(core_exe: &str) -> Result<Vec<(&'static str, PathBuf)>, String> {
    let args: Vec<String> = std::env::args().collect();
    let source = match args.iter().position(|arg| arg == "--geodata-dir") {
        Some(i) => PathBuf::from(args.get(i + 1).ok_or("missing --geodata-dir value")?),
        None => Path::new(core_exe).parent().ok_or("missing core directory")?.to_path_buf(),
    };
    Ok(geodata::bundled_sources(&source))
}

/// Copy the core binary (and, on Windows, wintun.dll beside it) into
/// `<dir>/core/` and return the path of the copy. The copy is what gets
/// pinned, so replacing the user-writable original does nothing.
fn pin_core(dir: &std::path::Path, core_exe: &str) -> Result<PathBuf, String> {
    let src = PathBuf::from(core_exe);
    if !src.is_file() {
        return Err(format!("core binary not found: {core_exe}"));
    }
    let name = if cfg!(windows) {
        "mihomo.exe"
    } else {
        "mihomo"
    };
    #[cfg(windows)]
    security::prepare_subdir(dir, "core")?;
    let core_dir = dir.join("core");
    reject_link(&core_dir)?;
    std::fs::create_dir_all(&core_dir)
        .map_err(|e| format!("create {}: {e}", core_dir.display()))?;
    #[cfg(unix)]
    unix::validate_dir(&core_dir)?;
    let dest = core_dir.join(&name);
    reject_link(&dest)?;
    // write to a temp name then rename so a half-written copy is never pinned
    let tmp = core_dir.join(format!("{name}.new"));
    reject_link(&tmp)?;
    std::fs::copy(&src, &tmp).map_err(|e| format!("copy core: {e}"))?;
    if dest.exists() {
        let _ = std::fs::remove_file(&dest);
    }
    std::fs::rename(&tmp, &dest).map_err(|e| format!("install core: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    #[cfg(windows)]
    {
        if let Some(dll) = src.parent().map(|p| p.join("wintun.dll")) {
            if dll.is_file() {
                let target = core_dir.join("wintun.dll");
                reject_link(&target)?;
                std::fs::copy(&dll, target).map_err(|e| e.to_string())?;
            }
        }
    }
    // Only the elevated installer accepts a resource source path. At runtime
    // the daemon reads exclusively from its protected copy beside the core.
    geodata::prepare_with_bundled(&core_dir, &install_geodata_sources(core_exe)?)?;
    Ok(dest)
}

/// `--owner <uid>` from the install arguments.
#[cfg(unix)]
fn owner_arg() -> Option<u32> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--owner")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
}

static CHILD: Mutex<Option<Child>> = Mutex::new(None);
static STOPPING: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static OPERATIONS: Mutex<()> = Mutex::new(());
struct Connection;
impl Drop for Connection {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Connection {
    fn acquire() -> Option<Self> {
        ACTIVE
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < 16).then_some(n + 1)
            })
            .ok()
            .map(|_| Self)
    }
}

fn stop_core() {
    if let Some(mut c) = CHILD.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

fn handle(req: &Req, core_bin: &str) -> Resp {
    let Ok(_operation) = OPERATIONS.try_lock() else {
        return Resp {
            ok: false,
            msg: Some("service busy; retry".into()),
            ..Default::default()
        };
    };
    if STOPPING.load(Ordering::SeqCst) {
        return Resp {
            ok: false,
            msg: Some("service stopping".into()),
            ..Default::default()
        };
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
            Resp {
                ok: true,
                core_running: Some(running),
                pid,
                ..Default::default()
            }
        }
        "start" => {
            // `dir` in the request is accepted for protocol compatibility
            // and ignored: the caller does not get to pick where root writes
            let Some(config) = &req.config else {
                return Resp {
                    ok: false,
                    msg: Some("missing config".into()),
                    ..Default::default()
                };
            };
            let cfg = PathBuf::from(config);
            if !cfg.is_absolute() || !cfg.is_file() {
                return Resp {
                    ok: false,
                    msg: Some("config must be an absolute path to a file".into()),
                    ..Default::default()
                };
            }
            let dir = match data_dir() {
                Ok(d) => d,
                Err(e) => {
                    return Resp {
                        ok: false,
                        msg: Some(e),
                        ..Default::default()
                    }
                }
            };
            if let Err(e) = reject_link(&dir)
                .and_then(|_| reject_link(&dir.join("core.log")))
                .and_then(|_| reject_link(&dir.join("core.err.log")))
            {
                return Resp {
                    ok: false,
                    msg: Some(e),
                    ..Default::default()
                };
            }
            if let Err(e) = std::fs::create_dir_all(&dir) {
                return Resp {
                    ok: false,
                    msg: Some(format!("create data dir: {e}")),
                    ..Default::default()
                };
            }
            stop_core();
            // Geodata readiness, before any log file is created and before the
            // core is spawned: a truncated MMDB (killed mid-download) is moved
            // aside within this protected data dir, and the GUI is told whether
            // the next start has to fetch/download geodata (long budget).
            let resources = Path::new(core_bin).parent().unwrap_or(Path::new(""));
            let preparation = match geodata::prepare_with_bundled(&dir, &geodata::bundled_sources(resources)) {
                Ok(p) => p,
                Err(e) => {
                    return Resp {
                        ok: false,
                        msg: Some(format!("geodata preparation failed: {e}")),
                        ..Default::default()
                    }
                }
            };
            if !preparation.quarantined.is_empty() {
                // names only, no file contents and no secrets
                eprintln!(
                    "geodata: quarantined {} incomplete file(s): {}",
                    preparation.quarantined.len(),
                    preparation
                        .quarantined
                        .iter()
                        .map(|p| p
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            let out = |name: &str| {
                std::fs::File::create(dir.join(name))
                    .map(std::process::Stdio::from)
                    .unwrap_or_else(|_| std::process::Stdio::null())
            };
            match Command::new(core_bin)
                .arg("-d")
                .arg(&dir)
                .arg("-f")
                .arg(&cfg)
                .stdout(out("core.log"))
                .stderr(out("core.err.log"))
                .spawn()
            {
                Ok(child) => {
                    let pid = child.id();
                    *CHILD.lock().unwrap() = Some(child);
                    Resp {
                        ok: true,
                        pid: Some(pid),
                        dir: Some(dir.to_string_lossy().to_string()),
                        cold_start: Some(preparation.cold_start),
                        ..Default::default()
                    }
                }
                Err(e) => Resp {
                    ok: false,
                    msg: Some(format!("spawn failed: {e}")),
                    ..Default::default()
                },
            }
        }
        "stop" => {
            stop_core();
            Resp {
                ok: true,
                ..Default::default()
            }
        }
        "version" => Resp {
            ok: true,
            msg: Some(env!("CARGO_PKG_VERSION").into()),
            ..Default::default()
        },
        _ => Resp {
            ok: false,
            msg: Some("unknown cmd".into()),
            ..Default::default()
        },
    }
}

/// Nonblocking accept loop, at most 16 bounded exchanges; commands serialize separately.
fn serve() {
    let result = (|| -> Result<(), String> {
        let root = install_dir()?;
        #[cfg(windows)]
        security::validate_root(&root)?;
        #[cfg(windows)]
        {
            security::validate_token(&root)?;
            let me = std::env::current_exe().map_err(|e| e.to_string())?;
            if me != root.join("mfork-service.exe") {
                return Err("service must execute from the protected installation".into());
            }
            security::validate_image(&root, &me)?;
            security::prepare_subdir(&root, "core")?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            unix::validate_dir(&root)?;
            let expected: u32 = read_small(&root.join("owner.uid"), 32)?
                .parse()
                .map_err(|_| "invalid service owner uid")?;
            let meta =
                std::fs::symlink_metadata(root.join("service.token")).map_err(|e| e.to_string())?;
            if !meta.is_file() || meta.uid() != expected || meta.mode() & 0o077 != 0 {
                return Err("unsafe service token permissions; reinstall required".into());
            }
        }
        let key = protocol::parse_key(read_small(&root.join("service.token"), 64)?.trim())?;
        // Never trust an executable path from disk metadata or the caller.
        let core = root.join("core").join(if cfg!(windows) {
            "mihomo.exe"
        } else {
            "mihomo"
        });
        reject_link(&root.join("core"))?;
        reject_link(&core)?;
        if !core.is_file() {
            return Err("pinned core missing; reinstall the service".into());
        }
        #[cfg(windows)]
        {
            security::validate_image(&root, &core)?;
            let dll = root.join("core/wintun.dll");
            if dll.exists() {
                security::validate_image(&root, &dll)?;
            }
        }
        let core_bin = core.to_string_lossy().to_string();
        let listener = TcpListener::bind(("127.0.0.1", PORT)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        while !STOPPING.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let Some(permit) = Connection::acquire() else {
                        continue;
                    };
                    let core_bin = core_bin.clone();
                    // Dropping a failed spawn's closure also releases the permit.
                    let _ = std::thread::Builder::new()
                        .name("service-ipc".into())
                        .spawn(move || {
                            let _permit = permit;
                            let _ = stream.set_nonblocking(false);
                            let _ = protocol::server_call(&mut stream, &key, |req: Req| {
                                handle(&req, &core_bin)
                            });
                        });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(20))
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("service refused to start: {e}");
    }
    STOPPING.store(true, Ordering::SeqCst);
    let _operation = OPERATIONS.lock().unwrap();
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
        set(
            ServiceState::Running,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        );
        serve();
        set(ServiceState::Stopped, ServiceControlAccept::empty());
    }

    pub fn run() -> Result<(), String> {
        service_dispatcher::start(SERVICE_NAME, ffi_service_main).map_err(|e| e.to_string())
    }

    fn stop_and_wait(service: &windows_service::service::Service) -> Result<(), String> {
        if service
            .query_status()
            .map_err(|e| e.to_string())?
            .current_state
            != ServiceState::Stopped
        {
            service
                .stop()
                .map_err(|e| format!("stop old service: {e}"))?;
        }
        for _ in 0..100 {
            if service
                .query_status()
                .map_err(|e| e.to_string())?
                .current_state
                == ServiceState::Stopped
            {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err("old service did not stop; refusing to replace or restart it".into())
    }

    pub fn install(core_exe: &str) -> Result<(), String> {
        use std::os::windows::fs::OpenOptionsExt;
        // Reject obvious bad repin inputs before disabling a working installation.
        let source = Path::new(core_exe);
        if !source.is_absolute() || !source.is_file() {
            return Err("core must be an absolute path to an existing file".into());
        }
        let _source_check =
            std::fs::File::open(source).map_err(|e| format!("read core source: {e}"))?;
        geodata::validate_bundled(&install_geodata_sources(core_exe)?)?;
        let args: Vec<String> = std::env::args().collect();
        let owner = args
            .iter()
            .position(|a| a == "--owner-sid")
            .and_then(|i| args.get(i + 1))
            .ok_or("missing --owner-sid; install from the GUI")?;
        let root = security::prepare_root(owner)?;
        let lock = root.join("install.lock");
        reject_link(&lock)?;
        let _lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(lock)
            .map_err(|e| format!("another installer is running: {e}"))?;
        let owner_file = root.join("owner.sid");
        if owner_file.exists() && read_small(&owner_file, 256)?.trim() != owner {
            return Err("service belongs to another user; administrator must remove the protected installation before transferring it".into());
        }
        let manager = ServiceManager::local_computer(
            None::<&str>,
            ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
        )
        .map_err(|e| format!("open SCM (需要管理员权限): {e}"))?;
        let target = root.join("mfork-service.exe");
        let mut info = ServiceInfo {
            name: OsString::from(SERVICE_NAME),
            display_name: OsString::from("Mfork Service"),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::Disabled,
            error_control: ServiceErrorControl::Normal,
            executable_path: target.clone(),
            launch_arguments: vec![],
            dependencies: vec![],
            account_name: None,
            account_password: None,
        };
        let access = ServiceAccess::START
            | ServiceAccess::STOP
            | ServiceAccess::QUERY_STATUS
            | ServiceAccess::CHANGE_CONFIG;
        let service = match manager.open_service(SERVICE_NAME, access) {
            Ok(s) => {
                // Set a protected image path AND disable auto-start before touching the old process.
                // Never restart an existing service using its legacy SCM image path.
                s.change_config(&info)
                    .map_err(|e| format!("disable/migrate old service: {e}"))?;
                stop_and_wait(&s)?;
                s
            }
            Err(windows_service::Error::Winapi(e)) if e.raw_os_error() == Some(1060) => manager
                .create_service(&info, access)
                .map_err(|e| e.to_string())?,
            Err(e) => return Err(e.to_string()),
        };
        reject_link(&target)?;
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        if me != target {
            let tmp = root.join("mfork-service.exe.new");
            reject_link(&tmp)?;
            std::fs::copy(me, &tmp).map_err(|e| e.to_string())?;
            if target.exists() {
                std::fs::remove_file(&target).map_err(|e| e.to_string())?;
            }
            std::fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
        }
        security::restrict_image(&target, owner)?;
        let core = pin_core(&root, core_exe)?;
        security::restrict_image(&core, owner)?;
        for name in geodata::BUNDLED_NAMES {
            security::restrict_image(&root.join("core").join(name), owner)?;
        }
        let dll = root.join("core/wintun.dll");
        if dll.exists() {
            security::restrict_image(&dll, owner)?;
        }
        security::prepare_subdir(&root, "data")?;
        reject_link(&owner_file)?;
        std::fs::write(owner_file, owner).map_err(|e| e.to_string())?;
        let token = root.join("service.token");
        reject_link(&token)?;
        // Exclusive temp + explicit ACL before the first secret byte is written.
        let temp = root.join(format!("token-{}.tmp", protocol::random_hex()?));
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let written = (|| -> Result<(), String> {
            security::restrict_file(&temp, owner)?;
            f.write_all(protocol::random_hex()?.as_bytes())
                .map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())
        })();
        drop(f);
        if let Err(e) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(e);
        }
        if let Err(e) = security::replace_file(&temp, &token) {
            let _ = std::fs::remove_file(&temp);
            return Err(e);
        }
        security::validate_root(&root)?;
        info.start_type = ServiceStartType::AutoStart;
        service.change_config(&info).map_err(|e| e.to_string())?;
        service
            .start(&[] as &[&std::ffi::OsStr])
            .map_err(|e| format!("start service: {e}"))?;
        println!("installed v2 service in protected directory");
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
        stop_and_wait(&service)?;
        service
            .delete()
            .map_err(|e| format!("delete service: {e}"))?;
        let root = security::install_root()?;
        if root.exists() {
            security::validate_root(&root)?;
            let _ = std::fs::remove_file(root.join("service.token"));
        }
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

    /// Where the daemon binary, pinned core copy, token and core path live
    /// (root-owned; the token is owned by the installing user, mode 0600).
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

    pub fn validate_dir(dir: &Path) -> Result<(), String> {
        use std::os::unix::fs::MetadataExt;
        for path in dir.ancestors() {
            reject_link(path)?;
            let m = std::fs::metadata(path).map_err(|e| e.to_string())?;
            if !m.is_dir() || m.uid() != 0 || m.mode() & 0o022 != 0 {
                return Err(format!("untrusted service directory: {}", path.display()));
            }
        }
        Ok(())
    }
    fn write_mode(path: &PathBuf, data: &str, mode: u32) -> Result<(), String> {
        use std::os::unix::fs::OpenOptionsExt;
        reject_link(path)?;
        let temp = path.with_extension(format!("{}.tmp", protocol::random_hex()?));
        let result = (|| -> Result<(), String> {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(mode)
                .open(&temp)
                .map_err(|e| e.to_string())?;
            f.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
            std::fs::rename(&temp, path).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result
    }

    pub fn install(core_exe: &str) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        let dir = install_dir();
        if unsafe { libc::geteuid() } != 0 {
            return Err("installation requires root".into());
        }
        let uid = owner_arg().ok_or("missing --owner uid")?;
        geodata::validate_bundled(&install_geodata_sources(core_exe)?)?;
        validate_dir(dir.parent().ok_or("invalid service directory")?)?;
        if !dir.exists() {
            std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
        }
        validate_dir(&dir)?;

        // copy ourselves into the root-owned dir so the daemon does not
        // depend on where the app bundle lives
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        let target = dir.join("mfork-service");
        reject_link(&target)?;
        // stop a previous instance first: the binary may be in use
        stop_daemon();
        if me != target {
            std::fs::copy(&me, &target).map_err(|e| format!("copy service: {e}"))?;
        }
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;

        // copy the core into the root-owned dir and pin the copy
        let pinned = pin_core(&dir, core_exe)?;
        write_mode(&dir.join("core-path.txt"), &pinned.to_string_lossy(), 0o644)?;
        // token: readable by root (us) and the installing user only
        let token_file = dir.join("service.token");
        write_mode(&token_file, &protocol::random_hex()?, 0o600)?;
        std::os::unix::fs::chown(&token_file, Some(uid), None)
            .map_err(|e| format!("chown token: {e}"))?;
        write_mode(&dir.join("owner.uid"), &uid.to_string(), 0o644)?;

        let exe = target.to_string_lossy().to_string();
        reject_link(&dir.join("service.log"))?;
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
    <key>StandardOutPath</key><string>/Library/Application Support/Mfork/service.log</string>
    <key>StandardErrorPath</key><string>/Library/Application Support/Mfork/service.log</string>
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

#[cfg(test)]
mod service_tests {
    use super::*;
    #[test]
    fn connection_budget_sheds_excess_and_releases_permits() {
        let permits: Vec<_> = (0..16).map(|_| Connection::acquire().unwrap()).collect();
        assert!(Connection::acquire().is_none());
        drop(permits);
        assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
        assert!(Connection::acquire().is_some());
        assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
    }
}
