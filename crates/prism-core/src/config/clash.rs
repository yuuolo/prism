//! Clash / mihomo YAML <-> [`Profile`].

use anyhow::{bail, Result};
use serde_yaml::{Mapping, Value};

use super::*;

fn s(m: &Mapping, k: &str) -> Option<String> {
    match m.get(k)? {
        Value::String(v) => Some(v.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn b(m: &Mapping, k: &str) -> bool {
    match m.get(k) {
        Some(Value::Bool(v)) => *v,
        Some(Value::String(v)) => v == "true",
        _ => false,
    }
}

fn n(m: &Mapping, k: &str) -> Option<u64> {
    match m.get(k)? {
        Value::Number(v) => v.as_u64(),
        Value::String(v) => v.trim().parse().ok(),
        _ => None,
    }
}

fn list(m: &Mapping, k: &str) -> Vec<String> {
    match m.get(k) {
        Some(Value::Sequence(seq)) => seq
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect(),
        Some(Value::String(s)) => s.split(',').map(|x| x.trim().to_string()).collect(),
        _ => Vec::new(),
    }
}

fn sub<'a>(m: &'a Mapping, k: &str) -> Option<&'a Mapping> {
    m.get(k)?.as_mapping()
}

pub fn parse_proxy(m: &Mapping) -> Result<ProxyConfig> {
    let name = s(m, "name").unwrap_or_default();
    let ty = s(m, "type").unwrap_or_default();
    let Some(kind) = ProxyKind::from_clash_type(&ty) else {
        bail!("proxy {name}: unsupported type {ty}");
    };
    // Plugin settings can't be carried over; without them the node is broken.
    if kind == ProxyKind::Shadowsocks && s(m, "plugin").is_some_and(|p| !p.is_empty()) {
        bail!("proxy {name}: shadowsocks plugin {} is not supported", s(m, "plugin").unwrap_or_default());
    }
    let mut p = ProxyConfig {
        name,
        kind: Some(kind),
        server: s(m, "server").unwrap_or_default(),
        port: n(m, "port").unwrap_or(0) as u16,
        username: s(m, "username"),
        password: s(m, "password"),
        uuid: s(m, "uuid"),
        cipher: s(m, "cipher"),
        alter_id: n(m, "alterId").unwrap_or(0) as u32,
        tls: b(m, "tls"),
        sni: s(m, "sni").or_else(|| s(m, "servername")),
        skip_cert_verify: b(m, "skip-cert-verify"),
        alpn: list(m, "alpn"),
        fingerprint: s(m, "client-fingerprint"),
        flow: s(m, "flow").filter(|f| !f.is_empty()),
        network: s(m, "network").filter(|x| x != "tcp"),
        obfs: s(m, "obfs"),
        obfs_password: s(m, "obfs-password"),
        udp: b(m, "udp"),
        ..Default::default()
    };
    if matches!(kind, ProxyKind::Trojan | ProxyKind::AnyTls | ProxyKind::Hysteria2) {
        p.tls = true;
    }
    if let Some(ws) = sub(m, "ws-opts") {
        p.ws_path = s(ws, "path");
        p.ws_host = sub(ws, "headers").and_then(|h| s(h, "Host").or_else(|| s(h, "host")));
    }
    if let Some(g) = sub(m, "grpc-opts") {
        p.grpc_service = s(g, "grpc-service-name");
    }
    if let Some(r) = sub(m, "reality-opts") {
        p.reality_public_key = s(r, "public-key");
        p.reality_short_id = s(r, "short-id");
    }
    if p.server.is_empty() || p.port == 0 {
        bail!("proxy {}: missing server/port", p.name);
    }
    Ok(p)
}

pub fn parse(v: &Value) -> Result<Profile> {
    let Some(root) = v.as_mapping() else { bail!("config is not a YAML mapping") };
    let mut p = Profile::default();
    let g = &mut p.general;
    if let Some(port) = n(root, "mixed-port").or_else(|| n(root, "port")).or_else(|| n(root, "socks-port")) {
        g.mixed_port = port as u16;
    }
    g.allow_lan = b(root, "allow-lan");
    if let Some(x) = s(root, "bind-address") {
        g.bind_address = x;
    }
    if let Some(m) = s(root, "mode").and_then(|m| Mode::parse(&m)) {
        g.mode = m;
    }
    if let Some(x) = s(root, "log-level") {
        g.log_level = x;
    }
    g.external_controller = s(root, "external-controller").filter(|x| !x.is_empty());
    g.secret = s(root, "secret").unwrap_or_default();
    g.unified_delay = b(root, "unified-delay");
    if let Some(m) = s(root, "find-process-mode") {
        g.find_process_mode = m;
    }
    if let Some(t) = sub(root, "tun") {
        g.tun.enable = b(t, "enable");
        g.tun.auto_route = !matches!(t.get("auto-route"), Some(Value::Bool(false)));
    }
    if let Some(d) = sub(root, "dns") {
        g.tun.dns = list(d, "default-nameserver").into_iter().filter(|x| x.parse::<std::net::IpAddr>().is_ok()).collect();
        p.dns = parse_dns(d);
        if d.contains_key("fallback") {
            p.warnings.push("dns: fallback / fallback-filter are not supported by prism-core and were ignored".into());
        }
    }
    if let Some(h) = sub(root, "hosts") {
        for (k, v) in h {
            let Some(name) = k.as_str() else { continue };
            let ips = match v {
                Value::String(s) => vec![s.clone()],
                Value::Sequence(seq) => seq.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
                _ => vec![],
            };
            for ip in ips {
                p.hosts.push((name.to_string(), ip));
            }
        }
    }
    if let Some(m) = sub(root, "proxy-providers") {
        for (k, v) in m {
            let (Some(name), Some(c)) = (k.as_str(), v.as_mapping()) else { continue };
            p.proxy_providers.push(parse_proxy_provider(name, c, &mut p.warnings));
        }
    }
    if let Some(m) = sub(root, "rule-providers") {
        for (k, v) in m {
            let (Some(name), Some(c)) = (k.as_str(), v.as_mapping()) else { continue };
            p.rule_providers.push(parse_rule_provider(name, c));
        }
    }
    if let Some(Value::Sequence(seq)) = root.get("proxies") {
        for item in seq {
            match item.as_mapping().map(parse_proxy) {
                Some(Ok(px)) => p.proxies.push(px),
                Some(Err(e)) => p.warnings.push(e.to_string()),
                None => {}
            }
        }
    }
    if let Some(Value::Sequence(seq)) = root.get("proxy-groups") {
        for item in seq {
            let Some(m) = item.as_mapping() else { continue };
            let name = s(m, "name").unwrap_or_default();
            let kind = match s(m, "type").unwrap_or_default().as_str() {
                "select" => GroupKind::Select,
                "url-test" => GroupKind::UrlTest,
                "fallback" => GroupKind::Fallback,
                "load-balance" => GroupKind::LoadBalance,
                other => {
                    p.warnings.push(format!("group {name}: unsupported type {other}, treated as select"));
                    GroupKind::Select
                }
            };
            let mut gc = GroupConfig::new(&name, kind, list(m, "proxies"));
            if let Some(u) = s(m, "url") {
                gc.url = u;
            }
            if let Some(i) = n(m, "interval") {
                gc.interval = i.max(10);
            }
            if let Some(t) = n(m, "tolerance") {
                gc.tolerance = t;
            }
            gc.use_providers = list(m, "use");
            gc.filter = s(m, "filter").filter(|f| !f.is_empty());
            gc.exclude_filter = s(m, "exclude-filter").filter(|f| !f.is_empty());
            p.groups.push(gc);
        }
    }
    if let Some(Value::Sequence(seq)) = root.get("rules") {
        for item in seq {
            if let Some(line) = item.as_str() {
                match RuleConfig::parse(line) {
                    Some(r) => p.rules.push(r),
                    None => p.warnings.push(format!("bad rule: {line}")),
                }
            }
        }
    }
    Ok(p)
}

fn parse_dns(d: &Mapping) -> DnsConfig {
    let mut policy = Vec::new();
    if let Some(np) = sub(d, "nameserver-policy") {
        for (k, v) in np {
            let Some(key) = k.as_str() else { continue };
            let servers = match v {
                Value::String(s) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
                Value::Sequence(_) => {
                    let mut tmp = Mapping::new();
                    tmp.insert(Value::String("v".into()), v.clone());
                    list(&tmp, "v")
                }
                _ => vec![],
            };
            policy.push((key.to_string(), servers));
        }
    }
    DnsConfig {
        enable: b(d, "enable"),
        listen: s(d, "listen"),
        ipv6: b(d, "ipv6"),
        enhanced_mode: s(d, "enhanced-mode").unwrap_or_else(|| "normal".into()),
        fake_ip_range: s(d, "fake-ip-range").unwrap_or_default(),
        fake_ip_filter: list(d, "fake-ip-filter"),
        default_nameserver: list(d, "default-nameserver"),
        nameserver: list(d, "nameserver"),
        proxy_server_nameserver: list(d, "proxy-server-nameserver"),
        nameserver_policy: policy,
    }
}

fn parse_proxy_provider(name: &str, c: &Mapping, warnings: &mut Vec<String>) -> ProxyProviderConfig {
    let hc = sub(c, "health-check");
    let mut inline = Vec::new();
    if let Some(Value::Sequence(seq)) = c.get("payload") {
        for item in seq {
            match item.as_mapping().map(parse_proxy) {
                Some(Ok(px)) => inline.push(px),
                Some(Err(e)) => warnings.push(format!("provider {name}: {e}")),
                None => {}
            }
        }
    }
    ProxyProviderConfig {
        name: name.to_string(),
        kind: s(c, "type").unwrap_or_else(|| "http".into()),
        url: s(c, "url"),
        path: s(c, "path"),
        interval: n(c, "interval").unwrap_or(0),
        filter: s(c, "filter").filter(|f| !f.is_empty()),
        exclude_filter: s(c, "exclude-filter").filter(|f| !f.is_empty()),
        health_check: hc.is_some_and(|h| b(h, "enable")),
        health_check_url: hc.and_then(|h| s(h, "url")),
        health_check_interval: hc.and_then(|h| n(h, "interval")).unwrap_or(300),
        proxy: s(c, "proxy"),
        inline,
    }
}

fn parse_rule_provider(name: &str, c: &Mapping) -> RuleProviderConfig {
    RuleProviderConfig {
        name: name.to_string(),
        kind: s(c, "type").unwrap_or_else(|| "http".into()),
        behavior: s(c, "behavior").unwrap_or_else(|| "classical".into()).to_ascii_lowercase(),
        format: s(c, "format").unwrap_or_else(|| "yaml".into()).to_ascii_lowercase(),
        url: s(c, "url"),
        path: s(c, "path"),
        interval: n(c, "interval").unwrap_or(0),
        proxy: s(c, "proxy"),
        inline: list(c, "payload"),
    }
}

fn put(m: &mut Mapping, k: &str, v: impl Into<Value>) {
    m.insert(Value::String(k.into()), v.into());
}

fn put_opt(m: &mut Mapping, k: &str, v: &Option<String>) {
    if let Some(v) = v {
        put(m, k, v.clone());
    }
}

pub fn emit_proxy(p: &ProxyConfig) -> Value {
    let kind = p.kind();
    let mut m = Mapping::new();
    put(&mut m, "name", p.name.clone());
    put(&mut m, "type", kind.clash_type());
    put(&mut m, "server", p.server.clone());
    put(&mut m, "port", p.port as u64);
    put_opt(&mut m, "username", &p.username);
    put_opt(&mut m, "password", &p.password);
    put_opt(&mut m, "uuid", &p.uuid);
    match kind {
        ProxyKind::Shadowsocks => put_opt(&mut m, "cipher", &p.cipher),
        ProxyKind::Vmess => {
            put(&mut m, "alterId", p.alter_id as u64);
            put(&mut m, "cipher", p.cipher.clone().unwrap_or_else(|| "auto".into()));
        }
        _ => {}
    }
    if p.tls && matches!(kind, ProxyKind::Vless | ProxyKind::Vmess | ProxyKind::Socks5 | ProxyKind::Http) {
        put(&mut m, "tls", true);
    }
    if let Some(sni) = &p.sni {
        let key = if matches!(kind, ProxyKind::Vless | ProxyKind::Vmess) { "servername" } else { "sni" };
        put(&mut m, key, sni.clone());
    }
    if p.skip_cert_verify {
        put(&mut m, "skip-cert-verify", true);
    }
    if !p.alpn.is_empty() {
        put(&mut m, "alpn", Value::Sequence(p.alpn.iter().cloned().map(Value::String).collect()));
    }
    put_opt(&mut m, "client-fingerprint", &p.fingerprint);
    put_opt(&mut m, "flow", &p.flow);
    if let Some(net) = &p.network {
        put(&mut m, "network", net.clone());
        if net == "ws" {
            let mut ws = Mapping::new();
            put(&mut ws, "path", p.ws_path.clone().unwrap_or_else(|| "/".into()));
            if let Some(h) = &p.ws_host {
                let mut hd = Mapping::new();
                put(&mut hd, "Host", h.clone());
                put(&mut ws, "headers", Value::Mapping(hd));
            }
            put(&mut m, "ws-opts", Value::Mapping(ws));
        } else if net == "grpc" {
            let mut g = Mapping::new();
            put_opt(&mut g, "grpc-service-name", &p.grpc_service);
            put(&mut m, "grpc-opts", Value::Mapping(g));
        }
    }
    if p.reality_public_key.is_some() {
        let mut r = Mapping::new();
        put_opt(&mut r, "public-key", &p.reality_public_key);
        put_opt(&mut r, "short-id", &p.reality_short_id);
        put(&mut m, "reality-opts", Value::Mapping(r));
    }
    put_opt(&mut m, "obfs", &p.obfs);
    put_opt(&mut m, "obfs-password", &p.obfs_password);
    put(&mut m, "udp", p.udp);
    Value::Mapping(m)
}

/// Emits a complete Clash config (general section + proxies + groups + rules).
pub fn emit(p: &Profile) -> Value {
    let mut root = Mapping::new();
    let g = &p.general;
    put(&mut root, "mixed-port", g.mixed_port as u64);
    put(&mut root, "allow-lan", g.allow_lan);
    put(&mut root, "mode", g.mode.as_str());
    put(&mut root, "log-level", g.log_level.clone());
    put(&mut root, "proxies", Value::Sequence(p.proxies.iter().map(emit_proxy).collect()));
    let groups = p
        .groups
        .iter()
        .map(|gc| {
            let mut m = Mapping::new();
            put(&mut m, "name", gc.name.clone());
            put(&mut m, "type", gc.kind.clash_type());
            put(&mut m, "proxies", Value::Sequence(gc.proxies.iter().cloned().map(Value::String).collect()));
            if gc.kind != GroupKind::Select {
                put(&mut m, "url", gc.url.clone());
                put(&mut m, "interval", gc.interval);
                if gc.kind == GroupKind::UrlTest {
                    put(&mut m, "tolerance", gc.tolerance);
                }
            }
            Value::Mapping(m)
        })
        .collect();
    put(&mut root, "proxy-groups", Value::Sequence(groups));
    put(&mut root, "rules", Value::Sequence(p.rules.iter().map(|r| Value::String(r.to_line())).collect()));
    Value::Mapping(root)
}

/// Puts per-application rules in front of the profile's rules. Rules whose
/// target doesn't exist in the profile are skipped (mihomo would refuse
/// the whole config); returns warnings for those.
pub fn add_app_rules(v: &mut Value, routes: &[AppRoute]) -> Vec<String> {
    let mut warnings = Vec::new();
    let Some(root) = v.as_mapping_mut() else { return warnings };
    let mut names: std::collections::HashSet<String> = ["DIRECT", "REJECT", "GLOBAL"].iter().map(|s| s.to_string()).collect();
    for key in ["proxies", "proxy-groups"] {
        if let Some(Value::Sequence(seq)) = root.get(key) {
            names.extend(seq.iter().filter_map(|x| x.as_mapping().and_then(|m| s(m, "name"))));
        }
    }
    let mut lines = Vec::new();
    for r in routes {
        if let Err(e) = r.valid() {
            warnings.push(format!("app rule skipped: {e}"));
            continue;
        }
        if !names.contains(&r.target) {
            warnings.push(format!("app rule {} skipped: {} is not in this profile", r.value, r.target));
            continue;
        }
        let kind = if r.by_path { "PROCESS-PATH" } else { "PROCESS-NAME" };
        lines.push(Value::String(format!("{kind},{},{}", r.process_name(), r.target)));
    }
    if !lines.is_empty() {
        let rules = root.entry(Value::String("rules".into())).or_insert_with(|| Value::Sequence(vec![]));
        if let Value::Sequence(seq) = rules {
            lines.append(seq);
            *seq = lines;
        }
    }
    warnings
}

/// Runtime settings the GUI forces onto every generated config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Overrides {
    pub mixed_port: u16,
    pub controller: String,
    pub secret: String,
    pub mode: Mode,
    pub allow_lan: bool,
    pub log_level: String,
    #[serde(default)]
    pub tun: bool,
    #[serde(default)]
    pub unified_delay: bool,
}

fn seq(items: &[&str]) -> Value {
    Value::Sequence(items.iter().map(|s| Value::String(s.to_string())).collect())
}

/// Patches a Clash YAML document in place with the GUI's runtime settings,
/// keeping everything else (DNS, providers, rule-providers...) untouched so
/// mihomo still sees the full original profile.
pub fn apply_overrides(v: &mut Value, o: &Overrides) {
    let Some(root) = v.as_mapping_mut() else { return };
    for k in ["port", "socks-port", "redir-port", "tproxy-port", "tun", "external-ui", "external-controller-tls"] {
        root.remove(k);
    }
    if o.tun {
        let mut t = Mapping::new();
        put(&mut t, "enable", true);
        put(&mut t, "stack", "mixed");
        put(&mut t, "device", "Prism");
        put(&mut t, "auto-route", true);
        put(&mut t, "auto-detect-interface", true);
        put(&mut t, "dns-hijack", seq(&["any:53"]));
        put(&mut t, "mtu", 9000u64);
        put(root, "tun", Value::Mapping(t));
        // TUN needs a DNS module; keep the profile's if it has one.
        let dns_on = root.get("dns").and_then(|d| d.as_mapping()).is_some_and(|d| b(d, "enable"));
        if !dns_on {
            let mut d = Mapping::new();
            put(&mut d, "enable", true);
            put(&mut d, "enhanced-mode", "fake-ip");
            put(&mut d, "fake-ip-range", "198.18.0.1/16");
            put(&mut d, "fake-ip-filter", seq(&["*.lan", "+.local", "+.msftconnecttest.com", "+.msftncsi.com", "localhost.ptlogin2.qq.com"]));
            put(&mut d, "default-nameserver", seq(&["223.5.5.5", "119.29.29.29"]));
            put(&mut d, "nameserver", seq(&["https://doh.pub/dns-query", "https://dns.alidns.com/dns-query"]));
            put(root, "dns", Value::Mapping(d));
        }
    }
    put(root, "mixed-port", o.mixed_port as u64);
    put(root, "external-controller", o.controller.clone());
    put(root, "secret", o.secret.clone());
    put(root, "mode", o.mode.as_str());
    put(root, "allow-lan", o.allow_lan);
    put(root, "log-level", o.log_level.clone());
    put(root, "unified-delay", o.unified_delay);
    // Lets the connections page show which program made each connection.
    put(root, "find-process-mode", "always");
}

#[cfg(test)]
mod tests {
    use super::*;

    const SBM: &str = r#"
mixed-port: 7890
mode: rule
proxies:
  - name: "东京"
    type: anytls
    server: "jp.example.com"
    port: 443
    password: "pw"
    sni: "jp.example.com"
    client-fingerprint: chrome
    udp: true
  - name: weird
    type: snell
    server: a
    port: 1
  - name: obfs
    type: ss
    server: b
    port: 2
    cipher: aes-128-gcm
    password: x
    plugin: obfs
proxy-groups:
  - name: PROXY
    type: select
    proxies: ["东京", weird, DIRECT]
rules:
  - GEOSITE,cn,DIRECT
  - GEOIP,private,DIRECT,no-resolve
  - MATCH,PROXY
"#;

    #[test]
    fn app_rules_prepended() {
        let mut v: Value = serde_yaml::from_str(SBM).unwrap();
        let routes = [
            AppRoute { by_path: false, value: "telegram".into(), target: "PROXY".into() },
            AppRoute { by_path: true, value: "D:\\Steam\\steam.exe".into(), target: "DIRECT".into() },
            AppRoute { by_path: false, value: "x.exe".into(), target: "NOPE".into() },
            AppRoute { by_path: false, value: "a,b".into(), target: "DIRECT".into() },
        ];
        let w = add_app_rules(&mut v, &routes);
        assert_eq!(w.len(), 2, "{w:?}");
        let p = parse(&v).unwrap();
        assert_eq!(p.rules[0].to_line(), "PROCESS-NAME,telegram.exe,PROXY");
        assert_eq!(p.rules[1].to_line(), "PROCESS-PATH,D:\\Steam\\steam.exe,DIRECT");
        assert_eq!(p.rules[2].to_line(), "GEOSITE,cn,DIRECT");
    }

    #[test]
    fn parses_sbm_subscription() {
        let mut p = parse(&serde_yaml::from_str(SBM).unwrap()).unwrap();
        p.sanitize();
        assert_eq!(p.proxies.len(), 1);
        assert_eq!(p.proxies[0].kind, Some(ProxyKind::AnyTls));
        assert!(p.proxies[0].tls);
        assert_eq!(p.groups[0].proxies, vec!["东京", "DIRECT"]);
        assert_eq!(p.rules.len(), 3);
        // round trip
        let again = parse(&emit(&p)).unwrap();
        assert_eq!(again.proxies[0].password.as_deref(), Some("pw"));
        assert_eq!(again.rules, p.rules);
    }
}
