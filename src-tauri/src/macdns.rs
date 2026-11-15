//! macOS TUN DNS: the system resolver usually asks the router, which sits
//! on the local network and so outside the tunnel's routes. While TUN runs,
//! every enabled network service gets public DNS servers instead; those
//! are routed into the tunnel, where the core answers them (fake-ip, rules
//! ...). Should Prism die without restoring, the public servers still work.
//!
//! The previous settings are kept in `dns-backup.json` in the data folder
//! and put back when TUN stops, Prism exits, or at the next start.

use std::collections::BTreeMap;
use std::net::IpAddr;
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

/// Points every enabled service at public DNS servers: `servers` (the
/// core's own, see config_servers), or 223.5.5.5 / 119.29.29.29 when there
/// are none. Saves what was there (an existing backup is kept: it holds
/// the real originals).
pub fn override_dns(dir: &Path, servers: &[String]) -> Result<()> {
    let ours: Vec<String> = if servers.is_empty() { SERVERS.iter().map(|s| s.to_string()).collect() } else { servers.to_vec() };
    override_on(dir, &crate::sysproxy::services(), &ours)
}

/// The public DNS servers, given by IP, that the core's config `config`
/// uses itself: mihomo / prism-core `dns.default-nameserver`, sing-box
/// `dns.servers`. The profile's choice suits where its users are (an
/// Alibaba DNS in China, Cloudflare elsewhere), and a server it reaches
/// directly also answers if Prism dies before putting the system's back.
/// Private, CGNAT (Tailscale) and fake-ip addresses are left out: the
/// system would reach them outside the tunnel. At most 3.
pub fn config_servers(config: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(config) else { return Vec::new() };
    let candidates: Vec<String> = if config.extension().is_some_and(|e| e == "json") {
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        v["dns"]["servers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| {
                // New format: {"type": "udp", "server": ip, "server_port"?}; old: {"address": "ip" | "udp://ip"}.
                let new = matches!(s["type"].as_str(), Some("udp") | None) && s["server_port"].as_u64().is_none_or(|p| p == 53);
                s["server"].as_str().filter(|_| new).or(s["address"].as_str()).map(String::from)
            })
            .collect()
    } else {
        let v: serde_yaml::Value = serde_yaml::from_str(&text).unwrap_or_default();
        v["dns"]["default-nameserver"].as_sequence().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect()
    };
    let mut out: Vec<String> = Vec::new();
    for ip in candidates.iter().filter_map(|c| plain_ip(c)).filter(is_public) {
        let ip = ip.to_string();
        if !out.contains(&ip) && out.len() < 3 {
            out.push(ip);
        }
    }
    out
}

/// `1.2.3.4`, `udp://1.2.3.4` or with port 53; not DoH / DoT / other ports.
fn plain_ip(s: &str) -> Option<IpAddr> {
    let s = s.trim().strip_prefix("udp://").unwrap_or(s.trim());
    s.parse().ok().or_else(|| s.parse::<std::net::SocketAddr>().ok().filter(|a| a.port() == 53).map(|a| a.ip()))
}

fn is_public(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified() || v4.is_broadcast() || v4.is_multicast()
                || a == 0
                || (a == 100 && (64..128).contains(&b)) // CGNAT, Tailscale
                || (a == 198 && (18..20).contains(&b))) // benchmarking, fake-ip
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80)
        }
    }
}

fn override_on(dir: &Path, services: &[String], ours: &[String]) -> Result<()> {
    let file = dir.join(FILE);
    let mut saved: BTreeMap<String, Vec<String>> = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let ours = ours.to_vec();
    for s in services {
        let now = current(s);
        if now != ours {
            saved.entry(s.clone()).or_insert(now);
        }
    }
    std::fs::write(&file, serde_json::to_vec_pretty(&saved)?)?;
    *ACTIVE.lock().unwrap() = Some(file);
    let errors: Vec<String> = services.iter().filter_map(|s| set(s, &ours).err().map(|e| format!("{e:#}"))).collect();
    if !errors.is_empty() {
        bail!("{}", errors.join("; "));
    }
    Ok(())
}

/// Puts back the settings saved in `dir`, if any. One service failing (it
/// was deleted, say) doesn't stop the others; only the failed ones stay in
/// the backup for the next try.
pub fn restore_from(dir: &Path) -> Result<()> {
    let file = dir.join(FILE);
    let Ok(bytes) = std::fs::read(&file) else { return Ok(()) };
    let saved: BTreeMap<String, Vec<String>> = serde_json::from_slice(&bytes).unwrap_or_default();
    let mut failed = BTreeMap::new();
    let mut errors = Vec::new();
    for (service, servers) in saved {
        if let Err(e) = set(&service, &servers) {
            errors.push(format!("{e:#}"));
            failed.insert(service, servers);
        }
    }
    if failed.is_empty() {
        std::fs::remove_file(&file)?;
        return Ok(());
    }
    std::fs::write(&file, serde_json::to_vec_pretty(&failed)?)?;
    bail!("{}", errors.join("; "))
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

#[cfg(test)]
mod tests {
    /// On the real network services, with one that fails in between:
    /// overrides the DNS servers and restores them. Changes this Mac's DNS
    /// for a moment and puts every service back, so only with
    /// PRISM_DNS_LIVE=1: `PRISM_DNS_LIVE=1 cargo test -p prism macdns -- --ignored`.
    #[test]
    #[ignore]
    fn live_override_and_restore_on_this_mac() {
        if std::env::var_os("PRISM_DNS_LIVE").is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("prism-macdns-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let real = crate::sysproxy::services();
        let before: Vec<_> = real.iter().map(|s| super::current(s)).collect();
        let mut svcs = real.clone();
        svcs.insert(svcs.len() / 2, "Prism No Such Service".into());
        let ours: Vec<String> = super::SERVERS.iter().map(|s| s.to_string()).collect();

        let over = super::override_on(&dir, &svcs, &ours).unwrap_err().to_string();
        let during: Vec<_> = real.iter().map(|s| super::current(s)).collect();
        let restore = super::restore_from(&dir).unwrap_err().to_string();
        let after: Vec<_> = real.iter().map(|s| super::current(s)).collect();
        let left = std::fs::read_to_string(dir.join(super::FILE)).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        *super::ACTIVE.lock().unwrap() = None;

        println!("{} services; before {before:?}; after {after:?}", real.len());
        assert!(over.contains("Prism No Such Service") && restore.contains("Prism No Such Service"), "{over} / {restore}");
        assert!(during.iter().all(|d| *d == ours), "{during:?}");
        assert_eq!(after, before, "every real service is back");
        assert!(left.contains("Prism No Such Service") && real.iter().all(|s| !left.contains(&format!("\"{s}\""))), "{left}");
    }

    #[test]
    fn servers_from_the_core_config() {
        let dir = std::env::temp_dir().join(format!("prism-macdns-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let yaml = dir.join("config.yaml");
        std::fs::write(&yaml, "dns:\n  default-nameserver: [1.1.1.1, 192.168.1.1, 'tls://8.8.4.4', 'udp://8.8.8.8', 100.100.100.100, 1.1.1.1, '9.9.9.9:53', 198.18.0.2, 2606:4700:4700::1111]\n").unwrap();
        assert_eq!(super::config_servers(&yaml), ["1.1.1.1", "8.8.8.8", "9.9.9.9"]);
        let json = dir.join("config.json");
        std::fs::write(
            &json,
            r#"{"dns":{"servers":[{"type":"udp","tag":"local","server":"223.5.5.5"},{"type":"https","server":"1.0.0.1"},{"type":"udp","server":"9.9.9.9","server_port":5353},{"address":"udp://119.29.29.29"},{"address":"10.0.0.1"},{"type":"fakeip","inet4_range":"198.18.0.0/16"}]}}"#,
        )
        .unwrap();
        assert_eq!(super::config_servers(&json), ["223.5.5.5", "119.29.29.29"]);
        std::fs::write(&yaml, "mixed-port: 7890\n").unwrap();
        assert!(super::config_servers(&yaml).is_empty());
        assert!(super::config_servers(&dir.join("missing.yaml")).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Only a service that doesn't exist, so no real setting is touched.
    #[test]
    fn failed_restore_keeps_only_the_failures() {
        let dir = std::env::temp_dir().join(format!("prism-macdns-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(super::FILE);
        std::fs::write(&file, r#"{"Prism No Such Service": []}"#).unwrap();
        let err = super::restore_from(&dir).unwrap_err().to_string();
        assert!(err.contains("Prism No Such Service"), "{err}");
        assert!(std::fs::read_to_string(&file).unwrap().contains("Prism No Such Service"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
