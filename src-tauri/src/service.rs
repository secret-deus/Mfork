//! Client for the elevated mfork-service plus install/uninstall helpers.
//! The service lets the core run as SYSTEM/root so TUN mode can create its
//! virtual adapter without the GUI ever being elevated.
//!
//! Windows: SCM service, UAC prompt via PowerShell RunAs.
//! macOS:   launchd daemon, admin prompt via osascript.
//! Linux:   systemd unit, polkit prompt via pkexec.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Manager};
#[path = "../../shared/service_protocol.rs"]
#[allow(dead_code)]
mod protocol;
#[cfg(windows)]
#[path = "../../shared/service_windows.rs"]
#[allow(dead_code)]
mod security;

pub const PORT: u16 = 33217;

#[derive(Serialize)]
struct Req<'a> {
    cmd: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    config: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dir: Option<&'a str>,
}

#[derive(Deserialize, Default)]
pub struct Resp {
    pub ok: bool,
    pub msg: Option<String>,
    pub core_running: Option<bool>,
    pub pid: Option<u32>,
    /// data dir the service started the core in (its core.err.log lives there)
    pub dir: Option<String>,
    /// Reported before spawning, from the service's own geodata directory.
    pub cold_start: Option<bool>,
}

#[path = "service_status.rs"]
mod status_probe;

const SERVICE_BIN: &str = if cfg!(windows) {
    "mfork-service.exe"
} else {
    "mfork-service"
};

/// Locate the bundled service binary: bundle resources first, then the
/// dev build.
fn service_exe(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(dir) = app.path().resource_dir() {
        let p = dir.join("resources").join("service").join(SERVICE_BIN);
        if p.exists() {
            return Ok(p);
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|r| {
            r.join("service")
                .join("target")
                .join("release")
                .join(SERVICE_BIN)
        })
        .filter(|p| p.exists());
    dev.ok_or_else(|| {
        format!("{SERVICE_BIN} 未找到 — 运行 `node scripts/build-service.mjs` 构建它")
    })
}

/// Only the fixed protected installation is trusted; never read legacy/dev tokens.
fn token_dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        security::install_root()
    }
    #[cfg(target_os = "macos")]
    {
        Ok(PathBuf::from("/Library/Application Support/Mfork"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Ok(PathBuf::from("/usr/local/lib/mfork"))
    }
}

fn read_token() -> Result<[u8; 32], String> {
    let root = token_dir()?;
    #[cfg(windows)]
    security::validate_token(&root)?;
    #[cfg(windows)]
    security::validate_root(&root)?;
    let p = root.join("service.token");
    #[cfg(windows)]
    security::reject_reparse(&p)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::symlink_metadata(&p).map_err(|e| e.to_string())?;
        if !m.is_file() || m.mode() & 0o077 != 0 || m.uid() != unsafe { libc::getuid() } {
            return Err("服务令牌权限不安全，请重新安装服务".into());
        }
    }
    let mut bytes = Vec::new();
    std::fs::File::open(p)
        .map_err(|_| "服务 v2 令牌不可用，请重新安装服务（不会回退到旧认证）".to_string())?
        .take(65)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    protocol::parse_key(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)
}

fn call(
    _app: &AppHandle,
    cmd: &str,
    config: Option<&str>,
    dir: Option<&str>,
) -> Result<Resp, String> {
    let key = read_token()?;
    let addr = format!("127.0.0.1:{PORT}");
    let mut stream = TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(600))
        .map_err(|_| "服务未响应".to_string())?;
    protocol::client_call(&mut stream, &key, &Req { cmd, config, dir })
}

pub fn ping(app: &AppHandle) -> bool {
    call(app, "status", None, None)
        .map(|r| r.ok)
        .unwrap_or(false)
}

pub fn core_running(app: &AppHandle) -> bool {
    call(app, "status", None, None)
        .ok()
        .and_then(|r| r.core_running)
        .unwrap_or(false)
}

/// Only a successful, explicit service response can confirm that the core
/// stopped. An IPC error or incomplete response is unknown, not an exit.
pub fn core_running_status(app: &AppHandle) -> Option<bool> {
    status_probe::from_response(
        call(app, "status", None, None).map(|r| (r.ok, r.core_running)),
    )
}

/// Returns (pid, actual data dir, whether geodata may need downloading).
pub fn start_core(
    app: &AppHandle,
    config: &str,
    dir: &str,
) -> Result<(u32, Option<String>, bool), String> {
    let r = call(app, "start", Some(config), Some(dir))?;
    if r.ok {
        // Older services cannot report their cache state. Never infer it from
        // the GUI's unrelated data directory: allow a conservative cold start.
        Ok((r.pid.unwrap_or(0), r.dir, r.cold_start.unwrap_or(true)))
    } else {
        Err(r.msg.unwrap_or_else(|| "服务启动内核失败".into()))
    }
}

pub fn stop_core(app: &AppHandle) {
    let _ = call(app, "stop", None, None);
}

#[derive(Serialize)]
pub struct ServiceStatus {
    pub installed: bool,
    pub reachable: bool,
    pub core_running: bool,
}

#[tauri::command]
pub fn service_status(app: AppHandle) -> ServiceStatus {
    let status = call(&app, "status", None, None).ok();
    let reachable = status.as_ref().map(|r| r.ok).unwrap_or(false);
    let installed = reachable || service_registered();
    ServiceStatus {
        installed,
        reachable,
        core_running: status.and_then(|r| r.core_running).unwrap_or(false),
    }
}

/// Reachable or at least registered with the OS service manager.
pub fn installed(app: &AppHandle) -> bool {
    ping(app) || service_registered()
}

/// Arguments for `mfork-service install <core>`: on Unix the installer is
/// told our uid so the token file can be owned by us with mode 0600.
#[allow(unused_mut)]
fn install_args(app: &AppHandle, core: &str) -> Result<Vec<String>, String> {
    let mut v = vec!["install".to_string(), core.to_string()];
    let bundled = crate::core::bundled_binary(app)?;
    v.push("--geodata-dir".into());
    v.push(bundled.parent().ok_or("missing resource directory")?.to_string_lossy().into_owned());
    #[cfg(unix)]
    {
        v.push("--owner".into());
        v.push(unsafe { libc::getuid() }.to_string());
    }
    #[cfg(windows)]
    {
        v.push("--owner-sid".into());
        v.push(security::current_user_sid()?);
    }
    Ok(v)
}

/// Point an installed service at a different core binary (re-runs the
/// installer elevated, which copies + pins the core, rewrites the token and
/// restarts the service).
pub fn repin(app: &AppHandle, core: &PathBuf) -> Result<(), String> {
    let svc = service_exe(app)?;
    let args = install_args(app, &core.to_string_lossy())?;
    run_elevated(&svc, &args.iter().map(String::as_str).collect::<Vec<_>>())?;
    for _ in 0..10 {
        if ping(app) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    Err("服务已重新安装但未响应".into())
}

/// Is the service registered with the OS (even if not running)?
fn service_registered() -> bool {
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("sc.exe");
        cmd.args(["query", "MforkService"]);
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        cmd.output().map(|o| o.status.success()).unwrap_or(false)
    }
    #[cfg(target_os = "macos")]
    {
        std::path::Path::new("/Library/LaunchDaemons/app.mfork.service.plist").exists()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::path::Path::new("/etc/systemd/system/mfork-service.service").exists()
    }
}

/// Run the service installer elevated (one UAC prompt).
#[tauri::command]
pub fn service_install(app: AppHandle) -> Result<(), String> {
    let svc = service_exe(&app)?;
    let core = crate::core::core_binary(&app)?;
    let args = install_args(&app, &core.to_string_lossy())?;
    run_elevated(&svc, &args.iter().map(String::as_str).collect::<Vec<_>>())?;
    // give the freshly started service a moment to open its port
    for _ in 0..10 {
        if ping(&app) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    Err(if cfg!(windows) {
        "服务已安装但未响应，请查看系统服务列表 (MforkService)".into()
    } else if cfg!(target_os = "macos") {
        "服务已安装但未响应，请查看 /Library/Application Support/Mfork/service.log".into()
    } else {
        "服务已安装但未响应，请查看 `systemctl status mfork-service`".into()
    })
}

#[tauri::command]
pub fn service_uninstall(app: AppHandle) -> Result<(), String> {
    let svc = service_exe(&app)?;
    run_elevated(&svc, &["uninstall"])
}

#[cfg(windows)]
fn quote_windows_argument(arg: &str) -> String {
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in arg.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        if c == '"' {
            out.push_str(&"\\".repeat(slashes * 2 + 1));
        } else {
            out.push_str(&"\\".repeat(slashes));
        }
        slashes = 0;
        out.push(c);
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    out
}

/// Run the service binary elevated and wait: PowerShell RunAs (UAC) on
/// Windows, osascript admin prompt on macOS, pkexec (polkit) on Linux.
fn run_elevated(exe: &PathBuf, args: &[&str]) -> Result<(), String> {
    #[cfg(windows)]
    {
        let arg_list = args
            .iter()
            .map(|a| quote_windows_argument(a))
            .collect::<Vec<_>>()
            .join(" ");
        let script = format!(
            "$ErrorActionPreference = 'Stop'; $p = Start-Process -FilePath '{}' -ArgumentList '{}' -Verb RunAs -Wait -PassThru; exit $p.ExitCode",
            exe.to_string_lossy().replace('\'', "''"),
            arg_list.replace('\'', "''")
        );
        let mut cmd = std::process::Command::new("powershell.exe");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let out = cmd.output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            if err.contains("canceled") || err.contains("已取消") {
                Err("用户取消了 UAC 授权".into())
            } else {
                Err(format!("提权执行失败: {}", err.trim()))
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        // shell-quote every argument, then embed the command line in the
        // AppleScript string literal
        let sh = std::iter::once(exe.to_string_lossy().to_string())
            .chain(args.iter().map(|a| a.to_string()))
            .map(|a| format!("'{}'", a.replace('\'', "'\\''")))
            .collect::<Vec<_>>()
            .join(" ");
        let script = format!(
            "do shell script \"{}\" with administrator privileges",
            sh.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let out = std::process::Command::new("osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            if err.contains("-128") {
                Err("用户取消了管理员授权".into())
            } else {
                Err(format!("提权执行失败: {}", err.trim()))
            }
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let out = std::process::Command::new("pkexec")
            .arg(exe)
            .args(args)
            .output()
            .map_err(|e| format!("pkexec 不可用（需要 polkit）: {e}"))?;
        match out.status.code() {
            Some(0) => Ok(()),
            Some(126) | Some(127) => Err("用户取消了授权".into()),
            _ => Err(format!(
                "提权执行失败: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )),
        }
    }
}
