//! Subscription profiles: download, storage, and config generation.

use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::core::CoreState;
use crate::settings::{normalize_process_rules, write_atomic, AppSettings, ProcessRule};

const MAX_PROFILE_BYTES: u64 = 20 * 1024 * 1024;
fn read_text_capped(reader: impl std::io::Read, max: u64) -> Result<String, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    reader
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err("订阅或索引超过大小限制，未覆盖原配置".into());
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

fn valid_id(id: &str) -> bool {
    id.len() == 8 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
fn validate_index(idx: &ProfilesIndex) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    for item in &idx.items {
        if !valid_id(&item.id) || !ids.insert(item.id.as_str()) {
            return Err("订阅索引包含非法或重复 ID".into());
        }
    }
    if idx.active.as_deref().is_some_and(|id| !ids.contains(id)) {
        return Err("当前订阅不在索引中".into());
    }
    Ok(())
}

fn new_id(app: &AppHandle, idx: &ProfilesIndex) -> Result<String, String> {
    for _ in 0..32 {
        let mut random = [0u8; 4];
        getrandom::getrandom(&mut random).map_err(|e| e.to_string())?;
        let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
        if !idx.items.iter().any(|m| m.id == id) && !profile_path(app, &id)?.exists() {
            return Ok(id);
        }
    }
    Err("无法分配唯一订阅 ID".into())
}

fn confirmed_mode(api: &crate::core::ApiInfo) -> Result<String, String> {
    let resp = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?
        .get(format!("http://127.0.0.1:{}/configs", api.port))
        .bearer_auth(&api.secret)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let value: serde_json::Value =
        serde_json::from_str(&read_text_capped(resp, 1024 * 1024)?).map_err(|e| e.to_string())?;
    let mode = value["mode"].as_str().ok_or("内核未返回当前模式")?;
    if !matches!(mode, "rule" | "global" | "direct") {
        return Err("内核当前模式无效".into());
    }
    Ok(mode.into())
}

/// Periodic auto-update: refresh due subscriptions; when the active one
/// changed, request frontend regeneration inside its serialized apply queue.
/// Never pre-generate a shared file that can race a foreground hot reload.
pub fn auto_update_tick(app: &AppHandle) {
    let idx = {
        let _guard = crate::settings::data_lock();
        let Ok(idx) = load_index(app) else { return };
        idx
    };
    let now = now_secs();
    let mut updated_any = false;
    for item in &idx.items {
        if item.url.is_some()
            && item.interval_hours > 0
            && now - item.updated_at >= (item.interval_hours as i64) * 3600
        {
            match update_by_id(app, &item.id) {
                Ok(_) => {
                    updated_any = true;
                }
                Err(e) => {
                    let _ = app.emit("profile-update-failed", format!("{}: {e}", item.name));
                }
            }
        }
    }
    if updated_any {
        let _ = app.emit("profiles-auto-updated", true);
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct SubInfo {
    pub upload: i64,
    pub download: i64,
    pub total: i64,
    pub expire: i64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ProfileMeta {
    pub id: String,
    pub name: String,
    pub url: Option<String>,
    pub updated_at: i64,
    pub sub: Option<SubInfo>,
    /// auto-update interval in hours; 0 = off
    #[serde(default)]
    pub interval_hours: u32,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ProfilesIndex {
    pub active: Option<String>,
    pub items: Vec<ProfileMeta>,
}

fn profiles_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("profiles");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn profile_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("订阅 ID 非法".into());
    }
    Ok(profiles_dir(app)?.join(format!("{id}.yaml")))
}

fn load_index(app: &AppHandle) -> Result<ProfilesIndex, String> {
    let p = profiles_dir(app)?.join("index.json");
    if !p.exists() {
        return Ok(ProfilesIndex::default());
    }
    let s = read_text_capped(
        std::fs::File::open(&p).map_err(|e| e.to_string())?,
        4 * 1024 * 1024,
    )?;
    let idx = serde_json::from_str(s.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    validate_index(&idx)?;
    Ok(idx)
}

fn save_index(app: &AppHandle, idx: &ProfilesIndex) -> Result<(), String> {
    validate_index(idx)?;
    let p = profiles_dir(app)?.join("index.json");
    let data = serde_json::to_vec_pretty(idx).map_err(|e| e.to_string())?;
    if data.len() > 4 * 1024 * 1024 {
        return Err("订阅索引过大".into());
    }
    write_atomic(&p, &data)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn parse_sub_info(header: &str) -> SubInfo {
    let mut info = SubInfo::default();
    for part in header.split(';') {
        let mut kv = part.trim().splitn(2, '=');
        let (k, v) = (kv.next().unwrap_or(""), kv.next().unwrap_or(""));
        let n = v.trim().parse::<f64>().unwrap_or(0.0) as i64;
        match k.trim() {
            "upload" => info.upload = n,
            "download" => info.download = n,
            "total" => info.total = n,
            "expire" => info.expire = n,
            _ => {}
        }
    }
    info
}

/// Download a subscription with the clash.meta UA and basic validation.
fn download(url: &str) -> Result<(String, Option<SubInfo>, Option<String>), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("clash.meta")
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }
    let sub = resp
        .headers()
        .get("subscription-userinfo")
        .and_then(|v| v.to_str().ok())
        .map(parse_sub_info);
    let filename = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.split("filename=").nth(1).map(|s| {
                s.trim_matches(|c| c == '"' || c == '\'' || c == ' ')
                    .to_string()
            })
        })
        .filter(|s| !s.is_empty());
    if resp.content_length().is_some_and(|n| n > MAX_PROFILE_BYTES) {
        return Err("订阅超过 20 MiB 限制".into());
    }
    let body = read_text_capped(resp, MAX_PROFILE_BYTES)?;
    let doc = crate::config_limits::parse_yaml(&body)?;
    let map = doc
        .as_mapping()
        .ok_or("内容不是有效的 Clash 配置".to_string())?;
    if !map.contains_key("proxies") && !map.contains_key("proxy-providers") {
        return Err("内容缺少 proxies / proxy-providers，不像 Clash 订阅".into());
    }
    Ok((body, sub, filename))
}

#[tauri::command]
pub fn profiles_list(app: AppHandle) -> Result<ProfilesIndex, String> {
    let _guard = crate::settings::data_lock();
    load_index(&app)
}

#[tauri::command]
pub fn profile_import(app: AppHandle, url: String) -> Result<ProfileMeta, String> {
    let (body, sub, filename) = download(&url)?;
    let _guard = crate::settings::data_lock();
    let mut idx = load_index(&app)?;
    let id = new_id(&app, &idx)?;
    write_atomic(&profile_path(&app, &id)?, body.as_bytes())?;

    let name = filename
        .map(|f| {
            f.trim_end_matches(".yaml")
                .trim_end_matches(".yml")
                .to_string()
        })
        .unwrap_or_else(|| {
            url.split("://")
                .nth(1)
                .and_then(|s| s.split('/').next())
                .unwrap_or("订阅")
                .to_string()
        });

    let meta = ProfileMeta {
        id: id.clone(),
        name,
        url: Some(url),
        updated_at: now_secs(),
        sub,
        interval_hours: 24,
    };
    if idx.active.is_none() {
        idx.active = Some(id);
    }
    idx.items.push(meta.clone());
    save_index(&app, &idx)?;
    Ok(meta)
}

pub fn update_by_id(app: &AppHandle, id: &str) -> Result<ProfileMeta, String> {
    let (url, updated_at, epoch) = {
        let _guard = crate::settings::data_lock();
        let idx = load_index(app)?;
        let item = idx.items.iter().find(|p| p.id == id).ok_or("订阅不存在")?;
        (
            item.url.clone().ok_or("本地配置无法更新")?,
            item.updated_at,
            crate::backup::restore_epoch(),
        )
    };
    let (body, sub, _) = download(&url)?;
    let _guard = crate::settings::data_lock();
    if epoch != crate::backup::restore_epoch() {
        return Err("下载期间配置已恢复，取消旧订阅写入，请重试".into());
    }
    let mut idx = load_index(app)?;
    let item = idx
        .items
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or("订阅已删除")?;
    if item.url.as_deref() != Some(url.as_str()) || item.updated_at != updated_at {
        return Err("订阅已变化，请重试更新".into());
    }
    write_atomic(&profile_path(app, id)?, body.as_bytes())?;
    item.updated_at = now_secs();
    if sub.is_some() {
        item.sub = sub;
    }
    let meta = item.clone();
    save_index(app, &idx)?;
    Ok(meta)
}

#[tauri::command]
pub fn profile_update(app: AppHandle, id: String) -> Result<ProfileMeta, String> {
    update_by_id(&app, &id)
}

#[tauri::command]
pub fn profile_import_local(app: AppHandle, path: String) -> Result<ProfileMeta, String> {
    let body = read_text_capped(
        std::fs::File::open(&path).map_err(|e| e.to_string())?,
        MAX_PROFILE_BYTES,
    )?;
    let body = body.trim_start_matches('\u{feff}').to_string();
    let doc = crate::config_limits::parse_yaml(&body)?;
    let map = doc.as_mapping().ok_or("内容不是有效的 Clash 配置")?;
    if !map.contains_key("proxies") && !map.contains_key("proxy-providers") {
        return Err("内容缺少 proxies / proxy-providers，不像 Clash 配置".into());
    }
    let _guard = crate::settings::data_lock();
    let mut idx = load_index(&app)?;
    let id = new_id(&app, &idx)?;
    write_atomic(&profile_path(&app, &id)?, body.as_bytes())?;
    let name = std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("本地配置")
        .to_string();
    let meta = ProfileMeta {
        id: id.clone(),
        name,
        url: None,
        updated_at: now_secs(),
        sub: None,
        interval_hours: 0,
    };
    if idx.active.is_none() {
        idx.active = Some(id);
    }
    idx.items.push(meta.clone());
    save_index(&app, &idx)?;
    Ok(meta)
}

#[tauri::command]
pub fn profile_set_interval(app: AppHandle, id: String, hours: u32) -> Result<(), String> {
    let _guard = crate::settings::data_lock();
    let mut idx = load_index(&app)?;
    let item = idx
        .items
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or("订阅不存在")?;
    item.interval_hours = hours;
    save_index(&app, &idx)
}

/// Regenerate the kernel config with current settings; returns the path
/// for the frontend to hot-reload, or None when the core is not running.
#[tauri::command]
pub fn config_regenerate(
    app: AppHandle,
    state: State<'_, CoreState>,
    mode: Option<String>,
) -> Result<Option<String>, String> {
    let _guard = crate::settings::data_lock();
    let api = state.api.lock().unwrap().clone();
    match api {
        Some(api) => {
            let mode = match mode {
                Some(mode) => mode,
                None => confirmed_mode(&api)?,
            };
            let path = generate_config_with_mode(
                &app,
                api.port,
                &api.secret,
                api.mixed_port,
                Some(&mode),
            )?;
            Ok(Some(path.to_string_lossy().to_string()))
        }
        None => Ok(None),
    }
}

/// Pure step of config generation: prepend the GUI-managed process-name
/// rules to `doc`'s `rules`. Rules are normalized again so a config or a
/// backup coming from disk can never inject a broken rule. Only enabled
/// rules are inserted, and every target must exist in the final `proxies` or
/// `proxy-groups` list — an enabled rule pointing at a missing target is an
/// error, never a silent DIRECT fallback. Insertion order = settings order,
/// all of them ahead of any existing rule.
pub fn inject_process_rules(mut doc: Mapping, rules: &[ProcessRule]) -> Result<Mapping, String> {
    let rules = normalize_process_rules(rules)?;
    let targets = gather_targets(&doc);

    let mut injected: Vec<Value> = Vec::new();
    for r in rules.iter().filter(|r| r.enabled) {
        if !targets.contains(&r.target.as_str()) {
            return Err(format!(
                "进程规则「{}」指向的目标「{}」在当前订阅中不存在，请修改目标或禁用该规则",
                r.process_name, r.target
            ));
        }
        injected.push(Value::String(format!(
            "PROCESS-NAME,{},{}",
            r.process_name, r.target
        )));
    }
    if injected.is_empty() {
        return Ok(doc);
    }

    let key = Value::String("rules".into());
    match doc.get(&key) {
        // existing rules must be a sequence; anything else cannot be extended
        Some(Value::Sequence(seq)) => {
            let mut out: Vec<Value> = Vec::with_capacity(seq.len() + injected.len());
            out.append(&mut injected);
            out.extend(seq.iter().cloned());
            doc.insert(key, Value::Sequence(out));
        }
        Some(other) => {
            return Err(format!(
                "当前订阅的 rules 字段不是列表（实际为 {other:?}），无法写入进程规则"
            ));
        }
        None => {
            doc.insert(key, Value::Sequence(injected));
        }
    }
    Ok(doc)
}

/// Exact valid targets for an injected rule: the mihomo builtins DIRECT and
/// REJECT, proxy group names, then explicit proxy names (the latter two only
/// when their container is a real sequence of maps).
fn gather_targets(doc: &Mapping) -> Vec<&str> {
    let mut out: Vec<&str> = vec!["DIRECT", "REJECT"];
    for key in ["proxy-groups", "proxies"] {
        if let Some(Value::Sequence(seq)) = doc.get(Value::String(key.into())) {
            for item in seq {
                if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                    out.push(name);
                }
            }
        }
    }
    out
}

#[tauri::command]
pub fn profile_delete(app: AppHandle, id: String) -> Result<(), String> {
    let _guard = crate::settings::data_lock();
    let mut idx = load_index(&app)?;
    idx.items.retain(|p| p.id != id);
    if idx.active.as_deref() == Some(id.as_str()) {
        idx.active = None;
    }
    save_index(&app, &idx)?;
    let _ = std::fs::remove_file(profile_path(&app, &id)?);
    Ok(())
}

/// Select on disk only. The frontend generates and applies inside its core queue.
#[tauri::command]
pub fn profile_select(app: AppHandle, id: String) -> Result<(), String> {
    let _guard = crate::settings::data_lock();
    let mut idx = load_index(&app)?;
    if !idx.items.iter().any(|p| p.id == id) {
        return Err("订阅不存在".into());
    }
    idx.active = Some(id);
    save_index(&app, &idx)?;
    Ok(())
}

/// Build the final kernel config: active profile with GUI-managed fields
/// injected on top. Only the *absence* of a selected profile yields the
/// DIRECT-only stub; a selected profile that cannot be read or parsed is an
/// error, never a silent fall-through to direct connections.
pub fn generate_config(
    app: &AppHandle,
    api_port: u16,
    secret: &str,
    mixed_port: u16,
) -> Result<PathBuf, String> {
    generate_config_with_mode(app, api_port, secret, mixed_port, None)
}

fn generate_config_with_mode(
    app: &AppHandle,
    api_port: u16,
    secret: &str,
    mixed_port: u16,
    mode: Option<&str>,
) -> Result<PathBuf, String> {
    let idx = load_index(app).map_err(|e| format!("订阅索引损坏（profiles/index.json）: {e}"))?;

    let mut doc: Mapping = match idx.active.as_deref() {
        None => {
            let mut m = Mapping::new();
            m.insert("proxies".into(), Value::Sequence(vec![]));
            m.insert("proxy-groups".into(), Value::Sequence(vec![]));
            m.insert(
                "rules".into(),
                Value::Sequence(vec![Value::String("MATCH,DIRECT".into())]),
            );
            m
        }
        Some(id) => {
            let name = idx
                .items
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| id.to_string());
            let p = profile_path(app, id)?;
            let text = read_text_capped(
                std::fs::File::open(&p).map_err(|e| format!("无法读取当前订阅「{name}」: {e}"))?,
                MAX_PROFILE_BYTES,
            )?;
            let v = crate::config_limits::parse_yaml(&text)
                .map_err(|e| format!("当前订阅「{name}」无效: {e}"))?;
            v.as_mapping()
                .cloned()
                .ok_or_else(|| format!("当前订阅「{name}」不是有效的 Clash 配置"))?
        }
    };

    // overrides run on the raw profile, before GUI-managed fields
    doc = crate::override_::apply_all(app, doc)?;

    let s = crate::settings::load(app);
    let tun_fd = *app.state::<CoreState>().tun_fd.lock().unwrap();
    let doc = inject_managed(doc, &s, api_port, secret, mixed_port, tun_fd);

    // process-name routing is a desktop-only feature: Android has no process
    // table access (find-process-mode stays "off" there), so the rules are
    // skipped entirely rather than inserted and ignored.
    let doc = if cfg!(target_os = "android") {
        doc
    } else {
        inject_process_rules(doc, &s.process_rules)?
    };
    let doc = preserve_runtime_mode(doc, mode)?;

    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("core");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("config.yaml");
    let text = serde_yaml::to_string(&Value::Mapping(doc)).map_err(|e| e.to_string())?;
    write_atomic(&path, text.as_bytes())?;
    Ok(path)
}

/// A settings reload must not replace the user's active mode with the subscription's mode.
/// Normal starts and profile switches pass None and retain their existing behavior.
fn preserve_runtime_mode(mut doc: Mapping, mode: Option<&str>) -> Result<Mapping, String> {
    if let Some(mode) = mode {
        if !matches!(mode, "rule" | "global" | "direct") {
            return Err("无效的代理模式".into());
        }
        doc.insert("mode".into(), mode.into());
    }
    Ok(doc)
}

/// Pure step of config generation: strip the fields the GUI owns (or that
/// must never come from a subscription) and inject the managed values.
/// `tun_fd` is the Android VPN interface: when set, the core attaches to
/// that descriptor instead of creating a device and routes of its own.
pub fn inject_managed(
    mut doc: Mapping,
    s: &AppSettings,
    api_port: u16,
    secret: &str,
    mixed_port: u16,
    tun_fd: Option<i32>,
) -> Mapping {
    for key in [
        "port",
        "socks-port",
        "redir-port",
        "tproxy-port",
        "mixed-port",
        "external-controller",
        "external-controller-tls",
        "external-ui",
        "secret",
        "tun",
        "allow-lan",
        "bind-address",
        "ipv6",
        "find-process-mode",
    ] {
        doc.remove(Value::String(key.into()));
    }

    doc.insert("mixed-port".into(), mixed_port.into());
    doc.insert(
        "bind-address".into(),
        if s.allow_lan { "*" } else { "127.0.0.1" }.into(),
    );
    doc.insert("allow-lan".into(), s.allow_lan.into());
    doc.insert("ipv6".into(), s.ipv6.into());
    doc.insert(
        "external-controller".into(),
        format!("127.0.0.1:{api_port}").into(),
    );
    doc.insert("secret".into(), secret.into());
    // Android has no process table access for an unprivileged app
    let fpm = if cfg!(target_os = "android") {
        "off"
    } else {
        "always"
    };
    doc.insert("find-process-mode".into(), fpm.into());

    // GUI-managed TUN / DNS (settings page); the raw profile's tun block is
    // always stripped above, its dns block survives unless the user opts in
    if s.tun.enable {
        let mut tun = Mapping::new();
        tun.insert("enable".into(), true.into());
        tun.insert("stack".into(), s.tun.stack.as_str().into());
        if let Some(fd) = tun_fd {
            // Android owns the interface addresses. The system/mixed TCP
            // forwarder instead binds the core's fake-IP-derived address.
            tun.insert("stack".into(), "gvisor".into());
            // the OS VPN service owns the device, addresses and routes
            tun.insert("file-descriptor".into(), fd.into());
            tun.insert("auto-route".into(), false.into());
            tun.insert("auto-detect-interface".into(), false.into());
        } else {
            tun.insert("auto-route".into(), s.tun.auto_route.into());
            tun.insert("strict-route".into(), s.tun.strict_route.into());
        }
        tun.insert("mtu".into(), s.tun.mtu.into());
        // Android advertises a synthetic DNS address through VpnService.
        // Queries to that address must always be handled by the core.
        if s.tun.dns_hijack || tun_fd.is_some() {
            tun.insert(
                "dns-hijack".into(),
                Value::Sequence(vec![Value::String("any:53".into())]),
            );
        }
        doc.insert("tun".into(), Value::Mapping(tun));
    }
    // behind the VPN every DNS query lands in the tunnel: the core must
    // answer it, so a profile without a working dns block gets ours
    let profile_dns_on = doc
        .get(Value::String("dns".into()))
        .and_then(|d| d.get("enable"))
        .and_then(|e| e.as_bool())
        .unwrap_or(false);
    if s.dns.enable || (tun_fd.is_some() && !profile_dns_on) {
        let list =
            |v: &[String]| Value::Sequence(v.iter().map(|x| Value::String(x.clone())).collect());
        let mut dns = Mapping::new();
        dns.insert("enable".into(), true.into());
        dns.insert("enhanced-mode".into(), s.dns.enhanced_mode.as_str().into());
        if s.dns.enhanced_mode == "fake-ip" {
            dns.insert("fake-ip-range".into(), "198.18.0.1/16".into());
        }
        dns.insert("nameserver".into(), list(&s.dns.nameserver));
        if !s.dns.fallback.is_empty() {
            dns.insert("fallback".into(), list(&s.dns.fallback));
        }
        doc.insert("dns".into(), Value::Mapping(dns));
    }

    // Startup uses bundled/cached databases. Updates are explicitly requested
    // from Settings, never triggered by a subscription's background schedule.
    doc.insert("geo-auto-update".into(), false.into());
    // geo databases: a CDN that is reachable where raw GitHub is not,
    // unless the profile brings its own sources
    if !doc.contains_key("geox-url") {
        let base = "https://fastly.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release";
        let mut g = Mapping::new();
        g.insert("geoip".into(), format!("{base}/geoip.dat").into());
        g.insert("geosite".into(), format!("{base}/geosite.dat").into());
        g.insert("mmdb".into(), format!("{base}/geoip.metadb").into());
        g.insert("asn".into(), format!("{base}/GeoLite2-ASN.mmdb").into());
        doc.insert("geox-url".into(), Value::Mapping(g));
    }
    if !doc.contains_key("mode") {
        doc.insert("mode".into(), "rule".into());
    }
    if !doc.contains_key("log-level") {
        doc.insert("log-level".into(), "info".into());
    }
    doc
}

#[cfg(test)]
mod tests {
    #[test]
    fn streamed_body_limit_counts_actual_bytes() {
        assert_eq!(
            read_text_capped(std::io::Cursor::new(b"12345678"), 8).unwrap(),
            "12345678"
        );
        assert!(read_text_capped(std::io::Cursor::new(b"123456789"), 8).is_err());
        assert!(read_text_capped(std::io::Cursor::new([0xff]), 8).is_err());
    }

    #[test]
    fn profile_index_rejects_path_ids_and_dangling_active() {
        let mut idx = ProfilesIndex {
            active: Some("aabbccdd".into()),
            items: vec![],
        };
        assert!(validate_index(&idx).is_err());
        idx.active = None;
        assert!(validate_index(&idx).is_ok());
        for id in ["../../..", "AABBCCDD", "aabbccdd/x", ""] {
            assert!(!valid_id(id));
        }
        assert!(valid_id("aabbccdd"));
    }

    use super::*;

    fn yaml(s: &str) -> Mapping {
        serde_yaml::from_str(s).unwrap()
    }

    #[test]
    fn settings_reload_preserves_requested_mode() {
        let doc = yaml("mode: rule\nrules: [MATCH,DIRECT]");
        for mode in ["rule", "global", "direct"] {
            let out = preserve_runtime_mode(doc.clone(), Some(mode)).unwrap();
            assert_eq!(get(&out, "mode").and_then(Value::as_str), Some(mode));
            assert_eq!(get(&out, "rules"), get(&doc, "rules"));
        }
        assert_eq!(preserve_runtime_mode(doc.clone(), None).unwrap(), doc);
    }

    #[test]
    fn settings_reload_rejects_invalid_mode() {
        assert!(preserve_runtime_mode(Mapping::new(), Some("invalid")).is_err());
    }

    fn get<'a>(m: &'a Mapping, k: &str) -> Option<&'a Value> {
        m.get(Value::String(k.into()))
    }

    #[test]
    fn sub_info_header_parses_floats_and_ignores_junk() {
        let i = parse_sub_info(
            "upload=123; download=4.5e9; total=1073741824000; expire=1735689600; foo=bar",
        );
        assert_eq!(i.upload, 123);
        assert_eq!(i.download, 4_500_000_000);
        assert_eq!(i.total, 1_073_741_824_000);
        assert_eq!(i.expire, 1_735_689_600);
        let empty = parse_sub_info("");
        assert_eq!(empty.total, 0);
    }

    #[test]
    fn managed_fields_override_subscription() {
        let doc = yaml(
            "mixed-port: 7890\nport: 7891\nsocks-port: 7892\nexternal-controller: 0.0.0.0:9090\n\
             secret: leaked\nallow-lan: true\nbind-address: '*'\nipv6: true\n\
             find-process-mode: 'off'\ntun: {enable: true}\nmode: global\nlog-level: debug\n\
             dns: {enable: true, nameserver: [1.1.1.1]}\nproxies: []",
        );
        let s = AppSettings::default();
        let out = inject_managed(doc, &s, 9999, "s3cret", 7897, None);
        assert_eq!(get(&out, "mixed-port").unwrap().as_u64(), Some(7897));
        assert!(get(&out, "port").is_none());
        assert!(get(&out, "socks-port").is_none());
        assert_eq!(
            get(&out, "external-controller").unwrap().as_str(),
            Some("127.0.0.1:9999")
        );
        assert_eq!(get(&out, "secret").unwrap().as_str(), Some("s3cret"));
        assert_eq!(get(&out, "allow-lan").unwrap().as_bool(), Some(false));
        assert_eq!(
            get(&out, "bind-address").unwrap().as_str(),
            Some("127.0.0.1")
        );
        assert_eq!(get(&out, "ipv6").unwrap().as_bool(), Some(false));
        assert_eq!(
            get(&out, "find-process-mode").unwrap().as_str(),
            Some("always")
        );
        // TUN is always GUI-owned: stripped when disabled in settings
        assert!(get(&out, "tun").is_none());
        // dns survives unless the user opts into the GUI dns
        assert_eq!(
            get(&out, "dns").unwrap()["nameserver"][0].as_str(),
            Some("1.1.1.1")
        );
        // mode / log-level are kept from the profile
        assert_eq!(get(&out, "mode").unwrap().as_str(), Some("global"));
        assert_eq!(get(&out, "log-level").unwrap().as_str(), Some("debug"));
    }

    #[test]
    fn geodata_updates_are_manual_even_when_profile_enables_schedule() {
        for input in ["proxies: []", "geo-auto-update: true\ngeox-url: {mmdb: 'https://example.com/custom.mmdb'}"] {
            let out = inject_managed(yaml(input), &AppSettings::default(), 1, "x", 2, None);
            assert_eq!(get(&out, "geo-auto-update").unwrap().as_bool(), Some(false));
            if input.contains("geox-url") {
                assert_eq!(get(&out, "geox-url").unwrap()["mmdb"].as_str(), Some("https://example.com/custom.mmdb"));
            }
        }
    }

    #[test]
    fn tun_and_dns_injected_when_enabled() {
        let mut s = AppSettings::default();
        s.tun.enable = true;
        s.tun.stack = "gvisor".into();
        s.tun.dns_hijack = false;
        s.dns.enable = true;
        s.dns.enhanced_mode = "redir-host".into();
        s.dns.nameserver = vec!["https://doh.pub/dns-query".into()];
        s.dns.fallback = vec![];
        s.allow_lan = true;
        let out = inject_managed(yaml("proxies: []"), &s, 1, "x", 2, None);
        let tun = get(&out, "tun").unwrap();
        assert_eq!(tun["enable"].as_bool(), Some(true));
        assert_eq!(tun["stack"].as_str(), Some("gvisor"));
        assert!(tun.get("dns-hijack").is_none());
        let dns = get(&out, "dns").unwrap();
        assert_eq!(dns["enhanced-mode"].as_str(), Some("redir-host"));
        assert!(dns.get("fake-ip-range").is_none());
        assert!(dns.get("fallback").is_none());
        assert_eq!(get(&out, "bind-address").unwrap().as_str(), Some("*"));
        // defaults filled in for a bare profile
        assert!(get(&out, "geox-url").unwrap()["mmdb"]
            .as_str()
            .unwrap()
            .contains("geoip.metadb"));
        assert_eq!(get(&out, "mode").unwrap().as_str(), Some("rule"));
        assert_eq!(get(&out, "log-level").unwrap().as_str(), Some("info"));
    }

    #[test]
    fn vpn_fd_attaches_tun_and_forces_dns() {
        let mut s = AppSettings::default();
        s.tun.enable = true;
        s.tun.dns_hijack = false;
        // profile has a dns block that is switched off: ours must replace it
        let out = inject_managed(
            yaml("dns: {enable: false}\nproxies: []"),
            &s,
            1,
            "x",
            2,
            Some(7),
        );
        let tun = get(&out, "tun").unwrap();
        assert_eq!(tun["file-descriptor"].as_i64(), Some(7));
        assert_eq!(tun["stack"].as_str(), Some("gvisor"));
        assert_eq!(tun["dns-hijack"][0].as_str(), Some("any:53"));
        assert_eq!(tun["auto-route"].as_bool(), Some(false));
        assert_eq!(tun["auto-detect-interface"].as_bool(), Some(false));
        assert!(tun.get("strict-route").is_none());
        let dns = get(&out, "dns").unwrap();
        assert_eq!(dns["enable"].as_bool(), Some(true));
        assert_eq!(dns["enhanced-mode"].as_str(), Some("fake-ip"));
        // a profile that already runs its own dns keeps it
        let out2 = inject_managed(
            yaml("dns: {enable: true, nameserver: [9.9.9.9]}\nproxies: []"),
            &s,
            1,
            "x",
            2,
            Some(7),
        );
        assert_eq!(
            get(&out2, "dns").unwrap()["nameserver"][0].as_str(),
            Some("9.9.9.9")
        );
    }

    fn rule(name: &str, target: &str, enabled: bool) -> ProcessRule {
        ProcessRule {
            process_name: name.into(),
            target: target.into(),
            enabled,
        }
    }

    fn rules_of(doc: &Mapping) -> Vec<String> {
        get(doc, "rules")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn process_rules_are_prepended_before_domain_and_match() {
        let doc = yaml(
            "proxy-groups:\n  - {name: Auto, type: select, proxies: [DIRECT]}\n\
             proxies:\n  - {name: HK, type: ss, server: 1.2.3.4, port: 1, cipher: aes-128-gcm, password: p}\n\
             rules:\n  - DOMAIN,example.com,DIRECT\n  - MATCH,Auto\n",
        );
        let out = inject_process_rules(doc, &[rule("curl", "Auto", true)]).unwrap();
        assert_eq!(
            rules_of(&out),
            vec![
                "PROCESS-NAME,curl,Auto",
                "DOMAIN,example.com,DIRECT",
                "MATCH,Auto"
            ]
        );
    }

    #[test]
    fn process_rules_keep_existing_order_and_their_own_order() {
        let doc = yaml(
            "proxy-groups:\n  - {name: Auto, type: select, proxies: [DIRECT]}\n\
             proxies:\n  - {name: DIRECT, type: direct}\n\
             rules:\n  - DOMAIN,a.com,DIRECT\n  - DOMAIN,b.com,DIRECT\n",
        );
        let out = inject_process_rules(
            doc,
            &[rule("curl", "Auto", true), rule("git", "DIRECT", true)],
        )
        .unwrap();
        assert_eq!(
            rules_of(&out),
            vec![
                "PROCESS-NAME,curl,Auto",
                "PROCESS-NAME,git,DIRECT",
                "DOMAIN,a.com,DIRECT",
                "DOMAIN,b.com,DIRECT"
            ]
        );
    }

    #[test]
    fn process_rules_no_rules_key_still_inserted() {
        let doc = yaml("proxies:\n  - {name: DIRECT, type: direct}\n");
        let out = inject_process_rules(doc, &[rule("curl", "DIRECT", true)]).unwrap();
        assert_eq!(rules_of(&out), vec!["PROCESS-NAME,curl,DIRECT"]);
    }

    #[test]
    fn process_rules_empty_set_leaves_doc_untouched() {
        let doc = yaml("proxies: []\nrules:\n  - MATCH,DIRECT\n");
        let out = inject_process_rules(doc.clone(), &[]).unwrap();
        assert_eq!(
            out, doc,
            "no rules selected: the doc must be returned unchanged"
        );
        // all-disabled is the same as empty
        let out2 = inject_process_rules(doc.clone(), &[rule("curl", "DIRECT", false)]).unwrap();
        assert_eq!(out2, doc);
    }

    #[test]
    fn process_rules_disabled_rule_with_missing_target_is_ignored() {
        let doc = yaml("proxies: []\nrules:\n  - MATCH,DIRECT\n");
        let out = inject_process_rules(doc.clone(), &[rule("curl", "NoSuchGroup", false)]).unwrap();
        assert_eq!(out, doc);
    }

    #[test]
    fn process_rules_enabled_missing_target_is_an_error() {
        let doc = yaml("proxies: []\nrules:\n  - MATCH,DIRECT\n");
        let err = inject_process_rules(doc, &[rule("curl", "NoSuchGroup", true)]).unwrap_err();
        assert!(err.contains("curl"), "error must name the process: {err}");
        assert!(
            err.contains("NoSuchGroup"),
            "error must name the target: {err}"
        );
        assert!(err.contains("禁用"), "error must advise disabling: {err}");
    }

    #[test]
    fn process_rules_builtin_targets_are_valid() {
        let doc = yaml("proxies: []\n");
        for t in ["DIRECT", "REJECT"] {
            let out = inject_process_rules(doc.clone(), &[rule("curl", t, true)]).unwrap();
            assert_eq!(rules_of(&out), vec![format!("PROCESS-NAME,curl,{t}")]);
        }
    }

    #[test]
    fn process_rules_group_and_explicit_proxy_targets_are_valid() {
        let doc = yaml(
            "proxy-groups:\n  - {name: Auto, type: select, proxies: [HK]}\n\
             proxies:\n  - {name: HK, type: ss, server: 1.2.3.4, port: 1, cipher: aes-128-gcm, password: p}\n",
        );
        assert!(inject_process_rules(doc.clone(), &[rule("curl", "Auto", true)]).is_ok());
        assert!(inject_process_rules(doc, &[rule("curl", "HK", true)]).is_ok());
    }

    #[test]
    fn process_rules_malformed_input_is_rejected() {
        let doc = yaml("proxies: []\n");
        // invalid name (path) is rejected before touching the doc
        assert!(inject_process_rules(doc.clone(), &[rule("a/b", "DIRECT", true)]).is_err());
        // duplicate names rejected
        let dup = vec![rule("curl", "DIRECT", true), rule("CURL", "REJECT", true)];
        assert!(inject_process_rules(doc.clone(), &dup).is_err());
        // wrong rules type when adding
        let bad_rules = yaml("proxies: []\nrules: not-a-list\n");
        assert!(inject_process_rules(bad_rules, &[rule("curl", "DIRECT", true)]).is_err());
    }
}
