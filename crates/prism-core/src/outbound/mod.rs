//! Outbounds: concrete protocols plus proxy groups, all addressed by name
//! through the [`Registry`].

pub mod anytls;
pub mod basic;
pub mod group;
pub mod shadowsocks;
pub mod tls;
pub mod trojan;
pub mod vless;

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use crate::common::{Address, BoxedStream};
use crate::config::{ProxyConfig, ProxyKind};
use crate::delay::DelayStore;

pub use group::ProxyGroup;

#[async_trait]
pub trait Outbound: Send + Sync {
    fn name(&self) -> &str;
    /// Type name for the Clash REST API (Shadowsocks, Trojan, Selector, ...).
    fn api_type(&self) -> &'static str;
    fn udp(&self) -> bool {
        false
    }
    async fn connect(&self, target: &Address) -> io::Result<BoxedStream>;
    fn as_group(&self) -> Option<&ProxyGroup> {
        None
    }
}

/// All outbounds of a running config, in profile order.
pub struct Registry {
    pub map: HashMap<String, Arc<dyn Outbound>>,
    pub order: Vec<String>,
    pub delays: Arc<DelayStore>,
}

impl Registry {
    pub fn get(&self, name: &str) -> Option<Arc<dyn Outbound>> {
        self.map.get(name).cloned()
    }

    /// Follows group selections down to a concrete outbound. Returns the
    /// outbound plus the chain of names, rule target first.
    pub fn resolve(&self, name: &str, target: &Address) -> io::Result<(Arc<dyn Outbound>, Vec<String>)> {
        let mut out = self.get(name).ok_or_else(|| crate::common::other(format!("unknown proxy {name}")))?;
        let mut chain = vec![name.to_string()];
        for _ in 0..16 {
            let Some(next) = out.as_group().map(|g| g.pick(target)) else { break };
            let next = next.ok_or_else(|| crate::common::other(format!("group {} has no usable member", out.name())))?;
            chain.push(next.name().to_string());
            out = next;
        }
        Ok((out, chain))
    }
}

/// Lazily-filled handle the groups use to look up their members (groups are
/// constructed before the registry that contains them).
pub type RegistryCell = Arc<OnceLock<Arc<Registry>>>;

/// Builds a protocol outbound. Errors mean "unsupported by prism-core".
pub fn build_proxy(p: &ProxyConfig) -> anyhow::Result<Arc<dyn Outbound>> {
    if p.network.as_deref().is_some_and(|n| n != "tcp") {
        anyhow::bail!("{}: transport {} is not supported by prism-core", p.name, p.network.as_deref().unwrap());
    }
    Ok(match p.kind() {
        ProxyKind::AnyTls => Arc::new(anytls::AnyTls::new(p)?),
        ProxyKind::Shadowsocks => Arc::new(shadowsocks::Shadowsocks::new(p)?),
        ProxyKind::Trojan => Arc::new(trojan::Trojan::new(p)?),
        ProxyKind::Vless => Arc::new(vless::Vless::new(p)?),
        ProxyKind::Socks5 => Arc::new(basic::Socks5::new(p)?),
        ProxyKind::Http => Arc::new(basic::Http::new(p)?),
        ProxyKind::Vmess | ProxyKind::Hysteria2 => {
            anyhow::bail!("{}: {} is not supported by prism-core yet (use mihomo or sing-box core)", p.name, p.kind().clash_type())
        }
    })
}
