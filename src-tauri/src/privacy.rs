//! Explicit Prism DNS privacy overrides, applied after subscription overrides.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DnsPrivacy {
    pub enabled: bool,
    pub via_proxy: bool,
    pub outbound: String,
    pub ipv6: bool,
    pub block_ipv6: bool,
    pub fake_ip: bool,
    pub nameservers: Vec<String>,
    pub bootstrap: Vec<String>,
}
impl Default for DnsPrivacy {
    fn default() -> Self {
        Self {
            enabled: false,
            via_proxy: true,
            outbound: String::new(),
            ipv6: false,
            block_ipv6: false,
            fake_ip: true,
            nameservers: vec!["https://1.1.1.1/dns-query".into()],
            bootstrap: vec!["https://223.5.5.5/dns-query".into()],
        }
    }
}
impl DnsPrivacy {
    pub fn validate(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if self.outbound.contains(['#', '&', '\n', '\r']) {
            bail!("DNS 出口名称不能包含 #、& 或换行");
        }
        for (label, list) in [
            ("DNS 上游", &self.nameservers),
            ("节点引导 DNS", &self.bootstrap),
        ] {
            if !list.iter().any(|s| !s.trim().is_empty()) {
                bail!("{label}不能为空");
            }
            for spec in list.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                let url = reqwest::Url::parse(spec)?;
                if url.scheme() != "https"
                    || url.host_str().is_none()
                    || url.fragment().is_some()
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    bail!("{label}必须是无出口后缀的 HTTPS DoH 地址");
                }
                if (label == "节点引导 DNS" || !self.via_proxy)
                    && url
                        .host_str()
                        .unwrap()
                        .trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .is_err()
                {
                    bail!("节点引导或直连 DNS 必须使用 IP 地址的 DoH，避免解析循环");
                }
            }
        }
        Ok(())
    }
}

pub fn apply(doc: &mut Value, p: &DnsPrivacy) -> Result<()> {
    if !p.enabled {
        return Ok(());
    }
    p.validate()?;
    let profile = prism_core::config::clash::parse(doc)?;
    let outbound = if !p.outbound.trim().is_empty() {
        p.outbound.trim().to_string()
    } else {
        profile
            .groups
            .iter()
            .find(|g| g.name.contains("节点选择"))
            .or_else(|| profile.groups.first())
            .map(|g| g.name.clone())
            .or_else(|| profile.proxies.first().map(|n| n.name.clone()))
            .unwrap_or_default()
    };
    if outbound.contains(['#', '&', '\n', '\r']) {
        bail!("DNS 出口名称不能包含 #、& 或换行");
    }
    if p.via_proxy
        && (outbound.is_empty()
            || !profile.groups.iter().any(|g| g.name == outbound)
                && !profile.proxies.iter().any(|n| n.name == outbound))
    {
        bail!("DNS 保护需要一个有效的代理节点或代理组，请指定 DNS 出口");
    }
    let mut dns = serde_yaml::Mapping::new();
    dns.insert("enable".into(), true.into());
    dns.insert("ipv6".into(), (p.ipv6 && !p.block_ipv6).into());
    dns.insert(
        "enhanced-mode".into(),
        if p.fake_ip { "fake-ip" } else { "redir-host" }.into(),
    );
    dns.insert("fake-ip-range".into(), "198.18.0.1/16".into());
    dns.insert(
        "fake-ip-filter".into(),
        serde_yaml::to_value(["*.lan", "+.local"])?,
    );
    let servers: Vec<_> = p
        .nameservers
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            if p.via_proxy {
                format!("{s}#{outbound}&proxy-only=true")
            } else {
                s.to_string()
            }
        })
        .collect();
    dns.insert("nameserver".into(), serde_yaml::to_value(servers)?);
    dns.insert(
        "proxy-server-nameserver".into(),
        serde_yaml::to_value(
            p.bootstrap
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>(),
        )?,
    );
    // Replace subscription policy and fallback to prevent hidden direct races.
    doc["dns"] = Value::Mapping(dns);
    doc["prism-block-ipv6"] = p.block_ipv6.into();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn privacy_removes_subscription_direct_fallback_and_keeps_tun_ipv6() {
        let mut doc: Value = serde_yaml::from_str("proxies: [{name: test, type: socks5, server: 127.0.0.1, port: 9}]\nproxy-groups: [{name: 节点选择, type: select, proxies: [test]}]\ntun: {enable: true, inet6-address: ['fdfe:dcba:9876::1/126']}\ndns: {enable: true, fallback: [223.5.5.5], nameserver-policy: {'+.test': 119.29.29.29}}\nrules: ['MATCH,节点选择']").unwrap();
        let p = DnsPrivacy {
            enabled: true,
            block_ipv6: true,
            ipv6: true,
            ..Default::default()
        };
        apply(&mut doc, &p).unwrap();
        assert!(doc["dns"].get("fallback").is_none());
        assert!(doc["dns"].get("nameserver-policy").is_none());
        assert_eq!(doc["dns"]["ipv6"].as_bool(), Some(false));
        assert!(doc["dns"]["nameserver"][0]
            .as_str()
            .unwrap()
            .ends_with("#节点选择&proxy-only=true"));
        assert!(doc["tun"]["inet6-address"].is_sequence());
        let parsed = prism_core::config::clash::parse(&doc).unwrap();
        assert!(parsed.general.block_ipv6);
        assert!(
            prism_core::config::clash::parse(&prism_core::config::clash::emit(&parsed))
                .unwrap()
                .general
                .block_ipv6
        );
    }
    #[test]
    fn disabled_preserves_config_and_invalid_bootstrap_is_rejected() {
        let mut doc: Value = serde_yaml::from_str("proxies: []").unwrap();
        let before = doc.clone();
        apply(&mut doc, &DnsPrivacy::default()).unwrap();
        assert_eq!(doc, before);
        for url in [
            "http://1.1.1.1/dns-query",
            "https://dns.example/dns-query",
            "https://1.1.1.1/dns-query#DIRECT",
        ] {
            let p = DnsPrivacy {
                enabled: true,
                bootstrap: vec![url.into()],
                ..Default::default()
            };
            assert!(p.validate().is_err(), "{url}");
        }
        assert!(apply(
            &mut doc,
            &DnsPrivacy {
                enabled: true,
                ..Default::default()
            }
        )
        .is_err());
    }
}
