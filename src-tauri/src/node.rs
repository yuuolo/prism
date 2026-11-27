//! Single-node import always produces an independent local profile.
use anyhow::{bail, Result};
use prism_core::config::{self, clash, Profile};

pub fn single_node(content: &str, name: &str) -> Result<(String, String)> {
    let mut profile = config::parse_any(content.trim())?;
    if profile.proxies.len() != 1 { bail!("请输入且仅输入一个有效节点"); }
    if !profile.warnings.is_empty() { bail!("节点内容含有无法识别的条目，请只输入一个完整节点"); }
    if profile.proxies[0].dialer_proxy.as_ref().is_some_and(|d| d != "DIRECT") {
        bail!("单个节点不能依赖其他节点或代理组");
    }
    if !name.trim().is_empty() { profile.proxies[0].name = name.trim().into(); }
    let name = profile.proxies[0].name.clone();
    let profile = Profile { proxies: profile.proxies, ..Default::default() }.with_default_groups();
    Ok((name, serde_yaml::to_string(&clash::emit(&profile))?))
}

#[cfg(test)]
mod tests {
    use super::*;
    const LINK: &str = "ss://YWVzLTI1Ni1nY206cHc@3.3.3.3:8388#old";
    #[test]
    fn renamed_node_has_valid_group_members() {
        let (name, raw) = single_node(LINK, "自建节点").unwrap();
        let p = config::parse_any(&raw).unwrap();
        assert_eq!(name, "自建节点");
        assert!(p.groups.iter().all(|g| !g.proxies.contains(&"old".into())));
        assert!(p.groups.iter().all(|g| g.proxies.contains(&name)));
    }
    #[test]
    fn refuses_multiple_or_partial_nodes() {
        assert!(single_node(&format!("{LINK}\n{LINK}"), "").is_err());
        assert!(single_node(&format!("{LINK}\ninvalid://bad"), "").is_err());
    }
    #[test]
    fn refuses_external_dependencies() {
        assert!(single_node("proxies: [{name: x, type: trojan, server: a.test, port: 443, password: x, dialer-proxy: other}]", "").is_err());
    }
}
