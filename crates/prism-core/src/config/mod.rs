//! Unified profile model. Clash/mihomo YAML, sing-box JSON and share-link
//! subscriptions are all parsed into [`Profile`], which can be emitted again
//! as Clash YAML (for prism-core and mihomo) or sing-box JSON.

pub mod clash;
pub mod singbox;
pub mod uri;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyKind {
    Shadowsocks,
    Trojan,
    Vless,
    Vmess,
    AnyTls,
    Hysteria2,
    Socks5,
    Http,
}

impl ProxyKind {
    /// Type string as used by Clash (`type:` field) and its REST API.
    pub fn clash_type(self) -> &'static str {
        match self {
            ProxyKind::Shadowsocks => "ss",
            ProxyKind::Trojan => "trojan",
            ProxyKind::Vless => "vless",
            ProxyKind::Vmess => "vmess",
            ProxyKind::AnyTls => "anytls",
            ProxyKind::Hysteria2 => "hysteria2",
            ProxyKind::Socks5 => "socks5",
            ProxyKind::Http => "http",
        }
    }

    pub fn from_clash_type(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "ss" | "shadowsocks" => ProxyKind::Shadowsocks,
            "trojan" => ProxyKind::Trojan,
            "vless" => ProxyKind::Vless,
            "vmess" => ProxyKind::Vmess,
            "anytls" => ProxyKind::AnyTls,
            "hysteria2" | "hy2" => ProxyKind::Hysteria2,
            "socks5" | "socks" => ProxyKind::Socks5,
            "http" => ProxyKind::Http,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub name: String,
    pub kind: Option<ProxyKind>,
    pub server: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub uuid: Option<String>,
    pub cipher: Option<String>,
    pub alter_id: u32,
    pub tls: bool,
    pub sni: Option<String>,
    pub skip_cert_verify: bool,
    pub alpn: Vec<String>,
    pub fingerprint: Option<String>,
    pub flow: Option<String>,
    /// Transport: tcp (default), ws, grpc.
    pub network: Option<String>,
    pub ws_path: Option<String>,
    pub ws_host: Option<String>,
    pub grpc_service: Option<String>,
    pub reality_public_key: Option<String>,
    pub reality_short_id: Option<String>,
    pub obfs: Option<String>,
    pub obfs_password: Option<String>,
    pub udp: bool,
}

impl ProxyConfig {
    pub fn kind(&self) -> ProxyKind {
        self.kind.unwrap_or(ProxyKind::Http)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GroupKind {
    Select,
    UrlTest,
    Fallback,
    LoadBalance,
}

impl GroupKind {
    pub fn clash_type(self) -> &'static str {
        match self {
            GroupKind::Select => "select",
            GroupKind::UrlTest => "url-test",
            GroupKind::Fallback => "fallback",
            GroupKind::LoadBalance => "load-balance",
        }
    }

    /// Type name reported by the Clash REST API.
    pub fn api_type(self) -> &'static str {
        match self {
            GroupKind::Select => "Selector",
            GroupKind::UrlTest => "URLTest",
            GroupKind::Fallback => "Fallback",
            GroupKind::LoadBalance => "LoadBalance",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupConfig {
    pub name: String,
    pub kind: GroupKind,
    pub proxies: Vec<String>,
    pub url: String,
    /// Health-check interval in seconds.
    pub interval: u64,
    /// url-test switch threshold in milliseconds.
    pub tolerance: u64,
    /// proxy-providers whose nodes are appended to `proxies` (Clash `use`).
    #[serde(default)]
    pub use_providers: Vec<String>,
    /// Regex on node names from providers (Clash `filter` / `exclude-filter`).
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub exclude_filter: Option<String>,
}

pub const DEFAULT_TEST_URL: &str = "https://www.gstatic.com/generate_204";

impl GroupConfig {
    pub fn new(name: &str, kind: GroupKind, proxies: Vec<String>) -> Self {
        GroupConfig {
            name: name.to_string(),
            kind,
            proxies,
            url: DEFAULT_TEST_URL.into(),
            interval: 300,
            tolerance: 50,
            use_providers: Vec::new(),
            filter: None,
            exclude_filter: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleConfig {
    /// Upper-case rule type, e.g. DOMAIN-SUFFIX, GEOIP, MATCH.
    pub kind: String,
    pub payload: String,
    pub target: String,
    pub no_resolve: bool,
}

impl RuleConfig {
    /// Parses a Clash rule line: `TYPE,payload,target[,no-resolve]` or `MATCH,target`.
    pub fn parse(line: &str) -> Option<Self> {
        let parts: Vec<&str> = line.split(',').map(str::trim).collect();
        let kind = parts.first()?.to_ascii_uppercase();
        if kind == "MATCH" || kind == "FINAL" {
            return Some(RuleConfig {
                kind: "MATCH".into(),
                payload: String::new(),
                target: parts.get(1)?.to_string(),
                no_resolve: false,
            });
        }
        Some(RuleConfig {
            kind,
            payload: parts.get(1)?.to_string(),
            target: parts.get(2)?.to_string(),
            no_resolve: parts.iter().skip(3).any(|p| p.eq_ignore_ascii_case("no-resolve")),
        })
    }

    pub fn to_line(&self) -> String {
        if self.kind == "MATCH" {
            return format!("MATCH,{}", self.target);
        }
        let mut s = format!("{},{},{}", self.kind, self.payload, self.target);
        if self.no_resolve {
            s.push_str(",no-resolve");
        }
        s
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Rule,
    Global,
    Direct,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "rule" => Some(Mode::Rule),
            "global" => Some(Mode::Global),
            "direct" => Some(Mode::Direct),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Rule => "rule",
            Mode::Global => "global",
            Mode::Direct => "direct",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TunConfig {
    pub enable: bool,
    pub auto_route: bool,
    /// Plain-IP DNS servers used for queries sent to the TUN's own DNS address.
    pub dns: Vec<String>,
}

impl TunConfig {
    pub fn dns_upstream(&self) -> Vec<std::net::IpAddr> {
        let v: Vec<std::net::IpAddr> = self.dns.iter().filter_map(|s| s.trim().parse().ok()).collect();
        if v.is_empty() {
            vec!["223.5.5.5".parse().unwrap(), "119.29.29.29".parse().unwrap()]
        } else {
            v
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct General {
    pub mixed_port: u16,
    pub allow_lan: bool,
    pub bind_address: String,
    pub mode: Mode,
    pub log_level: String,
    pub external_controller: Option<String>,
    pub secret: String,
    #[serde(default)]
    pub tun: TunConfig,
    #[serde(default)]
    pub unified_delay: bool,
}

impl Default for General {
    fn default() -> Self {
        General {
            mixed_port: 7890,
            allow_lan: false,
            bind_address: "*".into(),
            mode: Mode::Rule,
            log_level: "info".into(),
            external_controller: Some("127.0.0.1:9090".into()),
            secret: String::new(),
            tun: TunConfig::default(),
            unified_delay: false,
        }
    }
}

/// Clash `dns:` section.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DnsConfig {
    pub enable: bool,
    pub listen: Option<String>,
    pub ipv6: bool,
    /// normal / fake-ip / redir-host
    pub enhanced_mode: String,
    pub fake_ip_range: String,
    pub fake_ip_filter: Vec<String>,
    pub default_nameserver: Vec<String>,
    pub nameserver: Vec<String>,
    pub proxy_server_nameserver: Vec<String>,
    /// (domain patterns, servers), in order.
    pub nameserver_policy: Vec<(String, Vec<String>)>,
}

/// One entry of `proxy-providers:`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProxyProviderConfig {
    pub name: String,
    /// http / file / inline
    pub kind: String,
    pub url: Option<String>,
    pub path: Option<String>,
    /// Refresh interval in seconds (http).
    pub interval: u64,
    pub filter: Option<String>,
    pub exclude_filter: Option<String>,
    pub health_check: bool,
    pub health_check_url: Option<String>,
    pub health_check_interval: u64,
    /// Outbound used to download (default DIRECT).
    pub proxy: Option<String>,
    /// `inline` providers carry their nodes directly.
    pub inline: Vec<ProxyConfig>,
}

/// One entry of `rule-providers:`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RuleProviderConfig {
    pub name: String,
    /// http / file / inline
    pub kind: String,
    /// domain / ipcidr / classical
    pub behavior: String,
    /// yaml / text / mrs
    pub format: String,
    pub url: Option<String>,
    pub path: Option<String>,
    pub interval: u64,
    pub proxy: Option<String>,
    pub inline: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Profile {
    pub general: General,
    pub proxies: Vec<ProxyConfig>,
    pub groups: Vec<GroupConfig>,
    pub rules: Vec<RuleConfig>,
    pub dns: DnsConfig,
    /// Static `hosts:` entries (name, ip).
    pub hosts: Vec<(String, String)>,
    pub proxy_providers: Vec<ProxyProviderConfig>,
    pub rule_providers: Vec<RuleProviderConfig>,
    /// Things that were skipped while parsing (unknown proxy types, ...).
    pub warnings: Vec<String>,
}

/// Built-in outbound names that are always available.
pub const BUILTIN: [&str; 3] = ["DIRECT", "REJECT", "GLOBAL"];

impl Profile {
    /// Adds a default group layout + China-direct rules for subscriptions that
    /// only carry nodes (share-link lists).
    pub fn with_default_groups(mut self) -> Self {
        let names: Vec<String> = self.proxies.iter().map(|p| p.name.clone()).collect();
        if self.groups.is_empty() && !names.is_empty() {
            let mut sel = vec!["自动选择".to_string()];
            sel.extend(names.iter().cloned());
            sel.push("DIRECT".into());
            self.groups.push(GroupConfig::new("节点选择", GroupKind::Select, sel));
            self.groups.push(GroupConfig::new("自动选择", GroupKind::UrlTest, names));
        }
        if self.rules.is_empty() {
            let target = self.groups.first().map(|g| g.name.clone()).unwrap_or_else(|| "DIRECT".into());
            for line in [
                "GEOSITE,private,DIRECT",
                "GEOIP,private,DIRECT,no-resolve",
                "GEOSITE,cn,DIRECT",
                "GEOIP,CN,DIRECT",
            ] {
                self.rules.push(RuleConfig::parse(line).unwrap());
            }
            self.rules.push(RuleConfig::parse(&format!("MATCH,{target}")).unwrap());
        }
        self
    }

    /// Drops references to unknown outbounds so that every group / rule target
    /// resolves. Groups left empty fall back to DIRECT.
    pub fn sanitize(&mut self) {
        let mut known: std::collections::HashSet<String> =
            self.proxies.iter().map(|p| p.name.clone()).collect();
        known.extend(self.groups.iter().map(|g| g.name.clone()));
        known.extend(BUILTIN.iter().map(|s| s.to_string()));
        let mut warnings = Vec::new();
        for g in &mut self.groups {
            g.proxies.retain(|p| {
                let ok = known.contains(p);
                if !ok {
                    warnings.push(format!("group {}: unknown member {p}", g.name));
                }
                ok
            });
            // Groups filled from providers are completed when the core loads them.
            if g.proxies.is_empty() && g.use_providers.is_empty() {
                g.proxies.push("DIRECT".into());
            }
        }
        self.rules.retain(|r| {
            let ok = known.contains(&r.target);
            if !ok {
                warnings.push(format!("rule {}: unknown target", r.to_line()));
            }
            ok
        });
        self.warnings.extend(warnings);
    }
}

/// Detected format of a raw subscription / config file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Clash,
    SingBox,
    Uri,
}

pub fn detect(raw: &str) -> Format {
    let t = raw.trim_start_matches('\u{feff}').trim();
    if t.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(t) {
            if v.get("outbounds").is_some() || v.get("inbounds").is_some() {
                return Format::SingBox;
            }
            if v.get("proxies").is_some() {
                return Format::Clash;
            }
        }
    }
    if let Ok(serde_yaml::Value::Mapping(m)) = serde_yaml::from_str::<serde_yaml::Value>(t) {
        if m.contains_key("proxies") || m.contains_key("proxy-groups") || m.contains_key("proxy-providers") {
            return Format::Clash;
        }
    }
    // Broken YAML that is clearly a Clash config: report the YAML error
    // instead of "no nodes found".
    if t.lines().any(|l| l.starts_with("proxies:") || l.starts_with("proxy-groups:")) {
        return Format::Clash;
    }
    Format::Uri
}

/// Parses any supported format into a [`Profile`].
pub fn parse_any(raw: &str) -> anyhow::Result<Profile> {
    let t = raw.trim_start_matches('\u{feff}');
    let mut p = match detect(t) {
        Format::Clash => clash::parse(&serde_yaml::from_str(t)?)?,
        Format::SingBox => singbox::parse(&serde_json::from_str(t)?)?,
        Format::Uri => {
            let (proxies, warnings) = uri::parse_list(t);
            if proxies.is_empty() {
                anyhow::bail!("no supported nodes found in subscription");
            }
            Profile { proxies, warnings, ..Default::default() }.with_default_groups()
        }
    };
    p.sanitize();
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_lines() {
        let r = RuleConfig::parse("GEOIP,private,DIRECT,no-resolve").unwrap();
        assert!(r.no_resolve);
        assert_eq!(r.to_line(), "GEOIP,private,DIRECT,no-resolve");
        assert_eq!(RuleConfig::parse("FINAL,PROXY").unwrap().to_line(), "MATCH,PROXY");
    }

    #[test]
    fn detects_formats() {
        assert_eq!(detect("proxies:\n  - {name: a, type: ss}\n"), Format::Clash);
        assert_eq!(detect(r#"{"outbounds":[]}"#), Format::SingBox);
        assert_eq!(detect("anytls://pw@a.com:443#x"), Format::Uri);
    }
}
