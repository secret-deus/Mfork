//! Bounded backup validation and crash-recoverable restore. Caller holds DATA_LOCK.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};
use std::path::Path;

pub const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;
const MAX_FILE: u64 = 20 * 1024 * 1024;
const MAX_TOTAL: u64 = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
const JOURNAL: &str = ".restore-pending.json";
const PARTS: [&str; 3] = ["profiles", "overrides", "app-config.json"];

type Files = BTreeMap<String, Vec<u8>>;

pub fn read_bounded(reader: impl Read, max: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err(format!("数据超出大小限制（{} MiB）", max / 1024 / 1024));
    }
    Ok(bytes)
}

fn valid_id(id: &str) -> bool {
    id.len() == 8 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
fn parse_json<T: serde::de::DeserializeOwned>(data: &[u8]) -> Result<T, String> {
    let text = std::str::from_utf8(data).map_err(|e| e.to_string())?;
    serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("备份 JSON 无效: {e}"))
}
pub(super) fn file_name_allowed(name: &str) -> bool {
    if name == "app-config.json" {
        return true;
    }
    let Some((dir, file)) = name.split_once('/') else {
        return false;
    };
    if !["profiles", "overrides"].contains(&dir) {
        return false;
    }
    if file == "index.json" || file == "index.json.bak" {
        return true;
    }
    let Some((id, ext)) = file.split_once('.') else {
        return false;
    };
    valid_id(id) && (ext == "yaml" || dir == "overrides" && ext == "js")
}

fn validate_archive(bytes: &[u8]) -> Result<Files, String> {
    if bytes.len() as u64 > MAX_ARCHIVE {
        return Err("备份文件过大".into());
    }
    let mut zip =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("备份文件无效: {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err("备份条目过多".into());
    }
    let mut files = Files::new();
    let mut names = BTreeSet::new();
    let mut total = 0;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = f.name().to_string();
        if f.is_dir() && ["profiles/", "overrides/"].contains(&name.as_str()) {
            continue;
        }
        if !f.is_file()
            || !file_name_allowed(&name)
            || f.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err(format!("备份包含不支持的路径: {name}"));
        }
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(format!("备份路径重复或大小写冲突: {name}"));
        }
        if f.size() > MAX_FILE || total + f.size() > MAX_TOTAL {
            return Err("备份解压大小超过限制".into());
        }
        let entry_limit = if name.ends_with("/index.json") {
            4 * 1024 * 1024
        } else if name.starts_with("overrides/") && !name.ends_with(".bak") {
            1024 * 1024
        } else {
            MAX_FILE
        };
        // Enforce the type budget on decoded bytes, not only ZIP metadata.
        let data = read_bounded(&mut f, entry_limit.min(MAX_TOTAL - total))?;
        total += data.len() as u64;
        if files.insert(name.clone(), data).is_some() {
            return Err(format!("备份包含重复文件: {name}"));
        }
    }
    // Only primary indices are authoritative. Never restore a stale recovery index.
    files.remove("profiles/index.json.bak");
    files.remove("overrides/index.json.bak");
    if files.is_empty() {
        return Err("备份文件中没有 Mfork 数据".into());
    }
    if files.keys().any(|n| n.starts_with("profiles/")) {
        let idx: crate::profile::ProfilesIndex =
            parse_json(files.get("profiles/index.json").ok_or("备份缺少订阅索引")?)?;
        let mut ids = BTreeSet::new();
        for item in &idx.items {
            if !valid_id(&item.id) || !ids.insert(item.id.clone()) {
                return Err("备份订阅 ID 非法或重复".into());
            }
            let data = files
                .get(&format!("profiles/{}.yaml", item.id))
                .ok_or("备份缺少订阅文件")?;
            let value = crate::config_limits::parse_yaml(
                std::str::from_utf8(data).map_err(|e| e.to_string())?,
            )?;
            if !value.is_mapping() {
                return Err("备份订阅必须是 YAML 映射".into());
            }
        }
        if idx.active.as_ref().is_some_and(|id| !ids.contains(id)) {
            return Err("备份当前订阅不存在".into());
        }
    }
    if files.keys().any(|n| n.starts_with("overrides/")) {
        let idx: Vec<crate::override_::OverrideMeta> = parse_json(
            files
                .get("overrides/index.json")
                .ok_or("备份缺少覆写索引")?,
        )?;
        if idx.len() > 256 {
            return Err("备份覆写条目超过 256 条".into());
        }
        let mut ids = BTreeSet::new();
        for item in &idx {
            if !valid_id(&item.id) || !ids.insert(item.id.clone()) {
                return Err("备份覆写 ID 非法或重复".into());
            }
            let ext = match item.kind {
                crate::override_::OverrideKind::Yaml => "yaml",
                crate::override_::OverrideKind::Js => "js",
            };
            let data = files
                .get(&format!("overrides/{}.{ext}", item.id))
                .ok_or("备份缺少覆写文件")?;
            let text = std::str::from_utf8(data).map_err(|e| e.to_string())?;
            if ext == "yaml" {
                let value = crate::config_limits::parse_yaml(text)?;
                if !value.is_mapping() && !value.is_null() {
                    return Err("备份覆写必须是 YAML 映射".into());
                }
            }
        }
    }
    if let Some(data) = files.get("app-config.json") {
        let s: crate::settings::AppSettings = parse_json(data)?;
        crate::settings::normalize_process_rules(&s.process_rules)?;
    }
    Ok(files)
}

#[derive(Serialize, Deserialize)]
struct Journal {
    stage: String,
    parts: Vec<Part>,
}
#[derive(Serialize, Deserialize)]
struct Part {
    name: String,
    existed: bool,
}

pub(super) fn reject_link(path: &Path) -> Result<(), String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("无法检查备份路径 {}: {e}", path.display())),
    };
    if meta.file_type().is_symlink() {
        return Err(format!("拒绝备份符号链接: {}", path.display()));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(format!("拒绝备份重解析点: {}", path.display()));
        }
    }
    Ok(())
}
fn remove_entry(path: &Path) -> Result<(), String> {
    reject_link(path)?;
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else if path.exists() {
        std::fs::remove_file(path)
    } else {
        Ok(())
    }
    .map_err(|e| e.to_string())
}
fn rollback(root: &Path, journal: &Journal) -> Result<(), String> {
    let stage = root.join(&journal.stage);
    reject_link(&stage)?;
    reject_link(&stage.join("previous"))?;
    for part in journal.parts.iter().rev() {
        let current = root.join(&part.name);
        let old = stage.join("previous").join(&part.name);
        reject_link(&current)?;
        reject_link(&old)?;
        if old.exists() {
            remove_entry(&current)?;
            std::fs::rename(&old, &current).map_err(|e| format!("回滚 {} 失败: {e}", part.name))?;
        } else if !part.existed {
            remove_entry(&current)?;
        } else if !current.exists() {
            return Err(format!(
                "恢复日志与文件不一致，请从 {} 人工恢复",
                stage.display()
            ));
        }
    }
    std::fs::remove_file(root.join(JOURNAL)).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(stage);
    Ok(())
}

/// True only for the exact names restore_inner generates: `.restore-` + 32 lowercase hex.
fn is_generated_stage_name(name: &str) -> bool {
    let Some(hex) = name.strip_prefix(".restore-") else {
        return false;
    };
    hex.len() == 32 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
fn stage_file_allowed(name: &str) -> bool {
    file_name_allowed(name)
        || name
            .rsplit_once(".tmp-")
            .is_some_and(|(base, id)| valid_id(id) && file_name_allowed(base))
}

/// True when `dir` holds only files the restore flow ever writes or moves aside.
fn only_known_stage_files(dir: &Path) -> Result<bool, String> {
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => return Ok(false),
        };
        let path = entry.path();
        reject_link(&path)?;
        if !path.is_file() {
            return Ok(false);
        }
        // file_name_allowed expects archive-style relative paths ("dir/file"),
        // so rebuild that form from the part directory these files live in.
        let Some(part_dir) = dir.file_name().and_then(|n| n.to_str()) else {
            return Ok(false);
        };
        let relative = format!("{part_dir}/{name}");
        if !stage_file_allowed(&relative) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// True when the stage contains only the layout restore_inner creates
/// (`previous`/`next` holding known config parts). Anything ambiguous yields
/// Ok(false) so the caller can leave it for manual inspection instead of deleting.
fn stage_contents_orphaned(stage: &Path) -> Result<bool, String> {
    for top in std::fs::read_dir(stage).map_err(|e| e.to_string())? {
        let top = top.map_err(|e| e.to_string())?;
        let name = match top.file_name().into_string() {
            Ok(name) => name,
            Err(_) => return Ok(false),
        };
        if !matches!(name.as_str(), "previous" | "next") {
            return Ok(false);
        }
        let dir = top.path();
        reject_link(&dir)?;
        if !dir.is_dir() {
            return Ok(false);
        }
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = match entry.file_name().into_string() {
                Ok(name) => name,
                Err(_) => return Ok(false),
            };
            let settings_tmp = name
                .strip_prefix("app-config.json.tmp-")
                .is_some_and(valid_id);
            if !PARTS.contains(&name.as_str()) && !settings_tmp {
                return Ok(false);
            }
            let part = entry.path();
            reject_link(&part)?;
            if part.is_dir() {
                if !["profiles", "overrides"].contains(&name.as_str())
                    || !only_known_stage_files(&part)?
                {
                    return Ok(false);
                }
            } else if (name != "app-config.json" && !settings_tmp) || !part.is_file() {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Remove stage dirs orphaned by a process crash before the journal was written or
/// after a successful commit. Callers must have verified there is no pending journal,
/// or must have recovered it first. Only the exact generated names are considered,
/// links and reparse points are never followed, and ambiguous unexpected content is
/// left in place and reported instead of deleted. Process-crash recovery only; no
/// power-loss durability is guaranteed.
fn cleanup_orphan_stages(root: &Path) -> Result<(), String> {
    let mut kept = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !is_generated_stage_name(&name) {
            continue;
        }
        let stage = entry.path();
        if let Err(e) = reject_link(&stage) {
            kept.push(format!("{e}，保留等待人工处理"));
            continue;
        }
        if !stage.is_dir() {
            kept.push(format!(
                "恢复暂存 {} 不是目录，保留等待人工处理",
                stage.display()
            ));
            continue;
        }
        match stage_contents_orphaned(&stage) {
            Ok(true) => {
                if let Err(e) = std::fs::remove_dir_all(&stage) {
                    kept.push(format!("清理恢复暂存 {} 失败: {e}", stage.display()));
                }
            }
            Ok(false) => kept.push(format!(
                "恢复暂存 {} 内容异常，保留等待人工处理",
                stage.display()
            )),
            Err(e) => kept.push(format!("检查恢复暂存 {} 失败: {e}", stage.display())),
        }
    }
    if kept.is_empty() {
        Ok(())
    } else {
        Err(kept.join("；"))
    }
}

pub fn recover_pending(root: &Path) -> Result<(), String> {
    reject_link(root)?;
    match std::fs::symlink_metadata(root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
        Ok(m) if !m.is_dir() => return Err("恢复根路径不是目录".into()),
        Ok(_) => {}
    }
    let path = root.join(JOURNAL);
    reject_link(&path)?;
    if !path.exists() {
        // No pending journal: any remaining stage dirs are crash orphans from a run
        // that died before the journal was written or after a successful commit.
        cleanup_orphan_stages(root)?;
        return Ok(());
    }
    let data = read_bounded(
        std::fs::File::open(&path).map_err(|e| e.to_string())?,
        16 * 1024,
    )?;
    let journal: Journal = parse_json(&data)?;
    let suffix = journal
        .stage
        .strip_prefix(".restore-")
        .ok_or("恢复日志路径无效")?;
    if suffix.len() != 32
        || !suffix.bytes().all(|b| b.is_ascii_hexdigit())
        || journal.parts.is_empty()
        || journal.parts.len() > PARTS.len()
    {
        return Err("恢复日志无效，保留数据等待人工处理".into());
    }
    let mut seen = BTreeSet::new();
    for part in &journal.parts {
        if !PARTS.contains(&part.name.as_str()) || !seen.insert(&part.name) {
            return Err("恢复日志条目无效".into());
        }
    }
    rollback(root, &journal)?;
    cleanup_orphan_stages(root)
}

pub fn restore(
    root: &Path,
    bytes: &[u8],
    webdav: crate::settings::WebdavSettings,
) -> Result<(), String> {
    restore_inner(root, bytes, webdav, None)
}

fn restore_inner(
    root: &Path,
    bytes: &[u8],
    webdav: crate::settings::WebdavSettings,
    fail_after: Option<usize>,
) -> Result<(), String> {
    recover_pending(root)?;
    let mut files = validate_archive(bytes)?;
    if let Some(data) = files.get_mut("app-config.json") {
        let mut settings: crate::settings::AppSettings = parse_json(data)?;
        settings.webdav = webdav;
        settings.process_rules = crate::settings::normalize_process_rules(&settings.process_rules)?;
        *data = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
    }
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random).map_err(|e| e.to_string())?;
    let name = format!(
        ".restore-{}",
        random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let stage = root.join(&name);
    std::fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let prepare = (|| {
        std::fs::create_dir(stage.join("previous")).map_err(|e| e.to_string())?;
        for (file, data) in &files {
            let dest = stage.join("next").join(file);
            std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
            crate::settings::write_atomic(&dest, data)?;
        }
        let mut parts = Vec::new();
        for name in PARTS {
            if stage.join("next").join(name).exists() {
                reject_link(&root.join(name))?;
                parts.push(Part {
                    name: name.into(),
                    existed: root.join(name).exists(),
                });
            }
        }
        Ok::<_, String>(Journal { stage: name, parts })
    })();
    let journal = match prepare {
        Ok(j) => j,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(e);
        }
    };
    if let Err(e) = crate::settings::write_atomic(
        &root.join(JOURNAL),
        &serde_json::to_vec(&journal).map_err(|e| e.to_string())?,
    ) {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }
    let commit = (|| {
        for (i, part) in journal.parts.iter().enumerate() {
            if fail_after == Some(i) {
                return Err("注入的恢复提交失败".to_string());
            }
            let current = root.join(&part.name);
            if part.existed {
                std::fs::rename(&current, stage.join("previous").join(&part.name))
                    .map_err(|e| e.to_string())?;
            }
            std::fs::rename(stage.join("next").join(&part.name), current)
                .map_err(|e| e.to_string())?;
        }
        std::fs::remove_file(root.join(JOURNAL)).map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    })();
    if let Err(e) = commit {
        return match rollback(root, &journal) {
            Ok(()) => Err(format!("恢复失败，已还原原配置: {e}")),
            Err(r) => Err(format!(
                "恢复失败: {e}；回滚失败: {r}，请保留 {}",
                stage.display()
            )),
        };
    }
    let _ = std::fs::remove_dir_all(stage);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    fn dir() -> PathBuf {
        let base = std::env::var_os("PI_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let p = base.join(format!("mfork-restore-test-{:016x}", fastrand::u64(..)));
        std::fs::create_dir(&p).unwrap();
        p
    }
    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (n, d) in entries {
            z.start_file(
                *n,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
            std::io::Write::write_all(&mut z, d).unwrap();
        }
        z.finish().unwrap().into_inner()
    }
    #[test]
    fn bounded_reader_rejects_excess() {
        assert!(read_bounded(Cursor::new(vec![0; 9]), 8).is_err());
    }
    #[test]
    fn archive_paths_and_indices_are_validated() {
        for path in [
            "../app-config.json",
            "overrides/../../evil.js",
            "profiles/C:\\x.yaml",
            "profiles/12345678.yaml:stream",
        ] {
            assert!(validate_archive(&archive(&[(path, b"x")])).is_err());
        }
        assert!(validate_archive(&archive(&[(
            "profiles/index.json",
            br#"{"active":"12345678","items":[]}"#
        )]))
        .is_err());
        assert!(validate_archive(&archive(&[(
            "overrides/index.json",
            br#"[{"id":"../../bad","name":"bad","kind":"js","enabled":true}]"#
        )]))
        .is_err());
        assert!(validate_archive(&archive(&[
            ("overrides/index.json", b"[]"),
            ("overrides/aabbccdd.yaml", b"{}"),
            ("overrides/AABBCCDD.yaml", b"{}")
        ]))
        .is_err());
    }
    #[test]
    fn expanded_size_limit_rejects_compressed_bomb() {
        let large = vec![b' '; MAX_FILE as usize + 1];
        assert!(validate_archive(&archive(&[("app-config.json", &large)])).is_err());
    }
    #[test]
    fn invalid_archive_does_not_touch_existing_files() {
        let root = dir();
        std::fs::write(root.join("app-config.json"), b"original").unwrap();
        assert!(restore(
            &root,
            &archive(&[("app-config.json", b"{broken")]),
            Default::default()
        )
        .is_err());
        assert_eq!(
            std::fs::read(root.join("app-config.json")).unwrap(),
            b"original"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn commit_failure_rolls_back_earlier_parts() {
        let root = dir();
        std::fs::create_dir(root.join("profiles")).unwrap();
        std::fs::write(root.join("profiles/old"), b"original").unwrap();
        std::fs::write(root.join("app-config.json"), b"old settings").unwrap();
        let zip = archive(&[
            ("profiles/index.json", br#"{"active":null,"items":[]}"#),
            ("app-config.json", b"{}"),
        ]);
        assert!(restore_inner(&root, &zip, Default::default(), Some(1)).is_err());
        assert_eq!(
            std::fs::read(root.join("profiles/old")).unwrap(),
            b"original"
        );
        assert_eq!(
            std::fs::read(root.join("app-config.json")).unwrap(),
            b"old settings"
        );
        assert!(!root.join(JOURNAL).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn success_preserves_local_credentials() {
        let root = dir();
        let mut credentials = crate::settings::WebdavSettings::default();
        credentials.password = "local-password".into();
        restore(&root, &archive(&[("app-config.json", b"{}")]), credentials).unwrap();
        let settings: crate::settings::AppSettings =
            serde_json::from_slice(&std::fs::read(root.join("app-config.json")).unwrap()).unwrap();
        assert_eq!(settings.webdav.password, "local-password");
        assert!(!root.join(JOURNAL).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn recovery_after_partial_commit_restores_previous_tree() {
        let root = dir();
        let stage = ".restore-0123456789abcdef0123456789abcdef";
        std::fs::create_dir_all(root.join(stage).join("previous/profiles")).unwrap();
        std::fs::write(root.join(stage).join("previous/profiles/old"), b"old").unwrap();
        std::fs::create_dir(root.join("profiles")).unwrap();
        std::fs::write(root.join("profiles/new"), b"new").unwrap();
        let journal = Journal {
            stage: stage.into(),
            parts: vec![Part {
                name: "profiles".into(),
                existed: true,
            }],
        };
        std::fs::write(root.join(JOURNAL), serde_json::to_vec(&journal).unwrap()).unwrap();
        recover_pending(&root).unwrap();
        assert_eq!(std::fs::read(root.join("profiles/old")).unwrap(), b"old");
        assert!(!root.join("profiles/new").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cleanup_removes_pre_journal_orphans() {
        let root = dir();
        let stage = ".restore-0123456789abcdef0123456789abcdef";
        std::fs::create_dir_all(root.join(stage).join("next/profiles")).unwrap();
        std::fs::write(
            root.join(stage).join("next/profiles/12345678.yaml"),
            b"partial",
        )
        .unwrap();
        recover_pending(&root).unwrap();
        assert!(!root.join(stage).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cleanup_removes_post_commit_orphans() {
        let root = dir();
        let stage = ".restore-fedcba9876543210fedcba9876543210";
        std::fs::create_dir_all(root.join(stage).join("previous/profiles")).unwrap();
        std::fs::write(
            root.join(stage).join("previous/profiles/87654321.yaml"),
            b"old",
        )
        .unwrap();
        std::fs::create_dir_all(root.join(stage).join("next")).unwrap();
        std::fs::write(root.join(stage).join("next/app-config.json"), b"{}").unwrap();
        recover_pending(&root).unwrap();
        assert!(!root.join(stage).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cleanup_preserves_unrelated_names() {
        let root = dir();
        for name in [
            ".restore-0123456789ABCDEF0123456789ABCDEF",
            ".restore-0123456789abcdef0123456789abcde",
            ".restore-0123456789abcdef0123456789abcdefg",
            ".restore-notgeneratedname0123456789ab",
            ".restore-zzzz456789abcdef0123456789abcdef",
        ] {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
        recover_pending(&root).unwrap();
        for name in [
            ".restore-0123456789ABCDEF0123456789ABCDEF",
            ".restore-0123456789abcdef0123456789abcde",
            ".restore-0123456789abcdef0123456789abcdefg",
            ".restore-notgeneratedname0123456789ab",
            ".restore-zzzz456789abcdef0123456789abcdef",
        ] {
            assert!(root.join(name).exists(), "{name} 应当保留");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn stray_file_with_stage_name_is_kept_and_reported() {
        let root = dir();
        let stage = ".restore-0123456789abcdef0123456789abcdef";
        std::fs::write(root.join(stage), b"not a directory").unwrap();
        assert!(recover_pending(&root).is_err());
        assert!(root.join(stage).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn corrupt_journal_preserves_stage() {
        let root = dir();
        let stage = ".restore-0123456789abcdef0123456789abcdef";
        std::fs::create_dir_all(root.join(stage).join("next/profiles")).unwrap();
        std::fs::write(
            root.join(stage).join("next/profiles/12345678.yaml"),
            b"partial",
        )
        .unwrap();
        for journal in [
            b"{broken".as_slice(),
            b"[]".as_slice(),
            b"not utf8 \xff\xfe".as_slice(),
        ] {
            std::fs::write(root.join(JOURNAL), journal).unwrap();
            assert!(recover_pending(&root).is_err());
            assert!(root.join(stage).exists(), "损坏日志必须保留暂存目录");
            assert!(root.join(JOURNAL).exists());
        }
        std::fs::write(
            root.join(JOURNAL),
            serde_json::to_vec(&Journal {
                stage: "../escape".into(),
                parts: vec![Part {
                    name: "profiles".into(),
                    existed: true,
                }],
            })
            .unwrap(),
        )
        .unwrap();
        assert!(recover_pending(&root).is_err());
        assert!(root.join(stage).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn ambiguous_stage_content_is_kept_and_reported() {
        let root = dir();
        let stage = ".restore-0123456789abcdef0123456789abcdef";
        std::fs::create_dir_all(root.join(stage).join("next/profiles")).unwrap();
        std::fs::write(root.join(stage).join("next/profiles/12345678.yaml"), b"ok").unwrap();
        std::fs::write(root.join(stage).join("next/unexpected.txt"), b"?").unwrap();
        assert!(recover_pending(&root).is_err());
        assert!(root.join(stage).exists(), "内容不明的暂存目录应保留");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn empty_orphan_stage_is_removed() {
        let root = dir();
        let stage = ".restore-0123456789abcdef0123456789abcdef";
        std::fs::create_dir_all(root.join(stage)).unwrap();
        recover_pending(&root).unwrap();
        assert!(!root.join(stage).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn fresh_install_without_data_directory_needs_no_recovery() {
        let root = dir();
        std::fs::remove_dir(&root).unwrap();
        recover_pending(&root).unwrap();
        assert!(!root.exists());
    }
    #[test]
    fn interrupted_atomic_stage_files_are_cleaned() {
        let root = dir();
        let stage = root.join(".restore-0123456789abcdef0123456789abcdef");
        std::fs::create_dir_all(stage.join("next/overrides")).unwrap();
        std::fs::write(
            stage.join("next/overrides/aabbccdd.yaml.tmp-12345678"),
            b"partial",
        )
        .unwrap();
        std::fs::write(stage.join("next/app-config.json.tmp-12345678"), b"partial").unwrap();
        recover_pending(&root).unwrap();
        assert!(!stage.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
