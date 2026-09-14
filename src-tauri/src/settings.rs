//! GUI app settings persisted in app-config.json.

use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::{AppHandle, Emitter, Manager};

/// Write `data` to `path` via a temp file + rename so a crash mid-write
/// never leaves a truncated file behind (rename replaces on every OS).
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    let tmp = path.with_file_name(name);
    std::fs::write(&tmp, data).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("replace {}: {e}", path.display())
    })
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TunSettings {
    pub enable: bool,
    /// gvisor | system | mixed
    pub stack: String,
    pub auto_route: bool,
    pub strict_route: bool,
    pub dns_hijack: bool,
    pub mtu: u32,
}

impl Default for TunSettings {
    fn default() -> Self {
        Self {
            // Android: VPN mode is what every client does out of the box;
            // desktop TUN needs the elevated service, so it stays opt-in
            enable: cfg!(target_os = "android"),
            stack: "mixed".into(),
            auto_route: true,
            strict_route: true,
            dns_hijack: true,
            mtu: 1500,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DnsSettings {
    /// when false the profile's own dns block is left untouched
    pub enable: bool,
    /// fake-ip | redir-host
    pub enhanced_mode: String,
    pub nameserver: Vec<String>,
    pub fallback: Vec<String>,
}

impl Default for DnsSettings {
    fn default() -> Self {
        Self {
            enable: false,
            enhanced_mode: "fake-ip".into(),
            nameserver: vec!["https://223.5.5.5/dns-query".into(), "https://doh.pub/dns-query".into()],
            fallback: vec!["https://1.1.1.1/dns-query".into(), "https://8.8.8.8/dns-query".into()],
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WebdavSettings {
    pub url: String,
    pub username: String,
    pub password: String,
}

/// One rule-routing entry: connections of `process_name` are sent to the
/// proxy group or proxy `target`. `enabled` lets the UI keep a rule without
/// applying it; old files without the field keep it on.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct ProcessRule {
    pub process_name: String,
    pub target: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ProcessRule {
    fn default() -> Self {
        Self {
            process_name: String::new(),
            target: String::new(),
            enabled: true,
        }
    }
}

/// Upper bound on stored rules: keeps the generated config and the settings
/// UI bounded.
const MAX_PROCESS_RULES: usize = 256;

/// Validate and canonicalize process rules coming from the frontend or from a
/// config file on disk. Trims both fields, rejects anything that could not be
/// a literal process name or a Clash rule target, and rejects duplicate
/// process names (case-insensitively) so injection is deterministic. Order,
/// casing and `enabled` are preserved.
pub fn normalize_process_rules(rules: &[ProcessRule]) -> Result<Vec<ProcessRule>, String> {
    if rules.len() > MAX_PROCESS_RULES {
        return Err(format!("进程规则最多 {MAX_PROCESS_RULES} 条"));
    }
    let mut out: Vec<ProcessRule> = Vec::with_capacity(rules.len());
    let mut seen: Vec<String> = Vec::with_capacity(rules.len());
    for (i, r) in rules.iter().enumerate() {
        let n = i + 1;
        let name = r.process_name.trim();
        let target = r.target.trim();
        if name.is_empty() {
            return Err(format!("第 {n} 条进程规则的进程名不能为空"));
        }
        if target.is_empty() {
            return Err(format!("第 {n} 条进程规则的目标不能为空"));
        }
        if name.chars().count() > 260 {
            return Err(format!("第 {n} 条进程规则的进程名超过 260 个字符"));
        }
        if target.chars().count() > 256 {
            return Err(format!("第 {n} 条进程规则的目标超过 256 个字符"));
        }
        if name.contains('/') || name.contains('\\') || name.contains('*') || name.contains('?') {
            return Err(format!(
                "第 {n} 条进程规则的进程名「{name}」不能包含路径分隔符或通配符（/ \\ * ?）"
            ));
        }
        if bad_chars(name) {
            return Err(format!(
                "第 {n} 条进程规则的进程名「{name}」不能包含逗号或控制字符"
            ));
        }
        if bad_chars(target) {
            return Err(format!(
                "第 {n} 条进程规则的目标「{target}」不能包含逗号或控制字符"
            ));
        }
        let key = name.to_lowercase();
        if seen.contains(&key) {
            return Err(format!("进程名「{name}」重复（进程名不区分大小写）"));
        }
        seen.push(key);
        out.push(ProcessRule {
            process_name: name.to_string(),
            target: target.to_string(),
            enabled: r.enabled,
        });
    }
    Ok(out)
}

/// Comma or control character anywhere in a rule field: commas would split
/// the generated `PROCESS-NAME,<name>,<target>` rule line, control characters
/// (including newlines) would corrupt the YAML/rule list.
fn bad_chars(s: &str) -> bool {
    s.chars().any(|c| c == ',' || c.is_control())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// fixed mixed port; 0 = automatic (reuse `last_mixed_port` when it is
    /// still free, otherwise pick a free one)
    pub mixed_port: u16,
    /// the port the last automatic start used; not user-facing
    pub last_mixed_port: u16,
    /// settings schema version for one-time migrations (see `migrate`).
    /// Field-level default: the container default is `SCHEMA`, which
    /// would make every pre-schema file look already migrated.
    #[serde(default)]
    pub schema: u32,
    pub allow_lan: bool,
    pub ipv6: bool,
    pub system_proxy: bool,
    pub proxy_bypass: String,
    pub silent_start: bool,
    pub tun: TunSettings,
    pub dns: DnsSettings,
    /// mihomo release channel for self-upgrade: stable | alpha
    pub core_channel: String,
    /// dark | light | system
    pub theme: String,
    /// zh-CN | zh-TW | en
    pub locale: String,
    pub webdav: WebdavSettings,
    /// process-name routing rules; empty = nothing injected
    pub process_rules: Vec<ProcessRule>,
}

/// Bump when an existing app-config.json needs a one-time adjustment.
/// 1: Android installs made before VPN mode was the default keep
///    `tun.enable = false` on disk, so connecting never asks for VPN consent.
pub const SCHEMA: u32 = 1;

fn migrate(s: &mut AppSettings) -> bool {
    let from = s.schema;
    if s.schema < 1 {
        if cfg!(target_os = "android") {
            s.tun.enable = true;
        }
        s.schema = 1;
    }
    s.schema != from
}

pub fn default_bypass() -> String {
    if cfg!(windows) {
        "localhost;127.*;192.168.*;10.*;172.16.*;<local>".into()
    } else {
        "localhost,127.0.0.1/8,192.168.0.0/16,10.0.0.0/8,172.16.0.0/12".into()
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            mixed_port: 0,
            last_mixed_port: 0,
            schema: SCHEMA,
            allow_lan: false,
            ipv6: false,
            system_proxy: false,
            proxy_bypass: default_bypass(),
            silent_start: false,
            tun: TunSettings::default(),
            dns: DnsSettings::default(),
            core_channel: "stable".into(),
            theme: "dark".into(),
            locale: "zh-CN".into(),
            webdav: WebdavSettings::default(),
            process_rules: vec![],
        }
    }
}

fn path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("app-config.json"))
}

/// Load app-config.json. A file that exists but does not parse is moved
/// aside (never silently overwritten) and the frontend is told via the
/// `settings-corrupt` event; defaults are used from then on.
pub fn load(app: &AppHandle) -> AppSettings {
    let Some(p) = path(app).filter(|p| p.exists()) else {
        return AppSettings::default();
    };
    let Ok(text) = std::fs::read_to_string(&p) else {
        return AppSettings::default();
    };
    match serde_json::from_str::<AppSettings>(text.trim_start_matches('\u{feff}')) {
        Ok(mut s) => {
            if migrate(&mut s) {
                let _ = save(app, &s);
            }
            s
        }
        Err(e) => {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let aside = p.with_file_name(format!("app-config.corrupt-{stamp}.json"));
            let _ = std::fs::rename(&p, &aside);
            let name = aside
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let _ = app.emit("settings-corrupt", format!("{e} ({name})"));
            AppSettings::default()
        }
    }
}

pub fn save(app: &AppHandle, s: &AppSettings) -> Result<(), String> {
    let p = path(app).ok_or("no data dir")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    write_atomic(&p, serde_json::to_string_pretty(s).unwrap().as_bytes())
}

#[tauri::command]
pub fn settings_get(app: AppHandle) -> AppSettings {
    load(&app)
}

/// Persist settings; re-apply the OS proxy so a changed bypass list (or a
/// toggled system_proxy flag) takes effect immediately.
#[tauri::command]
pub fn settings_set(app: AppHandle, mut settings: AppSettings) -> Result<(), String> {
    let before = load(&app);
    // backend-owned: the frontend's copy may predate the last core start
    settings.last_mixed_port = before.last_mixed_port;
    settings.schema = before.schema.max(settings.schema);
    // canonicalize before anything is persisted: an invalid rule set must
    // never reach app-config.json (generate_config would then fail to load)
    settings.process_rules = normalize_process_rules(&settings.process_rules)?;
    save(&app, &settings)?;
    let r = crate::system::sync_system_proxy(&app);
    if before.locale != settings.locale {
        crate::tray::set_locale(&app, &settings.locale);
    }
    crate::tray::refresh(&app);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_schema_file_migrates() {
        let mut s: AppSettings =
            serde_json::from_str(r#"{"tun":{"enable":false},"theme":"light"}"#).unwrap();
        assert_eq!(s.schema, 0, "missing schema must read as 0, not the container default");
        assert!(migrate(&mut s));
        assert_eq!(s.schema, SCHEMA);
        assert_eq!(s.theme, "light");
        if cfg!(target_os = "android") {
            assert!(s.tun.enable);
        }
        assert!(!migrate(&mut s), "second pass is a no-op");
    }

    #[test]
    fn fresh_defaults_are_current() {
        let mut s = AppSettings::default();
        assert_eq!(s.schema, SCHEMA);
        assert!(!migrate(&mut s));
    }

    #[test]
    fn old_settings_without_process_rules_default_to_empty() {
        let s: AppSettings = serde_json::from_str(r#"{"theme":"light"}"#).unwrap();
        assert!(s.process_rules.is_empty());
    }

    #[test]
    fn process_rule_enabled_defaults_to_true() {
        let r: ProcessRule = serde_json::from_str(r#"{"process_name":"curl","target":"P"}"#).unwrap();
        assert!(r.enabled);
        let off: ProcessRule =
            serde_json::from_str(r#"{"process_name":"curl","target":"P","enabled":false}"#).unwrap();
        assert!(!off.enabled);
        assert!(ProcessRule::default().enabled);
    }

    #[test]
    fn normalize_trims_and_preserves_order_target_and_enabled() {
        let rules = vec![
            ProcessRule {
                process_name: "  curl  ".into(),
                target: "  Proxy  ".into(),
                enabled: false,
            },
            ProcessRule {
                process_name: "Git".into(),
                target: " DIRECT ".into(),
                enabled: true,
            },
        ];
        let out = normalize_process_rules(&rules).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].process_name, "curl");
        assert_eq!(out[0].target, "Proxy");
        assert!(!out[0].enabled);
        assert_eq!(out[1].process_name, "Git", "case is preserved");
        assert_eq!(out[1].target, "DIRECT");
        assert!(out[1].enabled);
    }

    #[test]
    fn normalize_rejects_blank_name_or_target() {
        let blank_name = vec![ProcessRule {
            process_name: "   ".into(),
            target: "P".into(),
            enabled: true,
        }];
        assert!(normalize_process_rules(&blank_name)
            .unwrap_err()
            .contains("进程名不能为空"));
        let blank_target = vec![ProcessRule {
            process_name: "curl".into(),
            target: "".into(),
            enabled: true,
        }];
        assert!(normalize_process_rules(&blank_target)
            .unwrap_err()
            .contains("目标不能为空"));
    }

    #[test]
    fn normalize_rejects_overlong_fields() {
        assert!(normalize_process_rules(&[ProcessRule {
            process_name: "a".repeat(261),
            target: "P".into(),
            enabled: true
        }])
        .is_err());
        assert!(normalize_process_rules(&[ProcessRule {
            process_name: "curl".into(),
            target: "b".repeat(257),
            enabled: true
        }])
        .is_err());
        // exactly at the limit is fine
        assert!(normalize_process_rules(&[ProcessRule {
            process_name: "a".repeat(260),
            target: "b".repeat(256),
            enabled: true
        }])
        .is_ok());
    }

    #[test]
    fn normalize_rejects_paths_globs_and_separators() {
        for bad in [r"C:\Windows\curl.exe", "/usr/bin/curl", "cur*l", "cur?l"] {
            let rules = vec![ProcessRule {
                process_name: bad.into(),
                target: "P".into(),
                enabled: true,
            }];
            assert!(normalize_process_rules(&rules).is_err(), "{bad} must be rejected");
        }
        // targets may contain separators / globs, only the name is restricted
        let ok = vec![ProcessRule {
            process_name: "curl".into(),
            target: "a/b*".into(),
            enabled: true,
        }];
        assert!(normalize_process_rules(&ok).is_ok());
    }

    #[test]
    fn normalize_rejects_commas_and_control_chars() {
        let comma_name = vec![ProcessRule {
            process_name: "cur,l".into(),
            target: "P".into(),
            enabled: true,
        }];
        assert!(normalize_process_rules(&comma_name).is_err());
        let comma_target = vec![ProcessRule {
            process_name: "curl".into(),
            target: "P,Q".into(),
            enabled: true,
        }];
        assert!(normalize_process_rules(&comma_target).is_err());
        let ctrl_name = vec![ProcessRule {
            process_name: "cur\nl".into(),
            target: "P".into(),
            enabled: true,
        }];
        assert!(normalize_process_rules(&ctrl_name).is_err());
        let ctrl_target = vec![ProcessRule {
            process_name: "curl".into(),
            target: "P\tQ".into(),
            enabled: true,
        }];
        assert!(normalize_process_rules(&ctrl_target).is_err());
    }

    #[test]
    fn normalize_rejects_case_insensitive_duplicates() {
        let rules = vec![
            ProcessRule {
                process_name: "curl".into(),
                target: "A".into(),
                enabled: true,
            },
            ProcessRule {
                process_name: "CURL".into(),
                target: "B".into(),
                enabled: false,
            },
        ];
        let err = normalize_process_rules(&rules).unwrap_err();
        assert!(err.contains("重复"), "got: {err}");
    }

    #[test]
    fn normalize_rejects_too_many_rules() {
        let rules: Vec<ProcessRule> = (0..257)
            .map(|i| ProcessRule {
                process_name: format!("p{i}"),
                target: "P".into(),
                enabled: true,
            })
            .collect();
        assert!(normalize_process_rules(&rules).is_err());
        assert!(normalize_process_rules(&rules[..256]).is_ok());
    }

    #[test]
    fn normalize_empty_is_ok() {
        assert_eq!(normalize_process_rules(&[]).unwrap(), Vec::<ProcessRule>::new());
    }
}
