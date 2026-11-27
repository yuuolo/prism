//! 合并订阅: the nodes of other profiles are added to the active profile,
//! so its rules and groups get more nodes to choose from.
//!
//! The merged nodes go into the active profile's `proxies` (renamed
//! `名字 [订阅名]` on a clash) and only into node-selection and automatic
//! selection entry groups (respecting `filter` / `exclude-filter`).
//! Groups made only of other groups / DIRECT (国内直连 and such) are left
//! alone. Runs before 覆写, so overrides see the merged nodes.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use prism_core::config::{self, clash, Format, BUILTIN};
use regex::Regex;
use serde_yaml::{Mapping, Value};

/// Another profile to take nodes from.
pub struct Part {
    pub name: String,
    pub raw: String,
}

fn s(m: &Mapping, k: &str) -> Option<String> {
    m.get(k).and_then(|v| v.as_str()).map(String::from)
}

fn names(seq: Option<&Value>) -> Vec<String> {
    match seq {
        Some(Value::Sequence(v)) => v.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        _ => Vec::new(),
    }
}



/// The profile with the parts' nodes merged in (Clash YAML), plus warnings.
/// Without parts the profile is returned unchanged.
pub fn merge(raw: &str, parts: &[Part]) -> Result<(String, Vec<String>)> {
    if parts.is_empty() {
        return Ok((raw.to_string(), Vec::new()));
    }
    let mut doc: Value = match config::detect(raw) {
        Format::Clash => serde_yaml::from_str(raw)?,
        _ => clash::emit(&config::parse_any(raw)?),
    };
    let mut warnings = Vec::new();
    let root = doc.as_mapping_mut().ok_or_else(|| anyhow::anyhow!("订阅不是有效的 Clash 配置"))?;
    let own_nodes: HashSet<String> = match root.get("proxies") {
        Some(Value::Sequence(v)) => v.iter().filter_map(|p| p.as_mapping().and_then(|m| s(m, "name"))).collect(),
        _ => HashSet::new(),
    };
    let mut taken: HashSet<String> = own_nodes.clone();
    if let Some(Value::Sequence(v)) = root.get("proxy-groups") {
        taken.extend(v.iter().filter_map(|g| g.as_mapping().and_then(|m| s(m, "name"))));
    }
    taken.extend(BUILTIN.iter().map(|b| b.to_string()));

    let mut added: Vec<String> = Vec::new();
    let mut new_nodes: Vec<Value> = Vec::new();
    for part in parts {
        let profile = match config::parse_any(part.raw.trim_start_matches('\u{feff}')) {
            Ok(p) => p,
            Err(e) => {
                warnings.push(format!("合并订阅「{}」失败：{e:#}", part.name));
                continue;
            }
        };
        if profile.proxies.is_empty() {
            warnings.push(format!("合并订阅「{}」：没有可用的节点", part.name));
            continue;
        }
        let mut rename: HashMap<String, String> = HashMap::new();
        for p in &profile.proxies {
            let mut name = p.name.clone();
            if taken.contains(&name) {
                name = format!("{} [{}]", p.name, part.name);
                let mut i = 2;
                while taken.contains(&name) {
                    name = format!("{} [{}] {i}", p.name, part.name);
                    i += 1;
                }
            }
            taken.insert(name.clone());
            rename.insert(p.name.clone(), name);
        }
        let mut dropped_detour = false;
        for p in &profile.proxies {
            let mut p = p.clone();
            p.name = rename[&p.name].clone();
            // A dialer-proxy naming one of its own nodes follows the rename;
            // one naming a group of the other profile can't be kept.
            if let Some(d) = p.dialer_proxy.take() {
                match rename.get(&d) {
                    Some(n) => p.dialer_proxy = Some(n.clone()),
                    None if BUILTIN.contains(&d.as_str()) => p.dialer_proxy = Some(d),
                    None => dropped_detour = true,
                }
            }
            added.push(p.name.clone());
            new_nodes.push(clash::emit_proxy(&p));
        }
        if dropped_detour {
            warnings.push(format!("合并订阅「{}」：部分节点的 dialer-proxy 指向那个订阅自己的代理组，已忽略", part.name));
        }
    }
    if new_nodes.is_empty() {
        return Ok((serde_yaml::to_string(&doc)?, warnings));
    }

    let root = doc.as_mapping_mut().unwrap();
    match root.entry(Value::from("proxies")).or_insert_with(|| Value::Sequence(Vec::new())) {
        Value::Sequence(v) => v.extend(new_nodes),
        other => *other = Value::Sequence(new_nodes),
    }

    let mut hit = 0;
    if let Some(Value::Sequence(groups)) = root.get_mut("proxy-groups") {
        for g in groups.iter_mut() {
            let Some(m) = g.as_mapping_mut() else { continue };
            let name = s(m, "name").unwrap_or_default();
            let canonical = name.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
            let ty = s(m, "type").unwrap_or_default();
            let target = (ty == "select" && ["节点选择", "手动选择", "proxy", "proxyselect", "nodeselect", "节点", "选择"].contains(&canonical.as_str()))
                || (ty == "url-test" && ["自动选择", "自动测速", "自动优选", "auto", "autoselect", "urltest"].contains(&canonical.as_str()));
            if !target {
                // Freeze implicit all-node membership, otherwise added nodes would
                // still leak into service/region groups through include-all.
                let all = m.get("include-all").and_then(Value::as_bool) == Some(true);
                let all_nodes = m.get("include-all-proxies").and_then(Value::as_bool) == Some(true);
                if all || all_nodes {
                    m.remove("include-all");
                    m.remove("include-all-proxies");
                    if all { m.insert(Value::from("include-all-providers"), Value::Bool(true)); }
                    let mut members = names(m.get("proxies"));
                    let mut own: Vec<_> = own_nodes.iter().cloned().collect();
                    own.sort();
                    for name in own { if !members.contains(&name) { members.push(name); } }
                    m.insert(Value::from("proxies"), Value::Sequence(members.into_iter().map(Value::from).collect()));
                }
                continue;
            }
            // These pick up every node by themselves in mihomo.
            if m.get("include-all").or(m.get("include-all-proxies")).and_then(|v| v.as_bool()) == Some(true) {
                hit += 1;
                continue;
            }
            let keep = s(m, "filter").and_then(|f| Regex::new(&f).ok());
            let skip = s(m, "exclude-filter").and_then(|f| Regex::new(&f).ok());
            let extra: Vec<Value> = added
                .iter()
                .filter(|n| keep.as_ref().is_none_or(|r| r.is_match(n)) && skip.as_ref().is_none_or(|r| !r.is_match(n)))
                .map(|n| Value::from(n.as_str()))
                .collect();
            if extra.is_empty() {
                continue;
            }
            hit += 1;
            match m.entry(Value::from("proxies")).or_insert_with(|| Value::Sequence(Vec::new())) {
                Value::Sequence(v) => v.extend(extra),
                other => *other = Value::Sequence(extra),
            }
        }
    }
    if hit == 0 {
        warnings.push("合并的节点没有可以加入的代理组，只能在全局模式下选择".into());
    }
    Ok((serde_yaml::to_string(&doc)?, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = "proxies:
  - {name: 东京, type: ss, server: 1.1.1.1, port: 1, cipher: aes-128-gcm, password: x}
  - {name: 香港, type: ss, server: 1.1.1.2, port: 1, cipher: aes-128-gcm, password: x}
proxy-groups:
  - {name: 节点选择, type: select, proxies: [自动选择, 东京, 香港, DIRECT]}
  - {name: 自动选择, type: url-test, proxies: [东京, 香港], url: 'https://www.gstatic.com/generate_204', interval: 300}
  - {name: 香港组, type: select, proxies: [香港], filter: '香港|HK'}
  - {name: 国内, type: select, proxies: [DIRECT, 节点选择]}
rules:
  - GEOIP,CN,国内
  - MATCH,节点选择
";

    fn part(name: &str, raw: &str) -> Part {
        Part { name: name.into(), raw: raw.into() }
    }

    fn group<'a>(doc: &'a Value, name: &str) -> Vec<&'a str> {
        doc["proxy-groups"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|g| g["name"].as_str() == Some(name))
            .unwrap()["proxies"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap())
            .collect()
    }

    #[test]
    fn never_falls_back_to_service_groups() {
        let raw = "proxy-groups: [{name: Netflix, type: select, proxies: [DIRECT]}]\nrules: ['MATCH,Netflix']";
        let (out, warnings) = merge(raw, &[part("B", "ss://YWVzLTI1Ni1nY206cHc@3.3.3.3:8388#new")]).unwrap();
        let doc: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(group(&doc, "Netflix"), ["DIRECT"]);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn nodes_join_node_groups() {
        let b = "proxies:
  - {name: 东京, type: trojan, server: 2.2.2.1, port: 443, password: p}
  - {name: HK 02, type: trojan, server: 2.2.2.2, port: 443, password: p, dialer-proxy: 东京}
  - {name: 美国, type: trojan, server: 2.2.2.3, port: 443, password: p, dialer-proxy: 机场B的组}
proxy-groups:
  - {name: 机场B的组, type: select, proxies: [美国]}
";
        let links = "ss://YWVzLTI1Ni1nY206cHc@3.3.3.3:8388#新加坡";
        let (out, warnings) = merge(MAIN, &[part("机场B", b), part("机场C", links), part("坏的", "not a profile")]).unwrap();
        // For checking with the real cores.
        if let Ok(p) = std::env::var("PRISM_MERGE_OUT") {
            std::fs::write(p, &out).unwrap();
        }
        let doc: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(group(&doc, "节点选择"), ["自动选择", "东京", "香港", "DIRECT", "东京 [机场B]", "HK 02", "美国", "新加坡"]);
        assert_eq!(group(&doc, "自动选择"), ["东京", "香港", "东京 [机场B]", "HK 02", "美国", "新加坡"]);
        assert_eq!(group(&doc, "香港组"), ["香港"], "regional groups remain unchanged");
        assert_eq!(group(&doc, "国内"), ["DIRECT", "节点选择"], "groups of groups are left alone");
        let p = config::parse_any(&out).unwrap();
        assert_eq!(p.proxies.len(), 6);
        let node = |n: &str| p.proxies.iter().find(|x| x.name == n).unwrap().clone();
        assert_eq!(node("HK 02").dialer_proxy.as_deref(), Some("东京 [机场B]"));
        assert_eq!(node("美国").dialer_proxy, None);
        assert_eq!(p.rules.len(), 2, "rules are the active profile's");
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("dialer-proxy") && warnings[1].contains("坏的"));
    }

    #[test]
    fn nothing_to_merge_keeps_the_profile() {
        assert_eq!(merge(MAIN, &[]).unwrap().0, MAIN);
    }

    #[test]
    fn only_recognized_entry_groups_receive_nodes() {
        let main = "proxy-providers:
  p: {type: http, url: 'https://a.example/sub', interval: 3600}
proxy-groups:
  - {name: 选择, type: select, proxies: [DIRECT]}
  - {name: 全部, type: select, include-all-proxies: true}
rules: ['MATCH,选择']
";
        let (out, w) = merge(main, &[part("B", "ss://YWVzLTI1Ni1nY206cHc@3.3.3.3:8388#x")]).unwrap();
        let doc: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(group(&doc, "选择"), ["DIRECT", "x"]);
        assert!(group(&doc, "全部").is_empty(), "non-entry include-all groups must not pick up merged nodes");
        assert!(w.is_empty(), "{w:?}");
    }
}
