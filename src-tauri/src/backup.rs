//! Backup & restore of user data (profiles, overrides, app settings) as a
//! zip archive — to a local file or a WebDAV folder.

use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Manager};
use zip::write::SimpleFileOptions;

const REMOTE_DIR: &str = "Mfork";

fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|e| e.to_string())
}

fn now_stamp() -> String {
    // yyyyMMdd-HHmmss in UTC, no chrono dependency
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}{mo:02}{d:02}-{h:02}{m:02}{s:02}")
}

fn add_dir(
    zw: &mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>,
    root: &Path,
    rel: &str,
) -> Result<(), String> {
    let dir = root.join(rel);
    if !dir.is_dir() {
        return Ok(());
    }
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        if p.is_file() {
            let name = format!("{rel}/{}", entry.file_name().to_string_lossy());
            zw.start_file(name, opts).map_err(|e| e.to_string())?;
            let data = std::fs::read(&p).map_err(|e| e.to_string())?;
            zw.write_all(&data).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Build the archive in memory: profiles/, overrides/, app-config.json.
pub fn build_archive(app: &AppHandle) -> Result<Vec<u8>, String> {
    let root = data_dir(app)?;
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    add_dir(&mut zw, &root, "profiles")?;
    add_dir(&mut zw, &root, "overrides")?;
    let cfg = root.join("app-config.json");
    if cfg.is_file() {
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zw.start_file("app-config.json", opts).map_err(|e| e.to_string())?;
        zw.write_all(&std::fs::read(&cfg).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    }
    let cursor = zw.finish().map_err(|e| e.to_string())?;
    Ok(cursor.into_inner())
}

/// Unpack an archive over the data dir. The current machine's WebDAV
/// credentials are kept so a restore never cuts the connection it came from.
pub fn restore_archive(app: &AppHandle, bytes: &[u8]) -> Result<(), String> {
    let root = data_dir(app)?;
    let keep_webdav = crate::settings::load(app).webdav;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("备份文件无效: {e}"))?;
    let mut has_any = false;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| e.to_string())?;
        let name = f.name().to_string();
        let allowed = name == "app-config.json"
            || name.starts_with("profiles/")
            || name.starts_with("overrides/");
        if !f.is_file() || !allowed || name.contains("..") {
            continue;
        }
        has_any = true;
        let dest = root.join(&name);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut data = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut data).map_err(|e| e.to_string())?;
        std::fs::write(&dest, data).map_err(|e| e.to_string())?;
    }
    if !has_any {
        return Err("备份文件中没有 Mfork 数据".into());
    }
    let mut s = crate::settings::load(app);
    s.webdav = keep_webdav;
    crate::settings::save(app, &s)?;
    Ok(())
}

#[tauri::command]
pub fn backup_export_local(app: AppHandle, path: String) -> Result<(), String> {
    let bytes = build_archive(&app)?;
    std::fs::write(&path, bytes).map_err(|e| format!("写入失败: {e}"))
}

#[tauri::command]
pub fn backup_import_local(app: AppHandle, path: String) -> Result<(), String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("读取失败: {e}"))?;
    restore_archive(&app, &bytes)
}

// ---------------------------------------------------------------- WebDAV

#[derive(Clone, Serialize)]
pub struct BackupEntry {
    pub name: String,
    pub size: u64,
    pub modified: String,
}

struct Dav {
    client: reqwest::blocking::Client,
    base: String, // ".../Mfork" without trailing slash
    user: String,
    pass: String,
}

fn dav(app: &AppHandle) -> Result<Dav, String> {
    let s = crate::settings::load(app).webdav;
    let url = s.url.trim().trim_end_matches('/').to_string();
    if url.is_empty() {
        return Err("未填写 WebDAV 地址".into());
    }
    Ok(Dav {
        // WebDAV servers are usually reachable directly (LAN / NAS / cloud
        // drives); never route them through the proxy core
        client: crate::updater::client(app, Duration::from_secs(60), false)?,
        base: format!("{url}/{REMOTE_DIR}"),
        user: s.username,
        pass: s.password,
    })
}

impl Dav {
    fn req(&self, method: &str, url: &str) -> reqwest::blocking::RequestBuilder {
        let m = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        self.client
            .request(m, url)
            .basic_auth(&self.user, Some(&self.pass))
    }

    fn file_url(&self, name: &str) -> Result<String, String> {
        if name.is_empty() || name.contains('/') || name.contains("..") {
            return Err("非法的备份文件名".into());
        }
        Ok(format!("{}/{name}", self.base))
    }

    /// Create the remote folder (idempotent: 405 = already exists).
    fn ensure_dir(&self) -> Result<(), String> {
        let resp = self
            .req("MKCOL", &self.base)
            .send()
            .map_err(|e| format!("WebDAV 连接失败: {e}"))?;
        match resp.status().as_u16() {
            200..=299 | 405 => Ok(()),
            401 => Err("WebDAV 认证失败（用户名或密码错误）".into()),
            s => Err(format!("WebDAV 创建目录失败: HTTP {s}")),
        }
    }
}

/// Minimal XML text extraction: value of the first `<...:tag>` inside `s`.
fn xml_text<'a>(s: &'a str, tag: &str) -> Option<&'a str> {
    let open = s.find(&format!("{tag}>"))?;
    let start = open + tag.len() + 1;
    let end = start + s[start..].find("</")?;
    Some(&s[start..end])
}

fn parse_propfind(body: &str) -> Vec<BackupEntry> {
    let mut out = Vec::new();
    for block in body.split("<D:response").skip(1) {
        let Some(href) = xml_text(block, "href") else { continue };
        let name = href.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        let name = percent_decode(name);
        if !name.ends_with(".zip") || out.iter().any(|e: &BackupEntry| e.name == name) {
            continue;
        }
        let size = xml_text(block, "getcontentlength")
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        let modified = xml_text(block, "getlastmodified")
            .unwrap_or("")
            .trim()
            .to_string();
        out.push(BackupEntry { name, size, modified });
    }
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[tauri::command]
pub fn webdav_check(app: AppHandle) -> Result<(), String> {
    dav(&app)?.ensure_dir()
}

#[tauri::command]
pub fn webdav_list(app: AppHandle) -> Result<Vec<BackupEntry>, String> {
    let d = dav(&app)?;
    d.ensure_dir()?;
    let resp = d
        .req("PROPFIND", &format!("{}/", d.base))
        .header("Depth", "1")
        .send()
        .map_err(|e| format!("WebDAV 连接失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("WebDAV 列表失败: HTTP {}", resp.status()));
    }
    let body = resp.text().map_err(|e| e.to_string())?;
    // normalise namespace prefixes so the parser only needs one form
    let body = body
        .replace("<d:response", "<D:response")
        .replace("<response", "<D:response");
    Ok(parse_propfind(&body))
}

#[tauri::command]
pub fn webdav_backup(app: AppHandle) -> Result<String, String> {
    let d = dav(&app)?;
    d.ensure_dir()?;
    let bytes = build_archive(&app)?;
    let name = format!("mfork-{}.zip", now_stamp());
    let resp = d
        .req("PUT", &d.file_url(&name)?)
        .header("content-type", "application/zip")
        .body(bytes)
        .send()
        .map_err(|e| format!("上传失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("上传失败: HTTP {}", resp.status()));
    }
    Ok(name)
}

#[tauri::command]
pub fn webdav_restore(app: AppHandle, name: String) -> Result<(), String> {
    let d = dav(&app)?;
    let resp = d
        .req("GET", &d.file_url(&name)?)
        .send()
        .map_err(|e| format!("下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().map_err(|e| e.to_string())?;
    restore_archive(&app, &bytes)
}

#[tauri::command]
pub fn webdav_delete(app: AppHandle, name: String) -> Result<(), String> {
    let d = dav(&app)?;
    let resp = d
        .req("DELETE", &d.file_url(&name)?)
        .send()
        .map_err(|e| format!("删除失败: {e}"))?;
    if !resp.status().is_success() && resp.status().as_u16() != 404 {
        return Err(format!("删除失败: HTTP {}", resp.status()));
    }
    Ok(())
}
