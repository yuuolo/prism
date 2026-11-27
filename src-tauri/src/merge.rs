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

/// A group's `filter` / `exclude-filter` / `exclude-type`, read the way
/// mihomo does: several patterns may be joined with a backtick, any one of
/// them matching counts; types are joined with `|` and ignore case.
struct NodeFilter {
    keep: Vec<Regex>,
    drop: Vec<Regex>,
    drop_types: Vec<String>,
}

/// Whether a group takes every node by itself (`include-all` / `include-all-proxies`).
fn includes_all(m: &Mapping) -> bool {
    ["include-all", "include-all-proxies"].iter().any(|k| m.get(*k).and_then(Value::as_bool) == Some(true))
}

/// mihomo's name for a node type (what `exclude-type` is compared with), lowercase.
fn adapter_type(ty: &str) -> String {
    match ty.to_lowercase().as_str() {
        "ss" => "shadowsocks".into(),
        "ssr" => "shadowsocksr".into(),
        other => other.into(),
    }
}

impl NodeFilter {
    /// Err(pattern) when a pattern can't be compiled here (look-around and
    /// other syntax mihomo's regexp2 accepts but Rust's regex doesn't).
    fn of(m: &Mapping) -> std::result::Result<Self, String> {
        let compile = |k: &str| -> std::result::Result<Vec<Regex>, String> {
            match s(m, k).filter(|f| !f.is_empty()) {
                None => Ok(Vec::new()),
                Some(f) => f.split('`').filter(|p| !p.is_empty()).map(|p| Regex::new(p).map_err(|_| p.to_string())).collect(),
            }
        };
        let drop_types = s(m, "exclude-type").map(|t| t.split('|').map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
        Ok(NodeFilter { keep: compile("filter")?, drop: compile("exclude-filter")?, drop_types })
    }

    /// `ty` is the node's Clash `type`.
    fn allows(&self, name: &str, ty: &str) -> bool {
        (self.keep.is_empty() || self.keep.iter().any(|r| r.is_match(name)))
            && !self.drop.iter().any(|r| r.is_match(name))
            && !self.drop_types.contains(&adapter_type(ty))
    }
}

/// The profile with the parts' nodes merged in (Clash YAML), plus warnings.
/// Without parts the profile is returned unchanged.
pub fn merge(raw: &str, parts: &[Part]) -> Result<(String, Vec<String>)> {
    if parts.is_empty() {
        return Ok((raw.to_string(), Vec::new()));
    }
    let format = config::detect(raw);
    let mut doc: Value = match format {
        Format::Clash => serde_yaml::from_str(raw)?,
        _ => clash::emit(&config::parse_any(raw)?),
    };
    let mut warnings = Vec::new();
    let root = doc.as_mapping_mut().ok_or_else(|| anyhow::anyhow!("订阅不是有效的 Clash 配置"))?;
    // In the profile's order: frozen groups list them the way include-all would.
    let own_nodes: Vec<String> = match root.get("proxies") {
        Some(Value::Sequence(v)) => v.iter().filter_map(|p| p.as_mapping().and_then(|m| s(m, "name"))).collect(),
        _ => Vec::new(),
    };
    let mut taken: HashSet<String> = own_nodes.iter().cloned().collect();
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
        // New name per node, by position: a profile may repeat a name, and
        // each copy needs its own. References (dialer-proxy) go to the first.
        let mut new_names: Vec<String> = Vec::with_capacity(profile.proxies.len());
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
            rename.entry(p.name.clone()).or_insert_with(|| name.clone());
            new_names.push(name);
        }
        let mut dropped_detour = false;
        for (p, name) in profile.proxies.iter().zip(new_names) {
            let mut p = p.clone();
            p.name = name;
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
        // Nothing added: same as having nothing to merge.
        return Ok((raw.to_string(), warnings));
    }

    if format == Format::SingBox {
        warnings.push("当前订阅是 sing-box 原始配置，合并后按通用配置重新生成，它自己的 DNS 等 sing-box 专有设置不会保留".into());
    }
    let root = doc.as_mapping_mut().unwrap();
    match root.entry(Value::from("proxies")).or_insert_with(|| Value::Sequence(Vec::new())) {
        Value::Sequence(v) => v.extend(new_nodes),
        other => *other = Value::Sequence(new_nodes),
    }
    let types: HashMap<String, String> = match root.get("proxies") {
        Some(Value::Sequence(v)) => v
            .iter()
            .filter_map(|p| p.as_mapping())
            .filter_map(|m| Some((s(m, "name")?, s(m, "type").unwrap_or_default())))
            .collect(),
        _ => HashMap::new(),
    };
    let ty_of = |n: &str| types.get(n).map(String::as_str).unwrap_or("");
    // The group the MATCH rule sends traffic to (rules after the first are never reached).
    let fallback: Option<String> = match root.get("rules") {
        Some(Value::Sequence(v)) => v
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|r| r.trim().strip_prefix("MATCH,"))
            .map(|t| t.split(',').next().unwrap_or("").trim().to_string())
            .next(),
        _ => None,
    };

    let mut hit = 0;
    if let Some(Value::Sequence(groups)) = root.get_mut("proxy-groups") {
        for g in groups.iter_mut() {
            let Some(m) = g.as_mapping_mut() else { continue };
            let name = s(m, "name").unwrap_or_default();
            let canonical = name.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
            let ty = s(m, "type").unwrap_or_default();
            let listed = names(m.get("proxies"));
            let target = (ty == "select"
                && ["节点选择", "手动选择", "代理选择", "手动切换", "节点切换", "选择节点", "选择代理", "代理", "proxy", "proxies", "proxyselect", "nodeselect", "节点", "选择"]
                    .contains(&canonical.as_str()))
                || (ty == "url-test" && ["自动选择", "自动测速", "自动优选", "延迟最低", "最低延迟", "auto", "autoselect", "urltest"].contains(&canonical.as_str()))
                // Whatever it's called, the select group MATCH goes to is the entry
                // when it holds nodes itself (a service group of DIRECT isn't).
                || (ty == "select" && fallback.as_deref() == Some(name.as_str()) && (includes_all(m) || listed.iter().any(|n| own_nodes.contains(n))));
            if !target && !includes_all(m) {
                continue;
            }
            let f = match NodeFilter::of(m) {
                Ok(f) => f,
                Err(p) => {
                    warnings.push(if includes_all(m) {
                        // Can't tell which nodes it holds: left as is, mihomo still
                        // filters it (merged nodes that match may show up too).
                        format!("代理组「{name}」的筛选规则 {p} 无法解析，保持原样，匹配它的合并节点也可能出现在该组")
                    } else {
                        format!("代理组「{name}」的筛选规则 {p} 无法解析，合并的节点没有加入该组")
                    });
                    continue;
                }
            };
            // Spell out include-all as the profile's own nodes: otherwise merged
            // nodes leak into service / region groups through it, and the
            // sing-box and Prism cores don't expand it at all. mihomo doesn't
            // apply `filter` to listed nodes, so the list is filtered here.
            if includes_all(m) {
                let all = m.get("include-all").and_then(Value::as_bool) == Some(true);
                m.remove("include-all");
                m.remove("include-all-proxies");
                if all { m.insert(Value::from("include-all-providers"), Value::Bool(true)); }
                let mut members = listed;
                for n in own_nodes.iter().filter(|n| f.allows(n, ty_of(n))) {
                    if !members.contains(n) { members.push(n.clone()); }
                }
                m.insert(Value::from("proxies"), Value::Sequence(members.into_iter().map(Value::from).collect()));
            }
            if !target {
                continue;
            }
            let extra: Vec<Value> = added.iter().filter(|n| f.allows(n, ty_of(n))).map(|n| Value::from(n.as_str())).collect();
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

    #[test]
    fn frozen_groups_keep_their_filters_and_order() {
        let main = "proxies:
  - {name: 香港 01, type: ss, server: 1.1.1.1, port: 1, cipher: aes-128-gcm, password: x}
  - {name: 东京, type: ss, server: 1.1.1.2, port: 1, cipher: aes-128-gcm, password: x}
  - {name: HK 03, type: ss, server: 1.1.1.3, port: 1, cipher: aes-128-gcm, password: x}
proxy-groups:
  - {name: 节点选择, type: select, proxies: [DIRECT]}
  - {name: 香港组, type: select, include-all: true, filter: '香港|HK', exclude-filter: '03'}
  - {name: 多条, type: select, include-all-proxies: true, filter: '东京`HK'}
  - {name: 全部, type: select, include-all-proxies: true}
rules: ['MATCH,节点选择']
";
        let (out, w) = merge(main, &[part("B", "ss://YWVzLTI1Ni1nY206cHc@3.3.3.3:8388#HK%2009")]).unwrap();
        let doc: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(group(&doc, "香港组"), ["香港 01"], "filter / exclude-filter still apply after freezing");
        assert_eq!(group(&doc, "多条"), ["东京", "HK 03"], "backtick-joined patterns: any one matches");
        assert_eq!(group(&doc, "全部"), ["香港 01", "东京", "HK 03"], "the profile's own order");
        assert_eq!(group(&doc, "节点选择"), ["DIRECT", "HK 09"]);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn repeated_names_inside_a_part_stay_distinct() {
        let b = "proxies:
  - {name: x, type: trojan, server: 2.2.2.1, port: 443, password: p}
  - {name: x, type: trojan, server: 2.2.2.2, port: 443, password: p}
  - {name: y, type: trojan, server: 2.2.2.3, port: 443, password: p, dialer-proxy: x}
";
        let (out, _) = merge(MAIN, &[part("B", b)]).unwrap();
        let p = config::parse_any(&out).unwrap();
        let names: Vec<_> = p.proxies.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["东京", "香港", "x", "x [B]", "y"]);
        let node = |n: &str| p.proxies.iter().find(|x| x.name == n).unwrap().clone();
        assert_eq!(node("x").server, "2.2.2.1");
        assert_eq!(node("x [B]").server, "2.2.2.2");
        assert_eq!(node("y").dialer_proxy.as_deref(), Some("x"), "a reference goes to the first copy");
    }

    #[test]
    fn unsupported_filter_syntax_adds_nothing_and_warns() {
        let main = "proxies:
  - {name: 香港 01, type: ss, server: 1.1.1.1, port: 1, cipher: aes-128-gcm, password: x}
proxy-groups:
  - {name: 节点选择, type: select, proxies: [DIRECT], filter: '^(?!.*美国)'}
  - {name: 非美, type: select, include-all: true, filter: '^(?!.*美国)'}
rules: ['MATCH,节点选择']
";
        let (out, w) = merge(main, &[part("B", "ss://YWVzLTI1Ni1nY206cHc@3.3.3.3:8388#美国")]).unwrap();
        let doc: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(group(&doc, "节点选择"), ["DIRECT"], "never falls back to adding every node");
        let g = doc["proxy-groups"][1].as_mapping().unwrap();
        assert_eq!(g.get("include-all").and_then(Value::as_bool), Some(true), "left for the core to filter");
        assert!(w.iter().any(|x| x.contains("节点选择")) && w.iter().any(|x| x.contains("非美")), "{w:?}");
    }

    #[test]
    fn the_match_group_is_an_entry_when_it_lists_nodes() {
        let main = "proxies:
  - {name: 东京, type: ss, server: 1.1.1.1, port: 1, cipher: aes-128-gcm, password: x}
proxy-groups:
  - {name: 🌍 我的出口, type: select, proxies: [东京, DIRECT]}
  - {name: 代理选择, type: select, proxies: [我的出口2]}
  - {name: 我的出口2, type: select, proxies: [DIRECT]}
rules: ['MATCH,🌍 我的出口']
";
        let (out, w) = merge(main, &[part("B", "ss://YWVzLTI1Ni1nY206cHc@3.3.3.3:8388#x")]).unwrap();
        let doc: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(group(&doc, "🌍 我的出口"), ["东京", "DIRECT", "x"]);
        assert_eq!(group(&doc, "代理选择"), ["我的出口2", "x"], "a common entry name");
        assert_eq!(group(&doc, "我的出口2"), ["DIRECT"]);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn exclude_type_applies_to_frozen_and_entry_groups() {
        let main = "proxies:
  - {name: a, type: ss, server: 1.1.1.1, port: 1, cipher: aes-128-gcm, password: x}
  - {name: b, type: trojan, server: 1.1.1.2, port: 443, password: x}
proxy-groups:
  - {name: 节点选择, type: select, proxies: [DIRECT], exclude-type: 'Shadowsocks|http'}
  - {name: 非SS, type: select, include-all-proxies: true, exclude-type: shadowsocks}
rules: ['MATCH,节点选择']
";
        let b = "proxies:
  - {name: c, type: ss, server: 2.2.2.1, port: 1, cipher: aes-128-gcm, password: x}
  - {name: d, type: vless, server: 2.2.2.2, port: 443, uuid: 00000000-0000-0000-0000-000000000000}
";
        let (out, _) = merge(main, &[part("B", b)]).unwrap();
        let doc: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(group(&doc, "节点选择"), ["DIRECT", "d"]);
        assert_eq!(group(&doc, "非SS"), ["b"]);
    }

    #[test]
    fn a_raw_sing_box_profile_warns_about_what_is_lost() {
        let sb = r#"{"outbounds":[{"type":"shadowsocks","tag":"东京","server":"4.4.4.4","server_port":8388,"method":"aes-256-gcm","password":"pw"},{"type":"selector","tag":"proxy","outbounds":["东京"]}]}"#;
        let (out, w) = merge(sb, &[part("B", "ss://YWVzLTI1Ni1nY206cHc@3.3.3.3:8388#x")]).unwrap();
        let doc: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(group(&doc, "proxy"), ["东京", "x"]);
        assert!(w.iter().any(|x| x.contains("sing-box")), "{w:?}");
    }

    #[test]
    fn the_first_match_counts_and_may_include_all() {
        let main = "proxies:
  - {name: 东京, type: ss, server: 1.1.1.1, port: 1, cipher: aes-128-gcm, password: x}
proxy-groups:
  - {name: 🚀 出口, type: select, include-all-proxies: true}
  - {name: 其他, type: select, include-all: false, include-all-proxies: true}
rules: ['MATCH,🚀 出口', 'MATCH,其他']
";
        let (out, w) = merge(main, &[part("B", "ss://YWVzLTI1Ni1nY206cHc@3.3.3.3:8388#x")]).unwrap();
        let doc: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(group(&doc, "🚀 出口"), ["东京", "x"], "spelled out: sing-box and Prism don't expand include-all");
        assert_eq!(group(&doc, "其他"), ["东京"], "unreachable MATCH: frozen like any other group");
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn nothing_added_keeps_the_profile_as_is() {
        let sb = r#"{"outbounds":[{"type":"shadowsocks","tag":"东京","server":"4.4.4.4","server_port":8388,"method":"aes-256-gcm","password":"pw"}]}"#;
        let (out, w) = merge(sb, &[part("坏的", "not a profile")]).unwrap();
        assert_eq!(out, sb);
        assert_eq!(w.len(), 1, "{w:?}");
    }
}

