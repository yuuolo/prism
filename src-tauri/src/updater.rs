//! App self-update from GitHub releases (tauri-plugin-updater).
//!
//! Each release carries a `latest.json` manifest and a minisign signature of
//! the installer; the app only installs packages signed with the key whose
//! public half is in tauri.conf.json. `PRISM_UPDATE_ENDPOINT` overrides the
//! manifest URL (testing, mirrors).

use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Clone, Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
}

#[derive(Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

/// Looks for a newer release. Downloads go through the running core's
/// proxy when there is one (GitHub is often slow without it).
pub async fn find(app: &AppHandle, proxy_port: Option<u16>) -> Result<Option<Update>> {
    let mut b = app.updater_builder().timeout(Duration::from_secs(30));
    if let Ok(ep) = std::env::var("PRISM_UPDATE_ENDPOINT") {
        b = b.endpoints(vec![ep.parse()?])?;
    }
    b = match proxy_port {
        Some(p) => b.proxy(format!("http://127.0.0.1:{p}").parse()?),
        None => b.no_proxy(),
    };
    Ok(b.build()?.check().await?)
}

pub fn info(u: &Update) -> UpdateInfo {
    UpdateInfo {
        current: u.current_version.clone(),
        version: u.version.clone(),
        notes: u.body.clone(),
        date: u.date.map(|d| d.to_string()),
    }
}

/// Downloads and verifies the update, emitting `update-progress` events.
/// Returns the verified installer bytes.
pub async fn download(app: &AppHandle, u: &Update) -> Result<Vec<u8>> {
    let mut downloaded = 0u64;
    let handle = app.clone();
    let bytes = u
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = handle.emit("update-progress", Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| anyhow!("下载或校验更新失败: {e}"))?;
    Ok(bytes)
}

/// Runs the installer (passive mode); on Windows this exits the app and the
/// installer starts the new version.
pub fn install(u: &Update, bytes: &[u8]) -> Result<()> {
    u.install(bytes).map_err(|e| anyhow!("启动安装程序失败: {e}"))
}
