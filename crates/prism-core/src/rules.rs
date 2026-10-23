//! Clash-style rule matching, including `RULE-SET` (rule-providers).

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ipnet::IpNet;
use regex::Regex;

use crate::common::{is_private_ip, Address};
use crate::config::RuleConfig;
use crate::domain::DomainMatcher;
use crate::geo::{self, DomainSet, IpSet};
use crate::tunnel::{Metadata, Network};

enum Cond {
    Domain(String),
    Suffix(String),
    Keyword(String),
    Regex(Regex),
    GeoSite(Arc<DomainSet>),
    GeoIp(Arc<IpSet>),
    Private,
    Cidr(IpNet),
    SrcCidr(IpNet),
    DstPort(Vec<(u16, u16)>),
    SrcPort(Vec<(u16, u16)>),
    InType(String),
    Network(Network),
    RuleSet(Arc<RuleSet>),
    Match,
}

impl Cond {
    fn needs_ip(&self) -> bool {
        match self {
            Cond::GeoIp(_) | Cond::Private | Cond::Cidr(_) => true,
            Cond::RuleSet(rs) => rs.needs_ip,
            _ => false,
        }
    }

    fn matches(&self, meta: &Metadata, ip: Option<IpAddr>) -> bool {
        let host = meta.dst.domain();
        let in_ports = |v: &[(u16, u16)], p: u16| v.iter().any(|(a, b)| p >= *a && p <= *b);
        match self {
            Cond::Domain(d) => host == Some(d.as_str()),
            Cond::Suffix(s) => host.is_some_and(|h| h == s || (h.ends_with(s.as_str()) && h[..h.len() - s.len()].ends_with('.'))),
            Cond::Keyword(k) => host.is_some_and(|h| h.contains(k.as_str())),
            Cond::Regex(r) => host.is_some_and(|h| r.is_match(h)),
            Cond::GeoSite(set) => host.is_some_and(|h| set.matches(h)),
            Cond::GeoIp(set) => ip.is_some_and(|ip| set.contains(ip)),
            Cond::Private => ip.is_some_and(is_private_ip) || host == Some("localhost"),
            Cond::Cidr(net) => ip.is_some_and(|ip| net.contains(&ip)),
            Cond::SrcCidr(net) => net.contains(&meta.src.ip()),
            Cond::DstPort(v) => in_ports(v, meta.dst.port()),
            Cond::SrcPort(v) => in_ports(v, meta.src.port()),
            Cond::InType(t) => meta.inbound.eq_ignore_ascii_case(t),
            Cond::Network(n) => meta.network == *n,
            Cond::RuleSet(rs) => match &rs.matcher {
                SetMatcher::Domain(m) => host.is_some_and(|h| m.matches(h)),
                SetMatcher::IpCidr(set) => ip.is_some_and(|ip| set.contains(ip)),
                SetMatcher::Classical(conds) => conds.iter().any(|c| c.matches(meta, ip)),
            },
            Cond::Match => true,
        }
    }
}

/// Conditions that need no external data. `None` = unknown rule type,
/// `Some(None)` = invalid payload.
fn simple_cond(kind: &str, p: &str) -> Option<Option<Cond>> {
    Some(match kind {
        "DOMAIN" => Some(Cond::Domain(p.to_ascii_lowercase())),
        "DOMAIN-SUFFIX" => Some(Cond::Suffix(p.trim_start_matches('.').to_ascii_lowercase())),
        "DOMAIN-KEYWORD" => Some(Cond::Keyword(p.to_ascii_lowercase())),
        "DOMAIN-REGEX" => Regex::new(p).ok().map(Cond::Regex),
        "GEOIP" if matches!(p.to_ascii_lowercase().as_str(), "private" | "lan") => Some(Cond::Private),
        "IP-CIDR" | "IP-CIDR6" => parse_net(p).map(Cond::Cidr),
        "SRC-IP-CIDR" => parse_net(p).map(Cond::SrcCidr),
        "DST-PORT" => ports(p).map(Cond::DstPort),
        "SRC-PORT" => ports(p).map(Cond::SrcPort),
        "IN-TYPE" => Some(Cond::InType(p.to_ascii_uppercase())),
        "NETWORK" => match p.to_ascii_lowercase().as_str() {
            "tcp" => Some(Cond::Network(Network::Tcp)),
            "udp" => Some(Cond::Network(Network::Udp)),
            _ => None,
        },
        "MATCH" => Some(Cond::Match),
        _ => return None,
    })
}

enum SetMatcher {
    Domain(DomainMatcher),
    IpCidr(IpSet),
    Classical(Vec<Cond>),
}

/// A loaded rule-provider.
pub struct RuleSet {
    matcher: SetMatcher,
    needs_ip: bool,
    pub len: usize,
}

impl RuleSet {
    /// Builds a rule-set from provider entries for `behavior`
    /// (domain / ipcidr / classical). Bad entries are reported in `warnings`.
    pub fn new(name: &str, behavior: &str, entries: &[String], warnings: &mut Vec<String>) -> RuleSet {
        let mut bad = 0;
        let (matcher, len) = match behavior {
            "domain" => {
                let mut m = DomainMatcher::default();
                for e in entries {
                    if !m.add(e) {
                        bad += 1;
                    }
                }
                let n = m.len();
                (SetMatcher::Domain(m), n)
            }
            "ipcidr" => {
                let nets: Vec<IpNet> = entries.iter().filter_map(|e| parse_net(e).or_else(|| { bad += 1; None })).collect();
                let n = nets.len();
                (SetMatcher::IpCidr(IpSet::from_nets(nets)), n)
            }
            _ => {
                let mut conds = Vec::new();
                for e in entries {
                    let mut parts = e.splitn(3, ',').map(str::trim);
                    let kind = parts.next().unwrap_or_default().to_ascii_uppercase();
                    let payload = parts.next().unwrap_or_default();
                    match simple_cond(&kind, payload) {
                        Some(Some(c)) => conds.push(c),
                        _ => bad += 1,
                    }
                }
                let n = conds.len();
                (SetMatcher::Classical(conds), n)
            }
        };
        if bad > 0 {
            warnings.push(format!("rule-provider {name}: {bad} unsupported entries skipped"));
        }
        let needs_ip = match &matcher {
            SetMatcher::IpCidr(_) => true,
            SetMatcher::Classical(c) => c.iter().any(Cond::needs_ip),
            SetMatcher::Domain(_) => false,
        };
        RuleSet { matcher, needs_ip, len }
    }
}

pub struct Rule {
    pub kind: String,
    pub payload: String,
    pub target: String,
    no_resolve: bool,
    cond: Cond,
}

impl Rule {
    fn needs_ip(&self) -> bool {
        self.cond.needs_ip()
    }

    /// Display name in mihomo style, e.g. `DomainSuffix(google.com)`.
    pub fn display(&self) -> String {
        let kind = match self.kind.as_str() {
            "MATCH" => return "Match".into(),
            "GEOSITE" => "GeoSite".to_string(),
            "GEOIP" => "GeoIP".to_string(),
            "IP-CIDR" | "IP-CIDR6" => "IPCIDR".to_string(),
            "SRC-IP-CIDR" => "SrcIPCIDR".to_string(),
            "RULE-SET" => "RuleSet".to_string(),
            k => k
                .split('-')
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        Some(f) => f.to_ascii_uppercase().to_string() + &c.as_str().to_ascii_lowercase(),
                        None => String::new(),
                    }
                })
                .collect::<String>(),
        };
        format!("{kind}({})", self.payload)
    }
}

fn ports(s: &str) -> Option<Vec<(u16, u16)>> {
    s.split(['/', ','])
        .map(|p| match p.split_once('-') {
            Some((a, b)) => Some((a.trim().parse().ok()?, b.trim().parse().ok()?)),
            None => {
                let v = p.trim().parse().ok()?;
                Some((v, v))
            }
        })
        .collect()
}

fn parse_net(s: &str) -> Option<IpNet> {
    s.parse::<IpNet>().ok().or_else(|| s.parse::<IpAddr>().ok().map(IpNet::from))
}

pub struct Router {
    pub rules: Vec<Rule>,
}

impl Router {
    pub fn build(cfgs: &[RuleConfig], home: &Path, sets: &HashMap<String, Arc<RuleSet>>) -> (Router, Vec<String>) {
        let mut warnings = Vec::new();
        let site_codes: Vec<String> = cfgs.iter().filter(|r| r.kind == "GEOSITE").map(|r| r.payload.clone()).collect();
        let ip_codes: Vec<String> = cfgs
            .iter()
            .filter(|r| r.kind == "GEOIP" && !matches!(r.payload.to_ascii_lowercase().as_str(), "private" | "lan"))
            .map(|r| r.payload.clone())
            .collect();
        let mut sites: HashMap<String, Arc<DomainSet>> = HashMap::new();
        if !site_codes.is_empty() {
            match geo::load_geosite(&home.join("geosite.dat"), &site_codes) {
                Ok(m) => sites = m.into_iter().map(|(k, v)| (k, Arc::new(v))).collect(),
                Err(e) => warnings.push(format!("geosite.dat: {e}; GEOSITE rules are disabled")),
            }
        }
        let mut ips: HashMap<String, Arc<IpSet>> = HashMap::new();
        if !ip_codes.is_empty() {
            match geo::load_geoip(&home.join("geoip.dat"), &ip_codes) {
                Ok(m) => ips = m.into_iter().map(|(k, v)| (k, Arc::new(v))).collect(),
                Err(e) => warnings.push(format!("geoip.dat: {e}; GEOIP rules are disabled")),
            }
        }

        let mut rules = Vec::new();
        for r in cfgs {
            let p = r.payload.as_str();
            let cond = match r.kind.as_str() {
                "GEOSITE" => sites.get(p).cloned().map(Cond::GeoSite),
                "GEOIP" if !matches!(p.to_ascii_lowercase().as_str(), "private" | "lan") => {
                    ips.get(&p.to_ascii_lowercase()).cloned().map(Cond::GeoIp)
                }
                "RULE-SET" => match sets.get(p) {
                    Some(rs) => Some(Cond::RuleSet(rs.clone())),
                    None => {
                        warnings.push(format!("rule {}: rule-provider {p} not available, skipped", r.to_line()));
                        continue;
                    }
                },
                kind => match simple_cond(kind, p) {
                    Some(c) => c,
                    None => {
                        warnings.push(format!("rule type {} is not supported, skipped", r.kind));
                        continue;
                    }
                },
            };
            match cond {
                Some(cond) => rules.push(Rule {
                    kind: r.kind.clone(),
                    payload: r.payload.clone(),
                    target: r.target.clone(),
                    no_resolve: r.no_resolve,
                    cond,
                }),
                None if matches!(r.kind.as_str(), "GEOSITE" | "GEOIP") => {}
                None => warnings.push(format!("invalid rule {}", r.to_line())),
            }
        }
        (Router { rules }, warnings)
    }

    /// Returns the first matching rule (None = no rule matched; caller falls
    /// back to DIRECT like mihomo). May resolve the destination domain.
    pub async fn route(&self, meta: &mut Metadata) -> Option<&Rule> {
        for rule in &self.rules {
            if rule.needs_ip() && meta.dst_ip.is_none() && !rule.no_resolve {
                if let Address::Domain(d, p) = &meta.dst {
                    let lookup = crate::net::resolve(d, *p);
                    if let Ok(Ok(addrs)) = tokio::time::timeout(Duration::from_secs(3), lookup).await {
                        meta.dst_ip = addrs.iter().min_by_key(|a| a.is_ipv6()).map(|a| a.ip());
                    }
                }
            }
            let ip = meta.dst.ip().or(meta.dst_ip);
            if rule.cond.matches(meta, ip) {
                return Some(rule);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(dst: &str) -> Metadata {
        Metadata {
            network: crate::tunnel::Network::Tcp,
            inbound: "HTTP",
            src: "127.0.0.1:50000".parse().unwrap(),
            dst: Address::parse_authority(dst).unwrap(),
            dst_ip: None,
        }
    }

    #[tokio::test]
    async fn matching_order() {
        let cfgs: Vec<RuleConfig> = [
            "DOMAIN-SUFFIX,google.com,PROXY",
            "DOMAIN-KEYWORD,baidu,DIRECT",
            "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve",
            "GEOIP,private,DIRECT,no-resolve",
            "DST-PORT,22,REJECT",
            "MATCH,PROXY",
        ]
        .iter()
        .map(|l| RuleConfig::parse(l).unwrap())
        .collect();
        let (r, w) = Router::build(&cfgs, Path::new("."), &HashMap::new());
        assert!(w.is_empty(), "{w:?}");
        let t = |d: &str| {
            let r = &r;
            let d = d.to_string();
            async move { r.route(&mut meta(&d)).await.unwrap().target.clone() }
        };
        assert_eq!(t("www.google.com:443").await, "PROXY");
        assert_eq!(t("notgoogle.com:443").await, "PROXY"); // MATCH, not suffix
        assert_eq!(t("www.baidu.com:80").await, "DIRECT");
        assert_eq!(t("10.1.2.3:80").await, "DIRECT");
        assert_eq!(t("192.168.1.1:80").await, "DIRECT");
        assert_eq!(t("8.8.8.8:22").await, "REJECT");
        let m = r.route(&mut meta("a.google.com:1")).await.unwrap();
        assert_eq!(m.display(), "DomainSuffix(google.com)");
    }

    #[tokio::test]
    async fn rule_sets_and_network() {
        let mut w = Vec::new();
        let mut sets = HashMap::new();
        let dom = ["+.google.com", "*.svc.io", "bad*pattern"].map(String::from);
        sets.insert("dom".to_string(), Arc::new(RuleSet::new("dom", "domain", &dom, &mut w)));
        let ips = ["91.108.4.0/22", "2001:b28:f23d::/48"].map(String::from);
        sets.insert("tg".to_string(), Arc::new(RuleSet::new("tg", "ipcidr", &ips, &mut w)));
        let cls = ["DOMAIN-KEYWORD,ads", "DST-PORT,6881-6889", "PROCESS-NAME,x.exe"].map(String::from);
        sets.insert("cls".to_string(), Arc::new(RuleSet::new("cls", "classical", &cls, &mut w)));
        assert_eq!(w.len(), 2, "{w:?}"); // bad*pattern, PROCESS-NAME
        let cfgs: Vec<RuleConfig> = [
            "RULE-SET,dom,G",
            "RULE-SET,tg,T,no-resolve",
            "RULE-SET,cls,C",
            "NETWORK,udp,U",
            "RULE-SET,missing,X",
            "MATCH,M",
        ]
        .iter()
        .map(|l| RuleConfig::parse(l).unwrap())
        .collect();
        let (r, w) = Router::build(&cfgs, Path::new("."), &sets);
        assert_eq!(w.len(), 1, "{w:?}"); // missing provider
        let route = |dst: &str, net: crate::tunnel::Network| {
            let mut m = meta(dst);
            m.network = net;
            let r = &r;
            async move { r.route(&mut m).await.unwrap().target.clone() }
        };
        use crate::tunnel::Network::{Tcp, Udp};
        assert_eq!(route("mail.google.com:443", Tcp).await, "G");
        assert_eq!(route("a.svc.io:443", Tcp).await, "G");
        assert_eq!(route("91.108.5.1:443", Tcp).await, "T");
        assert_eq!(route("ads.example.com:80", Tcp).await, "C");
        assert_eq!(route("1.2.3.4:6885", Tcp).await, "C");
        assert_eq!(route("1.2.3.4:53", Udp).await, "U");
        assert_eq!(route("1.2.3.4:53", Tcp).await, "M");
        assert_eq!(r.route(&mut meta("x.google.com:1")).await.unwrap().display(), "RuleSet(dom)");
    }
}
