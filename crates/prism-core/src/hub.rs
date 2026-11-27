//! Holds the running [`Core`] and swaps it on reload. Listeners (mixed
//! port, TUN, DNS, API) stay up and look up the current core per connection,
//! so a reload (config change, provider update) doesn't drop them; existing
//! connections finish on the core they started with.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use tokio::sync::mpsc;

use crate::config::{self, Profile};
use crate::tunnel::Stats;
use crate::{build, BuildEnv, Core};

/// Where the config comes from, plus command-line overrides that must
/// survive reloads.
#[derive(Clone, Debug, Default)]
pub struct Source {
    pub path: Option<PathBuf>,
    pub ext_ctl: Option<String>,
    pub secret: Option<String>,
    pub tun: bool,
}

impl Source {
    fn apply(&self, mut p: Profile) -> Profile {
        if let Some(c) = &self.ext_ctl {
            p.general.external_controller = Some(c.clone());
        }
        if let Some(s) = &self.secret {
            p.general.secret = s.clone();
        }
        if self.tun {
            p.general.tun.enable = true;
            p.general.tun.auto_route = true;
        }
        p
    }
}

pub struct Hub {
    pub tun_health: Arc<crate::tun_health::Health>,
    core: RwLock<Arc<Core>>,
    source: Source,
    // Last successfully applied profile, never exposed through diagnostics.
    // Automatic repairs must not accidentally reload a stale/on-disk edit.
    profile: RwLock<Profile>,
    home: PathBuf,
    env: BuildEnv,
    reload_lock: tokio::sync::Mutex<()>,
}

impl Hub {
    pub async fn new(source: Source, home: PathBuf, profile: Profile) -> Result<Arc<Hub>> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let env = BuildEnv { stats: Arc::new(Stats::default()), reload: tx, fake_pool: Default::default() };
        let profile = source.apply(profile);
        let core = build(profile.clone(), &home, &env).await?;
        let hub = Arc::new(Hub { tun_health: Default::default(), core: RwLock::new(Arc::new(core)), source, profile: RwLock::new(profile), home, env, reload_lock: Default::default() });
        hub.install();
        // Providers ask for a reload when their content changes.
        let weak = Arc::downgrade(&hub);
        tokio::spawn(async move {
            while rx.recv().await.is_some() {
                while rx.try_recv().is_ok() {}
                let Some(hub) = weak.upgrade() else { break };
                if let Err(e) = hub.repair().await {
                    tracing::warn!("reload after provider update failed: {e:#}");
                }
            }
        });
        Ok(hub)
    }

    pub fn core(&self) -> Arc<Core> {
        self.core.read().unwrap().clone()
    }

    pub fn home(&self) -> &std::path::Path {
        &self.home
    }

    /// Restore before listeners/TUN start, so no packets use a default exit.
    pub fn restore_state(&self, value: serde_json::Value) -> Result<()> {
        #[derive(serde::Deserialize)]
        struct Saved { mode: Option<String>, selections: std::collections::BTreeMap<String, String> }
        let saved: Saved = serde_json::from_value(value)?;
        let core = self.core();
        if let Some(mode) = saved.mode { *core.tunnel.mode.write().unwrap() = config::Mode::parse(&mode).context("invalid saved mode")?; }
        for (name, selected) in saved.selections {
            let outbound = core.tunnel.reg.get(&name).context("saved group no longer exists")?;
            let group = outbound.as_group().context("saved target is not a group")?;
            group.select(&selected).map_err(|_| anyhow::anyhow!("saved selection no longer exists"))?;
        }
        Ok(())
    }

    /// Process-wide state that follows the current core.
    fn install(&self) {
        crate::net::set_resolver(self.core().dns.clone());
        crate::net::set_tailscale_bypass(self.core().general.tailscale_bypass);
    }

    /// Rebuilds the core from the config file (or `payload`) and swaps it in.
    /// Node selections and the current mode carry over.
    pub async fn reload(&self, payload: Option<String>) -> Result<()> {
        let _guard = self.reload_lock.lock().await;
        let raw = match payload {
            Some(p) => p,
            None => {
                let path = self.source.path.as_ref().context("no config file to reload")?;
                std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?
            }
        };
        let profile = self.source.apply(config::parse_any(&raw)?);
        self.replace(profile).await
    }

    /// Rebuild pools, resolver and provider contents from the last applied
    /// profile. File edits are only applied by an explicit configuration reload.
    pub async fn repair(&self) -> Result<()> {
        let _guard = self.reload_lock.lock().await;
        let profile = self.profile.read().unwrap().clone();
        self.replace(profile).await
    }

    /// Caller must hold reload_lock across build and commit.
    async fn replace(&self, profile: Profile) -> Result<()> {
        let new = build(profile.clone(), &self.home, &self.env).await?;
        let old = self.core();
        *new.tunnel.mode.write().unwrap() = old.tunnel.mode();
        for name in &old.tunnel.reg.order {
            let (Some(o), Some(n)) = (old.tunnel.reg.get(name), new.tunnel.reg.get(name)) else { continue };
            if let (Some(og), Some(ng)) = (o.as_group(), n.as_group()) {
                if og.kind() == config::GroupKind::Select {
                    let _ = ng.select(&og.now());
                }
            }
        }
        for w in &new.warnings {
            tracing::warn!("{w}");
        }
        *self.core.write().unwrap() = Arc::new(new);
        *self.profile.write().unwrap() = profile;
        self.install();
        old.shutdown.cancel();
        tracing::info!("configuration reloaded");
        Ok(())
    }
}
