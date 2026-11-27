//! UI integration with the locally installed official Tailscale client.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::{path::PathBuf, time::Duration};
use tokio::process::Command;

#[derive(Clone, Default, Serialize)]
pub struct Peer {
    pub name: String,
    pub dns_name: String,
    pub ips: Vec<String>,
    pub online: bool,
    pub relay: String,
    pub direct: bool,
}
#[derive(Default, Serialize)]
pub struct Status {
    pub protection: Option<prism_service::TailscaleProtection>,
    pub installed: bool,
    pub state: String,
    pub version: String,
    pub tailnet: String,
    pub self_node: Option<Peer>,
    pub peers: Vec<Peer>,
    pub auth_url: Option<String>,
    pub error: Option<String>,
}
fn executable() -> Option<PathBuf> {
    let mut paths = Vec::new();
    if cfg!(windows) {
        if let Some(p) = std::env::var_os("ProgramFiles") {
            paths.push(PathBuf::from(p).join("Tailscale/tailscale.exe"));
        }
    } else {
        paths.extend(
            [
                "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
                "/usr/local/bin/tailscale",
                "/opt/homebrew/bin/tailscale",
            ]
            .into_iter()
            .map(PathBuf::from),
        );
    }
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path).map(|p| {
            p.join(if cfg!(windows) {
                "tailscale.exe"
            } else {
                "tailscale"
            })
        }));
    }
    paths.into_iter().find(|p| p.is_file())
}
async fn command(args: &[&str], seconds: u64) -> Result<std::process::Output> {
    let path = executable().context("未安装 Tailscale，请先从官方网站安装客户端")?;
    let mut cmd = Command::new(path);
    cmd.args(args).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    Ok(
        tokio::time::timeout(Duration::from_secs(seconds), cmd.output())
            .await
            .context("Tailscale 命令超时，请查看官方客户端状态")??,
    )
}
fn peer(v: &Value) -> Peer {
    Peer {
        name: v["HostName"].as_str().unwrap_or_default().into(),
        dns_name: v["DNSName"]
            .as_str()
            .unwrap_or_default()
            .trim_end_matches('.')
            .into(),
        ips: v["TailscaleIPs"]
            .as_array()
            .map(|v| {
                v.iter()
                    .filter_map(|p| p.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        online: v["Online"].as_bool().unwrap_or(false),
        relay: v["Relay"].as_str().unwrap_or_default().into(),
        direct: v["CurAddr"].as_str().is_some_and(|a| !a.is_empty()),
    }
}
fn auth_url(url: &str) -> Option<String> {
    let u = reqwest::Url::parse(url).ok()?;
    let host = u.host_str()?;
    (u.scheme() == "https"
        && (host == "tailscale.com" || host.ends_with(".tailscale.com"))
        && u.username().is_empty()
        && u.password().is_none())
    .then(|| u.to_string())
}
fn parse(v: Value) -> Status {
    let mut peers: Vec<_> = v["Peer"]
        .as_object()
        .map(|p| p.values().map(peer).collect())
        .unwrap_or_default();
    peers.sort_by(|a, b| b.online.cmp(&a.online).then(a.name.cmp(&b.name)));
    Status {
        installed: true,
        state: v["BackendState"].as_str().unwrap_or("Unknown").into(),
        version: v["Version"].as_str().unwrap_or_default().into(),
        tailnet: v["CurrentTailnet"]["Name"]
            .as_str()
            .unwrap_or_default()
            .into(),
        self_node: v.get("Self").filter(|v| v.is_object()).map(peer),
        peers,
        auth_url: v["AuthURL"].as_str().and_then(auth_url),
        error: None,
        protection: None,
    }
}
pub async fn status() -> Status {
    if executable().is_none() {
        return Status {
            state: "NotInstalled".into(),
            ..Default::default()
        };
    }
    match command(&["status", "--json"], 6).await {
        Ok(out) => match serde_json::from_slice(&out.stdout) {
            Ok(v) => parse(v),
            Err(_) => Status {
                installed: true,
                state: "Unavailable".into(),
                error: Some(
                    String::from_utf8_lossy(&out.stderr)
                        .chars()
                        .take(600)
                        .collect(),
                ),
                ..Default::default()
            },
        },
        Err(e) => Status {
            installed: true,
            state: "Unavailable".into(),
            error: Some(e.to_string()),
            ..Default::default()
        },
    }
}
pub async fn action(action: &str) -> Result<Status> {
    let args: &[&str] = match action {
        "connect" => &["up", "--json", "--timeout=15s"],
        "disconnect" => &["down"],
        _ => bail!("不支持的 Tailscale 操作"),
    };
    let output = command(args, 20).await?;
    let status = status().await;
    if !output.status.success() && status.auth_url.is_none() {
        bail!(
            "Tailscale 操作失败：{}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(1000)
                .collect::<String>()
        );
    }
    Ok(status)
}
pub async fn ping(ip: &str) -> Result<String> {
    let ip_addr: std::net::IpAddr = ip.parse()?;
    let status = status().await;
    if !status.peers.iter().chain(status.self_node.iter()).any(|p| {
        p.ips
            .iter()
            .any(|v| v.parse::<std::net::IpAddr>().ok() == Some(ip_addr))
    }) {
        bail!("只能测试当前组网中的设备 IP");
    }
    let output = command(
        &["ping", "--c=1", "--timeout=3s", "--until-direct=false", ip],
        8,
    )
    .await?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .chars()
        .take(1000)
        .collect())
}
pub async fn open(which: &str) -> Result<()> {
    let url = match which {
        "download" => "https://tailscale.com/download".to_string(),
        "admin" => "https://login.tailscale.com/admin/machines".to_string(),
        "login" => status()
            .await
            .auth_url
            .context("没有待处理的登录链接，请先点击连接或使用官方客户端登录")?,
        _ => bail!("不支持的 Tailscale 页面"),
    };
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        let op: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
        let target: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                op.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        if result as isize <= 32 {
            bail!("无法打开浏览器");
        }
    }
    #[cfg(not(windows))]
    {
        Command::new("/usr/bin/open").arg(url).spawn()?;
    }
    Ok(())
}

pub fn apply(doc: &mut serde_yaml::Value) -> Result<()> {
    use prism_core::config::{self, clash, UserRule};
    doc["prism-tailscale-bypass"] = true.into();
    let rules = [
        ("IP-CIDR", "100.64.0.0/10"),
        ("IP-CIDR", "fd7a:115c:a1e0::/48"),
        ("DOMAIN-SUFFIX", "ts.net"),
    ];
    let rules: Vec<_> = rules
        .into_iter()
        .map(|(k, v)| UserRule {
            kind: k.into(),
            value: v.into(),
            target: "DIRECT".into(),
            resolve: false,
            strict: true,
        })
        .collect();
    let _ = clash::add_user_rules(doc, &rules);
    if doc["dns"].is_mapping() {
        let dns = doc["dns"].as_mapping_mut().unwrap();
        let policy = dns
            .entry("nameserver-policy".into())
            .or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
        policy["+.ts.net"] = "100.100.100.100".into();
        let filters = dns
            .entry("fake-ip-filter".into())
            .or_insert_with(|| serde_yaml::Value::Sequence(vec![]));
        if let Some(v) = filters.as_sequence_mut() {
            v.push("+.ts.net".into());
        }
    }
    let _ = config::parse_any(&serde_yaml::to_string(doc)?)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_parsing_and_auth_links_are_bounded() {
        let s = parse(
            serde_json::json!({"BackendState":"Running","Self":{"HostName":"self","TailscaleIPs":["100.64.1.1"]},"Peer":{"b":{"HostName":"offline","Online":false},"a":{"HostName":"online","Online":true}},"AuthURL":"https://evil.example/login"}),
        );
        assert_eq!(s.peers[0].name, "online");
        assert!(s.auth_url.is_none());
        assert!(auth_url("https://login.tailscale.com/a/example").is_some());
        assert!(auth_url("https://login.tailscale.com.evil.example/a").is_none());
    }
    #[test]
    fn overlay_rules_and_magic_dns_are_explicit() {
        let mut doc =
            serde_yaml::from_str("proxies: []\ndns: {enable: true}\nrules: ['MATCH,DIRECT']")
                .unwrap();
        apply(&mut doc).unwrap();
        assert_eq!(
            doc["dns"]["nameserver-policy"]["+.ts.net"].as_str(),
            Some("100.100.100.100")
        );
        assert_eq!(doc["prism-tailscale-bypass"].as_bool(), Some(true));
    }
}
