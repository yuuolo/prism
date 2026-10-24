//! Start with Windows: a value under HKCU\...\Run. The NSIS uninstaller
//! already deletes a Run value named after the product, so it is "Prism".

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

#[cfg(not(windows))]
pub fn enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn set(_enable: bool) -> anyhow::Result<()> {
    anyhow::bail!("开机自启仅支持 Windows")
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
