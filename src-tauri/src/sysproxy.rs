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
    use anyhow::{bail, Context, Result};

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

    /// Turning off needs no address (Prism passes "" on exit).
    pub fn set(enable: bool, server: &str, bypass: &str) -> Result<()> {
        let svcs = services();
        if svcs.is_empty() {
            bail!("没有可用的网络服务");
        }
        let state = if enable { "on" } else { "off" };
        for s in &svcs {
            if enable {
                let (host, port) = server.rsplit_once(':').context("bad proxy address")?;
                networksetup(&["-setwebproxy", s, host, port])?;
                networksetup(&["-setsecurewebproxy", s, host, port])?;
                networksetup(&["-setsocksfirewallproxy", s, host, port])?;
                let mut args = vec!["-setproxybypassdomains".to_string(), s.clone()];
                args.extend(bypass_list(bypass));
                networksetup(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
            }
            networksetup(&["-setwebproxystate", s, state])?;
            networksetup(&["-setsecurewebproxystate", s, state])?;
            networksetup(&["-setsocksfirewallproxystate", s, state])?;
        }
        Ok(())
    }

    /// (enabled, "host:port") of the first service's HTTP proxy.
    pub fn get() -> (bool, String) {
        let Some(s) = services().into_iter().next() else { return (false, String::new()) };
        let text = networksetup(&["-getwebproxy", &s]).unwrap_or_default();
        let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k).map(|v| v.trim().to_string())).unwrap_or_default();
        (field("Enabled:") == "Yes", format!("{}:{}", field("Server:"), field("Port:")))
    }
}

#[cfg(target_os = "macos")]
pub use mac::{get, set};

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
}
