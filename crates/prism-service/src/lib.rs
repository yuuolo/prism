//! Prism TUN service: a Windows service (LocalSystem) that runs the proxy
//! core, so TUN mode works while the Prism window runs as a normal user.
//!
//! Security model:
//! - It only runs the cores copied into `%ProgramFiles%\Prism Service\cores`
//!   at install time (admin-only folder); a request names a core *kind*,
//!   never a path.
//! - Its pipe only admits the account that installed it, and only the
//!   Prism executable registered at install.
//! - Configs and data files arrive as contents; config paths the core
//!   would write to are forced into the service's own folder.
//!
//! This library holds the protocol, the client Prism uses and the config
//! sanitiser; `main.rs` is the service and its installer.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

pub const SERVICE_NAME: &str = "PrismService";
pub const PIPE: &str = r"\\.\pipe\prism-service";
/// Requests carry data files (geodata), so they can be large.
pub const MAX_MESSAGE: usize = 128 << 20;

/// `%ProgramFiles%\Prism Service`: the service and its cores (admin-only).
pub fn program_dir() -> PathBuf {
    let base = std::env::var_os("ProgramW6432").or_else(|| std::env::var_os("ProgramFiles")).unwrap_or_else(|| r"C:\Program Files".into());
    PathBuf::from(base).join("Prism Service")
}

/// `%ProgramData%\Prism Service`: settings and the core's working folder.
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into());
    PathBuf::from(base).join("Prism Service")
}

pub fn service_exe() -> PathBuf {
    program_dir().join("prism-service.exe")
}

/// Where the service keeps its files and listens. Tests use their own.
#[derive(Clone, Debug)]
pub struct Layout {
    /// Admin-only: `cores\`, `wintun.dll`.
    pub program: PathBuf,
    /// `service.json`, `service.log`, `run\` (the core's working folder).
    pub data: PathBuf,
    pub pipe: String,
    /// The executable the pipe's server process must be.
    pub service_exe: PathBuf,
}

impl Layout {
    pub fn system() -> Self {
        Layout { program: program_dir(), data: data_dir(), pipe: PIPE.into(), service_exe: service_exe() }
    }
    pub fn cores_dir(&self) -> PathBuf {
        self.program.join("cores")
    }
    pub fn wintun(&self) -> PathBuf {
        self.program.join("wintun.dll")
    }
    pub fn run_dir(&self) -> PathBuf {
        self.data.join("run")
    }
    pub fn owner_file(&self) -> PathBuf {
        self.data.join("service.json")
    }
    pub fn log_file(&self) -> PathBuf {
        self.data.join("service.log")
    }
}

/// Written at install: who may use the service.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Owner {
    pub user_sid: String,
    /// The Prism executable allowed to connect.
    pub gui_path: PathBuf,
}

/// Appends to the service log (kept under 1 MB).
pub fn log(layout: &Layout, msg: &str) {
    use std::io::Write;
    let p = layout.log_file();
    if p.metadata().map(|m| m.len() > 1 << 20).unwrap_or(false) {
        let _ = std::fs::remove_file(&p);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "{t} {msg}");
    }
}

/// Core kinds and their file names in the cores folder.
pub const CORES: [(&str, &str); 3] = [("prism", "prism-core.exe"), ("mihomo", "mihomo.exe"), ("singbox", "sing-box.exe")];

pub fn core_file(kind: &str) -> Option<&'static str> {
    CORES.iter().find(|(k, _)| *k == kind).map(|(_, f)| *f)
}

/// Config file name each core is started with.
pub fn config_file(kind: &str) -> &'static str {
    if kind == "singbox" {
        "config.json"
    } else {
        "config.yaml"
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Status,
    /// Starts (or restarts) the core. `files`: data files for its working
    /// folder, name -> base64 contents (see [`allowed_file`]).
    Start { core: String, config: String, files: BTreeMap<String, String> },
    Stop,
    /// Core output lines after sequence number `since`.
    Logs { since: u64 },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Status {
    pub running: bool,
    pub core: Option<String>,
    pub pid: Option<u32>,
    pub version: String,
    /// kind -> SHA-256 of the installed core.
    pub cores: BTreeMap<String, String>,
    /// SHA-256 of wintun.dll as installed.
    pub wintun: Option<String>,
    /// Data files in the working folder -> SHA-256.
    pub files: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    pub error: Option<String>,
    pub status: Option<Status>,
    pub lines: Vec<(u64, String)>,
}

impl Response {
    pub fn err(e: impl std::fmt::Display) -> Self {
        Response { ok: false, error: Some(format!("{e:#}")), ..Default::default() }
    }
}

/// Data files a client may place in the core's working folder.
pub fn allowed_file(name: &str) -> bool {
    const DATA: [&str; 5] = ["geosite.dat", "geoip.dat", "country.mmdb", "geoip.metadb", "asn.mmdb"];
    if DATA.contains(&name.to_ascii_lowercase().as_str()) {
        return true;
    }
    name.strip_prefix("rule-set/").is_some_and(|f| {
        f.len() > 4 && f.ends_with(".srs") && !f.starts_with('.') && f.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    })
}

pub fn sha256_file(p: &std::path::Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(format!("{:x}", h.finalize()))
}

/// A file name that stays inside its folder.
fn safe_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if c.is_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect();
    let n = n.trim_start_matches('.');
    if n.is_empty() {
        "provider".into()
    } else {
        n.to_string()
    }
}

/// Makes a config safe to run as SYSTEM: everything the core would read or
/// write by path stays inside its working folder.
pub fn sanitize_config(kind: &str, text: &str) -> Result<String> {
    match kind {
        "prism" | "mihomo" => {
            let mut doc: serde_yaml::Value = serde_yaml::from_str(text).context("config is not valid YAML")?;
            let map = doc.as_mapping_mut().context("config is not a YAML mapping")?;
            for k in ["external-ui", "external-ui-name", "external-ui-url"] {
                map.remove(k);
            }
            for (section, dir) in [("proxy-providers", "proxies"), ("rule-providers", "rules")] {
                let Some(providers) = map.get_mut(section).and_then(|v| v.as_mapping_mut()) else { continue };
                for (name, p) in providers.iter_mut() {
                    let Some(p) = p.as_mapping_mut() else { continue };
                    if p.get("path").is_some() || p.get("type").and_then(|t| t.as_str()) == Some("http") {
                        let file = format!("./providers/{dir}/{}", safe_name(name.as_str().unwrap_or("provider")));
                        if p.get("type").and_then(|t| t.as_str()) == Some("file") {
                            // A local file the core can't be pointed at; drop it.
                            p.insert("type".into(), "inline".into());
                            p.remove("path");
                            p.entry("payload".into()).or_insert(serde_yaml::Value::Sequence(vec![]));
                        } else {
                            p.insert("path".into(), file.into());
                        }
                    }
                }
            }
            Ok(serde_yaml::to_string(&doc)?)
        }
        "singbox" => {
            let mut doc: serde_json::Value = serde_json::from_str(text).context("config is not valid JSON")?;
            if let Some(log) = doc.get_mut("log").and_then(|l| l.as_object_mut()) {
                log.remove("output");
            }
            if let Some(exp) = doc.get_mut("experimental").and_then(|e| e.as_object_mut()) {
                if let Some(c) = exp.get_mut("cache_file").and_then(|c| c.as_object_mut()) {
                    c.insert("path".into(), "cache.db".into());
                }
                if let Some(api) = exp.get_mut("clash_api").and_then(|c| c.as_object_mut()) {
                    api.remove("external_ui");
                    api.remove("external_ui_download_url");
                }
            }
            if let Some(sets) = doc.pointer_mut("/route/rule_set").and_then(|r| r.as_array_mut()) {
                for rs in sets {
                    let Some(rs) = rs.as_object_mut() else { continue };
                    if rs.get("type").and_then(|t| t.as_str()) == Some("remote") {
                        continue;
                    }
                    let path = rs.get("path").and_then(|p| p.as_str()).unwrap_or_default();
                    let base = path.rsplit(['/', '\\']).next().unwrap_or_default();
                    let local = format!("rule-set/{base}");
                    if !allowed_file(&local) {
                        bail!("rule-set file {base:?} is not allowed");
                    }
                    rs.insert("path".into(), local.into());
                }
            }
            Ok(serde_json::to_string_pretty(&doc)?)
        }
        _ => bail!("unknown core {kind}"),
    }
}

#[cfg(windows)]
pub mod client;
#[cfg(windows)]
pub mod server;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_file_names() {
        for ok in ["geosite.dat", "GeoIP.dat", "country.mmdb", "rule-set/geosite-cn.srs", "rule-set/a_b-c.1.srs"] {
            assert!(allowed_file(ok), "{ok}");
        }
        for bad in ["config.yaml", "wintun.dll", "../geosite.dat", "rule-set/../x.srs", "rule-set/a/b.srs", "rule-set/.srs", "rule-set/x.exe", r"rule-set\x.srs", "C:/x/geosite.dat"] {
            assert!(!allowed_file(bad), "{bad}");
        }
    }

    #[test]
    fn clash_paths_stay_inside() {
        let y = r#"
mixed-port: 7899
external-ui: C:\Windows\System32
proxy-providers:
  "../../evil": { type: http, url: "http://x", path: 'C:\Windows\System32\drivers\etc\hosts' }
  local: { type: file, path: 'C:\Users\me\secret.yaml' }
rule-providers:
  r1: { type: http, behavior: domain, url: "http://x" }
proxies: []
"#;
        let out = sanitize_config("mihomo", y).unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert!(v.get("external-ui").is_none());
        assert_eq!(v["proxy-providers"]["../../evil"]["path"].as_str(), Some("./providers/proxies/_.._evil"));
        assert_eq!(v["proxy-providers"]["local"]["type"].as_str(), Some("inline"));
        assert!(v["proxy-providers"]["local"].get("path").is_none());
        assert_eq!(v["rule-providers"]["r1"]["path"].as_str(), Some("./providers/rules/r1"));
        assert_eq!(v["mixed-port"].as_u64(), Some(7899));
    }

    #[test]
    fn singbox_paths_stay_inside() {
        let j = r#"{
            "log": {"level": "info", "output": "C:\\Windows\\win.ini"},
            "experimental": {"cache_file": {"enabled": true, "path": "C:\\x.db"}, "clash_api": {"external_controller": "127.0.0.1:9097", "external_ui": "C:\\"}},
            "route": {"rule_set": [
                {"tag": "cn", "type": "local", "format": "binary", "path": "C:\\Users\\me\\AppData\\Roaming\\dev.prism.client\\runtime\\rule-set\\geosite-cn.srs"},
                {"tag": "r", "type": "remote", "url": "https://x/y.srs"}
            ]}
        }"#;
        let v: serde_json::Value = serde_json::from_str(&sanitize_config("singbox", j).unwrap()).unwrap();
        assert!(v["log"].get("output").is_none());
        assert_eq!(v["experimental"]["cache_file"]["path"], "cache.db");
        assert!(v["experimental"]["clash_api"].get("external_ui").is_none());
        assert_eq!(v["route"]["rule_set"][0]["path"], "rule-set/geosite-cn.srs");
        assert_eq!(v["route"]["rule_set"][1]["url"], "https://x/y.srs");

        let bad = r#"{"route": {"rule_set": [{"tag": "x", "type": "local", "path": "C:\\Windows\\System32\\config\\SAM"}]}}"#;
        assert!(sanitize_config("singbox", bad).is_err());
        assert!(sanitize_config("other", "{}").is_err());
    }
}
