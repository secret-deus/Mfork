//! Backup & restore of user data (profiles, overrides, app settings) as a
//! zip archive — to a local file or a WebDAV folder.

use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Manager};
use zip::write::SimpleFileOptions;

#[path = "backup_transaction.rs"]
mod transaction;

static RESTORE_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub fn restore_epoch() -> u64 {
    RESTORE_EPOCH.load(std::sync::atomic::Ordering::SeqCst)
}

pub fn recover_pending(app: &AppHandle) -> Result<(), String> {
    let _guard = crate::settings::data_lock();
    transaction::recover_pending(&data_dir(app)?)
}

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
    total: &mut u64,
    count: &mut usize,
) -> Result<(), String> {
    let dir = root.join(rel);
    transaction::reject_link(&dir)?;
    if !dir.is_dir() {
        return Ok(());
    }
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        transaction::reject_link(&p)?;
        if p.is_file() {
            let name = format!("{rel}/{}", entry.file_name().to_string_lossy());
            if !transaction::file_name_allowed(&name) || name.ends_with(".bak") {
                continue;
            }
            *count += 1;
            if *count > 4096 {
                return Err("备份条目过多".into());
            }
            let data = transaction::read_bounded(
                std::fs::File::open(&p).map_err(|e| e.to_string())?,
                20 * 1024 * 1024,
            )?;
            *total += data.len() as u64;
            if *total > 128 * 1024 * 1024 {
                return Err("备份总大小超过限制".into());
            }
            zw.start_file(name, opts).map_err(|e| e.to_string())?;
            zw.write_all(&data).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Build the archive in memory: profiles/, overrides/, app-config.json.
pub fn build_archive(app: &AppHandle) -> Result<Vec<u8>, String> {
    let _guard = crate::settings::data_lock();
    let root = data_dir(app)?;
    transaction::recover_pending(&root)?;
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let (mut total, mut count) = (0, 0);
    add_dir(&mut zw, &root, "profiles", &mut total, &mut count)?;
    add_dir(&mut zw, &root, "overrides", &mut total, &mut count)?;
    let cfg = root.join("app-config.json");
    transaction::reject_link(&cfg)?;
    if cfg.is_file() {
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zw.start_file("app-config.json", opts)
            .map_err(|e| e.to_string())?;
        // never ship the WebDAV credentials inside the archive: a restore
        // keeps the local ones anyway (see restore_archive)
        let mut s = crate::settings::load(app);
        s.webdav = crate::settings::WebdavSettings::default();
        let data = serde_json::to_vec_pretty(&s).map_err(|e| e.to_string())?;
        if data.len() > 20 * 1024 * 1024
            || total + data.len() as u64 > 128 * 1024 * 1024
            || count + 1 > 4096
        {
            return Err("备份大小或条目数超过限制".into());
        }
        zw.write_all(&data).map_err(|e| e.to_string())?;
    }
    let cursor = zw.finish().map_err(|e| e.to_string())?;
    let bytes = cursor.into_inner();
    if bytes.len() as u64 > transaction::MAX_ARCHIVE {
        return Err("备份文件超过 64 MiB".into());
    }
    Ok(bytes)
}

/// Unpack an archive over the data dir. The current machine's WebDAV
/// credentials are kept so a restore never cuts the connection it came from.
pub fn restore_archive(app: &AppHandle, bytes: &[u8]) -> Result<(), String> {
    let _guard = crate::settings::data_lock();
    // Invalidate downloads even if validation/rollback later fails.
    RESTORE_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let root = data_dir(app)?;
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    transaction::recover_pending(&root)?;
    let keep_webdav = crate::settings::load(app).webdav;
    transaction::restore(&root, bytes, keep_webdav)?;
    Ok(())
}

#[tauri::command]
pub fn backup_export_local(app: AppHandle, path: String) -> Result<(), String> {
    let bytes = build_archive(&app)?;
    std::fs::write(&path, bytes).map_err(|e| format!("写入失败: {e}"))
}

#[tauri::command]
pub fn backup_import_local(app: AppHandle, path: String) -> Result<(), String> {
    let bytes = transaction::read_bounded(
        std::fs::File::open(&path).map_err(|e| format!("读取失败: {e}"))?,
        transaction::MAX_ARCHIVE,
    )?;
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
    let mut base = reqwest::Url::parse(&url).map_err(|_| "WebDAV 地址无效")?;
    if !["http", "https"].contains(&base.scheme())
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err("WebDAV 地址必须是无查询参数的 HTTP(S) 路径".into());
    }
    base.path_segments_mut()
        .map_err(|_| "WebDAV 路径无效")?
        .pop_if_empty()
        .push(REMOTE_DIR);
    Ok(Dav {
        // WebDAV servers are usually reachable directly (LAN / NAS / cloud
        // drives); never route them through the proxy core
        client: crate::updater::client(app, Duration::from_secs(60), false)?,
        base: base.to_string(),
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
        if !name.ends_with(".zip")
            || name.len() > 200
            || name.contains("..")
            || name
                .chars()
                .any(|c| c.is_control() || c == '/' || c == '\\')
        {
            return Err("非法的备份文件名".into());
        }
        let mut url = reqwest::Url::parse(&self.base).map_err(|e| e.to_string())?;
        url.path_segments_mut()
            .map_err(|_| "WebDAV 路径无效")?
            .push(name);
        Ok(url.to_string())
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
        let Some(href) = xml_text(block, "href") else {
            continue;
        };
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
        out.push(BackupEntry {
            name,
            size,
            modified,
        });
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
            let hex = |b: u8| match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                b'A'..=b'F' => Some(b - b'A' + 10),
                _ => None,
            };
            if let (Some(a), Some(b)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((a << 4) | b);
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
    let body = String::from_utf8(transaction::read_bounded(resp, 4 * 1024 * 1024)?)
        .map_err(|e| e.to_string())?;
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
    let bytes = transaction::read_bounded(resp, transaction::MAX_ARCHIVE)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_handles_utf8_and_junk() {
        assert_eq!(
            percent_decode("mfork-%E5%A4%87%E4%BB%BD.zip"),
            "mfork-备份.zip"
        );
        assert_eq!(percent_decode("a%2Gb%"), "a%2Gb%");
        assert_eq!(percent_decode("%中文.zip"), "%中文.zip");
        assert_eq!(percent_decode("%中%2Ezip"), "%中.zip");
    }

    #[test]
    fn webdav_file_names_stay_in_one_url_segment() {
        let d = Dav {
            client: reqwest::blocking::Client::new(),
            base: "https://example.invalid/dav/Mfork".into(),
            user: String::new(),
            pass: String::new(),
        };
        let url = reqwest::Url::parse(&d.file_url("备份 #1?.zip").unwrap()).unwrap();
        assert_eq!(url.host_str(), Some("example.invalid"));
        assert_eq!(url.query(), None);
        assert_eq!(url.fragment(), None);
        assert_eq!(
            percent_decode(url.path_segments().unwrap().next_back().unwrap()),
            "备份 #1?.zip"
        );
        for name in ["../bad.zip", "x/y.zip", "x\\y.zip", "bad.zip\n", "bad.txt"] {
            assert!(d.file_url(name).is_err(), "{name}");
        }
        assert!(d.file_url("%2f.zip").unwrap().ends_with("%252f.zip"));
    }

    #[test]
    fn propfind_lists_zip_entries_newest_first() {
        let body = r#"<D:multistatus>
<D:response><D:href>/dav/Mfork/</D:href><D:propstat><D:prop><D:getcontentlength>0</D:getcontentlength></D:prop></D:propstat></D:response>
<D:response><D:href>/dav/Mfork/mfork-20240101-000000.zip</D:href><D:propstat><D:prop><D:getcontentlength>10</D:getcontentlength><D:getlastmodified>Mon, 01 Jan 2024 00:00:00 GMT</D:getlastmodified></D:prop></D:propstat></D:response>
<D:response><D:href>/dav/Mfork/mfork-20240202-000000.zip</D:href><D:propstat><D:prop><D:getcontentlength>20</D:getcontentlength></D:prop></D:propstat></D:response>
<D:response><D:href>/dav/Mfork/notes.txt</D:href></D:response>
</D:multistatus>"#;
        let list = parse_propfind(body);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "mfork-20240202-000000.zip");
        assert_eq!(list[0].size, 20);
        assert_eq!(list[1].modified, "Mon, 01 Jan 2024 00:00:00 GMT");
    }
}
