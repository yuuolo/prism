//! prism-core: a small rule-based proxy core.
//!
//! Mixed (SOCKS5 + HTTP) inbound -> rule router -> outbounds (AnyTLS,
//! Shadowsocks AEAD, Trojan, VLESS, SOCKS5, HTTP, groups), with a
//! Clash-compatible REST API. Config is Clash/mihomo YAML (or any format
//! understood by [`config::parse_any`]).

pub mod api;
pub mod common;
pub mod config;
pub mod delay;
pub mod geo;
pub mod inbound;
pub mod outbound;
pub mod rules;
pub mod tunnel;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

use anyhow::{Context, Result};

use config::{General, GroupConfig, GroupKind, Profile};
use outbound::{basic, Outbound, ProxyGroup, Registry};
use tunnel::{Stats, Tunnel};

pub struct Core {
    pub general: General,
    pub tunnel: Arc<Tunnel>,
    pub warnings: Vec<String>,
}

/// Builds outbounds, groups and rules from a profile. `home` holds
/// geosite.dat / geoip.dat.
pub fn build(mut profile: Profile, home: &Path) -> Result<Core> {
    let mut warnings = std::mem::take(&mut profile.warnings);

    // Protocol outbounds; drop the ones prism-core can't speak.
    let mut map: HashMap<String, Arc<dyn Outbound>> = HashMap::new();
    let mut order = Vec::new();
    let mut kept = Vec::new();
    for p in &profile.proxies {
        match outbound::build_proxy(p) {
            Ok(o) => {
                map.insert(p.name.clone(), o);
                kept.push(p.clone());
            }
            Err(e) => warnings.push(e.to_string()),
        }
    }
    profile.proxies = kept;
    profile.sanitize();
    warnings.append(&mut profile.warnings);

    map.insert("DIRECT".into(), Arc::new(basic::Direct));
    map.insert("REJECT".into(), Arc::new(basic::Reject));

    let cell: outbound::RegistryCell = Arc::new(OnceLock::new());
    let mut groups = Vec::new();
    let mut global_members = Vec::new();
    for g in &profile.groups {
        let grp = Arc::new(ProxyGroup::new(g.clone(), cell.clone()));
        map.insert(g.name.clone(), grp.clone());
        groups.push(grp);
        global_members.push(g.name.clone());
        order.push(g.name.clone());
    }
    for p in &profile.proxies {
        global_members.push(p.name.clone());
        order.push(p.name.clone());
    }
    global_members.extend(["DIRECT".to_string(), "REJECT".to_string()]);
    order.extend(["DIRECT".to_string(), "REJECT".to_string()]);
    let global = Arc::new(ProxyGroup::new(GroupConfig::new("GLOBAL", GroupKind::Select, global_members), cell.clone()));
    map.insert("GLOBAL".into(), global);
    order.insert(0, "GLOBAL".into());

    let reg = Arc::new(Registry { map, order, delays: Default::default() });
    let _ = cell.set(reg.clone());
    for g in groups {
        g.spawn_health_loop();
    }

    let (router, mut rule_warnings) = rules::Router::build(&profile.rules, home);
    warnings.append(&mut rule_warnings);

    let tunnel = Arc::new(Tunnel {
        reg,
        router,
        mode: RwLock::new(profile.general.mode),
        stats: Arc::new(Stats::default()),
    });
    Ok(Core { general: profile.general, tunnel, warnings })
}

fn listen_addr(g: &General) -> SocketAddr {
    let ip = if g.allow_lan {
        match g.bind_address.as_str() {
            "*" | "" => "0.0.0.0".parse().unwrap(),
            a => a.parse().unwrap_or_else(|_| "0.0.0.0".parse().unwrap()),
        }
    } else {
        "127.0.0.1".parse().unwrap()
    };
    SocketAddr::new(ip, g.mixed_port)
}

/// Starts the inbound and the API and runs until the process exits.
pub async fn run(core: Core) -> Result<()> {
    let core = Arc::new(core);
    for w in &core.warnings {
        tracing::warn!("{w}");
    }
    let addr = listen_addr(&core.general);
    let listener = tokio::net::TcpListener::bind(addr).await.with_context(|| format!("listen on {addr}"))?;
    tracing::info!("Mixed(http+socks) proxy listening at: {addr}");
    tokio::spawn(inbound::serve(listener, core.tunnel.clone()));

    if let Some(ctl) = core.general.external_controller.clone() {
        let ctl = if ctl.starts_with(':') { format!("127.0.0.1{ctl}") } else { ctl };
        let l = tokio::net::TcpListener::bind(&ctl).await.with_context(|| format!("listen on {ctl}"))?;
        tracing::info!("RESTful API listening at: {ctl}");
        let app = api::router(core.clone());
        tokio::spawn(async move {
            if let Err(e) = axum::serve(l, app).await {
                tracing::error!("api: {e}");
            }
        });
    }
    std::future::pending::<()>().await;
    Ok(())
}
