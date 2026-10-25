//! Backups of settings and profiles: a zip, kept locally or on a WebDAV
//! server (`<url>/prism/`). Machine-specific settings (core paths, system
//! proxy / TUN state) and the WebDAV login stay out of it.

use std::io::{Cursor, Read, Write};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::store::{Settings, Store};

/// WebDAV folder under the configured URL.
const DIR: &str = "prism";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WebDav {
    pub url: String,
    pub username: String,
    /// DPAPI-encrypted, base64 (see [`protect`]).
    pub password: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RemoteBackup {
    pub name: String,
    /// As the server reports it (RFC 1123).
    pub modified: String,
    pub size: u64,
}

/// Zip of profiles.json, profiles/*.txt and settings.json (minus what
/// belongs to this machine).
pub fn create(store: &Store, s: &Settings) -> Result<Vec<u8>> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut portable = s.clone();
    portable.webdav = WebDav::default();
    portable.mihomo_path = None;
    portable.singbox_path = None;
    zip.start_file("settings.json", opts)?;
    zip.write_all(&serde_json::to_vec_pretty(&portable)?)?;
    zip.start_file("profiles.json", opts)?;
    zip.write_all(&serde_json::to_vec_pretty(&store.profiles())?)?;
    for p in store.profiles() {
        if let Ok(content) = store.read_content(&p.id) {
            zip.start_file(format!("profiles/{}.txt", p.id), opts)?;
            zip.write_all(content.as_bytes())?;
        }
    }
    let mut extras = vec!["overrides.json".to_string()];
    extras.extend(store.profiles().iter().map(|p| format!("{}/{}.json", crate::patches::DIR, p.id)));
    for extra in extras {
        if let Ok(bytes) = std::fs::read(store.dir.join(&extra)) {
            zip.start_file(extra, opts)?;
            zip.write_all(&bytes)?;
        }
    }
    Ok(zip.finish()?.into_inner())
}

/// Unpacks a backup over the data folder. Returns the settings to use:
/// the backup's, with this machine's paths, live switches and login kept.
pub fn restore(store: &Store, current: &Settings, bytes: &[u8]) -> Result<Settings> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("不是有效的 Prism 备份文件")?;
    let mut files = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let name = f.name().to_string();
        let safe_id = |id: &str| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        let ok = matches!(name.as_str(), "settings.json" | "profiles.json" | "overrides.json")
            || name.strip_prefix("profiles/").and_then(|n| n.strip_suffix(".txt")).is_some_and(safe_id)
            || name.strip_prefix("overrides/").and_then(|n| n.strip_suffix(".json")).is_some_and(safe_id);
        if !ok {
            continue;
        }
        let mut data = Vec::new();
        f.read_to_end(&mut data)?;
        files.push((name, data));
    }
    let settings_bytes = files.iter().find(|(n, _)| n == "settings.json").map(|(_, d)| d.clone()).context("备份中没有 settings.json")?;
    let mut s: Settings = serde_json::from_slice(&settings_bytes).context("备份中的设置无法读取")?;
    if !files.iter().any(|(n, _)| n == "profiles.json") {
        bail!("备份中没有订阅列表");
    }
    s.webdav = current.webdav.clone();
    s.mihomo_path = current.mihomo_path.clone();
    s.singbox_path = current.singbox_path.clone();
    s.system_proxy = current.system_proxy;
    s.tun = current.tun;
    // Profiles not in the backup go away with their files.
    for p in store.profiles() {
        let _ = std::fs::remove_file(store.dir.join("profiles").join(format!("{}.txt", p.id)));
        crate::patches::delete(&store.dir, &p.id);
    }
    for (name, data) in files.iter().filter(|(n, _)| n != "settings.json") {
        let path = store.dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, data)?;
    }
    store.save_settings(&s)?;
    Ok(s)
}

/// `prism-<computer>-<YYYYmmdd-HHMMSS>.zip`
pub fn file_name() -> String {
    let host: String = std::env::var("COMPUTERNAME")
        .unwrap_or_else(|_| "pc".into())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    format!("prism-{host}-{}.zip", chrono::Local::now().format("%Y%m%d-%H%M%S"))
}

// ------------------------------------------------------------------ WebDAV

pub struct Dav {
    base: String,
    user: String,
    pass: String,
    http: reqwest::Client,
}

impl Dav {
    pub fn new(w: &WebDav) -> Result<Self> {
        let url = w.url.trim();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            bail!("请填写 WebDAV 地址（http:// 或 https:// 开头）");
        }
        let http = reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(120)).build()?;
        Ok(Dav { base: format!("{}/{DIR}/", url.trim_end_matches('/')), user: w.username.clone(), pass: unprotect(&w.password)?, http })
    }

    fn req(&self, method: &str, url: &str) -> reqwest::RequestBuilder {
        let m = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        self.http.request(m, url).basic_auth(&self.user, Some(&self.pass))
    }

    fn check(r: &reqwest::Response, what: &str) -> Result<()> {
        match r.status().as_u16() {
            200..=299 => Ok(()),
            401 | 403 => bail!("WebDAV {what}失败：用户名或密码不对（HTTP {}）", r.status().as_u16()),
            s => bail!("WebDAV {what}失败：HTTP {s}"),
        }
    }

    /// Makes sure `<url>/prism/` exists (and the login works).
    pub async fn ensure_dir(&self) -> Result<()> {
        let r = self.req("PROPFIND", &self.base).header("Depth", "0").send().await.context("无法连接 WebDAV 服务器")?;
        if r.status().as_u16() == 404 {
            let r = self.req("MKCOL", &self.base).send().await?;
            return Self::check(&r, "创建目录");
        }
        Self::check(&r, "连接")
    }

    pub async fn upload(&self, name: &str, data: Vec<u8>) -> Result<()> {
        self.ensure_dir().await?;
        let r = self.req("PUT", &format!("{}{name}", self.base)).body(data).send().await?;
        Self::check(&r, "上传")
    }

    pub async fn list(&self) -> Result<Vec<RemoteBackup>> {
        let r = self.req("PROPFIND", &self.base).header("Depth", "1").send().await.context("无法连接 WebDAV 服务器")?;
        if r.status().as_u16() == 404 {
            return Ok(Vec::new());
        }
        Self::check(&r, "读取列表")?;
        let mut list = parse_propfind(&r.text().await?);
        list.sort_by(|a, b| b.name.cmp(&a.name));
        Ok(list)
    }

    pub async fn download(&self, name: &str) -> Result<Vec<u8>> {
        check_name(name)?;
        let r = self.req("GET", &format!("{}{name}", self.base)).send().await?;
        Self::check(&r, "下载")?;
        Ok(r.bytes().await?.to_vec())
    }

    pub async fn delete(&self, name: &str) -> Result<()> {
        check_name(name)?;
        let r = self.req("DELETE", &format!("{}{name}", self.base)).send().await?;
        Self::check(&r, "删除")
    }
}

fn check_name(name: &str) -> Result<()> {
    if !name.ends_with(".zip") || name.contains(['/', '\\']) || name.starts_with('.') {
        bail!("无效的备份文件名 {name}");
    }
    Ok(())
}

/// Tags in `xml` as (local name, is closing, start of tag, end of tag).
fn tags(xml: &str) -> Vec<(String, bool, usize, usize)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(i) = xml[pos..].find('<') {
        let start = pos + i;
        let Some(len) = xml[start..].find('>') else { break };
        let end = start + len + 1;
        let inner = &xml[start + 1..end - 1];
        let closing = inner.starts_with('/');
        let name = inner.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or_default();
        let local = name.rsplit(':').next().unwrap_or_default().to_ascii_lowercase();
        if !local.is_empty() && !name.starts_with(['?', '!']) {
            out.push((local, closing, start, end));
        }
        pos = end;
    }
    out
}

/// Text inside each `<…:name …>…</…:name>` (any prefix or attributes).
fn elements<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let t = tags(xml);
    let mut out = Vec::new();
    for (i, (local, closing, _, end)) in t.iter().enumerate() {
        if local != name || *closing {
            continue;
        }
        if let Some((_, _, close_start, _)) = t[i + 1..].iter().find(|(l, c, _, _)| l == name && *c) {
            out.push(&xml[*end..*close_start]);
        }
    }
    out
}

/// Backups (`*.zip`) in a PROPFIND Depth:1 response.
pub fn parse_propfind(xml: &str) -> Vec<RemoteBackup> {
    let mut out = Vec::new();
    for block in elements(xml, "response") {
        let first = |n: &str| elements(block, n).first().map(|s| s.trim().to_string());
        let Some(href) = first("href") else { continue };
        let href = percent_encoding::percent_decode_str(&href).decode_utf8_lossy().into_owned();
        let name = href.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_string();
        if !name.ends_with(".zip") {
            continue;
        }
        out.push(RemoteBackup {
            name,
            modified: first("getlastmodified").unwrap_or_default(),
            size: first("getcontentlength").and_then(|s| s.parse().ok()).unwrap_or(0),
        });
    }
    out
}

// ------------------------------------------------------------------ DPAPI

/// Encrypts for the current Windows user (DPAPI), base64.
#[cfg(windows)]
pub fn protect(plain: &str) -> Result<String> {
    use base64::Engine;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};
    if plain.is_empty() {
        return Ok(String::new());
    }
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: plain.len() as u32, pbData: plain.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        if CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out) == 0 {
            bail!("CryptProtectData failed");
        }
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    }
}

#[cfg(windows)]
pub fn unprotect(enc: &str) -> Result<String> {
    use base64::Engine;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    if enc.is_empty() {
        return Ok(String::new());
    }
    let data = base64::engine::general_purpose::STANDARD.decode(enc).context("WebDAV 密码数据损坏，请重新填写")?;
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        if CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out) == 0 {
            bail!("无法解密 WebDAV 密码（可能来自另一台电脑或另一个用户），请重新填写");
        }
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        Ok(String::from_utf8(bytes)?)
    }
}

#[cfg(not(windows))]
pub fn protect(plain: &str) -> Result<String> {
    Ok(plain.to_string())
}

#[cfg(not(windows))]
pub fn unprotect(enc: &str) -> Result<String> {
    Ok(enc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_round_trip() {
        let enc = protect("p@ss 密码").unwrap();
        assert!(!enc.contains("p@ss"));
        assert_eq!(unprotect(&enc).unwrap(), "p@ss 密码");
        assert_eq!(unprotect("").unwrap(), "");
    }

    #[test]
    fn propfind_listing() {
        // Nextcloud-ish prefixes, a folder entry, encoded names.
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
 <d:response><d:href>/dav/prism/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
 <d:response><d:href>/dav/prism/prism-PC-20261025-120000.zip</d:href><d:propstat><d:prop>
   <d:getlastmodified>Sat, 25 Oct 2026 04:00:00 GMT</d:getlastmodified><d:getcontentlength>1234</d:getcontentlength></d:prop></d:propstat></d:response>
 <D:response xmlns:D="DAV:"><D:href>http://h/dav/prism/prism-%E5%AE%B6-20261024-080000.zip</D:href><D:propstat><D:prop><lp1:getcontentlength xmlns:lp1="DAV:">99</lp1:getcontentlength></D:prop></D:propstat></D:response>
 <d:response><d:href>/dav/prism/notes.txt</d:href></d:response>
</d:multistatus>"#;
        assert_eq!(
            parse_propfind(xml),
            vec![
                RemoteBackup { name: "prism-PC-20261025-120000.zip".into(), modified: "Sat, 25 Oct 2026 04:00:00 GMT".into(), size: 1234 },
                RemoteBackup { name: "prism-家-20261024-080000.zip".into(), modified: "".into(), size: 99 },
            ]
        );
    }

    #[test]
    fn backup_round_trip_keeps_machine_settings() {
        let dir = std::env::temp_dir().join(format!("prism-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone()).unwrap();
        let yaml = "proxies:\n  - {name: a, type: ss, server: 1.2.3.4, port: 1, cipher: aes-128-gcm, password: x}\n";
        let p = store.put(None, "机场", Some("https://sub.example/x".into()), yaml, None, Some(720)).unwrap();
        let mut s = store.settings();
        s.active_profile = Some(p.id.clone());
        s.mixed_port = 7000;
        s.mihomo_path = Some(r"C:\old\mihomo.exe".into());
        s.webdav = WebDav { url: "https://dav.example".into(), username: "u".into(), password: protect("secret").unwrap() };
        let zip = create(&store, &s).unwrap();
        let text = String::from_utf8_lossy(&zip).to_string();
        assert!(!text.contains("dav.example"), "login leaked into the backup");

        // Another machine: different paths, live switches, login; one extra profile.
        let other = store.put(None, "本地", None, yaml, None, None).unwrap();
        let mut here = store.settings();
        here.mixed_port = 1;
        here.mihomo_path = Some(r"D:\new\mihomo.exe".into());
        here.system_proxy = true;
        here.webdav = WebDav { url: "https://mine".into(), username: "me".into(), password: String::new() };
        let restored = restore(&store, &here, &zip).unwrap();
        assert_eq!(restored.mixed_port, 7000);
        assert_eq!(restored.active_profile, Some(p.id.clone()));
        assert_eq!(restored.mihomo_path.as_deref(), Some(r"D:\new\mihomo.exe"));
        assert!(restored.system_proxy);
        assert_eq!(restored.webdav.url, "https://mine");
        let ids: Vec<String> = store.profiles().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![p.id.clone()]);
        assert!(store.read_content(&other.id).is_err());
        assert_eq!(store.profiles()[0].interval, 720);
        assert!(restore(&store, &here, b"not a zip").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
