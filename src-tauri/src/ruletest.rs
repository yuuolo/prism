//! 规则测试: which rule a destination hits, evaluated with prism-core's rule
//! engine on the rules in effect (the same for all three cores).

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use prism_core::common::Address;
use prism_core::process::ProcessInfo;
use prism_core::rules::{self, Hit};
use prism_core::tunnel::{Metadata, Network};
use serde::Serialize;

use crate::store::{CoreKind, Settings, Store};

#[derive(Serialize)]
pub struct Outcome {
    pub host: String,
    pub port: u16,
    /// The destination's address as the rules saw it (looked up for domains).
    pub ip: Option<String>,
    pub hit: Hit,
    /// Rules in the list (to show "rule 12 of 340").
    pub total: usize,
    pub warnings: Vec<String>,
}

/// `https://a.com/x`, `a.com:8443`, `a.com`, `1.2.3.4`, `[::1]:53` → address.
pub fn parse_target(input: &str) -> Result<Address> {
    let s = input.trim();
    let (scheme, rest) = match s.split_once("://") {
        Some((sc, r)) => (sc.to_ascii_lowercase(), r),
        None => (String::new(), s),
    };
    let auth = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let auth = auth.rsplit_once('@').map(|(_, h)| h).unwrap_or(auth);
    if auth.is_empty() || auth.contains(char::is_whitespace) {
        return Err(anyhow!("请输入域名、IP 或网址"));
    }
    let default = if matches!(scheme.as_str(), "http" | "ws") { 80 } else { 443 };
    // A bare IPv6 address has colons but no port.
    if auth.parse::<IpAddr>().is_ok() {
        return Ok(Address::new(auth, default));
    }
    Ok(Address::parse_authority(auth).unwrap_or_else(|| Address::new(auth, default)))
}

/// Not a server's address: the fake-ip range a core's DNS hands out.
pub fn is_fake(ip: IpAddr) -> bool {
    matches!(ip, IpAddr::V4(v4) if v4.octets()[0] == 198 && (v4.octets()[1] & 0xfe) == 18)
}

/// Looks `host` up the way the core would: through its own DNS (`/dns/query`)
/// when it has one, else the system resolver, skipping fake-ips.
pub async fn lookup(s: &Settings, running: Option<CoreKind>, host: &str) -> Option<IpAddr> {
    if matches!(running, Some(CoreKind::Prism | CoreKind::Mihomo)) {
        let path = format!("/dns/query?name={}&type=A", urlencoding(host));
        if let Ok(Ok(v)) = tokio::time::timeout(Duration::from_secs(6), crate::core::api(s, "GET", &path, None)).await {
            let ip = v["Answer"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| a["data"].as_str()?.parse::<IpAddr>().ok())
                .find(|ip| !is_fake(*ip));
            if ip.is_some() {
                return ip;
            }
        }
    }
    let addrs = tokio::time::timeout(Duration::from_secs(6), tokio::net::lookup_host((host, 0))).await.ok()?.ok()?;
    let ips: Vec<IpAddr> = addrs.map(|a| a.ip()).filter(|ip| !is_fake(*ip)).collect();
    ips.iter().find(|ip| ip.is_ipv4()).or(ips.first()).copied()
}

fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

pub async fn run(store: &Store, s: &Settings, running: Option<CoreKind>, target: &str, network: &str, process: &str) -> Result<Outcome> {
    let network = if network.eq_ignore_ascii_case("udp") { Network::Udp } else { Network::Tcp };
    let dst = parse_target(target)?;
    let profile = crate::core::rule_test_profile(store, s)?;
    let (router, warnings) = rules::tester(&profile, &store.runtime_dir()).await;

    let mut meta = Metadata::new(network, "HTTP", "127.0.0.1:50000".parse().unwrap(), dst.clone(), None);
    meta.process_checked = true;
    let process = process.trim();
    if !process.is_empty() {
        let name = process.rsplit(['\\', '/']).next().unwrap_or(process).to_string();
        let path = if process.contains(['\\', '/']) { process.to_string() } else { String::new() };
        meta.process = Some(Arc::new(ProcessInfo { pid: 0, name, path }));
    }
    let ip = match &dst {
        Address::Ip(sa) => Some(sa.ip()),
        Address::Domain(d, _) => lookup(s, running, d).await,
    };
    let hit = router.test(&meta, ip);
    let shown_ip = if hit.resolved || dst.ip().is_some() { ip.map(|i| i.to_string()) } else { None };
    Ok(Outcome { host: dst.host(), port: dst.port(), ip: shown_ip, hit, total: router.rules.len(), warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets() {
        let t = |s: &str| parse_target(s).unwrap().to_string();
        assert_eq!(t("https://www.Google.com/search?q=1"), "www.google.com:443");
        assert_eq!(t("http://example.com"), "example.com:80");
        assert_eq!(t("example.com:8443"), "example.com:8443");
        assert_eq!(t("1.2.3.4"), "1.2.3.4:443");
        assert_eq!(t("[2001:db8::1]:53"), "[2001:db8::1]:53");
        assert_eq!(t("2001:db8::1"), "[2001:db8::1]:443");
        assert!(parse_target("  ").is_err());
        assert_eq!(parse_target("https://wiki.corp.test:8443/x").unwrap().host(), "wiki.corp.test", "the real IP test looks up the host only");
        assert!(is_fake("198.19.0.5".parse().unwrap()) && !is_fake("198.20.0.1".parse().unwrap()));
    }
}
