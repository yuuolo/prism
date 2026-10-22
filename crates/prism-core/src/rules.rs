//! Clash-style rule matching.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ipnet::IpNet;
use regex::Regex;

use crate::common::{is_private_ip, Address};
use crate::config::RuleConfig;
use crate::geo::{self, DomainSet, IpSet};
use crate::tunnel::Metadata;

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
    Match,
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
        matches!(self.cond, Cond::GeoIp(_) | Cond::Private | Cond::Cidr(_))
    }

    /// Display name in mihomo style, e.g. `DomainSuffix(google.com)`.
    pub fn display(&self) -> String {
        let kind = match self.kind.as_str() {
            "MATCH" => return "Match".into(),
            "GEOSITE" => "GeoSite".to_string(),
            "GEOIP" => "GeoIP".to_string(),
            "IP-CIDR" | "IP-CIDR6" => "IPCIDR".to_string(),
            "SRC-IP-CIDR" => "SrcIPCIDR".to_string(),
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
    pub fn build(cfgs: &[RuleConfig], home: &Path) -> (Router, Vec<String>) {
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
                "DOMAIN" => Some(Cond::Domain(p.to_ascii_lowercase())),
                "DOMAIN-SUFFIX" => Some(Cond::Suffix(p.trim_start_matches('.').to_ascii_lowercase())),
                "DOMAIN-KEYWORD" => Some(Cond::Keyword(p.to_ascii_lowercase())),
                "DOMAIN-REGEX" => Regex::new(p).ok().map(Cond::Regex),
                "GEOSITE" => sites.get(p).cloned().map(Cond::GeoSite),
                "GEOIP" if matches!(p.to_ascii_lowercase().as_str(), "private" | "lan") => Some(Cond::Private),
                "GEOIP" => ips.get(&p.to_ascii_lowercase()).cloned().map(Cond::GeoIp),
                "IP-CIDR" | "IP-CIDR6" => parse_net(p).map(Cond::Cidr),
                "SRC-IP-CIDR" => parse_net(p).map(Cond::SrcCidr),
                "DST-PORT" => ports(p).map(Cond::DstPort),
                "SRC-PORT" => ports(p).map(Cond::SrcPort),
                "IN-TYPE" => Some(Cond::InType(p.to_ascii_uppercase())),
                "MATCH" => Some(Cond::Match),
                _ => {
                    warnings.push(format!("rule type {} is not supported, skipped", r.kind));
                    continue;
                }
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
                    let lookup = tokio::net::lookup_host((d.as_str(), *p));
                    if let Ok(Ok(mut addrs)) = tokio::time::timeout(Duration::from_secs(3), lookup).await {
                        meta.dst_ip = addrs.next().map(|a| a.ip());
                    }
                }
            }
            if self.matches(rule, meta) {
                return Some(rule);
            }
        }
        None
    }

    fn matches(&self, rule: &Rule, meta: &Metadata) -> bool {
        let host = meta.dst.domain();
        let ip = meta.dst.ip().or(meta.dst_ip);
        let in_ports = |v: &[(u16, u16)], p: u16| v.iter().any(|(a, b)| p >= *a && p <= *b);
        match &rule.cond {
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
            Cond::Match => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(dst: &str) -> Metadata {
        Metadata {
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
        let (r, w) = Router::build(&cfgs, Path::new("."));
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
}
