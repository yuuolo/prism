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
    pub logs: Arc<Mutex<VecDeque<String>>>,
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

fn active_profile(store: &Store, s: &Settings) -> Result<String> {
    let id = s.active_profile.clone().ok_or_else(|| anyhow!("请先在「订阅」页导入并启用一个订阅"))?;
    Ok(store.read_content(&id)?.trim_start_matches('\u{feff}').to_string())
}

/// sing-box config for the active profile, before rule-sets are localized.
fn singbox_config(store: &Store, s: &Settings) -> Result<Value> {
    let raw = active_profile(store, s)?;
    let o = overrides(s);
    let mut doc: Value = match config::detect(&raw) {
        Format::SingBox => serde_json::from_str(&raw)?,
        _ => singbox::emit(&config::parse_any(&raw)?, &o),
    };
    singbox::apply_overrides(&mut doc, &o);
    Ok(doc)
}

/// Remote rule-sets the sing-box config uses (the app downloads them).
pub fn singbox_rule_sets(store: &Store, s: &Settings) -> Result<Vec<singbox::RemoteRuleSet>> {
    Ok(singbox::remote_rule_sets(&singbox_config(store, s)?))
}

pub struct RuntimeConfig {
    pub path: PathBuf,
    /// Things the user should know about (e.g. skipped rule-sets).
    pub warnings: Vec<String>,
}

/// Converts the active profile into the selected core's config format and
/// writes it to the runtime dir.
pub fn write_runtime_config(store: &Store, s: &Settings) -> Result<RuntimeConfig> {
    let dir = store.runtime_dir();
    match s.core {
        CoreKind::Prism | CoreKind::Mihomo => {
            let raw = active_profile(store, s)?;
            let mut doc: serde_yaml::Value = match config::detect(&raw) {
                Format::Clash => serde_yaml::from_str(&raw)?,
                _ => clash::emit(&config::parse_any(&raw)?),
            };
            clash::apply_overrides(&mut doc, &overrides(s));
            let path = dir.join("config.yaml");
            std::fs::write(&path, serde_yaml::to_string(&doc)?)?;
            Ok(RuntimeConfig { path, warnings: Vec::new() })
        }
        CoreKind::Singbox => {
            let mut doc = singbox_config(store, s)?;
            // sing-box exits if a remote rule-set can't be fetched at startup,
            // so it only ever gets the files the app downloaded.
            let warnings = singbox::localize_rule_sets(&mut doc, &store.rule_set_dir())
                .into_iter()
                .map(|t| format!("规则集 {t} 未能下载，使用它的规则已跳过（可在设置页「更新 GeoSite / GeoIP」重试）"))
                .collect();
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

        // Wait for the API to come up (or the process to die).
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if self.running().await.is_none() {
                bail!("内核启动后立即退出，请查看「日志」页");
            }
            if api(s, "GET", "/version", None).await.is_ok() {
                return Ok(());
            }
        }
        bail!("内核 API 无响应（端口 {} 是否被占用？）", s.controller_port)
    }

    pub async fn stop(&self) {
        if let Some((_, mut child)) = self.child.lock().await.take() {
            let _ = child.kill().await;
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
