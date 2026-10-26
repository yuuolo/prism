//! TUN service mode: when Prism isn't elevated, a TUN core runs inside the
//! Prism TUN Service (crates/prism-service) instead of as a child process.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::Engine;
use prism_service::{allowed_file, sha256_file, Request, Response, Status};

use crate::core::core_binary;
use crate::store::{CoreKind, Settings, Store};

pub use prism_service::client::State;

/// Error prefix the UI recognises to offer refreshing the service's cores.
pub const NEED_UPDATE: &str = "NEED_SERVICE_UPDATE: TUN 服务中的内核与当前版本不一致，需要更新服务";

pub fn kind_name(k: CoreKind) -> &'static str {
    match k {
        CoreKind::Prism => "prism",
        CoreKind::Mihomo => "mihomo",
        CoreKind::Singbox => "singbox",
    }
}

pub fn state() -> State {
    prism_service::client::state()
}

/// prism-service ships next to the prism executable (a Tauri sidecar).
fn service_binary() -> Result<PathBuf> {
    let p = std::env::current_exe()?.with_file_name(format!("prism-service{}", prism_service::EXE));
    if !p.exists() {
        bail!("找不到 {}", p.display());
    }
    Ok(p)
}

async fn call(req: &Request) -> Result<Response> {
    let r = tokio::time::timeout(Duration::from_secs(60), prism_service::client::request(req)).await.context("TUN 服务无响应")??;
    if !r.ok {
        bail!("TUN 服务: {}", r.error.unwrap_or_default());
    }
    Ok(r)
}

pub async fn status() -> Result<Status> {
    call(&Request::Status).await?.status.context("no status")
}

/// Copies the cores this Prism uses into the service (installing it if
/// needed). Shows a UAC prompt (Windows) / the administrator password
/// prompt (macOS).
pub async fn install(store: &Store, s: &Settings) -> Result<()> {
    let exe = service_binary()?;
    let rt = store.runtime_dir();
    let sid = prism_service::client::current_user_sid()?;
    #[cfg(windows)]
    let q = |p: &std::path::Path| format!("\"{}\"", p.display());
    #[cfg(not(windows))]
    let q = |p: &std::path::Path| prism_service::client::sh_quote(&p.to_string_lossy());
    let mut args = format!("install --user {sid} --gui {}", q(&std::env::current_exe()?));
    if cfg!(windows) {
        if !rt.join("wintun.dll").exists() {
            crate::download::wintun(&rt, None).await.context("下载 wintun.dll 失败")?;
        }
        args += &format!(" --wintun {}", q(&rt.join("wintun.dll")));
    }
    for k in [CoreKind::Prism, CoreKind::Mihomo, CoreKind::Singbox] {
        let bin = core_binary(store, s, k);
        if bin.exists() {
            args += &format!(" --core {}={}", kind_name(k), q(&bin));
        }
    }
    let code = tokio::task::spawn_blocking(move || prism_service::client::run_elevated(&exe, &args)).await??;
    if code != 0 {
        let log = std::fs::read_to_string(prism_service::data_dir().join("service.log")).unwrap_or_default();
        bail!("安装 TUN 服务失败（代码 {code}）{}", log.lines().last().map(|l| format!(": {l}")).unwrap_or_default());
    }
    Ok(())
}

pub async fn uninstall() -> Result<()> {
    let exe = service_binary()?;
    let code = tokio::task::spawn_blocking(move || prism_service::client::run_elevated(&exe, "uninstall")).await??;
    if code != 0 {
        bail!("卸载 TUN 服务失败（代码 {code}）");
    }
    Ok(())
}

/// Whether the service's copy of `kind` (and wintun) differs from ours.
pub fn stale(store: &Store, s: &Settings, st: &Status, kind: CoreKind) -> bool {
    let ours = sha256_file(&core_binary(store, s, kind));
    if ours.is_none() || ours.as_ref() != st.cores.get(kind_name(kind)) {
        return true;
    }
    cfg!(windows) && kind == CoreKind::Prism && sha256_file(&store.runtime_dir().join("wintun.dll")).as_ref() != st.wintun.as_ref()
}

/// Starts `kind` in the service with `config`, sending the data files it
/// lacks (geodata, sing-box rule-sets).
pub async fn start(store: &Store, s: &Settings, config: String) -> Result<()> {
    let st = status().await?;
    if stale(store, s, &st, s.core) {
        bail!(NEED_UPDATE);
    }
    let rt = store.runtime_dir();
    let mut local: Vec<(String, PathBuf)> = std::fs::read_dir(&rt)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    local.extend(
        std::fs::read_dir(store.rule_set_dir())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| (format!("rule-set/{}", e.file_name().to_string_lossy()), e.path())),
    );
    let mut files = BTreeMap::new();
    for (name, path) in local.into_iter().filter(|(n, _)| allowed_file(n)) {
        let hash = sha256_file(&path);
        if hash.is_some() && hash.as_ref() != st.files.get(&name) {
            let bytes = std::fs::read(&path)?;
            files.insert(name, base64::engine::general_purpose::STANDARD.encode(bytes));
        }
    }
    call(&Request::Start { core: kind_name(s.core).into(), config, files }).await?;
    Ok(())
}

pub async fn stop() -> Result<()> {
    call(&Request::Stop).await.map(|_| ())
}

/// Core output after line `since`.
pub async fn logs(since: u64) -> Result<Vec<(u64, String)>> {
    Ok(call(&Request::Logs { since }).await?.lines)
}
