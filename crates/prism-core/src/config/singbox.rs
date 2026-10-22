//! sing-box JSON <-> [`Profile`]. Targets sing-box 1.12+ (new DNS server
//! format, rule actions, anytls outbound).

use std::collections::{BTreeSet, HashMap};

use anyhow::{bail, Result};
use serde_json::{json, Map, Value};

use super::clash::Overrides;
use super::*;

const GEOSITE_URL: &str = "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-{}.srs";
const GEOIP_URL: &str = "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-{}.srs";

fn tls_json(p: &ProxyConfig) -> Value {
    let mut t = json!({ "enabled": true });
    let o = t.as_object_mut().unwrap();
    o.insert("server_name".into(), json!(p.sni.clone().unwrap_or_else(|| p.server.clone())));
    if p.skip_cert_verify {
        o.insert("insecure".into(), json!(true));
    }
    if !p.alpn.is_empty() {
        o.insert("alpn".into(), json!(p.alpn));
    }
    let fp = p.fingerprint.clone().or_else(|| p.reality_public_key.as_ref().map(|_| "chrome".to_string()));
    if let Some(fp) = fp {
        o.insert("utls".into(), json!({ "enabled": true, "fingerprint": fp }));
    }
    if let Some(pk) = &p.reality_public_key {
        o.insert(
            "reality".into(),
            json!({ "enabled": true, "public_key": pk, "short_id": p.reality_short_id.clone().unwrap_or_default() }),
        );
    }
    t
}

fn transport_json(p: &ProxyConfig) -> Option<Value> {
    match p.network.as_deref()? {
        "ws" => {
            let mut t = json!({ "type": "ws", "path": p.ws_path.clone().unwrap_or_else(|| "/".into()) });
            if let Some(h) = &p.ws_host {
                t["headers"] = json!({ "Host": h });
            }
            Some(t)
        }
        "grpc" => Some(json!({ "type": "grpc", "service_name": p.grpc_service.clone().unwrap_or_default() })),
        _ => None,
    }
}

pub fn emit_outbound(p: &ProxyConfig) -> Value {
    let mut o = Map::new();
    let ty = match p.kind() {
        ProxyKind::Shadowsocks => "shadowsocks",
        ProxyKind::Trojan => "trojan",
        ProxyKind::Vless => "vless",
        ProxyKind::Vmess => "vmess",
        ProxyKind::AnyTls => "anytls",
        ProxyKind::Hysteria2 => "hysteria2",
        ProxyKind::Socks5 => "socks",
        ProxyKind::Http => "http",
    };
    o.insert("type".into(), json!(ty));
    o.insert("tag".into(), json!(p.name));
    o.insert("server".into(), json!(p.server));
    o.insert("server_port".into(), json!(p.port));
    match p.kind() {
        ProxyKind::Shadowsocks => {
            o.insert("method".into(), json!(p.cipher.clone().unwrap_or_default()));
            o.insert("password".into(), json!(p.password.clone().unwrap_or_default()));
        }
        ProxyKind::Vless => {
            o.insert("uuid".into(), json!(p.uuid.clone().unwrap_or_default()));
            if let Some(f) = &p.flow {
                o.insert("flow".into(), json!(f));
            }
        }
        ProxyKind::Vmess => {
            o.insert("uuid".into(), json!(p.uuid.clone().unwrap_or_default()));
            o.insert("alter_id".into(), json!(p.alter_id));
            o.insert("security".into(), json!(p.cipher.clone().unwrap_or_else(|| "auto".into())));
        }
        ProxyKind::Socks5 => {
            o.insert("version".into(), json!("5"));
            if let Some(u) = &p.username {
                o.insert("username".into(), json!(u));
            }
            if let Some(pw) = &p.password {
                o.insert("password".into(), json!(pw));
            }
        }
        ProxyKind::Http => {
            if let Some(u) = &p.username {
                o.insert("username".into(), json!(u));
            }
            if let Some(pw) = &p.password {
                o.insert("password".into(), json!(pw));
            }
        }
        ProxyKind::Hysteria2 => {
            o.insert("password".into(), json!(p.password.clone().unwrap_or_default()));
            if let Some(obfs) = &p.obfs {
                o.insert("obfs".into(), json!({ "type": obfs, "password": p.obfs_password.clone().unwrap_or_default() }));
            }
        }
        ProxyKind::Trojan | ProxyKind::AnyTls => {
            o.insert("password".into(), json!(p.password.clone().unwrap_or_default()));
        }
    }
    if p.tls {
        o.insert("tls".into(), tls_json(p));
    }
    if let Some(t) = transport_json(p) {
        o.insert("transport".into(), t);
    }
    Value::Object(o)
}

fn rule_set_entry(tag: &str, url: String, detour: &str) -> Value {
    json!({ "type": "remote", "tag": tag, "format": "binary", "url": url, "download_detour": detour })
}

/// Converts a profile to a complete sing-box config that exposes a mixed
/// inbound and the Clash API (so the GUI can drive it like mihomo).
pub fn emit(p: &Profile, o: &Overrides) -> Value {
    let mut outbounds: Vec<Value> = Vec::new();
    let mut members_all: Vec<String> = Vec::new();
    let strip = |v: &Vec<String>| -> Vec<String> {
        let l: Vec<String> = v.iter().filter(|x| x.as_str() != "REJECT").cloned().collect();
        if l.is_empty() { vec!["DIRECT".into()] } else { l }
    };
    for g in &p.groups {
        members_all.push(g.name.clone());
        let mut v = match g.kind {
            GroupKind::Select => json!({ "type": "selector", "tag": g.name, "outbounds": strip(&g.proxies) }),
            _ => json!({
                "type": "urltest", "tag": g.name, "outbounds": strip(&g.proxies),
                "url": g.url, "interval": format!("{}s", g.interval), "tolerance": g.tolerance,
            }),
        };
        if g.kind == GroupKind::Select {
            v["interrupt_exist_connections"] = json!(true);
        }
        outbounds.push(v);
    }
    for px in &p.proxies {
        members_all.push(px.name.clone());
        outbounds.push(emit_outbound(px));
    }
    members_all.push("DIRECT".into());
    outbounds.push(json!({ "type": "direct", "tag": "DIRECT" }));
    outbounds.push(json!({ "type": "selector", "tag": "GLOBAL", "outbounds": members_all }));

    let final_target = p
        .rules
        .iter()
        .find(|r| r.kind == "MATCH")
        .map(|r| r.target.clone())
        .or_else(|| p.groups.first().map(|g| g.name.clone()))
        .unwrap_or_else(|| "DIRECT".into());
    let detour = if final_target == "REJECT" { "DIRECT".to_string() } else { final_target.clone() };

    let mut rules = vec![json!({ "action": "sniff" })];
    if o.tun {
        rules.push(json!({ "protocol": "dns", "action": "hijack-dns" }));
    }
    rules.push(json!({ "clash_mode": "Direct", "outbound": "DIRECT" }));
    rules.push(json!({ "clash_mode": "Global", "outbound": "GLOBAL" }));
    let mut sets: BTreeSet<(String, String)> = BTreeSet::new();
    for r in &p.rules {
        let pl = r.payload.clone();
        let cond = match r.kind.as_str() {
            "DOMAIN" => json!({ "domain": [pl] }),
            "DOMAIN-SUFFIX" => json!({ "domain_suffix": [pl] }),
            "DOMAIN-KEYWORD" => json!({ "domain_keyword": [pl] }),
            "DOMAIN-REGEX" => json!({ "domain_regex": [pl] }),
            "IP-CIDR" | "IP-CIDR6" => json!({ "ip_cidr": [pl] }),
            "SRC-IP-CIDR" => json!({ "source_ip_cidr": [pl] }),
            "DST-PORT" => match pl.parse::<u16>() {
                Ok(port) => json!({ "port": [port] }),
                Err(_) => json!({ "port_range": [pl.replace('-', ":")] }),
            },
            "PROCESS-NAME" => json!({ "process_name": [pl] }),
            "GEOIP" if pl.eq_ignore_ascii_case("private") || pl.eq_ignore_ascii_case("lan") => {
                json!({ "ip_is_private": true })
            }
            "GEOIP" => {
                let tag = format!("geoip-{}", pl.to_ascii_lowercase());
                sets.insert((tag.clone(), GEOIP_URL.replace("{}", &pl.to_ascii_lowercase())));
                json!({ "rule_set": [tag] })
            }
            "GEOSITE" => {
                let tag = format!("geosite-{}", pl.to_ascii_lowercase());
                sets.insert((tag.clone(), GEOSITE_URL.replace("{}", &pl.to_ascii_lowercase())));
                json!({ "rule_set": [tag] })
            }
            _ => continue,
        };
        let mut rule = cond;
        if r.target == "REJECT" {
            rule["action"] = json!("reject");
        } else {
            rule["outbound"] = json!(r.target);
        }
        rules.push(rule);
    }
    let dns = if o.tun {
        // System DNS would loop through the TUN: resolve explicitly, foreign
        // names through the proxy (same layout as the sbm subscription).
        sets.insert(("geosite-cn".into(), GEOSITE_URL.replace("{}", "cn")));
        let mut remote = json!({ "type": "https", "tag": "remote", "server": "1.1.1.1" });
        if detour != "DIRECT" {
            remote["detour"] = json!(detour);
        }
        json!({
            "servers": [remote, { "type": "https", "tag": "local", "server": "223.5.5.5" }],
            "rules": [{ "rule_set": "geosite-cn", "server": "local" }],
            "final": "remote",
            "strategy": "prefer_ipv4",
        })
    } else {
        json!({ "servers": [{ "type": "local", "tag": "local" }], "final": "local", "strategy": "prefer_ipv4" })
    };
    json!({
        "log": { "level": singbox_level(&o.log_level), "timestamp": true },
        "dns": dns,
        "inbounds": inbounds(o),
        "outbounds": outbounds,
        "route": {
            "rules": rules,
            "rule_set": sets.iter().map(|(t, u)| rule_set_entry(t, u.clone(), &detour)).collect::<Vec<_>>(),
            "final": if final_target == "REJECT" { "DIRECT".to_string() } else { final_target },
            "auto_detect_interface": true,
            "default_domain_resolver": "local",
        },
        "experimental": clash_api(o),
    })
}

fn inbounds(o: &Overrides) -> Value {
    let listen = if o.allow_lan { "::" } else { "127.0.0.1" };
    let mut v = vec![json!({ "type": "mixed", "tag": "mixed-in", "listen": listen, "listen_port": o.mixed_port })];
    if o.tun {
        v.push(json!({
            "type": "tun", "tag": "tun-in", "interface_name": "Prism",
            "address": ["172.19.0.1/30", "fdfe:dcba:9876::1/126"],
            "mtu": 9000, "auto_route": true, "strict_route": true, "stack": "mixed",
        }));
    }
    Value::Array(v)
}

fn singbox_level(l: &str) -> &'static str {
    match l {
        "debug" => "debug",
        "warning" | "warn" => "warn",
        "error" => "error",
        "silent" => "panic",
        _ => "info",
    }
}

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Rule => "Rule",
        Mode::Global => "Global",
        Mode::Direct => "Direct",
    }
}

fn clash_api(o: &Overrides) -> Value {
    json!({
        "clash_api": {
            "external_controller": o.controller,
            "secret": o.secret,
            "default_mode": mode_name(o.mode),
        },
        "cache_file": { "enabled": true },
    })
}

/// Patches a native sing-box profile: replaces inbounds (TUN needs admin
/// rights, the GUI uses system proxy) and forces the Clash API settings.
pub fn apply_overrides(v: &mut Value, o: &Overrides) {
    let Some(root) = v.as_object_mut() else { return };
    root.insert("inbounds".into(), inbounds(o));
    if o.tun {
        // Make sure DNS from the TUN is answered by sing-box.
        let route = root.entry("route").or_insert_with(|| json!({}));
        if let Some(r) = route.as_object_mut() {
            r.insert("auto_detect_interface".into(), json!(true));
            let rules = r.entry("rules").or_insert_with(|| json!([]));
            if let Some(list) = rules.as_array_mut() {
                if !list.iter().any(|x| x.get("action").and_then(|a| a.as_str()) == Some("hijack-dns")) {
                    list.insert(0, json!({ "protocol": "dns", "action": "hijack-dns" }));
                }
                if !list.iter().any(|x| x.get("action").and_then(|a| a.as_str()) == Some("sniff")) {
                    list.insert(0, json!({ "action": "sniff" }));
                }
            }
        }
    }
    let exp = root.entry("experimental").or_insert_with(|| json!({}));
    if let Some(e) = exp.as_object_mut() {
        if let Value::Object(api) = clash_api(o) {
            for (k, v) in api {
                e.insert(k, v);
            }
        }
    }
    if let Some(log) = root.entry("log").or_insert_with(|| json!({})).as_object_mut() {
        log.insert("level".into(), json!(singbox_level(&o.log_level)));
        log.remove("output");
    }
    // Rules that depended on the removed TUN inbound are harmless; sniff /
    // hijack-dns simply never match.
}

// ---------------------------------------------------------------- parsing

fn st(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(String::from).filter(|s| !s.is_empty())
}

fn strs(v: &Value, k: &str) -> Vec<String> {
    match v.get(k) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(String::from).or_else(|| x.as_u64().map(|n| n.to_string())))
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Number(n)) => vec![n.to_string()],
        _ => Vec::new(),
    }
}

fn secs(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(m) = s.strip_suffix('m') {
        return m.parse::<u64>().ok().map(|x| x * 60);
    }
    s.trim_end_matches('s').parse().ok()
}

pub fn parse(v: &Value) -> Result<Profile> {
    let Some(obs) = v.get("outbounds").and_then(|o| o.as_array()) else { bail!("sing-box config has no outbounds") };
    let mut p = Profile::default();
    let mut rename: HashMap<String, String> = HashMap::new();
    for ob in obs {
        let ty = st(ob, "type").unwrap_or_default();
        let tag = st(ob, "tag").unwrap_or_else(|| ty.clone());
        let kind = match ty.as_str() {
            "direct" => {
                rename.insert(tag, "DIRECT".into());
                continue;
            }
            "block" => {
                rename.insert(tag, "REJECT".into());
                continue;
            }
            "dns" => continue,
            "selector" | "urltest" => {
                let kind = if ty == "selector" { GroupKind::Select } else { GroupKind::UrlTest };
                let mut g = GroupConfig::new(&tag, kind, strs(ob, "outbounds"));
                if let Some(u) = st(ob, "url") {
                    g.url = u;
                }
                if let Some(i) = st(ob, "interval").and_then(|i| secs(&i)) {
                    g.interval = i.max(10);
                }
                if let Some(t) = ob.get("tolerance").and_then(|t| t.as_u64()) {
                    g.tolerance = t;
                }
                p.groups.push(g);
                continue;
            }
            "shadowsocks" => ProxyKind::Shadowsocks,
            "trojan" => ProxyKind::Trojan,
            "vless" => ProxyKind::Vless,
            "vmess" => ProxyKind::Vmess,
            "anytls" => ProxyKind::AnyTls,
            "hysteria2" => ProxyKind::Hysteria2,
            "socks" => ProxyKind::Socks5,
            "http" => ProxyKind::Http,
            other => {
                p.warnings.push(format!("outbound {tag}: unsupported type {other}"));
                continue;
            }
        };
        let tls = ob.get("tls").cloned().unwrap_or(Value::Null);
        let tr = ob.get("transport").cloned().unwrap_or(Value::Null);
        let reality = tls.get("reality").cloned().unwrap_or(Value::Null);
        p.proxies.push(ProxyConfig {
            name: tag,
            kind: Some(kind),
            server: st(ob, "server").unwrap_or_default(),
            port: ob.get("server_port").and_then(|x| x.as_u64()).unwrap_or(0) as u16,
            username: st(ob, "username"),
            password: st(ob, "password"),
            uuid: st(ob, "uuid"),
            cipher: st(ob, "method").or_else(|| st(ob, "security")),
            alter_id: ob.get("alter_id").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
            tls: tls.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false),
            sni: st(&tls, "server_name"),
            skip_cert_verify: tls.get("insecure").and_then(|x| x.as_bool()).unwrap_or(false),
            alpn: strs(&tls, "alpn"),
            fingerprint: tls.get("utls").and_then(|u| st(u, "fingerprint")),
            flow: st(ob, "flow"),
            network: st(&tr, "type"),
            ws_path: st(&tr, "path"),
            ws_host: tr.get("headers").and_then(|h| st(h, "Host")),
            grpc_service: st(&tr, "service_name"),
            reality_public_key: st(&reality, "public_key"),
            reality_short_id: st(&reality, "short_id"),
            obfs: ob.get("obfs").and_then(|o| st(o, "type")),
            obfs_password: ob.get("obfs").and_then(|o| st(o, "password")),
            udp: true,
            ..Default::default()
        });
    }
    let fix = |n: &String| rename.get(n).cloned().unwrap_or_else(|| n.clone());
    for g in &mut p.groups {
        g.proxies = g.proxies.iter().map(fix).collect();
    }
    let route = v.get("route").cloned().unwrap_or(Value::Null);
    for r in route.get("rules").and_then(|r| r.as_array()).cloned().unwrap_or_default() {
        if r.get("clash_mode").is_some() || r.get("protocol").is_some() || r.get("inbound").is_some() {
            continue;
        }
        let target = match (st(&r, "action").as_deref(), st(&r, "outbound")) {
            (Some("reject"), _) => "REJECT".to_string(),
            (None | Some("route"), Some(o)) => fix(&o),
            _ => continue,
        };
        let mut push = |kind: &str, payload: String| {
            p.rules.push(RuleConfig { kind: kind.into(), payload, target: target.clone(), no_resolve: false });
        };
        for (field, kind) in [
            ("domain", "DOMAIN"),
            ("domain_suffix", "DOMAIN-SUFFIX"),
            ("domain_keyword", "DOMAIN-KEYWORD"),
            ("domain_regex", "DOMAIN-REGEX"),
            ("ip_cidr", "IP-CIDR"),
            ("port", "DST-PORT"),
            ("process_name", "PROCESS-NAME"),
        ] {
            for x in strs(&r, field) {
                push(kind, x.trim_start_matches('.').to_string());
            }
        }
        if r.get("ip_is_private").and_then(|x| x.as_bool()) == Some(true) {
            push("GEOIP", "private".into());
        }
        for tag in strs(&r, "rule_set") {
            if let Some(code) = tag.strip_prefix("geosite-") {
                push("GEOSITE", code.into());
            } else if let Some(code) = tag.strip_prefix("geoip-") {
                push("GEOIP", code.to_ascii_uppercase());
            }
        }
    }
    if let Some(f) = st(&route, "final") {
        p.rules.push(RuleConfig { kind: "MATCH".into(), payload: String::new(), target: fix(&f), no_resolve: false });
    }
    if let Some(inb) = v.get("inbounds").and_then(|i| i.as_array()) {
        if let Some(port) = inb.iter().find_map(|i| i.get("listen_port").and_then(|x| x.as_u64())) {
            p.general.mixed_port = port as u16;
        }
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overrides() -> Overrides {
        Overrides {
            mixed_port: 7890,
            controller: "127.0.0.1:9097".into(),
            secret: "s".into(),
            mode: Mode::Rule,
            allow_lan: false,
            log_level: "info".into(),
            tun: false,
        }
    }

    #[test]
    fn clash_to_singbox_and_back() {
        let raw = "anytls://pw@jp.example.com:443/?sni=jp.example.com#jp";
        let p = parse_any(raw).unwrap();
        let sb = emit(&p, &overrides());
        let tags: Vec<&str> = sb["outbounds"].as_array().unwrap().iter().map(|o| o["tag"].as_str().unwrap()).collect();
        assert!(tags.contains(&"jp") && tags.contains(&"GLOBAL") && tags.contains(&"节点选择"));
        assert_eq!(sb["route"]["final"], "节点选择");
        assert!(sb["route"]["rule_set"].as_array().unwrap().iter().any(|r| r["tag"] == "geosite-cn"));

        let back = parse(&sb).unwrap();
        assert_eq!(back.proxies.len(), 1);
        assert_eq!(back.proxies[0].sni.as_deref(), Some("jp.example.com"));
        assert!(back.rules.iter().any(|r| r.kind == "GEOSITE" && r.payload == "cn" && r.target == "DIRECT"));
        assert_eq!(back.rules.last().unwrap().kind, "MATCH");
    }
}
