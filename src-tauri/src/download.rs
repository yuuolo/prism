//! Downloads mihomo / sing-box releases and geodata files.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

use crate::store::CoreKind;

const GEO_MIRRORS: [&str; 2] = [
    "https://fastly.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/",
    "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/",
];

/// HTTP client, optionally through the running core's mixed port.
pub fn client(proxy_port: Option<u16>) -> Result<reqwest::Client> {
    let mut b = reqwest::Client::builder()
        .user_agent(concat!("prism/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600));
    b = match proxy_port {
        Some(p) => b.proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{p}"))?),
        None => b.no_proxy(),
    };
    Ok(b.build()?)
}

async fn fetch(c: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let r = c.get(url).send().await?;
    if !r.status().is_success() {
        bail!("{url}: HTTP {}", r.status());
    }
    Ok(r.bytes().await?.to_vec())
}

fn pick_asset(kind: CoreKind, name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    if !n.ends_with(".zip") || !n.contains("windows-amd64") {
        return false;
    }
    match kind {
        // mihomo-windows-amd64-v1.19.x.zip (skip -compatible, -v1/-v2/-v3, -goXXX builds)
        CoreKind::Mihomo => {
            n.starts_with("mihomo-windows-amd64-v")
                && !["-v1-", "-v2-", "-v3-", "-go1", "compatible"].iter().any(|x| n.contains(x))
        }
        // sing-box-1.12.x-windows-amd64.zip
        CoreKind::Singbox => n.starts_with("sing-box-") && n.ends_with("-windows-amd64.zip"),
        CoreKind::Prism => false,
    }
}

/// Downloads the latest release of mihomo or sing-box into `dest_dir`.
/// Returns the version tag.
pub async fn core(kind: CoreKind, dest_dir: &Path, proxy_port: Option<u16>) -> Result<String> {
    let (repo, exe) = match kind {
        CoreKind::Mihomo => ("MetaCubeX/mihomo", "mihomo.exe"),
        CoreKind::Singbox => ("SagerNet/sing-box", "sing-box.exe"),
        CoreKind::Prism => bail!("prism-core is bundled with the app"),
    };
    let c = client(proxy_port)?;
    let rel: serde_json::Value = c
        .get(format!("https://api.github.com/repos/{repo}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let tag = rel["tag_name"].as_str().unwrap_or_default().to_string();
    let asset = rel["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["name"].as_str().is_some_and(|n| pick_asset(kind, n))))
        .ok_or_else(|| anyhow!("{repo} {tag}: no windows-amd64 asset found"))?;
    let url = asset["browser_download_url"].as_str().unwrap_or_default();
    let data = fetch(&c, url).await?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data)).context("bad zip")?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        if f.name().to_ascii_lowercase().ends_with(".exe") {
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            std::fs::create_dir_all(dest_dir)?;
            let tmp = dest_dir.join(format!("{exe}.download"));
            std::fs::write(&tmp, buf)?;
            std::fs::rename(&tmp, dest_dir.join(exe))?;
            return Ok(tag);
        }
    }
    bail!("no executable inside {url}")
}

/// Downloads geosite.dat and geoip.dat (used by prism-core and mihomo).
pub async fn geodata(dir: &Path, proxy_port: Option<u16>) -> Result<()> {
    let c = client(proxy_port)?;
    for file in ["geosite.dat", "geoip.dat"] {
        let mut last = None;
        for m in GEO_MIRRORS {
            match fetch(&c, &format!("{m}{file}")).await {
                Ok(data) if data.len() > 1024 => {
                    let tmp = dir.join(format!("{file}.download"));
                    std::fs::write(&tmp, data)?;
                    std::fs::rename(&tmp, dir.join(file))?;
                    last = None;
                    break;
                }
                Ok(_) => last = Some(anyhow!("{file}: empty response")),
                Err(e) => last = Some(e),
            }
        }
        if let Some(e) = last {
            return Err(e);
        }
    }
    Ok(())
}

/// Version string printed by a core binary (`-v` / `version`).
pub async fn core_version(kind: CoreKind, bin: &Path) -> Option<String> {
    if !bin.exists() {
        return None;
    }
    let mut cmd = tokio::process::Command::new(bin);
    match kind {
        CoreKind::Singbox => cmd.arg("version"),
        CoreKind::Mihomo => cmd.arg("-v"),
        CoreKind::Prism => cmd.arg("--version"),
    };
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let out = tokio::time::timeout(Duration::from_secs(5), cmd.output()).await.ok()?.ok()?;
    String::from_utf8_lossy(&out.stdout).lines().next().map(|l| l.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_names() {
        assert!(pick_asset(CoreKind::Mihomo, "mihomo-windows-amd64-v1.19.15.zip"));
        assert!(!pick_asset(CoreKind::Mihomo, "mihomo-windows-amd64-compatible-v1.19.15.zip"));
        assert!(!pick_asset(CoreKind::Mihomo, "mihomo-windows-amd64-v1-v1.19.15.zip"));
        assert!(!pick_asset(CoreKind::Mihomo, "mihomo-windows-amd64-go120-v1.19.15.zip"));
        assert!(pick_asset(CoreKind::Singbox, "sing-box-1.12.9-windows-amd64.zip"));
        assert!(!pick_asset(CoreKind::Singbox, "sing-box-1.12.9-windows-amd64-legacy-windows-7.zip"));
    }
}
