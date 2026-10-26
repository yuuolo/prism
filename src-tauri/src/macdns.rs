//! macOS TUN DNS: the system resolver usually asks the router, which sits
//! on the local network and so outside the tunnel's routes. While TUN runs,
//! every enabled network service gets public DNS servers instead; those
//! are routed into the tunnel, where the core answers them (fake-ip, rules
//! ...). Should Prism die without restoring, the public servers still work.
//!
//! The previous settings are kept in `dns-backup.json` in the data folder
//! and put back when TUN stops, Prism exits, or at the next start.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use anyhow::{bail, Context, Result};

const SERVERS: [&str; 2] = ["223.5.5.5", "119.29.29.29"];
const FILE: &str = "dns-backup.json";

/// Backup file of the override in effect.
static ACTIVE: Mutex<Option<PathBuf>> = Mutex::new(None);

fn networksetup(args: &[&str]) -> Result<String> {
    let out = Command::new("/usr/sbin/networksetup").args(args).output().context("run networksetup")?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() || text.contains("** Error") {
        bail!("networksetup {}: {}", args.join(" "), text.trim());
    }
    Ok(text)
}

/// A service's DNS servers; empty when it uses the ones from DHCP.
fn current(service: &str) -> Vec<String> {
    networksetup(&["-getdnsservers", service])
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| l.parse::<std::net::IpAddr>().is_ok())
        .map(String::from)
        .collect()
}

fn set(service: &str, servers: &[String]) -> Result<()> {
    let mut args = vec!["-setdnsservers", service];
    if servers.is_empty() {
        args.push("Empty"); // back to DHCP
    } else {
        args.extend(servers.iter().map(String::as_str));
    }
    networksetup(&args).map(|_| ())
}

/// Points every enabled service at the public servers, saving what was
/// there (an existing backup is kept: it holds the real originals).
pub fn override_dns(dir: &Path) -> Result<()> {
    let file = dir.join(FILE);
    let services = crate::sysproxy::services();
    let mut saved: BTreeMap<String, Vec<String>> = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let ours: Vec<String> = SERVERS.iter().map(|s| s.to_string()).collect();
    for s in &services {
        let now = current(s);
        if now != ours {
            saved.entry(s.clone()).or_insert(now);
        }
    }
    std::fs::write(&file, serde_json::to_vec_pretty(&saved)?)?;
    *ACTIVE.lock().unwrap() = Some(file);
    for s in &services {
        set(s, &ours)?;
    }
    Ok(())
}

/// Puts back the settings saved in `dir`, if any.
pub fn restore_from(dir: &Path) -> Result<()> {
    let file = dir.join(FILE);
    let Ok(bytes) = std::fs::read(&file) else { return Ok(()) };
    let saved: BTreeMap<String, Vec<String>> = serde_json::from_slice(&bytes).unwrap_or_default();
    for (service, servers) in &saved {
        set(service, servers)?;
    }
    std::fs::remove_file(&file)?;
    Ok(())
}

/// Undoes the override made by this run, if any.
pub fn restore() {
    let file = ACTIVE.lock().unwrap().take();
    if let Some(dir) = file.as_deref().and_then(Path::parent) {
        if let Err(e) = restore_from(dir) {
            eprintln!("[prism] 恢复 DNS 设置失败: {e:#}");
        }
    }
}
