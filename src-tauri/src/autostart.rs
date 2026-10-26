//! Start at login: a value under HKCU\...\Run on Windows (the NSIS
//! uninstaller already deletes a Run value named after the product, so it
//! is "Prism"), a LaunchAgent on macOS.

/// Passed by the Run entry; such launches may stay in the tray.
pub const ARG: &str = "--autostart";

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const VALUE: &str = "Prism";

#[cfg(windows)]
fn command() -> anyhow::Result<String> {
    Ok(format!("\"{}\" {ARG}", std::env::current_exe()?.display()))
}

/// Whether Windows starts this executable at logon.
#[cfg(windows)]
pub fn enabled() -> bool {
    enabled_as(VALUE)
}

#[cfg(windows)]
pub fn set(enable: bool) -> anyhow::Result<()> {
    set_as(VALUE, enable)
}

#[cfg(windows)]
fn enabled_as(name: &str) -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    let cur: Option<String> = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN_KEY).and_then(|k| k.get_value(name)).ok();
    // An entry left by an install elsewhere doesn't start this copy.
    matches!((cur, command()), (Some(c), Ok(want)) if c.eq_ignore_ascii_case(&want))
}

#[cfg(windows)]
fn set_as(name: &str, enable: bool) -> anyhow::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    let (key, _) = winreg::RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    if enable {
        key.set_value(name, &command()?)?;
    } else if key.get_value::<String, _>(name).is_ok() {
        key.delete_value(name)?;
    }
    Ok(())
}

/// macOS: a LaunchAgent in ~/Library/LaunchAgents that runs this
/// executable at login.
#[cfg(target_os = "macos")]
mod mac {
    use std::path::PathBuf;

    use anyhow::{Context, Result};

    use super::ARG;

    const LABEL: &str = "dev.prism.client";

    fn plist_path() -> Result<PathBuf> {
        let home = std::env::var_os("HOME").context("HOME not set")?;
        Ok(PathBuf::from(home).join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    pub fn plist(exe: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{}</string><string>{ARG}</string></array>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#,
            xml_escape(exe)
        )
    }

    /// Whether the agent starts this executable (not a copy elsewhere).
    pub fn enabled() -> bool {
        let (Ok(p), Ok(exe)) = (plist_path(), std::env::current_exe()) else { return false };
        std::fs::read_to_string(p).is_ok_and(|s| s.contains(&format!("<string>{}</string>", xml_escape(&exe.to_string_lossy()))))
    }

    pub fn set(enable: bool) -> Result<()> {
        let p = plist_path()?;
        if enable {
            std::fs::create_dir_all(p.parent().unwrap())?;
            std::fs::write(&p, plist(&std::env::current_exe()?.to_string_lossy()))?;
        } else if p.exists() {
            std::fs::remove_file(&p)?;
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub use mac::{enabled, set};

#[cfg(not(any(windows, target_os = "macos")))]
pub fn enabled() -> bool {
    false
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn set(_enable: bool) -> anyhow::Result<()> {
    anyhow::bail!("开机自启仅支持 Windows 和 macOS")
}

#[cfg(all(test, target_os = "macos"))]
mod mac_tests {
    #[test]
    fn launch_agent_plist() {
        let p = super::mac::plist("/Applications/Prism & Co.app/Contents/MacOS/prism");
        assert!(p.contains("<string>/Applications/Prism &amp; Co.app/Contents/MacOS/prism</string><string>--autostart</string>"));
        assert!(p.contains("<key>RunAtLoad</key><true/>"));
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn run_entry_round_trip() {
        let name = format!("PrismTest-{}", std::process::id());
        assert!(!enabled_as(&name));
        set_as(&name, true).unwrap();
        let on = enabled_as(&name);
        let value: String = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
            .open_subkey(RUN_KEY)
            .unwrap()
            .get_value(&name)
            .unwrap();
        set_as(&name, false).unwrap();
        set_as(&name, false).unwrap(); // already gone: no error
        assert!(on);
        assert!(value.starts_with('"') && value.ends_with(" --autostart"), "{value}");
        assert!(!enabled_as(&name));
    }
}
