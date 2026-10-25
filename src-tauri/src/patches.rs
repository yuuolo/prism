//! 覆写: global changes applied to every profile before Prism generates the
//! core's config, like Clash Verge Rev's Merge and Script profiles.
//!
//! - Merge (YAML): `prepend-` / `append-` + `rules` / `proxies` /
//!   `proxy-groups` add list entries; any other key is merged in (mappings
//!   recursively, everything else replaced).
//! - Script (JavaScript): `function main(config, profileName)` returns the
//!   changed config. Runs in Boa with loop / recursion limits.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use prism_core::config::{self, clash, Format};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Patches {
    pub merge_enabled: bool,
    pub merge: String,
    pub script_enabled: bool,
    pub script: String,
}

pub const MERGE_TEMPLATE: &str = "# 对所有订阅生效的修改（YAML）。示例：
# prepend-rules:            # 插到规则最前面
#   - DOMAIN-SUFFIX,example.com,DIRECT
# append-rules:             # 加在最后的 MATCH 之前
#   - GEOIP,LAN,DIRECT
# append-proxies:           # 追加节点（prepend-proxies 插到最前）
#   - { name: 我的节点, type: ss, server: 1.2.3.4, port: 8388, cipher: aes-256-gcm, password: pw }
# append-proxy-groups: []   # 追加代理组
# dns:                      # 其他键直接覆盖（字典会逐项合并）
#   nameserver: [223.5.5.5, 119.29.29.29]
";

pub const SCRIPT_TEMPLATE: &str = "// 对所有订阅生效的脚本（JavaScript）。
// config 是订阅转换成的 Clash 配置对象，profileName 是订阅名称；返回修改后的 config。
function main(config, profileName) {
  // 例：给所有代理组加上 DIRECT 选项
  // for (const g of config[\"proxy-groups\"] || []) {
  //   if (g.type === \"select\" && !g.proxies.includes(\"DIRECT\")) g.proxies.push(\"DIRECT\");
  // }
  return config;
}
";

const FILE: &str = "overrides.json";

impl Patches {
    pub fn active(&self) -> bool {
        (self.merge_enabled && !self.merge.trim().is_empty()) || (self.script_enabled && !self.script.trim().is_empty())
    }
}

pub fn load(dir: &Path) -> Patches {
    std::fs::read_to_string(dir.join(FILE)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| Patches {
        merge: MERGE_TEMPLATE.into(),
        script: SCRIPT_TEMPLATE.into(),
        ..Default::default()
    })
}

pub fn save(dir: &Path, p: &Patches) -> Result<()> {
    std::fs::write(dir.join(FILE), serde_json::to_vec_pretty(p)?)?;
    Ok(())
}

/// The profile with the patches applied: Clash YAML when anything is
/// active, otherwise the profile unchanged.
pub fn apply(p: &Patches, raw: &str, profile_name: &str) -> Result<String> {
    if !p.active() {
        return Ok(raw.to_string());
    }
    let mut doc: Value = match config::detect(raw) {
        Format::Clash => serde_yaml::from_str(raw)?,
        _ => clash::emit(&config::parse_any(raw)?),
    };
    if p.merge_enabled && !p.merge.trim().is_empty() {
        let m: Value = serde_yaml::from_str(&p.merge).map_err(|e| anyhow!("覆写 Merge 不是有效的 YAML：{e}"))?;
        merge(&mut doc, m)?;
    }
    if p.script_enabled && !p.script.trim().is_empty() {
        doc = run_script(&p.script, doc, profile_name)?;
    }
    Ok(serde_yaml::to_string(&doc)?)
}

fn merge(doc: &mut Value, m: Value) -> Result<()> {
    let m = match m {
        Value::Mapping(m) => m,
        Value::Null => return Ok(()),
        _ => bail!("覆写 Merge 的顶层必须是键值对"),
    };
    if !doc.is_mapping() {
        *doc = Value::Mapping(Mapping::new());
    }
    for (k, v) in m {
        let key = k.as_str().unwrap_or_default().to_string();
        let list_op = ["prepend-", "append-"].iter().find_map(|p| key.strip_prefix(p).map(|rest| (*p, rest)));
        match list_op {
            Some((op, target @ ("rules" | "proxies" | "proxy-groups"))) => {
                let Value::Sequence(items) = v else { bail!("覆写 Merge 的 {key} 必须是列表") };
                let map = doc.as_mapping_mut().unwrap();
                let list = map.entry(Value::from(target)).or_insert_with(|| Value::Sequence(vec![]));
                let Value::Sequence(list) = list else { bail!("订阅里的 {target} 不是列表") };
                if op == "prepend-" {
                    list.splice(0..0, items);
                } else {
                    // Rules after MATCH would never be reached.
                    let at = match list.last().and_then(|r| r.as_str()) {
                        Some(r) if target == "rules" && r.trim_start().starts_with("MATCH") => list.len() - 1,
                        _ => list.len(),
                    };
                    list.splice(at..at, items);
                }
            }
            _ => {
                let slot = doc.as_mapping_mut().unwrap().entry(k).or_insert(Value::Null);
                deep_merge(slot, v);
            }
        }
    }
    Ok(())
}

fn deep_merge(a: &mut Value, b: Value) {
    match (a, b) {
        (Value::Mapping(a), Value::Mapping(b)) => {
            for (k, v) in b {
                deep_merge(a.entry(k).or_insert(Value::Null), v);
            }
        }
        (a, b) => *a = b,
    }
}

fn run_script(code: &str, doc: Value, profile_name: &str) -> Result<Value> {
    use boa_engine::{js_string, Context, JsValue, Source};
    let js_err = |what: &str, e: boa_engine::JsError| anyhow!("覆写脚本{what}：{e}");
    let json = serde_json::to_value(&doc)?;
    let mut ctx = Context::default();
    ctx.runtime_limits_mut().set_loop_iteration_limit(20_000_000);
    ctx.runtime_limits_mut().set_recursion_limit(1000);
    ctx.eval(Source::from_bytes(code)).map_err(|e| js_err("出错", e))?;
    let main = ctx.global_object().get(js_string!("main"), &mut ctx).map_err(|e| js_err("出错", e))?;
    let main = main.as_callable().context("覆写脚本中没有 main(config, profileName) 函数")?;
    let arg = JsValue::from_json(&json, &mut ctx).map_err(|e| js_err("出错", e))?;
    let name = JsValue::from(js_string!(profile_name));
    let out = main.call(&JsValue::undefined(), &[arg, name], &mut ctx).map_err(|e| js_err("运行出错", e))?;
    let out = out.to_json(&mut ctx).map_err(|e| js_err("返回值无法转换", e))?.context("覆写脚本的 main 没有返回配置")?;
    if !out.is_object() {
        bail!("覆写脚本的 main 必须返回配置对象");
    }
    Ok(serde_yaml::to_value(out)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = "proxies:
  - {name: a, type: ss, server: 1.2.3.4, port: 1, cipher: aes-128-gcm, password: x}
proxy-groups:
  - {name: PROXY, type: select, proxies: [a]}
dns: {enable: true, nameserver: [8.8.8.8], ipv6: false}
rules:
  - DOMAIN-SUFFIX,google.com,PROXY
  - MATCH,PROXY
";

    fn doc(s: &str) -> Value {
        serde_yaml::from_str(s).unwrap()
    }

    #[test]
    fn inactive_leaves_the_profile_alone() {
        let p = Patches { merge: "rules: []".into(), script: "garbage(".into(), ..Default::default() };
        assert_eq!(apply(&p, PROFILE, "x").unwrap(), PROFILE);
        assert!(!load(Path::new("Z:/nowhere")).active(), "templates are off by default");
    }

    #[test]
    fn merge_lists_and_keys() {
        let p = Patches {
            merge_enabled: true,
            merge: "prepend-rules: ['DOMAIN,a.com,DIRECT']
append-rules:
  - GEOIP,LAN,DIRECT
append-proxies: [{name: b, type: ss, server: 5.6.7.8, port: 2, cipher: aes-128-gcm, password: y}]
prepend-proxy-groups: [{name: Mine, type: select, proxies: [b, DIRECT]}]
dns: {nameserver: [223.5.5.5]}
mode: global
"
            .into(),
            ..Default::default()
        };
        let out = doc(&apply(&p, PROFILE, "x").unwrap());
        let rules: Vec<&str> = out["rules"].as_sequence().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
        assert_eq!(rules, ["DOMAIN,a.com,DIRECT", "DOMAIN-SUFFIX,google.com,PROXY", "GEOIP,LAN,DIRECT", "MATCH,PROXY"]);
        assert_eq!(out["proxies"][1]["name"].as_str(), Some("b"));
        assert_eq!(out["proxy-groups"][0]["name"].as_str(), Some("Mine"));
        // Deep merge: only nameserver replaced.
        assert_eq!(out["dns"]["nameserver"][0].as_str(), Some("223.5.5.5"));
        assert_eq!(out["dns"]["enable"].as_bool(), Some(true));
        assert_eq!(out["mode"].as_str(), Some("global"));
        // The result still parses as a profile with both nodes.
        assert_eq!(config::parse_any(&apply(&p, PROFILE, "x").unwrap()).unwrap().proxies.len(), 2);
    }

    #[test]
    fn script_changes_config() {
        let p = Patches {
            script_enabled: true,
            script: "function main(config, name) {
  config.rules.unshift('DOMAIN,' + name + '.example,DIRECT');
  for (const g of config['proxy-groups']) g.proxies.push('DIRECT');
  return config;
}"
            .into(),
            ..Default::default()
        };
        let out = doc(&apply(&p, PROFILE, "机场").unwrap());
        assert_eq!(out["rules"][0].as_str(), Some("DOMAIN,机场.example,DIRECT"));
        assert_eq!(out["proxy-groups"][0]["proxies"][1].as_str(), Some("DIRECT"));
        // Merge runs first, then the script sees its result.
        let both = Patches { merge_enabled: true, merge: "append-proxies: [{name: z, type: ss, server: 9.9.9.9, port: 3, cipher: aes-128-gcm, password: z}]".into(), ..p };
        assert_eq!(doc(&apply(&both, PROFILE, "n").unwrap())["proxies"][1]["name"].as_str(), Some("z"));
    }

    #[test]
    fn script_errors_are_reported() {
        let run = |code: &str| {
            let p = Patches { script_enabled: true, script: code.into(), ..Default::default() };
            apply(&p, PROFILE, "x").unwrap_err().to_string()
        };
        assert!(run("function main(c) { return c; ").contains("覆写脚本出错"));
        assert!(run("var x = 1;").contains("没有 main"));
        assert!(run("function main(c) { throw new Error('boom'); }").contains("boom"));
        assert!(run("function main(c) { return 1; }").contains("必须返回配置对象"));
        // Runaway scripts stop instead of hanging Prism.
        assert!(run("function main(c) { while (true) {} }").contains("运行出错"));
        let bad = Patches { merge_enabled: true, merge: "rules: [a, b".into(), ..Default::default() };
        assert!(apply(&bad, PROFILE, "x").unwrap_err().to_string().contains("YAML"));
    }

    #[test]
    fn non_clash_profiles_are_converted_first() {
        let uri = "ss://YWVzLTI1Ni1nY206cHc@1.2.3.4:8388#node";
        let p = Patches { merge_enabled: true, merge: "prepend-rules:
  - DOMAIN,x.com,DIRECT".into(), ..Default::default() };
        let out = doc(&apply(&p, uri, "x").unwrap());
        assert_eq!(out["rules"][0].as_str(), Some("DOMAIN,x.com,DIRECT"));
        assert_eq!(out["proxies"][0]["name"].as_str(), Some("node"));
    }
}
