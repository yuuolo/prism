//! Persistent settings and subscription profiles (JSON files in app data).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use prism_core::config::{self, Mode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoreKind {
    Prism,
    Mihomo,
    Singbox,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub core: CoreKind,
    pub mixed_port: u16,
    pub controller_port: u16,
    pub secret: String,
    pub mode: Mode,
    pub allow_lan: bool,
    pub log_level: String,
    pub system_proxy: bool,
    /// Look for a new Prism release on startup.
    pub auto_update_check: bool,
    pub tun: bool,
    /// Latency tests time a warm second request (mihomo unified-delay).
    pub unified_delay: bool,
    pub proxy_bypass: String,
    pub active_profile: Option<String>,
    pub mihomo_path: Option<String>,
    pub singbox_path: Option<String>,
    pub test_url: String,
    /// profile id -> group -> selected member, re-applied after (re)starts.
    pub selections: HashMap<String, HashMap<String, String>>,
    /// Per-application routing (应用分流), applied before the profile's rules.
    pub app_rules: Vec<AppRule>,
}

impl Default for Settings {
    fn default() -> Self {
        let secret: String = (0..16).map(|_| format!("{:x}", rand::random::<u8>() % 16)).collect();
        Settings {
            core: CoreKind::Prism,
            mixed_port: 7899,
            controller_port: 9097,
            secret,
            mode: Mode::Rule,
            allow_lan: false,
            log_level: "info".into(),
            system_proxy: false,
            auto_update_check: true,
            tun: false,
            unified_delay: true,
            proxy_bypass: "localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.2*;172.30.*;172.31.*;192.168.*;<local>".into(),
            active_profile: None,
            mihomo_path: None,
            singbox_path: None,
            test_url: config::DEFAULT_TEST_URL.into(),
            selections: HashMap::new(),
            app_rules: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppRule {
    pub id: String,
    pub enabled: bool,
    /// Match the full path instead of the file name.
    pub by_path: bool,
    /// File name (`chrome.exe`) or full path.
    pub value: String,
    /// Proxy / group name, DIRECT or REJECT.
    pub target: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Usage {
    pub upload: u64,
    pub download: u64,
    pub total: u64,
    pub expire: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileMeta {
    pub id: String,
    pub name: String,
    pub url: Option<String>,
    pub format: config::Format,
    pub nodes: usize,
    pub updated_at: i64,
    pub usage: Option<Usage>,
}

pub struct Store {
    pub dir: PathBuf,
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(p: &Path) -> T {
    std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write_json<T: Serialize>(p: &Path, v: &T) -> Result<()> {
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    std::fs::rename(tmp, p)?;
    Ok(())
}

impl Store {
    pub fn new(dir: PathBuf) -> Result<Self> {
        for d in ["profiles", "runtime", "runtime/rule-set", "cores"] {
            std::fs::create_dir_all(dir.join(d))?;
        }
        Ok(Store { dir })
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.dir.join("runtime")
    }

    /// sing-box rule-set files (.srs), downloaded by the app.
    pub fn rule_set_dir(&self) -> PathBuf {
        self.dir.join("runtime").join("rule-set")
    }

    pub fn cores_dir(&self) -> PathBuf {
        self.dir.join("cores")
    }

    pub fn settings(&self) -> Settings {
        let path = self.dir.join("settings.json");
        if !path.exists() {
            let s = Settings::default();
            let _ = write_json(&path, &s);
            return s;
        }
        read_json(&path)
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        write_json(&self.dir.join("settings.json"), s)
    }

    pub fn profiles(&self) -> Vec<ProfileMeta> {
        read_json(&self.dir.join("profiles.json"))
    }

    fn save_profiles(&self, list: &[ProfileMeta]) -> Result<()> {
        write_json(&self.dir.join("profiles.json"), &list)
    }

    fn content_path(&self, id: &str) -> PathBuf {
        self.dir.join("profiles").join(format!("{id}.txt"))
    }

    pub fn read_content(&self, id: &str) -> Result<String> {
        std::fs::read_to_string(self.content_path(id)).with_context(|| format!("profile {id} not found"))
    }

    /// Validates and stores profile content. `id` = None creates a new profile.
    pub fn put(&self, id: Option<&str>, name: &str, url: Option<String>, content: &str, usage: Option<Usage>) -> Result<ProfileMeta> {
        let parsed = config::parse_any(content).context("无法解析订阅内容")?;
        if parsed.proxies.is_empty() {
            bail!("订阅中没有可用节点");
        }
        let mut list = self.profiles();
        let meta = ProfileMeta {
            id: id.map(String::from).unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string()[..12].to_string()),
            name: name.to_string(),
            url,
            format: config::detect(content),
            nodes: parsed.proxies.len(),
            updated_at: chrono::Utc::now().timestamp(),
            usage,
        };
        std::fs::write(self.content_path(&meta.id), content)?;
        match list.iter_mut().find(|p| p.id == meta.id) {
            Some(p) => *p = meta.clone(),
            None => list.push(meta.clone()),
        }
        self.save_profiles(&list)?;
        Ok(meta)
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<()> {
        let mut list = self.profiles();
        if let Some(p) = list.iter_mut().find(|p| p.id == id) {
            p.name = name.to_string();
        }
        self.save_profiles(&list)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let mut list = self.profiles();
        list.retain(|p| p.id != id);
        let _ = std::fs::remove_file(self.content_path(id));
        self.save_profiles(&list)
    }
}

/// Parses the `subscription-userinfo` header: upload=1; download=2; total=3; expire=4
pub fn parse_userinfo(h: &str) -> Usage {
    let mut u = Usage::default();
    for part in h.split(';') {
        if let Some((k, v)) = part.trim().split_once('=') {
            let v = v.trim().parse::<f64>().unwrap_or(0.0) as u64;
            match k.trim() {
                "upload" => u.upload = v,
                "download" => u.download = v,
                "total" => u.total = v,
                "expire" => u.expire = v,
                _ => {}
            }
        }
    }
    u
}
