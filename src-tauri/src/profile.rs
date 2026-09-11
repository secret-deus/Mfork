//! Subscription profiles: download, storage, and config generation.

use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::core::CoreState;
use crate::settings::{write_atomic, AppSettings};

/// Periodic auto-update: refresh due subscriptions; when the active one
/// changed and the core is running, regenerate and tell the frontend to
/// hot-reload (event payload = config path).
pub fn auto_update_tick(app: &AppHandle) {
    let Ok(idx) = load_index(app) else { return };
    let now = now_secs();
    let mut updated_any = false;
    let mut active_updated = false;
    for item in &idx.items {
        if item.url.is_some()
            && item.interval_hours > 0
            && now - item.updated_at >= (item.interval_hours as i64) * 3600
        {
            match update_by_id(app, &item.id) {
                Ok(_) => {
                    updated_any = true;
                    if idx.active.as_deref() == Some(item.id.as_str()) {
                        active_updated = true;
                    }
                }
                Err(e) => {
                    let _ = app.emit("profile-update-failed", format!("{}: {e}", item.name));
                }
            }
        }
    }
    if updated_any {
        let mut payload: Option<String> = None;
        if active_updated {
            let state = app.state::<CoreState>();
            let api = state.api.lock().unwrap().clone();
            if let Some(api) = api {
                if let Ok(p) = generate_config(app, api.port, &api.secret, api.mixed_port) {
                    payload = Some(p.to_string_lossy().to_string());
                }
            }
        }
        let _ = app.emit("profiles-auto-updated", payload);
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
    Ok(profiles_dir(app)?.join(format!("{id}.yaml")))
}

fn load_index(app: &AppHandle) -> Result<ProfilesIndex, String> {
    let p = profiles_dir(app)?.join("index.json");
    if !p.exists() {
        return Ok(ProfilesIndex::default());
    }
    let s = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
    serde_json::from_str(s.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())
}

fn save_index(app: &AppHandle, idx: &ProfilesIndex) -> Result<(), String> {
    let p = profiles_dir(app)?.join("index.json");
    write_atomic(&p, serde_json::to_string_pretty(idx).unwrap().as_bytes())
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
            v.split("filename=")
                .nth(1)
                .map(|s| s.trim_matches(|c| c == '"' || c == '\'' || c == ' ').to_string())
        })
        .filter(|s| !s.is_empty());
    let body = resp.text().map_err(|e| e.to_string())?;

    let doc: Value =
        serde_yaml::from_str(&body).map_err(|_| "内容不是有效的 YAML".to_string())?;
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
    load_index(&app)
}

#[tauri::command]
pub fn profile_import(app: AppHandle, url: String) -> Result<ProfileMeta, String> {
    let (body, sub, filename) = download(&url)?;
    let id: String = (0..8)
        .map(|_| char::from_digit(fastrand::u32(0..16), 16).unwrap())
        .collect();
    write_atomic(&profile_path(&app, &id)?, body.as_bytes())?;

    let name = filename
        .map(|f| f.trim_end_matches(".yaml").trim_end_matches(".yml").to_string())
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
    let mut idx = load_index(&app)?;
    if idx.active.is_none() {
        idx.active = Some(id);
    }
    idx.items.push(meta.clone());
    save_index(&app, &idx)?;
    Ok(meta)
}

pub fn update_by_id(app: &AppHandle, id: &str) -> Result<ProfileMeta, String> {
    let mut idx = load_index(app)?;
    let item = idx
        .items
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or("订阅不存在")?;
    let url = item.url.clone().ok_or("本地配置无法更新")?;
    let (body, sub, _) = download(&url)?;
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
    let body = std::fs::read_to_string(&path).map_err(|e| format!("读取失败: {e}"))?;
    let body = body.trim_start_matches('\u{feff}').to_string();
    let doc: Value = serde_yaml::from_str(&body).map_err(|_| "内容不是有效的 YAML".to_string())?;
    let map = doc.as_mapping().ok_or("内容不是有效的 Clash 配置")?;
    if !map.contains_key("proxies") && !map.contains_key("proxy-providers") {
        return Err("内容缺少 proxies / proxy-providers，不像 Clash 配置".into());
    }
    let id: String = (0..8)
        .map(|_| char::from_digit(fastrand::u32(0..16), 16).unwrap())
        .collect();
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
    let mut idx = load_index(&app)?;
    if idx.active.is_none() {
        idx.active = Some(id);
    }
    idx.items.push(meta.clone());
    save_index(&app, &idx)?;
    Ok(meta)
}

#[tauri::command]
pub fn profile_set_interval(app: AppHandle, id: String, hours: u32) -> Result<(), String> {
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
) -> Result<Option<String>, String> {
    let api = state.api.lock().unwrap().clone();
    match api {
        Some(api) => {
            let path = generate_config(&app, api.port, &api.secret, api.mixed_port)?;
            Ok(Some(path.to_string_lossy().to_string()))
        }
        None => Ok(None),
    }
}

#[tauri::command]
pub fn profile_delete(app: AppHandle, id: String) -> Result<(), String> {
    let mut idx = load_index(&app)?;
    idx.items.retain(|p| p.id != id);
    if idx.active.as_deref() == Some(id.as_str()) {
        idx.active = None;
    }
    let _ = std::fs::remove_file(profile_path(&app, &id)?);
    save_index(&app, &idx)
}

/// Set the active profile and regenerate the kernel config. Returns the
/// generated config path (for the frontend to hot-reload via PUT /configs)
/// when the core is running.
#[tauri::command]
pub fn profile_select(
    app: AppHandle,
    state: State<'_, CoreState>,
    id: String,
) -> Result<Option<String>, String> {
    let mut idx = load_index(&app)?;
    if !idx.items.iter().any(|p| p.id == id) {
        return Err("订阅不存在".into());
    }
    idx.active = Some(id);
    save_index(&app, &idx)?;

    let api = state.api.lock().unwrap().clone();
    match api {
        Some(api) => {
            let path = generate_config(&app, api.port, &api.secret, api.mixed_port)?;
            Ok(Some(path.to_string_lossy().to_string()))
        }
        None => Ok(None),
    }
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
            let text = std::fs::read_to_string(&p)
                .map_err(|e| format!("无法读取当前订阅「{name}」: {e}"))?;
            let v: Value = serde_yaml::from_str(text.trim_start_matches('\u{feff}'))
                .map_err(|e| format!("当前订阅「{name}」不是有效的 YAML: {e}"))?;
            v.as_mapping()
                .cloned()
                .ok_or_else(|| format!("当前订阅「{name}」不是有效的 Clash 配置"))?
        }
    };

    // overrides run on the raw profile, before GUI-managed fields
    doc = crate::override_::apply_all(app, doc)?;

    let s = crate::settings::load(app);
    let doc = inject_managed(doc, &s, api_port, secret, mixed_port);

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

/// Pure step of config generation: strip the fields the GUI owns (or that
/// must never come from a subscription) and inject the managed values.
pub fn inject_managed(
    mut doc: Mapping,
    s: &AppSettings,
    api_port: u16,
    secret: &str,
    mixed_port: u16,
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
    let fpm = if cfg!(target_os = "android") { "off" } else { "always" };
    doc.insert("find-process-mode".into(), fpm.into());

    // GUI-managed TUN / DNS (settings page); the raw profile's tun block is
    // always stripped above, its dns block survives unless the user opts in
    if s.tun.enable {
        let mut tun = Mapping::new();
        tun.insert("enable".into(), true.into());
        tun.insert("stack".into(), s.tun.stack.as_str().into());
        tun.insert("auto-route".into(), s.tun.auto_route.into());
        tun.insert("strict-route".into(), s.tun.strict_route.into());
        tun.insert("mtu".into(), s.tun.mtu.into());
        if s.tun.dns_hijack {
            tun.insert(
                "dns-hijack".into(),
                Value::Sequence(vec![Value::String("any:53".into())]),
            );
        }
        doc.insert("tun".into(), Value::Mapping(tun));
    }
    if s.dns.enable {
        let list = |v: &[String]| {
            Value::Sequence(v.iter().map(|x| Value::String(x.clone())).collect())
        };
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
    use super::*;

    fn yaml(s: &str) -> Mapping {
        serde_yaml::from_str(s).unwrap()
    }

    fn get<'a>(m: &'a Mapping, k: &str) -> Option<&'a Value> {
        m.get(Value::String(k.into()))
    }

    #[test]
    fn sub_info_header_parses_floats_and_ignores_junk() {
        let i = parse_sub_info("upload=123; download=4.5e9; total=1073741824000; expire=1735689600; foo=bar");
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
        let out = inject_managed(doc, &s, 9999, "s3cret", 7897);
        assert_eq!(get(&out, "mixed-port").unwrap().as_u64(), Some(7897));
        assert!(get(&out, "port").is_none());
        assert!(get(&out, "socks-port").is_none());
        assert_eq!(get(&out, "external-controller").unwrap().as_str(), Some("127.0.0.1:9999"));
        assert_eq!(get(&out, "secret").unwrap().as_str(), Some("s3cret"));
        assert_eq!(get(&out, "allow-lan").unwrap().as_bool(), Some(false));
        assert_eq!(get(&out, "bind-address").unwrap().as_str(), Some("127.0.0.1"));
        assert_eq!(get(&out, "ipv6").unwrap().as_bool(), Some(false));
        assert_eq!(get(&out, "find-process-mode").unwrap().as_str(), Some("always"));
        // TUN is always GUI-owned: stripped when disabled in settings
        assert!(get(&out, "tun").is_none());
        // dns survives unless the user opts into the GUI dns
        assert_eq!(get(&out, "dns").unwrap()["nameserver"][0].as_str(), Some("1.1.1.1"));
        // mode / log-level are kept from the profile
        assert_eq!(get(&out, "mode").unwrap().as_str(), Some("global"));
        assert_eq!(get(&out, "log-level").unwrap().as_str(), Some("debug"));
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
        let out = inject_managed(yaml("proxies: []"), &s, 1, "x", 2);
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
        assert!(get(&out, "geox-url").unwrap()["mmdb"].as_str().unwrap().contains("geoip.metadb"));
        assert_eq!(get(&out, "mode").unwrap().as_str(), Some("rule"));
        assert_eq!(get(&out, "log-level").unwrap().as_str(), Some("info"));
    }
}
