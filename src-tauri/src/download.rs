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

const WINTUN_URL: &str = "https://www.wintun.net/builds/wintun-0.14.1.zip";
const WINTUN_SHA256: &str = "07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51";

/// Downloads the official, signed wintun.dll (needed by prism-core's TUN).
pub async fn wintun(dir: &Path, proxy_port: Option<u16>) -> Result<()> {
    use sha2::{Digest, Sha256};
    let data = fetch(&client(proxy_port)?, WINTUN_URL).await?;
    let hash = hex(&Sha256::digest(&data));
    if hash != WINTUN_SHA256 {
        bail!("wintun.zip checksum mismatch ({hash})");
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data))?;
    let mut f = zip.by_name("wintun/bin/amd64/wintun.dll").context("wintun.dll not in zip")?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    std::fs::write(dir.join("wintun.dll"), buf)?;
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// jsdelivr mirror of a raw.githubusercontent.com URL (usually reachable
/// when GitHub is not).
pub fn mirror(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://raw.githubusercontent.com/")?;
    let mut parts = rest.splitn(4, '/');
    let (owner, repo, branch, path) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    Some(format!("https://fastly.jsdelivr.net/gh/{owner}/{repo}@{branch}/{path}"))
}

/// Downloads a sing-box rule-set to `dest`: through the running proxy if
/// any, then the jsdelivr mirror, then the original URL.
pub async fn rule_set(url: &str, dest: &Path, proxy_port: Option<u16>) -> Result<()> {
    let mut tries: Vec<(String, Option<u16>)> = Vec::new();
    if proxy_port.is_some() {
        tries.push((url.to_string(), proxy_port));
    }
    if let Some(m) = mirror(url) {
        tries.push((m, None));
    }
    tries.push((url.to_string(), None));
    let binary = dest.extension().is_some_and(|e| e == "srs");
    let mut last = anyhow!("no source");
    for (u, port) in tries {
        let c = client(port)?;
        match tokio::time::timeout(Duration::from_secs(60), fetch(&c, &u)).await {
            // Reject HTML error pages and the like.
            Ok(Ok(data)) if (binary && data.starts_with(b"SRS")) || (!binary && serde_json::from_slice::<serde_json::Value>(&data).is_ok()) => {
                let tmp = dest.with_extension("download");
                std::fs::write(&tmp, data)?;
                std::fs::rename(&tmp, dest)?;
                return Ok(());
            }
            Ok(Ok(_)) => last = anyhow!("{u}: not a rule-set file"),
            Ok(Err(e)) => last = e,
            Err(_) => last = anyhow!("{u}: timeout"),
        }
    }
    Err(last)
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
    fn mirrors_raw_github() {
        assert_eq!(
            mirror("https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-cn.srs").as_deref(),
            Some("https://fastly.jsdelivr.net/gh/SagerNet/sing-geosite@rule-set/geosite-cn.srs")
        );
        assert_eq!(mirror("https://example.com/a.srs"), None);
    }

    #[test]
    #[ignore = "network"]
    fn downloads_real_rule_set() {
        let dir = std::env::temp_dir().join(format!("prism-dl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("geosite-private.srs");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(rule_set("https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-private.srs", &dest, None))
            .unwrap();
        assert!(std::fs::read(&dest).unwrap().starts_with(b"SRS"));
    }

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
