//! Config overrides: YAML deep-merge patches and JS transform scripts,
//! applied in list order to the active profile before GUI-managed fields.

use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

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
    let ext = match meta.kind {
        OverrideKind::Yaml => "yaml",
        OverrideKind::Js => "js",
    };
    Ok(overrides_dir(app)?.join(format!("{}.{ext}", meta.id)))
}

pub fn load_list(app: &AppHandle) -> Result<Vec<OverrideMeta>, String> {
    let p = overrides_dir(app)?.join("index.json");
    if !p.exists() {
        return Ok(vec![]);
    }
    let s = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
    serde_json::from_str(s.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())
}

fn save_list(app: &AppHandle, list: &[OverrideMeta]) -> Result<(), String> {
    let p = overrides_dir(app)?.join("index.json");
    std::fs::write(p, serde_json::to_string_pretty(list).unwrap()).map_err(|e| e.to_string())
}

const YAML_TEMPLATE: &str = "# YAML 覆写：与订阅配置深度合并（映射递归合并，其余替换）\n\
# 特殊键：prepend-rules / append-rules / prepend-proxies / append-proxies /\n\
#         prepend-proxy-groups / append-proxy-groups\n\
# append-rules 会插在最后的 MATCH 规则之前，保证追加的规则仍能命中\n\
# 示例：\n\
# append-rules:\n\
#   - DOMAIN-SUFFIX,example.com,DIRECT\n";

const JS_TEMPLATE: &str = "// JS 覆写：实现 main(config)，返回修改后的配置对象\n\
function main(config) {\n  return config;\n}\n";

#[tauri::command]
pub fn overrides_list(app: AppHandle) -> Result<Vec<OverrideMeta>, String> {
    load_list(&app)
}

#[tauri::command]
pub fn override_create(app: AppHandle, name: String, kind: String) -> Result<OverrideMeta, String> {
    let kind = match kind.as_str() {
        "yaml" => OverrideKind::Yaml,
        "js" => OverrideKind::Js,
        _ => return Err("kind 必须是 yaml 或 js".into()),
    };
    let id: String = (0..8)
        .map(|_| char::from_digit(fastrand::u32(0..16), 16).unwrap())
        .collect();
    let meta = OverrideMeta {
        id,
        name,
        kind,
        enabled: true,
    };
    let template = match meta.kind {
        OverrideKind::Yaml => YAML_TEMPLATE,
        OverrideKind::Js => JS_TEMPLATE,
    };
    std::fs::write(file_path(&app, &meta)?, template).map_err(|e| e.to_string())?;
    let mut list = load_list(&app)?;
    list.push(meta.clone());
    save_list(&app, &list)?;
    Ok(meta)
}

#[tauri::command]
pub fn override_read(app: AppHandle, id: String) -> Result<String, String> {
    let list = load_list(&app)?;
    let meta = list.iter().find(|o| o.id == id).ok_or("覆写不存在")?;
    std::fs::read_to_string(file_path(&app, meta)?).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn override_write(app: AppHandle, id: String, content: String) -> Result<(), String> {
    let list = load_list(&app)?;
    let meta = list.iter().find(|o| o.id == id).ok_or("覆写不存在")?;
    if meta.kind == OverrideKind::Yaml {
        let v: Value = serde_yaml::from_str(content.trim_start_matches('\u{feff}'))
            .map_err(|e| format!("YAML 语法错误: {e}"))?;
        if !v.is_mapping() && !v.is_null() {
            return Err("覆写内容必须是 YAML 映射".into());
        }
    }
    std::fs::write(file_path(&app, meta)?, content).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn override_delete(app: AppHandle, id: String) -> Result<(), String> {
    let mut list = load_list(&app)?;
    if let Some(meta) = list.iter().find(|o| o.id == id).cloned() {
        let _ = std::fs::remove_file(file_path(&app, &meta)?);
    }
    list.retain(|o| o.id != id);
    save_list(&app, &list)
}

#[tauri::command]
pub fn override_toggle(app: AppHandle, id: String, enabled: bool) -> Result<(), String> {
    let mut list = load_list(&app)?;
    let meta = list.iter_mut().find(|o| o.id == id).ok_or("覆写不存在")?;
    meta.enabled = enabled;
    save_list(&app, &list)
}

#[tauri::command]
pub fn override_move(app: AppHandle, id: String, up: bool) -> Result<(), String> {
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

/// Apply all enabled overrides to `doc` in list order.
pub fn apply_all(app: &AppHandle, doc: Mapping) -> Result<Mapping, String> {
    let mut doc = doc;
    for meta in load_list(app)?.iter().filter(|o| o.enabled) {
        let content = std::fs::read_to_string(file_path(app, meta)?)
            .map_err(|e| format!("覆写「{}」读取失败: {e}", meta.name))?;
        let content = content.trim_start_matches('\u{feff}');
        doc = match meta.kind {
            OverrideKind::Yaml => {
                let patch: Value = serde_yaml::from_str(content)
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
                let json = serde_json::to_value(Value::Mapping(doc))
                    .map_err(|e| e.to_string())?;
                let out = apply_js(content, json)
                    .map_err(|e| format!("覆写「{}」{e}", meta.name))?;
                let v: Value = serde_yaml::to_value(out).map_err(|e| e.to_string())?;
                v.as_mapping()
                    .cloned()
                    .ok_or(format!("覆写「{}」main 未返回配置对象", meta.name))?
            }
        };
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

/// Run a JS override: the script must define `main(config)` returning the
/// modified config. Executed in an embedded engine (no filesystem/network).
pub fn apply_js(script: &str, config: serde_json::Value) -> Result<serde_json::Value, String> {
    use boa_engine::{js_string, Context, JsValue, Source};

    let mut ctx = Context::default();
    ctx.eval(Source::from_bytes(script))
        .map_err(|e| format!("脚本错误: {e}"))?;
    let main = ctx
        .global_object()
        .get(js_string!("main"), &mut ctx)
        .map_err(|e| e.to_string())?;
    let main = main.as_callable().ok_or("脚本未定义 main 函数")?;
    let arg = JsValue::from_json(&config, &mut ctx).map_err(|e| e.to_string())?;
    let result = main
        .call(&JsValue::undefined(), &[arg], &mut ctx)
        .map_err(|e| format!("main 执行失败: {e}"))?;
    result
        .to_json(&mut ctx)
        .map_err(|e| format!("main 返回值无法序列化: {e}"))
}

#[cfg(test)]
mod tests {
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
        let patch = yaml(
            "prepend-rules:\n  - DOMAIN,a.com,DIRECT\nappend-rules:\n  - DOMAIN,b.com,DIRECT",
        );
        deep_merge(&mut base, &patch);
        let rules = base.get("rules").unwrap().as_sequence().unwrap();
        assert_eq!(rules[0].as_str().unwrap(), "DOMAIN,a.com,DIRECT");
        assert_eq!(rules[1].as_str().unwrap(), "DOMAIN,b.com,DIRECT");
        assert_eq!(rules[2].as_str().unwrap(), "MATCH,DIRECT");
    }

    #[test]
    fn append_rules_lands_before_match() {
        let mut base = yaml("rules:\n  - DOMAIN,a.com,DIRECT\n  - MATCH,Proxy");
        let patch = yaml(
            "append-rules:\n  - DOMAIN-SUFFIX,b.com,DIRECT\n  - DOMAIN-SUFFIX,c.com,DIRECT",
        );
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
    fn js_transform_runs() {
        let cfg = serde_json::json!({"rules": ["MATCH,DIRECT"], "log-level": "info"});
        let out = apply_js(
            "function main(c){ c['log-level']='debug'; c.rules.unshift('DOMAIN,x.com,REJECT'); return c; }",
            cfg,
        )
        .unwrap();
        assert_eq!(out["log-level"], "debug");
        assert_eq!(out["rules"][0], "DOMAIN,x.com,REJECT");
    }

    #[test]
    fn js_errors_are_reported() {
        assert!(apply_js("not valid js {{{", serde_json::json!({})).is_err());
        assert!(apply_js("var x = 1;", serde_json::json!({})).is_err());
    }
}
