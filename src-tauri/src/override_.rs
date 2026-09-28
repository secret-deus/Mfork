//! Config overrides: YAML deep-merge patches and JS transform scripts,
//! applied in list order to the active profile before GUI-managed fields.
//!
//! Security (S5): JS overrides never run inside the GUI process. On desktop
//! they are serialized and sent to a restricted `--mfork-js-worker`
//! subprocess (see js_worker.rs) with CPU/memory/IO budgets enforced by the
//! OS (Windows Job Object / Unix rlimit) plus Boa runtime limits. On
//! Android/iOS a subprocess sandbox is not available, so JS overrides fail
//! closed with an explicit "not supported" error — the user is told to use
//! a YAML override instead. There is no filesystem, network or environment
//! API exposed to scripts in either path.

use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OverrideKind {
    Yaml,
    Js,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OverrideMeta {
    pub id: String,
    pub name: String,
    pub kind: OverrideKind,
    pub enabled: bool,
}

/// Historical and new override ids are exactly 8 lowercase hex chars. The
/// id becomes a file name in the overrides directory, so anything that is
/// not this exact shape must be rejected: a crafted index.json with
/// `../../foo` or absolute-path ids could otherwise make reads/writes leave
/// the overrides directory (audit B5 hardening).
pub fn valid_override_id(id: &str) -> bool {
    id.len() == 8 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn overrides_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("overrides");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn file_path(app: &AppHandle, meta: &OverrideMeta) -> Result<PathBuf, String> {
    if !valid_override_id(&meta.id) {
        return Err(format!("覆写 ID 非法: {:?}", meta.id));
    }
    let ext = match meta.kind {
        OverrideKind::Yaml => "yaml",
        OverrideKind::Js => "js",
    };
    Ok(overrides_dir(app)?.join(format!("{}.{ext}", meta.id)))
}

/// Read a UTF-8 file bounded to `max` bytes: oversized files are rejected,
/// not truncated (the size cap must never silently change a script/patch).
fn read_capped(p: &std::path::Path, max: u64, what: &str) -> Result<String, String> {
    let md = std::fs::metadata(p).map_err(|e| format!("读取{what}失败: {e}"))?;
    if md.len() > max {
        return Err(format!("{what}过大（{} 字节，上限 {max}）", md.len()));
    }
    let mut s = String::new();
    std::fs::File::open(p)
        .map_err(|e| e.to_string())?
        .take(max + 1)
        .read_to_string(&mut s)
        .map_err(|e| format!("读取{what}失败: {e}"))?;
    if s.len() as u64 > max {
        return Err(format!("{what}超过大小限制"));
    }
    Ok(s)
}

use std::io::Read as _;

/// Size caps for override content (S5 input budgets; index.json is small
/// but bounded as well: MAX_OVERRIDES entries cannot exceed this).
const MAX_OVERRIDE_BYTES: u64 = 1 << 20; // 1 MiB per script/patch file
const MAX_INDEX_BYTES: u64 = 4 << 20; // 4 MiB for the whole index
const MAX_OVERRIDES: usize = 256;

fn index_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(overrides_dir(app)?.join("index.json"))
}

fn backup_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(overrides_dir(app)?.join("index.json.bak"))
}

fn parse_index(text: &str) -> Result<Vec<OverrideMeta>, String> {
    let list: Vec<OverrideMeta> = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("覆写索引 JSON 损坏: {e}"))?;
    if list.len() > MAX_OVERRIDES {
        return Err(format!("覆写索引条目超过上限 {MAX_OVERRIDES}"));
    }
    let mut ids = std::collections::HashSet::new();
    for item in &list {
        if !valid_override_id(&item.id) || !ids.insert(&item.id) {
            return Err("覆写索引包含非法或重复 ID".into());
        }
    }
    Ok(list)
}

/// Caller holds DATA_LOCK. Missing both indices means a fresh installation;
/// a corrupt live index never silently disables all overrides.
pub fn load_list(app: &AppHandle) -> Result<Vec<OverrideMeta>, String> {
    let (list, recovered) = load_index_at(&overrides_dir(app)?)?;
    if recovered {
        let _ = app.emit(
            "overrides-index-recovered",
            "覆写索引已从有效备份恢复，损坏原件已保留",
        );
    }
    Ok(list)
}

fn load_index_at(dir: &std::path::Path) -> Result<(Vec<OverrideMeta>, bool), String> {
    let p = dir.join("index.json");
    let bak = dir.join("index.json.bak");
    let raw = match std::fs::File::open(&p) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(MAX_INDEX_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("读取索引失败（未回滚）: {e}"))?;
            if bytes.len() as u64 > MAX_INDEX_BYTES {
                return Err("索引超过大小上限，请保留原件并人工恢复 index.json.bak".into());
            }
            Some(bytes)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("读取索引失败（未回滚）: {e}")),
    };
    if raw.is_none() && !bak.exists() {
        return Ok((vec![], false));
    }
    let parse = |bytes: &[u8]| {
        std::str::from_utf8(bytes)
            .map_err(|e| e.to_string())
            .and_then(parse_index)
    };
    let validate_files = |list: &[OverrideMeta]| -> Result<(), String> {
        for meta in list {
            let ext = if meta.kind == OverrideKind::Yaml {
                "yaml"
            } else {
                "js"
            };
            if !dir.join(format!("{}.{ext}", meta.id)).is_file() {
                return Err(format!("覆写内容缺失: {}", meta.id));
            }
        }
        Ok(())
    };
    if let Some(Ok(list)) = raw.as_deref().map(parse) {
        validate_files(&list)?;
        return Ok((list, false));
    }
    let list = parse_index(&read_capped(&bak, MAX_INDEX_BYTES, "备用覆写索引")?)
        .map_err(|e| format!("覆写索引及备份不可恢复: {e}；未自动清空覆写"))?;
    validate_files(&list)?;
    if let Some(bytes) = raw {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random).map_err(|e| e.to_string())?;
        let name = random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        crate::settings::write_atomic(&dir.join(format!("index.corrupt-{name}.json")), &bytes)?;
    }
    crate::settings::write_atomic(
        &p,
        &serde_json::to_vec_pretty(&list).map_err(|e| e.to_string())?,
    )?;
    Ok((list, true))
}

/// Both versions are valid indices; content referenced by the backup is retained.
fn save_list(app: &AppHandle, list: &[OverrideMeta]) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_INDEX_BYTES {
        return Err("覆写索引过大".into());
    }
    parse_index(std::str::from_utf8(&data).map_err(|e| e.to_string())?)?;
    let p = index_path(app)?;
    let previous = if p.exists() {
        let text = read_capped(&p, MAX_INDEX_BYTES, "覆写索引")?;
        parse_index(&text)?;
        text.into_bytes()
    } else {
        data.clone()
    };
    crate::settings::write_atomic(&backup_path(app)?, &previous)?;
    crate::settings::write_atomic(&p, &data)
}

const YAML_TEMPLATE: &str = "# YAML 覆写：与订阅配置深度合并（映射递归合并，其余替换）\n\
# 特殊键：prepend-rules / append-rules / prepend-proxies / append-proxies /\n\
#         prepend-proxy-groups / append-proxy-groups\n\
# append-rules 会插在最后的 MATCH 规则之前，保证追加的规则仍能命中\n\
# 示例：\n\
# append-rules:\n\
#   - DOMAIN-SUFFIX,example.com,DIRECT\n";

const JS_TEMPLATE: &str = "// JS 覆写：实现 main(config)，返回修改后的配置对象\n\
// 脚本在受限子进程中执行：无文件/网络/环境访问，CPU/内存/输出均有预算\n\
function main(config) {\n  return config;\n}\n";

#[tauri::command]
pub fn overrides_list(app: AppHandle) -> Result<Vec<OverrideMeta>, String> {
    let _guard = crate::settings::data_lock();
    load_list(&app)
}

#[tauri::command]
pub fn override_create(
    app: AppHandle,
    name: String,
    kind: String,
    enabled: Option<bool>,
) -> Result<OverrideMeta, String> {
    let _guard = crate::settings::data_lock();
    let kind = match kind.as_str() {
        "yaml" => OverrideKind::Yaml,
        "js" => OverrideKind::Js,
        _ => return Err("kind 必须是 yaml 或 js".into()),
    };
    let mut list = load_list(&app)?;
    if list.len() >= MAX_OVERRIDES {
        return Err(format!("覆写最多 {MAX_OVERRIDES} 条"));
    }
    // 8 hex chars, same shape the id validator accepts; from the OS CSPRNG
    let dir = overrides_dir(&app)?;
    let mut chosen = None;
    for _ in 0..32 {
        let mut b = [0u8; 4];
        getrandom::getrandom(&mut b).map_err(|e| format!("随机 ID 生成失败: {e}"))?;
        let id: String = b.iter().map(|x| format!("{x:02x}")).collect();
        if !list.iter().any(|m| m.id == id)
            && !dir.join(format!("{id}.yaml")).exists()
            && !dir.join(format!("{id}.js")).exists()
        {
            chosen = Some(id);
            break;
        }
    }
    let id = chosen.ok_or("无法分配唯一覆写 ID")?;
    let meta = OverrideMeta {
        id,
        name,
        kind,
        enabled: enabled.unwrap_or(true),
    };
    let template = match meta.kind {
        OverrideKind::Yaml => YAML_TEMPLATE,
        OverrideKind::Js => JS_TEMPLATE,
    };
    crate::settings::write_atomic(&file_path(&app, &meta)?, template.as_bytes())
        .map_err(|e| e.to_string())?;
    list.push(meta.clone());
    save_list(&app, &list)?;
    Ok(meta)
}

#[tauri::command]
pub fn override_read(app: AppHandle, id: String) -> Result<String, String> {
    let _guard = crate::settings::data_lock();
    let list = load_list(&app)?;
    let meta = list.iter().find(|o| o.id == id).ok_or("覆写不存在")?;
    read_capped(&file_path(&app, meta)?, MAX_OVERRIDE_BYTES, "覆写内容")
}

#[tauri::command]
pub fn override_write(app: AppHandle, id: String, content: String) -> Result<(), String> {
    let _guard = crate::settings::data_lock();
    let list = load_list(&app)?;
    let meta = list.iter().find(|o| o.id == id).ok_or("覆写不存在")?;
    if content.len() as u64 > MAX_OVERRIDE_BYTES {
        return Err(format!("覆写内容过大，上限 {} 字节", MAX_OVERRIDE_BYTES));
    }
    if meta.kind == OverrideKind::Yaml {
        let v = crate::config_limits::parse_yaml(content.trim_start_matches('\u{feff}'))
            .map_err(|e| format!("YAML 语法错误: {e}"))?;
        if !v.is_mapping() && !v.is_null() {
            return Err("覆写内容必须是 YAML 映射".into());
        }
    }
    crate::settings::write_atomic(&file_path(&app, meta)?, content.as_bytes())
}

#[tauri::command]
pub fn override_delete(app: AppHandle, id: String) -> Result<(), String> {
    let _guard = crate::settings::data_lock();
    let mut list = load_list(&app)?;
    // Keep the content referenced by index.json.bak so recovery is coherent.
    list.retain(|o| o.id != id);
    save_list(&app, &list)
}

#[tauri::command]
pub fn override_toggle(app: AppHandle, id: String, enabled: bool) -> Result<(), String> {
    let _guard = crate::settings::data_lock();
    let mut list = load_list(&app)?;
    let meta = list.iter_mut().find(|o| o.id == id).ok_or("覆写不存在")?;
    meta.enabled = enabled;
    save_list(&app, &list)
}

#[tauri::command]
pub fn override_move(app: AppHandle, id: String, up: bool) -> Result<(), String> {
    let _guard = crate::settings::data_lock();
    let mut list = load_list(&app)?;
    let i = list.iter().position(|o| o.id == id).ok_or("覆写不存在")?;
    let j = if up { i.checked_sub(1) } else { Some(i + 1) };
    if let Some(j) = j {
        if j < list.len() {
            list.swap(i, j);
        }
    }
    save_list(&app, &list)
}

/// Apply all enabled overrides to `doc` in list order. Caller holds
/// DATA_LOCK (config generation entry points lock once around the whole
/// generation, this function never locks itself).
pub fn apply_all(app: &AppHandle, doc: Mapping) -> Result<Mapping, String> {
    let start = std::time::Instant::now();
    crate::config_limits::validate_mapping(&doc)?;
    let mut doc = doc;
    for meta in load_list(app)?.iter().filter(|o| o.enabled) {
        if start.elapsed() >= std::time::Duration::from_secs(30) {
            return Err("整轮覆写超过时间预算".into());
        }
        let content = read_capped(&file_path(app, meta)?, MAX_OVERRIDE_BYTES, "覆写内容")?;
        let content = content.trim_start_matches('\u{feff}');
        doc = match meta.kind {
            OverrideKind::Yaml => {
                let patch = crate::config_limits::parse_yaml(content)
                    .map_err(|e| format!("覆写「{}」YAML 错误: {e}", meta.name))?;
                match patch {
                    Value::Mapping(m) => {
                        let mut d = doc;
                        deep_merge(&mut d, &m);
                        d
                    }
                    Value::Null => doc,
                    _ => return Err(format!("覆写「{}」必须是 YAML 映射", meta.name)),
                }
            }
            OverrideKind::Js => {
                let json = serde_json::to_value(Value::Mapping(doc)).map_err(|e| e.to_string())?;
                let out =
                    apply_js(content, json).map_err(|e| format!("覆写「{}」{e}", meta.name))?;
                let v: Value = serde_yaml::to_value(out).map_err(|e| e.to_string())?;
                v.as_mapping()
                    .cloned()
                    .ok_or(format!("覆写「{}」main 未返回配置对象", meta.name))?
            }
        };
        crate::config_limits::validate_mapping(&doc)?;
    }
    Ok(doc)
}

/// Recursive merge: mappings merge key-by-key, everything else replaces.
/// `prepend-X` / `append-X` splice into the sequence at key `X`.
/// `append-rules` lands before the trailing `MATCH` rule, otherwise mihomo
/// would never reach the appended entries.
pub fn deep_merge(base: &mut Mapping, patch: &Mapping) {
    for (k, v) in patch {
        if let Some(key) = k.as_str() {
            let (prepend, target) = if let Some(t) = key.strip_prefix("prepend-") {
                (true, Some(t))
            } else if let Some(t) = key.strip_prefix("append-") {
                (false, Some(t))
            } else {
                (false, None)
            };
            if let (Some(target), Some(items)) = (target, v.as_sequence()) {
                let tkey = Value::String(target.to_string());
                let seq = match base.get_mut(&tkey) {
                    Some(Value::Sequence(s)) => s,
                    _ => {
                        base.insert(tkey.clone(), Value::Sequence(vec![]));
                        base.get_mut(&tkey).unwrap().as_sequence_mut().unwrap()
                    }
                };
                let at = if prepend {
                    0
                } else if target == "rules" {
                    match_rule_index(seq).unwrap_or(seq.len())
                } else {
                    seq.len()
                };
                for (i, item) in items.iter().enumerate() {
                    seq.insert(at + i, item.clone());
                }
                continue;
            }
        }
        match (base.get_mut(k), v) {
            (Some(Value::Mapping(b)), Value::Mapping(p)) => deep_merge(b, p),
            _ => {
                base.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Index of the first `MATCH` rule, which mihomo treats as the terminal
/// catch-all. `None` when the rule list has no such entry.
fn match_rule_index(rules: &[Value]) -> Option<usize> {
    rules.iter().position(|r| {
        r.as_str()
            .map(|s| s.trim_start().to_ascii_uppercase().starts_with("MATCH"))
            .unwrap_or(false)
    })
}

/// Worker protocol magic; keep in sync with js_worker.rs.
pub const JS_WORKER_MAGIC: u32 = 0x4D46_4A53; // "MFJS"

/// Request limits (S5): the whole request (script + config) is bounded so
/// the worker's input framing can never balloon memory.
pub const MAX_JS_REQUEST_BYTES: usize = 16 << 20; // 16 MiB script+config
/// Output limit: a worker returning more than this fails with a clear
/// error instead of ballooning the GUI's memory.
pub const MAX_JS_RESULT_BYTES: usize = 8 << 20; // 8 MiB
/// Wall-clock deadline per JS execution (parent side, covers spawn+IO).
pub const JS_TIMEOUT_SECS: u64 = 20;

/// One serialized JS execution request (worker protocol payload).
#[derive(Serialize, Deserialize)]
pub struct JsWorkerRequest {
    pub magic: u32,
    pub script: String,
    pub config: serde_json::Value,
}

/// Worker reply: Ok(json) or a human-readable error.
#[derive(Serialize, Deserialize)]
pub enum JsWorkerReply {
    Ok(serde_json::Value),
    Err(String),
}

/// Serialize a JS execution request with a 4-byte little-endian length
/// header + payload. Pure function: unit-tested without spawning anything
/// (the `main` binary cannot be invoked from the test harness).
pub fn encode_js_request(script: &str, config: &serde_json::Value) -> Result<Vec<u8>, String> {
    if script.len() > MAX_JS_REQUEST_BYTES {
        return Err("JS 脚本超过大小限制".into());
    }
    let req = JsWorkerRequest {
        magic: JS_WORKER_MAGIC,
        script: script.to_string(),
        config: config.clone(),
    };
    let payload = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
    if payload.len() > MAX_JS_REQUEST_BYTES {
        return Err(format!(
            "JS 覆写请求过大（{} 字节，上限 {MAX_JS_REQUEST_BYTES}）",
            payload.len()
        ));
    }
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Decode a worker reply from raw stdout bytes (length-prefixed). Pure
/// function, unit-tested.
pub fn decode_js_reply(raw: &[u8]) -> Result<serde_json::Value, String> {
    if raw.len() < 4 {
        return Err("JS worker 无有效输出（可能是超时或被系统终止）".into());
    }
    let len = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
    if len > MAX_JS_RESULT_BYTES {
        return Err(format!(
            "JS 覆写输出超过上限（{len} 字节，上限 {MAX_JS_RESULT_BYTES}）"
        ));
    }
    if raw.len() < 4 + len {
        return Err("JS worker 输出不完整".into());
    }
    if raw[4 + len..].iter().any(|b| !b.is_ascii_whitespace()) {
        return Err("JS worker 输出包含多余数据".into());
    }
    let reply: JsWorkerReply = serde_json::from_slice(&raw[4..4 + len])
        .map_err(|e| format!("JS worker 输出无法解析: {e}"))?;
    match reply {
        JsWorkerReply::Ok(v) => Ok(v),
        JsWorkerReply::Err(e) => Err(format!("JS 覆写执行失败: {e}")),
    }
}

/// Maximum current_exe path length tolerated when spawning the worker.
fn worker_program() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("定位 worker 可执行文件失败: {e}"))
}

/// Mobile platforms cannot run a restricted desktop subprocess: fail closed
/// with an explicit message instead of executing unbounded JS in-process.
#[cfg(not(desktop))]
fn js_unsupported() -> String {
    "JS 覆写在 Android/iOS 上不受支持（无法提供受限执行环境）。\
     请改用 YAML 覆写实现相同的配置修改"
        .to_string()
}

/// Run a JS override: the script must define `main(config)` returning the
/// modified config. Desktop: executed by the restricted `--mfork-js-worker`
/// subprocess of this same binary (memory/job/rlimit limits, Boa runtime
/// limits, strict IO caps, wall deadline). Mobile: unsupported, error.
pub fn apply_js(script: &str, config: serde_json::Value) -> Result<serde_json::Value, String> {
    #[cfg(not(desktop))]
    {
        let _ = (script, config);
        return Err(js_unsupported());
    }
    #[cfg(desktop)]
    {
        desktop::run_in_worker(script, config)
    }
}

#[cfg(desktop)]
mod desktop {
    use super::*;

    /// Drain stdout concurrently: waiting for exit first would deadlock on a full pipe.
    pub fn run_in_worker(
        script: &str,
        config: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        use std::io::{Read, Write};
        use std::process::{Command, Stdio};
        let input = encode_js_request(script, &config)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(JS_TIMEOUT_SECS);
        let mut cmd = Command::new(worker_program()?);
        cmd.arg(crate::js_worker::WORKER_FLAG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("JS worker 启动失败: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("JS worker 无 stdin")?;
        let writer = std::thread::spawn(move || stdin.write_all(&input));
        let stdout = child.stdout.take().ok_or("JS worker 无 stdout")?;
        let reader = std::thread::spawn(move || {
            let mut raw = Vec::new();
            stdout
                .take(MAX_JS_RESULT_BYTES as u64 + 5)
                .read_to_end(&mut raw)
                .map(|_| raw)
        });
        let completion = loop {
            match child.try_wait() {
                Ok(Some(_)) => break Ok(()),
                Err(e) => break Err(format!("等待 JS worker 失败: {e}")),
                Ok(None) => {}
            }
            if std::time::Instant::now() >= deadline {
                break Err(format!("JS 覆写执行超时（上限 {JS_TIMEOUT_SECS} 秒）"));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let _ = child.kill();
        let _ = child.wait();
        let wrote = writer.join().map_err(|_| "JS worker 输入线程失败")?;
        let raw = reader
            .join()
            .map_err(|_| "JS worker 输出线程失败")?
            .map_err(|e| e.to_string())?;
        completion?;
        if raw.len() > MAX_JS_RESULT_BYTES + 4 {
            return Err("JS 结果超过大小限制".into());
        }
        // Prefer a structured worker error (including failed OS limits) over BrokenPipe.
        let result = decode_js_reply(&raw);
        if result.is_ok() {
            wrote.map_err(|e| format!("JS 输入未完整写入: {e}"))?;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn binary_corrupt_index_recovers_without_losing_original() {
        let base = std::env::var_os("PI_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let dir = base.join(format!("mfork-index-test-{:016x}", fastrand::u64(..)));
        std::fs::create_dir(&dir).unwrap();
        let original = [0xff, 0xfe, 0x00, 0x81];
        std::fs::write(dir.join("index.json"), original).unwrap();
        std::fs::write(dir.join("index.json.bak"), b"[]").unwrap();
        let (list, recovered) = load_index_at(&dir).unwrap();
        assert!(recovered && list.is_empty());
        let aside = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("index.corrupt-")
            })
            .unwrap();
        assert_eq!(std::fs::read(aside).unwrap(), original);
        std::fs::remove_file(dir.join("index.json")).unwrap();
        assert!(
            load_index_at(&dir).unwrap().1,
            "missing live index must recover backup"
        );
        std::fs::remove_file(dir.join("index.json")).unwrap();
        std::fs::create_dir(dir.join("index.json")).unwrap();
        assert!(
            load_index_at(&dir).is_err(),
            "IO error must not overwrite live path"
        );
        assert!(dir.join("index.json").is_dir());
        std::fs::remove_dir_all(dir).unwrap();
    }

    use super::*;

    fn yaml(s: &str) -> Mapping {
        serde_yaml::from_str(s).unwrap()
    }

    #[test]
    fn merge_recurses_and_replaces() {
        let mut base = yaml("dns: {enable: true, ipv6: false}\nlog-level: info");
        let patch = yaml("dns: {ipv6: true}\nlog-level: debug");
        deep_merge(&mut base, &patch);
        let out = serde_yaml::to_string(&base).unwrap();
        assert!(out.contains("enable: true"));
        assert!(out.contains("ipv6: true"));
        assert!(out.contains("log-level: debug"));
    }

    #[test]
    fn merge_prepend_append() {
        let mut base = yaml("rules:\n  - MATCH,DIRECT");
        let patch =
            yaml("prepend-rules:\n  - DOMAIN,a.com,DIRECT\nappend-rules:\n  - DOMAIN,b.com,DIRECT");
        deep_merge(&mut base, &patch);
        let rules = base.get("rules").unwrap().as_sequence().unwrap();
        assert_eq!(rules[0].as_str().unwrap(), "DOMAIN,a.com,DIRECT");
        assert_eq!(rules[1].as_str().unwrap(), "DOMAIN,b.com,DIRECT");
        assert_eq!(rules[2].as_str().unwrap(), "MATCH,DIRECT");
    }

    #[test]
    fn append_rules_lands_before_match() {
        let mut base = yaml("rules:\n  - DOMAIN,a.com,DIRECT\n  - MATCH,Proxy");
        let patch =
            yaml("append-rules:\n  - DOMAIN-SUFFIX,b.com,DIRECT\n  - DOMAIN-SUFFIX,c.com,DIRECT");
        deep_merge(&mut base, &patch);
        let rules: Vec<&str> = base
            .get("rules")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(
            rules,
            [
                "DOMAIN,a.com,DIRECT",
                "DOMAIN-SUFFIX,b.com,DIRECT",
                "DOMAIN-SUFFIX,c.com,DIRECT",
                "MATCH,Proxy",
            ]
        );
    }

    #[test]
    fn append_other_sequences_stays_at_end() {
        let mut base = yaml("proxies:\n  - {name: a, type: direct}");
        let patch = yaml("append-proxies:\n  - {name: b, type: direct}");
        deep_merge(&mut base, &patch);
        let names: Vec<&str> = base
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .map(|v| v.get("name").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(names, ["a", "b"]);
    }

    #[test]
    fn override_ids_are_strictly_eight_hex() {
        assert!(valid_override_id("0123abcd"));
        assert!(valid_override_id("deadbeef"));
        assert!(!valid_override_id("0123ABCD"), "uppercase rejected");
        assert!(!valid_override_id("0123abc"));
        assert!(!valid_override_id("0123abcde"));
        assert!(!valid_override_id("../..abcd"));
        assert!(!valid_override_id("../../.."));
        assert!(!valid_override_id("a/bc/def"));
        assert!(!valid_override_id(""));
        assert!(!valid_override_id("0123abcg"));
    }

    #[test]
    fn js_request_roundtrip_and_limits() {
        let cfg = serde_json::json!({"proxies": []});
        let bytes = encode_js_request("function main(c){return c;}", &cfg).unwrap();
        assert_eq!(
            u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize,
            bytes.len() - 4
        );
        let req: JsWorkerRequest = serde_json::from_slice(&bytes[4..]).expect("request must parse");
        assert_eq!(req.magic, JS_WORKER_MAGIC);
        assert_eq!(req.config, cfg);

        // oversized requests are rejected before any IO
        let big = "x".repeat(MAX_JS_REQUEST_BYTES + 1);
        assert!(encode_js_request(&big, &cfg).is_err());

        // reply decoding: ok, error, junk, oversized
        let ok = serde_json::to_vec(&JsWorkerReply::Ok(cfg.clone())).unwrap();
        let mut framed = (ok.len() as u32).to_le_bytes().to_vec();
        framed.extend_from_slice(&ok);
        let v = decode_js_reply(&framed).unwrap();
        assert_eq!(v, cfg);

        let err = serde_json::to_vec(&JsWorkerReply::Err("boom".into())).unwrap();
        let mut framed_err = (err.len() as u32).to_le_bytes().to_vec();
        framed_err.extend_from_slice(&err);
        assert!(decode_js_reply(&framed_err).unwrap_err().contains("boom"));

        assert!(decode_js_reply(&[0, 0, 0]).is_err());
        let mut oversize = (u32::from_le_bytes([0xff, 0xff, 0xff, 0x7f]))
            .to_le_bytes()
            .to_vec();
        oversize.extend_from_slice(&[0u8; 8]);
        assert!(decode_js_reply(&oversize).is_err());
        let mut truncated = (10u32).to_le_bytes().to_vec();
        truncated.extend_from_slice(&[1, 2, 3]);
        assert!(decode_js_reply(&truncated).is_err());
    }

    #[test]
    fn index_rejects_corruption_traversal_and_duplicate_ids() {
        assert!(parse_index("[").is_err());
        assert!(
            parse_index(r#"[{"id":"../../..","name":"x","kind":"js","enabled":true}]"#).is_err()
        );
        let item = r#"{"id":"aabbccdd","name":"x","kind":"yaml","enabled":false}"#;
        assert!(parse_index(&format!("[{item}]")).is_ok());
        assert!(parse_index(&format!("[{item},{item}]")).is_err());
    }
}
