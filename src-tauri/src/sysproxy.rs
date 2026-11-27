//! System proxy: WinINet registry settings on Windows, `networksetup` on
//! macOS.

#[cfg(windows)]
pub fn set(enable: bool, server: &str, bypass: &str) -> anyhow::Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(
        r"Software\Microsoft\Windows\CurrentVersion\Internet Settings",
        KEY_READ | KEY_WRITE,
    )?;
    key.set_value("ProxyEnable", &(enable as u32))?;
    if enable {
        key.set_value("ProxyServer", &server.to_string())?;
        key.set_value("ProxyOverride", &bypass.to_string())?;
    }
    unsafe {
        use windows_sys::Win32::Networking::WinInet::{
            InternetSetOptionW, INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED,
        };
        InternetSetOptionW(std::ptr::null(), INTERNET_OPTION_SETTINGS_CHANGED, std::ptr::null(), 0);
        InternetSetOptionW(std::ptr::null(), INTERNET_OPTION_REFRESH, std::ptr::null(), 0);
    }
    Ok(())
}

/// Returns (enabled, server) as currently configured in Windows.
#[cfg(windows)]
pub fn get() -> (bool, String) {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
    else {
        return (false, String::new());
    };
    let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    let server: String = key.get_value("ProxyServer").unwrap_or_default();
    (enabled == 1, server)
}

/// macOS: HTTP, HTTPS and SOCKS proxies on every enabled network service
/// (Wi-Fi, Ethernet, ...), through `networksetup`.
#[cfg(target_os = "macos")]
mod mac {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use anyhow::{bail, Context, Result};
    use serde::{Deserialize, Serialize};

    /// The proxy settings from before Prism set its own, per service, kept
    /// in the data folder until Prism turns its proxy off (or exits) and
    /// puts them back.
    pub(super) const BACKUP_FILE: &str = "proxy-backup.json";
    static DIR: OnceLock<PathBuf> = OnceLock::new();

    /// The data folder, for the backup (set once at startup).
    pub fn set_backup_dir(dir: &Path) {
        let _ = DIR.set(dir.to_path_buf());
    }

    /// One service's proxies: (host, port) of each one that was on.
    #[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
    pub(super) struct Saved {
        pub(super) web: Option<(String, String)>,
        pub(super) secure: Option<(String, String)>,
        pub(super) socks: Option<(String, String)>,
        pub(super) bypass: Vec<String>,
    }

    /// (get, set address, set state) for HTTP, HTTPS and SOCKS.
    const KINDS: [[&str; 3]; 3] = [
        ["-getwebproxy", "-setwebproxy", "-setwebproxystate"],
        ["-getsecurewebproxy", "-setsecurewebproxy", "-setsecurewebproxystate"],
        ["-getsocksfirewallproxy", "-setsocksfirewallproxy", "-setsocksfirewallproxystate"],
    ];

    /// What `service` has now. A proxy that already points at `ours`
    /// (Prism's, left by a crash) counts as off: putting it back would
    /// point the Mac at a core that isn't running.
    pub(super) fn read_service(service: &str, ours: (&str, &str)) -> Result<Saved> {
        let mut on = KINDS.iter().map(|[get, ..]| -> Result<Option<(String, String)>> {
            let text = networksetup(&[get, service])?;
            let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k).map(|v| v.trim().to_string())).unwrap_or_default();
            let (host, port) = (field("Server:"), field("Port:"));
            Ok((field("Enabled:") == "Yes" && (host.as_str(), port.as_str()) != ours).then_some((host, port)))
        });
        let (web, secure, socks) = (on.next().unwrap()?, on.next().unwrap()?, on.next().unwrap()?);
        let bypass = networksetup(&["-getproxybypassdomains", service])?
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("There aren't any"))
            .map(String::from)
            .collect();
        Ok(Saved { web, secure, socks, bypass })
    }

    /// Puts `saved` back on `service`: setting an address turns that proxy
    /// on; the others are turned off. Tries every part even when one fails.
    pub(super) fn write_service(service: &str, saved: &Saved) -> Result<()> {
        let mut results = Vec::new();
        for ([_, set, state], entry) in KINDS.iter().zip([&saved.web, &saved.secure, &saved.socks]) {
            results.push(match entry {
                Some((host, port)) => networksetup(&[set, service, host, port]),
                None => networksetup(&[state, service, "off"]),
            });
        }
        let mut args = vec!["-setproxybypassdomains", service];
        if saved.bypass.is_empty() {
            args.push("Empty");
        } else {
            args.extend(saved.bypass.iter().map(String::as_str));
        }
        results.push(networksetup(&args));
        results.into_iter().collect::<Result<Vec<_>>>().map(|_| ())
    }

    fn networksetup(args: &[&str]) -> Result<String> {
        let out = std::process::Command::new("/usr/sbin/networksetup").args(args).output().context("run networksetup")?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        // networksetup reports some failures on stdout with status 0.
        if !out.status.success() || text.contains("** Error") {
            bail!("networksetup {}: {}{}", args.join(" "), text.trim(), String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(text)
    }

    /// Enabled network services (disabled ones are listed with a `*`).
    pub fn services() -> Vec<String> {
        networksetup(&["-listallnetworkservices"])
            .unwrap_or_default()
            .lines()
            .skip(1) // "An asterisk (*) denotes that a network service is disabled."
            .filter(|l| !l.is_empty() && !l.starts_with('*'))
            .map(String::from)
            .collect()
    }

    /// The Windows-style bypass list (`localhost;127.*;<local>`) as macOS
    /// takes it.
    pub fn bypass_list(bypass: &str) -> Vec<String> {
        let mut v: Vec<String> = bypass
            .split([';', ',', ' '])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| if s == "<local>" { "*.local".to_string() } else { s.to_string() })
            .collect();
        for must in ["localhost", "127.0.0.1"] {
            if !v.iter().any(|x| x == must) {
                v.push(must.into());
            }
        }
        v
    }

    /// Turning off needs no address (Prism passes "" on exit). One service
    /// failing doesn't stop the others: on exit especially, a service left
    /// pointing at the stopped core would cut it off.
    ///
    /// Turning on first saves each service's own proxies (once: an existing
    /// backup holds the real originals); turning off puts them back, so a
    /// proxy that was set before Prism works again after it. Without a
    /// backup, turning off just turns the proxies off.
    pub fn set(enable: bool, server: &str, bypass: &str) -> Result<()> {
        set_on(&services(), enable, server, bypass, DIR.get().map(PathBuf::as_path))
    }

    pub(super) fn set_on(svcs: &[String], enable: bool, server: &str, bypass: &str, backup_dir: Option<&Path>) -> Result<()> {
        if svcs.is_empty() {
            bail!("没有可用的网络服务");
        }
        let file = backup_dir.map(|d| d.join(BACKUP_FILE));
        let addr = if enable { Some(server.rsplit_once(':').context("bad proxy address")?) } else { None };
        let mut errors = Vec::new();
        if let (Some(file), Some(ours)) = (&file, addr) {
            if !file.exists() {
                // A service that can't be read isn't saved; it is just turned off later.
                let saved: BTreeMap<String, Saved> = svcs.iter().filter_map(|s| read_service(s, ours).ok().map(|v| (s.clone(), v))).collect();
                std::fs::write(file, serde_json::to_vec_pretty(&saved)?)?;
            }
        }
        let mut saved: BTreeMap<String, Saved> = match (&file, addr) {
            (Some(file), None) => std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default(),
            _ => BTreeMap::new(),
        };
        for s in svcs {
            let r = match saved.remove(s) {
                Some(old) => write_service(s, &old).map_err(|e| (e, Some(old))),
                None => set_service(s, addr, bypass).map_err(|e| (e, None)),
            };
            if let Err((e, old)) = r {
                errors.push(format!("{e:#}"));
                if let Some(old) = old {
                    saved.insert(s.clone(), old); // try again next time
                }
            }
        }
        if let (Some(file), None) = (&file, addr) {
            // Services in the backup no longer listed (deleted, disabled) stay
            // in it with the ones that failed.
            if saved.is_empty() {
                let _ = std::fs::remove_file(file);
            } else if file.exists() {
                std::fs::write(file, serde_json::to_vec_pretty(&saved)?)?;
            }
        }
        if !errors.is_empty() {
            bail!("{}", errors.join("; "));
        }
        Ok(())
    }

    /// Sets (`addr` given) or turns off one service's proxies. Setting a
    /// proxy's address also turns it on, so that takes 4 networksetup calls
    /// per service instead of 7. Turning off tries all three states even
    /// when one fails.
    fn set_service(s: &str, addr: Option<(&str, &str)>, bypass: &str) -> Result<()> {
        if let Some((host, port)) = addr {
            networksetup(&["-setwebproxy", s, host, port])?;
            networksetup(&["-setsecurewebproxy", s, host, port])?;
            networksetup(&["-setsocksfirewallproxy", s, host, port])?;
            let mut args = vec!["-setproxybypassdomains".to_string(), s.to_string()];
            args.extend(bypass_list(bypass));
            networksetup(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
            return Ok(());
        }
        let results = ["-setwebproxystate", "-setsecurewebproxystate", "-setsocksfirewallproxystate"].map(|flag| networksetup(&[flag, s, "off"]));
        results.into_iter().collect::<Result<Vec<_>>>().map(|_| ())
    }

    /// Each proxy's (enabled, host, port), also when off, and setting them
    /// back exactly: the live test leaves the Mac as it found it.
    #[cfg(test)]
    pub(super) fn raw(service: &str) -> Vec<(bool, String, String)> {
        KINDS
            .iter()
            .map(|[get, ..]| {
                let text = networksetup(&[get, service]).unwrap_or_default();
                let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k).map(|v| v.trim().to_string())).unwrap_or_default();
                (field("Enabled:") == "Yes", field("Server:"), field("Port:"))
            })
            .collect()
    }

    #[cfg(test)]
    pub(super) fn put_raw(service: &str, raw: &[(bool, String, String)]) {
        for ([_, set, state], (on, host, port)) in KINDS.iter().zip(raw) {
            if !host.is_empty() {
                networksetup(&[set, service, host, port]).unwrap();
            }
            networksetup(&[state, service, if *on { "on" } else { "off" }]).unwrap();
        }
    }

    /// (http, https, socks) enabled on `service`.
    #[cfg(test)]
    pub(super) fn states(service: &str) -> [bool; 3] {
        ["-getwebproxy", "-getsecurewebproxy", "-getsocksfirewallproxy"]
            .map(|flag| networksetup(&[flag, service]).unwrap_or_default().lines().any(|l| l.trim() == "Enabled: Yes"))
    }

    /// (enabled, "host:port") of the HTTP proxy in effect (the primary
    /// service's, as `scutil --proxy` shows it). Read in process: the UI
    /// asks every few seconds, and two networksetup runs each time looked
    /// at the first service in the list, not necessarily the one in use.
    pub fn get() -> (bool, String) {
        use system_configuration::core_foundation::number::CFNumber;
        use system_configuration::core_foundation::string::CFString;
        use system_configuration::dynamic_store::SCDynamicStoreBuilder;

        let Some(p) = SCDynamicStoreBuilder::new("prism").build().and_then(|s| s.get_proxies()) else { return (false, String::new()) };
        let item = |k: &'static str| p.find(CFString::from_static_string(k)).map(|v| v.clone());
        let num = |k| item(k).and_then(|v| v.downcast::<CFNumber>()).and_then(|n| n.to_i64());
        let host = item("HTTPProxy").and_then(|v| v.downcast::<CFString>()).map(|s| s.to_string()).unwrap_or_default();
        (num("HTTPEnable") == Some(1), format!("{host}:{}", num("HTTPPort").unwrap_or(0)))
    }
}

#[cfg(target_os = "macos")]
pub use mac::{get, services, set, set_backup_dir};

#[cfg(not(any(windows, target_os = "macos")))]
pub fn set(_: bool, _: &str, _: &str) -> anyhow::Result<()> {
    anyhow::bail!("system proxy is not implemented on this platform")
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn get() -> (bool, String) {
    (false, String::new())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn bypass_list_for_macos() {
        assert_eq!(super::mac::bypass_list("localhost;127.*;10.*;<local>"), ["localhost", "127.*", "10.*", "*.local", "127.0.0.1"]);
    }

    /// On the real network services, with one that fails in between: turns
    /// the system proxy on and off again. Changes this Mac's settings for a
    /// moment and leaves every proxy off, so only with PRISM_SYSPROXY_LIVE=1
    /// and the address / bypass list the services already have, e.g.
    /// `PRISM_SYSPROXY_LIVE=1 PRISM_SYSPROXY_ADDR=127.0.0.1:7890 cargo test -p prism sysproxy -- --ignored`.
    #[test]
    #[ignore]
    fn live_round_trip_on_this_mac() {
        if std::env::var_os("PRISM_SYSPROXY_LIVE").is_none() {
            return;
        }
        let addr = std::env::var("PRISM_SYSPROXY_ADDR").unwrap_or_else(|_| "127.0.0.1:7890".into());
        let bypass = std::env::var("PRISM_SYSPROXY_BYPASS").unwrap_or_default();
        let real = super::mac::services();
        assert!(!real.is_empty());
        // Another tool's proxy on one service before Prism (an inactive one
        // is best: PRISM_SYSPROXY_OTHER, e.g. "iPhone USB").
        let other = std::env::var("PRISM_SYSPROXY_OTHER").unwrap_or_else(|_| real.last().unwrap().clone());
        let original = (super::mac::read_service(&other, ("", "")).unwrap(), super::mac::raw(&other));
        let theirs = super::mac::Saved { web: Some(("127.0.0.2".into(), "3128".into())), secure: None, socks: None, bypass: vec!["example.com".into()] };
        super::mac::write_service(&other, &theirs).unwrap();
        // And one left on Prism's own address by a crash (PRISM_SYSPROXY_CRASHED):
        // that one must come back off, not pointing at a stopped core.
        let ours = addr.rsplit_once(':').unwrap();
        let crashed = std::env::var("PRISM_SYSPROXY_CRASHED").ok();
        let crashed_original = crashed.as_ref().map(|c| (super::mac::read_service(c, ("", "")).unwrap(), super::mac::raw(c)));
        if let Some(c) = &crashed {
            let prism = Some((ours.0.to_string(), ours.1.to_string()));
            super::mac::write_service(c, &super::mac::Saved { web: prism.clone(), secure: prism.clone(), socks: prism, bypass: vec![] }).unwrap();
        }
        // Read with Prism's address counted as off: what should come back.
        let before: Vec<_> = real.iter().map(|s| super::mac::read_service(s, ours).unwrap()).collect();

        let dir = std::env::temp_dir().join(format!("prism-sysproxy-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut svcs = real.clone();
        svcs.insert(svcs.len() / 2, "Prism No Such Service".into());
        let t = std::time::Instant::now();
        let on = super::mac::set_on(&svcs, true, &addr, &bypass, Some(&dir)).unwrap_err().to_string();
        let on_time = t.elapsed();
        let on_states: Vec<_> = real.iter().map(|s| (s.clone(), super::mac::states(s))).collect();
        let in_effect = super::mac::get();
        let t = std::time::Instant::now();
        let off = super::mac::set_on(&svcs, false, &addr, &bypass, Some(&dir)).unwrap_err().to_string();
        let off_time = t.elapsed();
        let after: Vec<_> = real.iter().map(|s| super::mac::read_service(s, ("", "")).unwrap()).collect();
        let backup_left = dir.join(super::mac::BACKUP_FILE).exists();
        let crashed_after = crashed.as_ref().map(|c| super::mac::states(c));
        // This Mac as it was before the test.
        super::mac::write_service(&other, &original.0).unwrap();
        super::mac::put_raw(&other, &original.1);
        if let (Some(c), Some((saved, raw))) = (&crashed, &crashed_original) {
            super::mac::write_service(c, saved).unwrap();
            super::mac::put_raw(c, raw);
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert!(on.contains("Prism No Such Service") && !real.iter().any(|s| on.contains(&format!(" {s} "))), "{on}");
        assert!(off.contains("Prism No Such Service"), "{off}");
        assert!(on_states.iter().all(|(_, st)| *st == [true; 3]), "{on_states:?}");
        assert_eq!(in_effect, (true, addr.clone()));
        assert_eq!(after, before, "every service has its own proxies back");
        assert_eq!(after[real.iter().position(|s| *s == other).unwrap()].web, Some(("127.0.0.2".into(), "3128".into())));
        assert!(!backup_left, "the backup is gone once everything is back");
        if let Some(st) = crashed_after {
            assert_eq!(st, [false; 3], "a proxy on Prism's own address comes back off");
        }
        println!("{} services on ({on_time:?}) and back ({off_time:?}); {other}'s own proxy restored; only the missing one failed", real.len());
    }

    /// Reads what `scutil --proxy` shows.
    #[test]
    fn reads_the_proxy_in_effect() {
        let out = std::process::Command::new("/usr/sbin/scutil").arg("--proxy").output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let field = |k: &str| text.lines().find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim_start_matches([' ', ':']).trim().to_string()));
        let (on, addr) = super::mac::get();
        assert_eq!(on, field("HTTPEnable").as_deref() == Some("1"), "{text}");
        if on {
            assert_eq!(addr, format!("{}:{}", field("HTTPProxy").unwrap(), field("HTTPPort").unwrap()));
        }
    }

    /// Only services that don't exist, so no real setting is touched.
    #[test]
    fn one_failing_service_doesnt_stop_the_rest() {
        let svcs = ["Prism No Such A".to_string(), "Prism No Such B".to_string()];
        for enable in [false, true] {
            let err = super::mac::set_on(&svcs, enable, "127.0.0.1:7890", "", None).unwrap_err().to_string();
            assert!(err.contains("Prism No Such A") && err.contains("Prism No Such B"), "{err}");
        }
    }
}
