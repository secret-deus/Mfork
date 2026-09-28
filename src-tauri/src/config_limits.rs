//! Budgets enforced while deserializing, before an unbounded Value tree is allocated.
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_yaml::{Mapping, Value};
use std::cell::Cell;

const MAX_BYTES: usize = 20 * 1024 * 1024;
const MAX_NODES: usize = 200_000;
const MAX_DEPTH: usize = 64;

struct Seed<'a> {
    nodes: &'a Cell<usize>,
    depth: usize,
}
impl Seed<'_> {
    fn child(&self) -> Seed<'_> {
        Seed {
            nodes: self.nodes,
            depth: self.depth + 1,
        }
    }
}
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        let n = self.nodes.get();
        if n >= MAX_NODES || self.depth > MAX_DEPTH {
            return Err(de::Error::custom("YAML 超过节点数或嵌套深度限制"));
        }
        self.nodes.set(n + 1);
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("bounded, untagged YAML configuration")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.into()))
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let mut seq = Vec::new();
        while let Some(value) = access.next_element_seed(self.child())? {
            seq.push(value);
        }
        Ok(Value::Sequence(seq))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let mut map = Mapping::new();
        while let Some(key) = access.next_key_seed(self.child())? {
            if map.contains_key(&key) {
                return Err(de::Error::custom("YAML 包含重复键"));
            }
            map.insert(key, access.next_value_seed(self.child())?);
        }
        Ok(Value::Mapping(map))
    }
}

pub fn parse_yaml(text: &str) -> Result<Value, String> {
    if text.len() > MAX_BYTES {
        return Err("YAML 超过 20 MiB 限制".into());
    }
    let nodes = Cell::new(0);
    let mut docs = serde_yaml::Deserializer::from_str(text.trim_start_matches('\u{feff}'));
    let value = match docs.next() {
        Some(doc) => Seed {
            nodes: &nodes,
            depth: 0,
        }
        .deserialize(doc)
        .map_err(|e| e.to_string())?,
        None => Value::Null,
    };
    if docs.next().is_some() {
        return Err("只允许一个 YAML 文档".into());
    }
    Ok(value)
}

/// Check aggregate size after merging individually bounded overrides.
pub fn validate_mapping(map: &Mapping) -> Result<(), String> {
    fn visit(v: &Value, depth: usize, nodes: &mut usize, bytes: &mut usize) -> Result<(), String> {
        *nodes += 1;
        if *nodes > MAX_NODES || depth > MAX_DEPTH {
            return Err("合并后的配置超过节点数或嵌套限制".into());
        }
        match v {
            Value::String(s) => {
                *bytes = bytes.saturating_add(s.len());
            }
            Value::Sequence(seq) => {
                for child in seq {
                    visit(child, depth + 1, nodes, bytes)?;
                }
            }
            Value::Mapping(map) => {
                for (k, v) in map {
                    visit(k, depth + 1, nodes, bytes)?;
                    visit(v, depth + 1, nodes, bytes)?;
                }
            }
            Value::Tagged(_) => return Err("配置不支持自定义 YAML 标签".into()),
            _ => {}
        }
        if *bytes > MAX_BYTES {
            return Err("合并后的配置文本超过大小限制".into());
        }
        Ok(())
    }
    let (mut nodes, mut bytes) = (1, 0);
    for (k, v) in map {
        visit(k, 1, &mut nodes, &mut bytes)?;
        visit(v, 1, &mut nodes, &mut bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_yaml_and_aliases_work() {
        let value = parse_yaml("mode: global\nproxies: []\na: &base [one, two]\nb: *base").unwrap();
        assert_eq!(value["mode"].as_str(), Some("global"));
        assert_eq!(value["a"], value["b"]);
    }
    #[test]
    fn rejects_excess_depth_nodes_and_duplicate_keys() {
        assert!(parse_yaml(&format!("{}0{}", "[".repeat(80), "]".repeat(80))).is_err());
        assert!(parse_yaml(&format!("[{}]", "0,".repeat(MAX_NODES))).is_err());
        assert!(parse_yaml("a: 1\na: 2").is_err());
        assert!(parse_yaml("a: 1\n---\nb: 2").is_err());
        assert!(parse_yaml("a: !custom value").is_err());
    }
}
