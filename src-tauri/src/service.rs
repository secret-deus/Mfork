//! Client for the elevated mfork-service (Windows) plus install/uninstall
//! helpers. The service lets the core run as SYSTEM so TUN mode can create
//! its virtual adapter without the GUI ever being elevated.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Manager};

pub const PORT: u16 = 33217;

#[derive(Serialize)]
struct Req<'a> {
    token: &'a str,
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
}

/// Locate mfork-service.exe: bundle resources first, then the dev build.
fn service_exe(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(dir) = app.path().resource_dir() {
        let p = dir.join("resources").join("service").join("mfork-service.exe");
        if p.exists() {
            return Ok(p);
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|r| r.join("service").join("target").join("release").join("mfork-service.exe"))
        .filter(|p| p.exists());
    dev.ok_or_else(|| {
        "mfork-service.exe 未找到 — 运行 `cargo build --release` (service/ 目录) 构建它".into()
    })
}

fn read_token(app: &AppHandle) -> Option<String> {
    let exe = service_exe(app).ok()?;
    let p = exe.parent()?.join("service.token");
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn call(app: &AppHandle, cmd: &str, config: Option<&str>, dir: Option<&str>) -> Result<Resp, String> {
    let token = read_token(app).unwrap_or_default();
    let addr = format!("127.0.0.1:{PORT}");
    let stream = TcpStream::connect_timeout(
        &addr.parse().unwrap(),
        Duration::from_millis(600),
    )
    .map_err(|_| "服务未响应".to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    let mut w = stream.try_clone().map_err(|e| e.to_string())?;
    let req = Req { token: &token, cmd, config, dir };
    writeln!(w, "{}", serde_json::to_string(&req).unwrap()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| format!("读取服务响应失败: {e}"))?;
    serde_json::from_str::<Resp>(&line).map_err(|e| format!("服务响应异常: {e}"))
}

pub fn ping(app: &AppHandle) -> bool {
    call(app, "status", None, None).map(|r| r.ok).unwrap_or(false)
}

pub fn core_running(app: &AppHandle) -> bool {
    call(app, "status", None, None)
        .ok()
        .and_then(|r| r.core_running)
        .unwrap_or(false)
}

pub fn start_core(app: &AppHandle, config: &str, dir: &str) -> Result<u32, String> {
    let r = call(app, "start", Some(config), Some(dir))?;
    if r.ok {
        Ok(r.pid.unwrap_or(0))
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

/// Reachable or at least registered with the SCM.
pub fn installed(app: &AppHandle) -> bool {
    ping(app) || service_registered()
}

/// Point an installed service at a different core binary (re-runs the
/// installer elevated, which rewrites core-path.txt + token and restarts
/// the service).
pub fn repin(app: &AppHandle, core: &PathBuf) -> Result<(), String> {
    let svc = service_exe(app)?;
    run_elevated(&svc, &["install", &core.to_string_lossy()])?;
    for _ in 0..10 {
        if ping(app) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    Err("服务已重新安装但未响应".into())
}

/// Is the Windows service registered with the SCM (even if not running)?
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
    #[cfg(not(windows))]
    false
}

/// Run the service installer elevated (one UAC prompt).
#[tauri::command]
pub fn service_install(app: AppHandle) -> Result<(), String> {
    let svc = service_exe(&app)?;
    let core = crate::core::core_binary(&app)?;
    run_elevated(&svc, &["install", &core.to_string_lossy()])?;
    // give the freshly started service a moment to open its port
    for _ in 0..10 {
        if ping(&app) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    Err("服务已安装但未响应，请查看系统服务列表 (MforkService)".into())
}

#[tauri::command]
pub fn service_uninstall(app: AppHandle) -> Result<(), String> {
    let svc = service_exe(&app)?;
    run_elevated(&svc, &["uninstall"])
}

/// Launch an exe elevated via powershell Start-Process -Verb RunAs and wait.
fn run_elevated(exe: &PathBuf, args: &[&str]) -> Result<(), String> {
    #[cfg(windows)]
    {
        let arg_list = args
            .iter()
            .map(|a| format!("'{}'", a.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        let script = format!(
            "$p = Start-Process -FilePath '{}' -ArgumentList {} -Verb RunAs -Wait -PassThru; exit $p.ExitCode",
            exe.to_string_lossy().replace('\'', "''"),
            arg_list
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
    #[cfg(not(windows))]
    {
        let _ = (exe, args);
        Err("提权服务目前仅支持 Windows".into())
    }
}
