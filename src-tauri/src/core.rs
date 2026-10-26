//! Runs the selected core (prism-core, mihomo or sing-box) as a child
//! process with a generated config and talks to it over the Clash API.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use prism_core::config::{self, clash, singbox, Format};
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::store::{CoreKind, Settings, Store};

const MAX_LOGS: usize = 2000;

#[derive(Default)]
pub struct CoreManager {
    child: tokio::sync::Mutex<Option<(CoreKind, Child)>>,
    /// Set while the core runs inside the TUN service instead.
    service: tokio::sync::Mutex<Option<CoreKind>>,
    /// Copies the service's core output into `logs`.
    service_logs: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    pub logs: Arc<Mutex<VecDeque<String>>>,
}

/// TUN without administrator rights goes through the TUN service.
pub fn use_service(s: &Settings) -> bool {
    s.tun && !crate::elevate::is_elevated() && crate::tunsvc::state() == crate::tunsvc::State::Running
}

pub fn core_binary(store: &Store, s: &Settings, kind: CoreKind) -> PathBuf {
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    let ext = if cfg!(windows) { ".exe" } else { "" };
    match kind {
        CoreKind::Prism => exe.join(format!("prism-core{ext}")),
        CoreKind::Mihomo => s
            .mihomo_path
            .clone()
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| store.cores_dir().join(format!("mihomo{ext}"))),
        CoreKind::Singbox => s
            .singbox_path
            .clone()
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| store.cores_dir().join(format!("sing-box{ext}"))),
    }
}

fn overrides(s: &Settings) -> clash::Overrides {
    clash::Overrides {
        mixed_port: s.mixed_port,
        controller: format!("127.0.0.1:{}", s.controller_port),
        secret: s.secret.clone(),
        mode: s.mode,
        allow_lan: s.allow_lan,
        log_level: s.log_level.clone(),
        tun: s.tun,
        unified_delay: s.unified_delay,
    }
}

/// The active profile's content with the merged profiles' nodes added and
/// the user's 覆写 applied, plus warnings from merging.
fn active_profile_full(store: &Store, s: &Settings) -> Result<(String, Vec<String>)> {
    let id = s.active_profile.clone().ok_or_else(|| anyhow!("请先在「订阅」页导入并启用一个订阅"))?;
    let raw = store.read_content(&id)?.trim_start_matches('\u{feff}').to_string();
    let profiles = store.profiles();
    let name = profiles.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default();
    let parts: Vec<crate::merge::Part> = s
        .merged()
        .iter()
        .filter_map(|m| {
            let meta = profiles.iter().find(|p| &p.id == m)?;
            Some(crate::merge::Part { name: meta.name.clone(), raw: store.read_content(m).ok()? })
        })
        .collect();
    let (raw, warnings) = crate::merge::merge(&raw, &parts)?;
    let out = crate::patches::apply_both(&crate::patches::load(&store.dir, None), &crate::patches::load(&store.dir, Some(&id)), &raw, &name)?;
    Ok((out, warnings))
}

fn active_profile(store: &Store, s: &Settings) -> Result<String> {
    active_profile_full(store, s).map(|(raw, _)| raw)
}

/// sing-box config for the active profile, before rule-sets are localized.
fn singbox_config(store: &Store, s: &Settings) -> Result<(Value, Vec<String>)> {
    let (raw, warnings) = active_profile_full(store, s)?;
    let o = overrides(s);
    let mut doc: Value = match config::detect(&raw) {
        Format::SingBox => serde_json::from_str(&raw)?,
        _ => singbox::emit(&config::parse_any(&raw)?, &o),
    };
    singbox::apply_overrides(&mut doc, &o);
    // Here rather than with the app rules: the rule-sets these need must
    // be among the ones the app downloads.
    let mut warnings = warnings;
    warnings.extend(route_warnings(singbox::add_user_rules(&mut doc, &user_rules(s))));
    Ok((doc, warnings))
}

/// Remote rule-sets the sing-box config uses (the app downloads them).
pub fn singbox_rule_sets(store: &Store, s: &Settings) -> Result<Vec<singbox::RemoteRuleSet>> {
    Ok(singbox::remote_rule_sets(&singbox_config(store, s)?.0))
}

pub struct RuntimeConfig {
    pub path: PathBuf,
    /// Things the user should know about (e.g. skipped rule-sets).
    pub warnings: Vec<String>,
}

fn app_routes(s: &Settings) -> Vec<config::AppRoute> {
    s.app_rules
        .iter()
        .filter(|r| r.enabled)
        .map(|r| config::AppRoute { by_path: r.by_path, value: r.value.clone(), target: r.target.clone() })
        .collect()
}

fn user_rules(s: &Settings) -> Vec<config::UserRule> {
    s.route_rules
        .iter()
        .filter(|r| r.enabled)
        .map(|r| config::UserRule { kind: r.kind.clone(), value: r.value.clone(), target: r.target.clone(), resolve: r.resolve })
        .collect()
}

fn route_warnings(w: Vec<String>) -> Vec<String> {
    w.into_iter().map(|w| format!("域名 / IP 分流: {w}")).collect()
}

fn app_warnings(w: Vec<String>) -> Vec<String> {
    w.into_iter().map(|w| format!("应用分流: {w}")).collect()
}

/// Groups and nodes of the active profile, as exits for app rules.
pub fn profile_outbounds(store: &Store, s: &Settings) -> Result<(Vec<String>, Vec<String>)> {
    let p = config::parse_any(&active_profile(store, s)?)?;
    Ok((p.groups.into_iter().map(|g| g.name).collect(), p.proxies.into_iter().map(|x| x.name).collect()))
}

/// Converts the active profile into the selected core's config format and
/// writes it to the runtime dir.
pub fn write_runtime_config(store: &Store, s: &Settings) -> Result<RuntimeConfig> {
    let dir = store.runtime_dir();
    match s.core {
        CoreKind::Prism | CoreKind::Mihomo => {
            let (raw, mut warnings) = active_profile_full(store, s)?;
            let mut doc: serde_yaml::Value = match config::detect(&raw) {
                Format::Clash => serde_yaml::from_str(&raw)?,
                _ => clash::emit(&config::parse_any(&raw)?),
            };
            clash::apply_overrides(&mut doc, &overrides(s));
            if s.core == CoreKind::Mihomo {
                clash::relays_to_dialer_proxy(&mut doc);
            }
            warnings.extend(route_warnings(clash::add_user_rules(&mut doc, &user_rules(s))));
            warnings.extend(app_warnings(clash::add_app_rules(&mut doc, &app_routes(s))));
            let path = dir.join("config.yaml");
            std::fs::write(&path, serde_yaml::to_string(&doc)?)?;
            Ok(RuntimeConfig { path, warnings })
        }
        CoreKind::Singbox => {
            let (mut doc, mut warnings) = singbox_config(store, s)?;
            warnings.extend(app_warnings(singbox::add_app_rules(&mut doc, &app_routes(s))));
            // sing-box exits if a remote rule-set can't be fetched at startup,
            // so it only ever gets the files the app downloaded.
            warnings.extend(
                singbox::localize_rule_sets(&mut doc, &store.rule_set_dir())
                    .into_iter()
                    .map(|t| format!("规则集 {t} 未能下载，使用它的规则已跳过（可在设置页「更新 GeoSite / GeoIP」重试）")),
            );
            let path = dir.join("config.json");
            std::fs::write(&path, serde_json::to_string_pretty(&doc)?)?;
            Ok(RuntimeConfig { path, warnings })
        }
    }
}

impl CoreManager {
    pub fn push_log(&self, app: &AppHandle, line: String) {
        push_log(&self.logs, app, line);
    }

    pub async fn running(&self) -> Option<CoreKind> {
        if let Some(kind) = *self.service.lock().await {
            return match crate::tunsvc::status().await {
                Ok(st) if st.running && st.core.as_deref() == Some(crate::tunsvc::kind_name(kind)) => Some(kind),
                _ => None,
            };
        }
        let mut g = self.child.lock().await;
        if let Some((kind, child)) = g.as_mut() {
            match child.try_wait() {
                Ok(None) => return Some(*kind),
                _ => *g = None,
            }
        }
        None
    }

    pub async fn start(&self, app: &AppHandle, store: &Store, s: &Settings) -> Result<()> {
        self.stop().await;
        let bin = core_binary(store, s, s.core);
        if !bin.exists() {
            bail!(match s.core {
                CoreKind::Prism => format!("找不到 prism-core: {}（先运行 cargo build -p prism-core）", bin.display()),
                _ => format!("找不到内核 {}，请在「设置」页下载或指定路径", bin.display()),
            });
        }
        let RuntimeConfig { path: cfg, warnings } = write_runtime_config(store, s)?;
        for w in warnings {
            self.push_log(app, format!("[prism] {w}"));
        }
        if use_service(s) {
            let since = crate::tunsvc::logs(0).await.ok().and_then(|l| l.last().map(|x| x.0)).unwrap_or(0);
            crate::tunsvc::start(store, s, std::fs::read_to_string(&cfg)?).await?;
            *self.service.lock().await = Some(s.core);
            self.push_log(app, format!("[prism] 通过 TUN 服务启动 {:?} 内核（无需以管理员身份运行 Prism）", s.core));
            self.forward_service_logs(app, since);
        } else {
            self.spawn_local(app, store, s, &bin, &cfg).await?;
        }

        // Wait for the API to come up (or the process to die).
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if self.running().await.is_none() {
                bail!("内核启动后立即退出，请查看「日志」页");
            }
            if api(s, "GET", "/version", None).await.is_ok() {
                #[cfg(target_os = "macos")]
                if s.tun {
                    if let Err(e) = crate::macdns::override_dns(&store.dir) {
                        self.push_log(app, format!("[prism] 设置 TUN 的系统 DNS 失败，部分域名可能不经过 TUN: {e:#}"));
                    }
                }
                return Ok(());
            }
        }
        bail!("内核 API 无响应（端口 {} 是否被占用？）", s.controller_port)
    }

    fn forward_service_logs(&self, app: &AppHandle, mut since: u64) {
        let (logs, app) = (self.logs.clone(), app.clone());
        let task = tauri::async_runtime::spawn(async move {
            loop {
                if let Ok(lines) = crate::tunsvc::logs(since).await {
                    for (n, line) in lines {
                        since = n;
                        push_log(&logs, &app, line);
                    }
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
        if let Some(old) = self.service_logs.lock().unwrap().replace(task) {
            old.abort();
        }
    }

    async fn spawn_local(&self, app: &AppHandle, store: &Store, s: &Settings, bin: &std::path::Path, cfg: &std::path::Path) -> Result<()> {
        let dir = store.runtime_dir();
        let mut cmd = Command::new(&bin);
        match s.core {
            CoreKind::Prism | CoreKind::Mihomo => {
                cmd.arg("-f").arg(&cfg).arg("-d").arg(&dir);
            }
            CoreKind::Singbox => {
                cmd.arg("run").arg("-c").arg(&cfg).arg("-D").arg(&dir).arg("--disable-color");
            }
        }
        cmd.current_dir(&dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn().with_context(|| format!("启动 {} 失败", bin.display()))?;
        self.push_log(app, format!("[prism] 启动 {:?} 内核: {}", s.core, bin.display()));
        for out in [child.stdout.take().map(|o| Box::new(o) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
                    child.stderr.take().map(|e| Box::new(e) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)]
            .into_iter()
            .flatten()
        {
            let logs = self.logs.clone();
            let app = app.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    push_log(&logs, &app, line);
                }
            });
        }
        *self.child.lock().await = Some((s.core, child));
        Ok(())
    }

    pub async fn stop(&self) {
        if let Some((_, mut child)) = self.child.lock().await.take() {
            let _ = child.kill().await;
        }
        #[cfg(target_os = "macos")]
        crate::macdns::restore();
        if self.service.lock().await.take().is_some() {
            let _ = crate::tunsvc::stop().await;
        }
        if let Some(task) = self.service_logs.lock().unwrap().take() {
            task.abort();
        }
    }
}

fn push_log(logs: &Mutex<VecDeque<String>>, app: &AppHandle, line: String) {
    {
        let mut l = logs.lock().unwrap();
        l.push_back(line.clone());
        while l.len() > MAX_LOGS {
            l.pop_front();
        }
    }
    let _ = app.emit("core-log", line);
}

/// Calls the running core's Clash API.
pub async fn api(s: &Settings, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
    let url = format!("http://127.0.0.1:{}{}", s.controller_port, path);
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(30)).build()?;
    let mut req = client.request(method.parse()?, &url).bearer_auth(&s.secret);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        let msg = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from))
            .unwrap_or(text);
        bail!("{status}: {msg}");
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}
