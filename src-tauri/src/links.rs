//! One-click import links (`prism://install-config?url=…&name=…`, and the
//! `clash://` form other clients and subscription panels use): parsing, and
//! registering the schemes under HKCU\Software\Classes.
//!
//! `prism://` is always ours. `clash://` usually belongs to another client,
//! so Prism only takes it over on request, keeping the previous command in
//! the key's `PrismPrevious` value to restore it later (the NSIS uninstall
//! hook in windows/hooks.nsh does the same).

use serde::Serialize;

pub const SCHEMES: [&str; 2] = ["prism", "clash"];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ImportRequest {
    /// Subscription URL (http/https).
    pub url: String,
    pub name: Option<String>,
}

/// The first argument that is one of our links.
pub fn find(args: impl IntoIterator<Item = String>) -> Option<String> {
    args.into_iter().find(|a| {
        let lower = a.to_ascii_lowercase();
        SCHEMES.iter().any(|s| lower.starts_with(&format!("{s}://")))
    })
}

/// Parses an `install-config` link. Errors never echo the link, which
/// carries the subscription token.
pub fn parse(link: &str) -> Result<ImportRequest, String> {
    let u = reqwest::Url::parse(link.trim()).map_err(|_| "无法识别的导入链接".to_string())?;
    if !SCHEMES.contains(&u.scheme()) {
        return Err("无法识别的导入链接".into());
    }
    if !u.host_str().is_some_and(|h| h.eq_ignore_ascii_case("install-config")) {
        return Err(format!("不支持的链接类型 {}://{}", u.scheme(), u.host_str().unwrap_or("")));
    }
    let (mut url, mut name) = (None, None);
    for (k, v) in u.query_pairs() {
        match k.as_ref() {
            "url" => url = Some(v.trim().to_string()),
            "name" => name = Some(v.trim().to_string()),
            _ => {}
        }
    }
    if name.is_none() {
        name = u.fragment().map(|f| percent_encoding::percent_decode_str(f).decode_utf8_lossy().trim().to_string());
    }
    let url = url.filter(|u| !u.is_empty()).ok_or("导入链接中没有订阅地址")?;
    match reqwest::Url::parse(&url) {
        Ok(sub) if matches!(sub.scheme(), "http" | "https") => {}
        _ => return Err("导入链接中的订阅地址不是 http(s) 链接".into()),
    }
    Ok(ImportRequest { url, name: name.filter(|n| !n.is_empty()) })
}

#[cfg(windows)]
mod reg {
    use anyhow::Result;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    const BACKUP: &str = "PrismPrevious";

    fn key_path(scheme: &str) -> String {
        format!(r"Software\Classes\{scheme}")
    }

    /// What Windows runs for our links: `"…\prism.exe" "%1"`, the same
    /// string the uninstall hook compares against.
    fn open_command() -> Result<String> {
        Ok(format!("\"{}\" \"%1\"", std::env::current_exe()?.display()))
    }

    fn command_in(root: &RegKey, scheme: &str) -> Option<String> {
        root.open_subkey(format!(r"{}\shell\open\command", key_path(scheme))).and_then(|k| k.get_value("")).ok()
    }

    fn hkcu() -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER)
    }

    /// Points the scheme's open command at us. `full` also sets the
    /// description and icon, which a borrowed key keeps as they were.
    fn write(scheme: &str, full: bool) -> Result<RegKey> {
        let (key, _) = hkcu().create_subkey(key_path(scheme))?;
        if full {
            key.set_value("", &format!("URL:{scheme} protocol"))?;
            let exe = std::env::current_exe()?;
            key.create_subkey("DefaultIcon")?.0.set_value("", &format!("\"{}\",0", exe.display()))?;
        }
        key.set_value("URL Protocol", &"")?;
        key.create_subkey(r"shell\open\command")?.0.set_value("", &open_command()?)?;
        Ok(key)
    }

    /// Whether links of `scheme` open this executable.
    pub fn owned(scheme: &str) -> bool {
        matches!((command_in(&hkcu(), scheme), open_command()), (Some(c), Ok(want)) if c.eq_ignore_ascii_case(&want))
    }

    /// Points `scheme` at this executable (no backup: for `prism://`).
    pub fn register(scheme: &str) -> Result<()> {
        if !owned(scheme) {
            write(scheme, true)?;
        }
        Ok(())
    }

    /// Takes `scheme` over, remembering the current per-user handler.
    pub fn take_over(scheme: &str) -> Result<()> {
        if owned(scheme) {
            return Ok(());
        }
        let created = hkcu().open_subkey(key_path(scheme)).is_err();
        // Empty = there was no per-user handler: delete the key on release.
        let prev = command_in(&hkcu(), scheme).unwrap_or_default();
        let key = write(scheme, created)?;
        key.set_value(BACKUP, &prev)?;
        Ok(())
    }

    /// Gives `scheme` back to the handler it had before [`take_over`]. Does
    /// nothing if another program has registered it since.
    pub fn release(scheme: &str) -> Result<()> {
        if !owned(scheme) {
            return Ok(());
        }
        let key = hkcu().open_subkey_with_flags(key_path(scheme), winreg::enums::KEY_ALL_ACCESS)?;
        let prev: String = key.get_value(BACKUP).unwrap_or_default();
        if prev.is_empty() {
            drop(key);
            hkcu().delete_subkey_all(key_path(scheme))?;
        } else {
            key.create_subkey(r"shell\open\command")?.0.set_value("", &prev)?;
            key.delete_value(BACKUP)?;
        }
        Ok(())
    }

    /// File name of the program that currently opens `scheme` links.
    pub fn handler(scheme: &str) -> Option<String> {
        let cmd = command_in(&hkcu(), scheme).or_else(|| command_in(&RegKey::predef(HKEY_LOCAL_MACHINE), scheme))?;
        let exe = match cmd.strip_prefix('"') {
            Some(rest) => rest.split('"').next()?,
            None => cmd.split_whitespace().next()?,
        };
        std::path::Path::new(exe).file_name().map(|n| n.to_string_lossy().into_owned())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn previous(scheme: &str) -> Option<String> {
            hkcu().open_subkey(key_path(scheme)).and_then(|k| k.get_value(BACKUP)).ok()
        }

        #[test]
        fn take_over_and_restore() {
            let pid = std::process::id();
            // A scheme another program already handles.
            let taken = format!("prism-test-a{pid}");
            let other = r#""C:\Other\other.exe" "%1""#;
            hkcu().create_subkey(format!(r"{}\shell\open\command", key_path(&taken))).unwrap().0.set_value("", &other).unwrap();
            assert!(!owned(&taken));
            assert_eq!(handler(&taken).as_deref(), Some("other.exe"));
            take_over(&taken).unwrap();
            take_over(&taken).unwrap(); // again: must not back up ourselves
            let owned_now = owned(&taken);
            let exe = handler(&taken);
            let backup = previous(&taken);
            release(&taken).unwrap();
            let restored = command_in(&hkcu(), &taken);
            let backup_after = previous(&taken);
            hkcu().delete_subkey_all(key_path(&taken)).unwrap();
            assert!(owned_now);
            assert_eq!(exe, std::env::current_exe().unwrap().file_name().map(|n| n.to_string_lossy().into_owned()));
            assert_eq!(backup.as_deref(), Some(other));
            assert_eq!(restored.as_deref(), Some(other));
            assert_eq!(backup_after, None);

            // A scheme nobody had: release removes the key again.
            let fresh = format!("prism-test-b{pid}");
            take_over(&fresh).unwrap();
            let owned_now = owned(&fresh);
            release(&fresh).unwrap();
            assert!(owned_now);
            assert!(hkcu().open_subkey(key_path(&fresh)).is_err());

            // Someone else re-registered after us: release leaves theirs alone.
            let lost = format!("prism-test-c{pid}");
            take_over(&lost).unwrap();
            hkcu().open_subkey_with_flags(format!(r"{}\shell\open\command", key_path(&lost)), winreg::enums::KEY_SET_VALUE)
                .unwrap()
                .set_value("", &other)
                .unwrap();
            release(&lost).unwrap();
            let kept = command_in(&hkcu(), &lost);
            hkcu().delete_subkey_all(key_path(&lost)).unwrap();
            assert_eq!(kept.as_deref(), Some(other));

            // register() is idempotent and needs no backup.
            let ours = format!("prism-test-d{pid}");
            register(&ours).unwrap();
            register(&ours).unwrap();
            let ok = owned(&ours) && previous(&ours).is_none();
            hkcu().delete_subkey_all(key_path(&ours)).unwrap();
            assert!(ok);
        }
    }
}

#[cfg(windows)]
pub use reg::{handler, owned, register, release, take_over};

#[cfg(not(windows))]
mod stub {
    pub fn owned(_: &str) -> bool {
        false
    }
    pub fn handler(_: &str) -> Option<String> {
        None
    }
    pub fn register(_: &str) -> anyhow::Result<()> {
        Ok(())
    }
    pub fn take_over(_: &str) -> anyhow::Result<()> {
        anyhow::bail!("仅支持 Windows")
    }
    pub fn release(_: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(not(windows))]
pub use stub::{handler, owned, register, release, take_over};

#[cfg(test)]
mod tests {
    use super::*;

    fn req(url: &str, name: Option<&str>) -> ImportRequest {
        ImportRequest { url: url.into(), name: name.map(String::from) }
    }

    #[test]
    fn parses_install_config_links() {
        // What sbm and most panels generate: the URL percent-encoded.
        assert_eq!(
            parse("clash://install-config?url=https%3A%2F%2Fsub.example.com%2Fapi%2Fsub%3Ftoken%3Dabc%26flag%3Dmeta&name=%E6%88%91%E7%9A%84%E8%AE%A2%E9%98%85"),
            Ok(req("https://sub.example.com/api/sub?token=abc&flag=meta", Some("我的订阅")))
        );
        assert_eq!(parse("prism://install-config?url=http://1.2.3.4:8080/s/x"), Ok(req("http://1.2.3.4:8080/s/x", None)));
        // Trailing slash, upper case, name in the fragment.
        assert_eq!(
            parse("CLASH://Install-Config/?url=https%3A%2F%2Fa.b%2Fc#%E6%9C%BA%E5%9C%BA"),
            Ok(req("https://a.b/c", Some("机场")))
        );
        assert_eq!(parse("clash://install-config?url=https%3A%2F%2Fa.b%2Fc&name="), Ok(req("https://a.b/c", None)));
    }

    #[test]
    fn rejects_bad_links() {
        assert!(parse("clash://install-config?name=x").is_err());
        assert!(parse("clash://install-config?url=").is_err());
        assert!(parse("clash://install-config?url=file%3A%2F%2F%2FC%3A%2Fx.yaml").is_err());
        assert!(parse("clash://install-config?url=not%20a%20url").is_err());
        assert!(parse("clash://open-page?url=https%3A%2F%2Fa.b").is_err());
        assert!(parse("https://install-config?url=https%3A%2F%2Fa.b").is_err());
        // Errors don't leak the token.
        let err = parse("clash://install-config?url=ftp%3A%2F%2Fa.b%2F%3Ftoken%3Dsecret").unwrap_err();
        assert!(!err.contains("secret"), "{err}");
    }

    #[test]
    fn finds_link_arguments() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(find(args(&["--autostart"])), None);
        assert_eq!(find(args(&["--x", "clash://install-config?url=a"])).as_deref(), Some("clash://install-config?url=a"));
        assert_eq!(find(args(&["Prism://install-config"])).as_deref(), Some("Prism://install-config"));
        assert_eq!(find(args(&["clashx://install-config"])), None);
    }
}
