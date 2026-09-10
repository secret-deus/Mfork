//! mihomo core self-upgrade from the official GitHub releases (stable /
//! alpha channels). An upgraded binary lives in `<app-data>/core-bin/` and
//! takes precedence over the bundled one; reverting just deletes it.

use serde::Serialize;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::core::{CoreState, CORE_BIN};

const GH_API: &str = "https://api.github.com/repos/MetaCubeX/mihomo/releases";

#[derive(Clone, Serialize)]
pub struct CoreRelease {
    pub channel: String,
    pub version: String,
    pub asset: String,
    pub url: String,
    pub size: u64,
}

#[derive(Clone, Serialize)]
pub struct CoreVersionInfo {
    /// version of the binary that `core_binary()` resolves to
    pub installed: Option<String>,
    /// version of the bundled binary (None when the bundle is missing)
    pub bundled: Option<String>,
    /// an upgraded binary is present in core-bin/
    pub upgraded: bool,
}

fn os_arch() -> Result<(&'static str, &'static str), String> {
    let os = match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "darwin",
        "linux" => "linux",
        o => return Err(format!("unsupported OS: {o}")),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        a => return Err(format!("unsupported arch: {a}")),
    };
    Ok((os, arch))
}

/// HTTP client; with `via_core` it goes through the running core's mixed
/// port so GitHub stays reachable behind restrictive networks.
pub fn client(
    app: &AppHandle,
    timeout: Duration,
    via_core: bool,
) -> Result<reqwest::blocking::Client, String> {
    let mut b = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent("mfork");
    if via_core {
        let api = app.state::<CoreState>().api.lock().unwrap().clone();
        match api {
            Some(api) => {
                let p = reqwest::Proxy::all(format!("http://127.0.0.1:{}", api.mixed_port))
                    .map_err(|e| e.to_string())?;
                b = b.proxy(p);
            }
            None => return Err("core not running".into()),
        }
    } else {
        b = b.no_proxy();
    }
    b.build().map_err(|e| e.to_string())
}

/// Route order: through the core first (when running), then direct — the
/// active profile may have no usable node, and GitHub may be reachable
/// either way depending on the network.
fn routes(app: &AppHandle) -> Vec<bool> {
    let running = app.state::<CoreState>().api.lock().unwrap().is_some();
    if running {
        vec![true, false]
    } else {
        vec![false]
    }
}

pub fn upgraded_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("core-bin"))
}

pub fn upgraded_binary(app: &AppHandle) -> Option<PathBuf> {
    upgraded_dir(app)
        .ok()
        .map(|d| d.join(CORE_BIN))
        .filter(|p| p.exists())
}

/// `mihomo -v` → "Mihomo Meta v1.19.30 windows amd64 ..." → "v1.19.30"
pub fn binary_version(bin: &PathBuf) -> Option<String> {
    let mut cmd = Command::new(bin);
    cmd.arg("-v");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    parse_version_output(&String::from_utf8_lossy(&out.stdout))
}

/// First line of `mihomo -v`: "Mihomo Meta <version> <os> <arch> ..."
fn parse_version_output(text: &str) -> Option<String> {
    let first = text.lines().next()?;
    let mut it = first.split_whitespace();
    it.next()?;
    it.next()?;
    it.next().map(|s| s.to_string())
}

#[tauri::command]
pub fn core_version_info(app: AppHandle) -> CoreVersionInfo {
    let upgraded = upgraded_binary(&app);
    let bundled = crate::core::bundled_binary(&app).ok();
    CoreVersionInfo {
        installed: upgraded
            .as_ref()
            .or(bundled.as_ref())
            .and_then(binary_version),
        bundled: bundled.as_ref().and_then(binary_version),
        upgraded: upgraded.is_some(),
    }
}

/// Returns the release plus which route reached GitHub (true = via core).
fn fetch_release(app: &AppHandle, channel: &str) -> Result<(CoreRelease, bool), String> {
    let (os, arch) = os_arch()?;
    let ext = if os == "windows" { "zip" } else { "gz" };
    let url = match channel {
        "alpha" => format!("{GH_API}/tags/Prerelease-Alpha"),
        _ => format!("{GH_API}/latest"),
    };
    let mut last_err = String::new();
    let mut found: Option<(serde_json::Value, bool)> = None;
    for via_core in routes(app) {
        let client = client(app, Duration::from_secs(15), via_core)?;
        match client
            .get(&url)
            .header("accept", "application/vnd.github+json")
            .send()
        {
            Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>() {
                Ok(j) => {
                    found = Some((j, via_core));
                    break;
                }
                Err(e) => last_err = e.to_string(),
            },
            Ok(resp) => last_err = format!("GitHub API: HTTP {}", resp.status()),
            Err(e) => last_err = format!("GitHub 请求失败: {e}"),
        }
    }
    let Some((json, via_core)) = found else { return Err(last_err) };
    let prefix = format!("mihomo-{os}-{arch}-");
    let suffix = format!(".{ext}");
    let assets = json["assets"].as_array().ok_or("GitHub 响应缺少 assets")?;
    for a in assets {
        let name = a["name"].as_str().unwrap_or("");
        let Some(rest) = name.strip_prefix(&prefix) else { continue };
        let Some(ver) = rest.strip_suffix(&suffix) else { continue };
        // skip -compatible / -v1 / -go120 variants: version is "vX.Y.Z" or
        // "alpha-<hash>" (older builds used "alpha-g<hash>")
        let ok = if channel == "alpha" {
            ver.strip_prefix("alpha-")
                .map(|h| h.strip_prefix('g').unwrap_or(h))
                .map(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()))
                .unwrap_or(false)
        } else {
            ver.strip_prefix('v')
                .map(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.'))
                .unwrap_or(false)
        };
        if !ok {
            continue;
        }
        return Ok((
            CoreRelease {
                channel: channel.to_string(),
                version: ver.to_string(),
                asset: name.to_string(),
                url: a["browser_download_url"].as_str().unwrap_or("").to_string(),
                size: a["size"].as_u64().unwrap_or(0),
            },
            via_core,
        ));
    }
    Err(format!("未找到 {os}/{arch} 的 {channel} 内核资源"))
}

#[tauri::command]
pub fn core_check_update(app: AppHandle, channel: String) -> Result<CoreRelease, String> {
    fetch_release(&app, &channel).map(|(r, _)| r)
}

#[derive(Clone, Serialize)]
struct Progress {
    received: u64,
    total: u64,
}

#[derive(Clone, Serialize)]
pub struct UpgradeResult {
    pub version: String,
    /// set when the binary was replaced but the elevated service could not
    /// be re-pinned to it: TUN mode keeps running the previous version
    pub warning: Option<String>,
}

/// Download + install the release for `channel`; stops the core first (a
/// running exe cannot be replaced on Windows). The frontend restarts it.
#[tauri::command]
pub fn core_upgrade(
    app: AppHandle,
    state: State<'_, CoreState>,
    channel: String,
) -> Result<UpgradeResult, String> {
    let (rel, via_core) = fetch_release(&app, &channel)?;
    let client = client(&app, Duration::from_secs(600), via_core)?;
    let mut resp = client
        .get(&rel.url)
        .send()
        .map_err(|e| format!("下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(rel.size);
    let mut buf: Vec<u8> = Vec::with_capacity(total as usize);
    let mut chunk = [0u8; 64 * 1024];
    let mut last_emit = 0u64;
    loop {
        let n = resp.read(&mut chunk).map_err(|e| format!("下载中断: {e}"))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        let got = buf.len() as u64;
        if got - last_emit > 256 * 1024 || got == total {
            last_emit = got;
            let _ = app.emit("core-upgrade-progress", Progress { received: got, total });
        }
    }

    let bin = if rel.asset.ends_with(".zip") {
        extract_zip(&buf)?
    } else {
        let mut d = flate2::read::GzDecoder::new(&buf[..]);
        let mut out = Vec::new();
        d.read_to_end(&mut out).map_err(|e| format!("解压失败: {e}"))?;
        out
    };
    if bin.len() < 1024 * 1024 {
        return Err("下载的内核文件异常（过小）".into());
    }

    let dir = upgraded_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!("{CORE_BIN}.new"));
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(&bin).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    // sanity: the new binary must run and report a version
    let new_ver = match binary_version(&tmp) {
        Some(v) => v,
        None => {
            let _ = std::fs::remove_file(&tmp);
            return Err("新内核无法运行（-v 无输出）".into());
        }
    };

    // the service (if any) and our child both need the core stopped
    crate::core::stop_for_upgrade(&app, &state);
    let dest = dir.join(CORE_BIN);
    // keep the previous upgrade aside until the new one is in place
    let old = dir.join(format!("{CORE_BIN}.old"));
    let _ = std::fs::remove_file(&old);
    let had_old = dest.exists();
    if had_old {
        std::fs::rename(&dest, &old).map_err(|e| format!("备份旧内核失败: {e}"))?;
    }
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        if had_old {
            let _ = std::fs::rename(&old, &dest);
        }
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("替换内核失败: {e}"));
    }
    if had_old {
        let _ = std::fs::remove_file(&old);
    }

    // TUN on Windows loads wintun.dll from the core's own directory
    if cfg!(windows) {
        if let Ok(b) = crate::core::bundled_binary(&app) {
            if let Some(src) = b.parent().map(|p| p.join("wintun.dll")) {
                if src.exists() {
                    let _ = std::fs::copy(src, dir.join("wintun.dll"));
                }
            }
        }
    }

    // the elevated service pins its own copy of the core: re-pin (one UAC
    // prompt) so TUN mode uses the new binary too. Failure is reported in
    // the result rather than hidden behind a success return.
    let mut warning = None;
    if crate::service::installed(&app) {
        if let Err(e) = crate::service::repin(&app, &dest) {
            warning = Some(format!(
                "内核文件已更新，但系统服务仍指向旧版本（{e}）。TUN 模式将继续使用旧内核，可在 TUN 设置中「重新指向内核」。"
            ));
        }
    }
    Ok(UpgradeResult { version: new_ver, warning })
}

/// Remove the upgraded binary so the bundled one is used again. Order:
/// verify the bundled binary, re-pin the service, and only then delete —
/// a failure part-way leaves the upgraded copy usable.
#[tauri::command]
pub fn core_revert(app: AppHandle, state: State<'_, CoreState>) -> Result<(), String> {
    let dir = upgraded_dir(&app)?;
    if !dir.exists() {
        return Ok(());
    }
    let bundled = crate::core::bundled_binary(&app)?;
    crate::core::stop_for_upgrade(&app, &state);
    if crate::service::installed(&app) {
        crate::service::repin(&app, &bundled)?;
    }
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(())
}

fn extract_zip(buf: &[u8]) -> Result<Vec<u8>, String> {
    let cursor = std::io::Cursor::new(buf);
    let mut z = zip::ZipArchive::new(cursor).map_err(|e| format!("zip 打开失败: {e}"))?;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| e.to_string())?;
        let name = f.name().to_string();
        let base = name.rsplit('/').next().unwrap_or(&name);
        if f.is_file() && (base.ends_with(".exe") || !base.contains('.')) {
            let mut out = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut out).map_err(|e| e.to_string())?;
            return Ok(out);
        }
    }
    Err("zip 内未找到内核可执行文件".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_parses() {
        assert_eq!(
            parse_version_output("Mihomo Meta v1.19.30 windows amd64 with go1.24 Fri Jan 1\nUse tags: with_gvisor"),
            Some("v1.19.30".into())
        );
        assert_eq!(parse_version_output("Mihomo Meta alpha-abc123 linux arm64"), Some("alpha-abc123".into()));
        assert_eq!(parse_version_output(""), None);
        assert_eq!(parse_version_output("garbage"), None);
    }
}
