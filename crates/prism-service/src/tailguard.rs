//! Independent service-side Tailscale attestation. No caller-supplied paths,
//! PIDs, interface identifiers or peer addresses are trusted.
use crate::client::{process_image, same_path};
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::{
    fs::File,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use windows_service::{
    service::{ServiceAccess, ServiceState},
    service_manager::{ServiceManager, ServiceManagerAccess},
};
use windows_sys::Win32::{NetworkManagement::IpHelper::*, Networking::WinSock::*};

#[derive(Default)]
pub struct Control {
    pub requested: bool,
    pub generation: u64,
    pub suspended: bool,
    pub lease_until: Option<std::time::Instant>,
    pub status: crate::TailscaleProtection,
}
impl Control {
    pub fn request(&mut self, on: bool) {
        if self.requested != on {
            self.suspended = false;
            self.lease_until = None;
        }
        if self.requested != on {
            self.generation = self.generation.wrapping_add(1);
        }
        self.requested = on;
    }
    pub fn connect(&mut self, on: bool, now: std::time::Instant) {
        self.generation = self.generation.wrapping_add(1);
        self.suspended = !on;
        self.lease_until = on.then(|| now + Duration::from_secs(60));
    }
    pub fn lease_left(&self, now: std::time::Instant) -> u64 {
        self.lease_until
            .map(|t| t.saturating_duration_since(now).as_secs())
            .unwrap_or(0)
    }
}

#[derive(Debug)]
pub struct Identity {
    pub pid: u32,
    pub daemon: PathBuf,
    pub cli: PathBuf,
    // Deny write/delete while this identity has live WFP permits.
    _daemon_file: File,
    _cli_file: File,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub index: u32,
    pub luid: u64,
    pub addresses: Vec<IpAddr>,
}
#[derive(Clone, Debug)]
pub struct Permit {
    pub identity: Arc<Identity>,
    pub interface: Option<Interface>,
    pub peers: Vec<IpAddr>,
}
impl Permit {
    pub fn key(&self) -> (u32, Option<Interface>, Vec<IpAddr>) {
        (
            self.identity.pid,
            self.interface.clone(),
            self.peers.clone(),
        )
    }
}
pub fn is_overlay(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let b = v.octets();
            b[0] == 100 && (64..=127).contains(&b[1])
        }
        IpAddr::V6(v) => v.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}
fn same_image(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => same_path(&a, &b),
        _ => false,
    }
}
fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().chain(Some(0)).collect()
}
fn service_pid_and_path() -> Result<(u32, PathBuf)> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(
        "Tailscale",
        ServiceAccess::QUERY_STATUS | ServiceAccess::QUERY_CONFIG,
    )?;
    let status = service.query_status()?;
    ensure!(
        status.current_state == ServiceState::Running,
        "Tailscale 服务未运行"
    );
    let pid = status.process_id.context("Tailscale 服务没有进程 ID")?;
    let config = service.query_config()?;
    ensure!(
        config.account_name.as_ref().is_some_and(|a| a
            .to_string_lossy()
            .eq_ignore_ascii_case("LocalSystem")
            || a.to_string_lossy()
                .eq_ignore_ascii_case("NT AUTHORITY\\SYSTEM")),
        "Tailscale 服务必须以 LocalSystem 运行"
    );
    let line = wide(config.executable_path.as_os_str());
    let mut count = 0;
    let args =
        unsafe { windows_sys::Win32::UI::Shell::CommandLineToArgvW(line.as_ptr(), &mut count) };
    ensure!(!args.is_null() && count > 0, "无效的 Tailscale 服务路径");
    let path = unsafe {
        let first = *args;
        let mut len = 0;
        while *first.add(len) != 0 {
            len += 1;
        }
        let p = PathBuf::from(String::from_utf16_lossy(std::slice::from_raw_parts(
            first, len,
        )));
        windows_sys::Win32::Foundation::LocalFree(args.cast());
        p
    };
    Ok((pid, path))
}
fn trusted_file(path: &Path) -> Result<File> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Security::{Cryptography::*, WinTrust::*};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)?;
    let name = wide(path.as_os_str());
    unsafe {
        let mut info: WINTRUST_FILE_INFO = std::mem::zeroed();
        info.cbStruct = std::mem::size_of_val(&info) as u32;
        info.pcwszFilePath = name.as_ptr();
        info.hFile = file.as_raw_handle();
        let mut data: WINTRUST_DATA = std::mem::zeroed();
        data.cbStruct = std::mem::size_of_val(&data) as u32;
        data.dwUIChoice = WTD_UI_NONE;
        data.dwUnionChoice = WTD_CHOICE_FILE;
        data.Anonymous.pFile = &mut info;
        data.dwStateAction = WTD_STATEACTION_VERIFY;
        data.dwProvFlags = WTD_CACHE_ONLY_URL_RETRIEVAL | WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT;
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let result = WinVerifyTrust(
            std::ptr::null_mut(),
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast(),
        );
        let signer_ok = if result == 0 {
            let provider = WTHelperProvDataFromStateData(data.hWVTStateData);
            let signer = if provider.is_null() {
                std::ptr::null_mut()
            } else {
                WTHelperGetProvSignerFromChain(provider, 0, 0, 0)
            };
            if signer.is_null() || (*signer).csCertChain == 0 {
                false
            } else {
                let cert = (*(*signer).pasCertChain).pCert;
                let mut subject = vec![0u16; 512];
                let n = CertGetNameStringW(
                    cert,
                    CERT_NAME_ATTR_TYPE,
                    0,
                    szOID_ORGANIZATION_NAME as *mut _,
                    subject.as_mut_ptr(),
                    subject.len() as u32,
                );
                n > 1
                    && String::from_utf16_lossy(&subject[..n.saturating_sub(1) as usize])
                        == "Tailscale Inc."
            }
        } else {
            false
        };
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        let _ = WinVerifyTrust(
            std::ptr::null_mut(),
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast(),
        );
        ensure!(
            signer_ok,
            "Tailscale 文件签名或发行者验证失败（0x{:08x}）",
            result as u32
        );
    }
    Ok(file)
}
pub fn identity(cached: Option<Arc<Identity>>) -> Result<Arc<Identity>> {
    let (pid, service_path) = service_pid_and_path()?;
    if let Some(old) = cached {
        if old.pid == pid
            && process_image(pid).is_some_and(|p| same_image(&p, &old.daemon))
            && same_image(&service_path, &old.daemon)
        {
            return Ok(old);
        }
    }
    let root = std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .context("Program Files 未找到")?;
    let root = PathBuf::from(root).canonicalize()?;
    let expected = root.join("Tailscale/tailscaled.exe");
    let daemon = service_path.canonicalize()?;
    ensure!(
        same_path(&daemon, &expected),
        "Tailscale 服务必须来自受保护的官方安装目录"
    );
    ensure!(
        process_image(pid).is_some_and(|p| same_image(&p, &daemon)),
        "Tailscale 服务进程路径不匹配"
    );
    let cli = root.join("Tailscale/tailscale.exe");
    ensure!(
        same_path(&cli.canonicalize()?, &cli),
        "Tailscale CLI 必须来自受保护的官方安装目录"
    );
    let daemon_file = trusted_file(&daemon)?;
    let cli_file = trusted_file(&cli)?;
    Ok(Arc::new(Identity {
        pid,
        daemon,
        cli,
        _daemon_file: daemon_file,
        _cli_file: cli_file,
    }))
}
unsafe fn sockaddr(addr: &SOCKADDR_INET) -> Option<IpAddr> {
    match addr.si_family {
        AF_INET => {
            Some(std::net::Ipv4Addr::from(addr.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes()).into())
        }
        AF_INET6 => Some(std::net::Ipv6Addr::from(addr.Ipv6.sin6_addr.u.Byte).into()),
        _ => None,
    }
}
fn text(v: &[u16]) -> String {
    String::from_utf16_lossy(&v[..v.iter().position(|n| *n == 0).unwrap_or(v.len())])
}
pub fn interface(self_ips: &[IpAddr]) -> Result<Interface> {
    ensure!(!self_ips.is_empty(), "Tailscale 缺少本机组网地址");
    unsafe {
        let mut table = std::ptr::null_mut();
        ensure!(GetIfTable2(&mut table) == 0, "读取网卡表失败");
        let rows =
            std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        let candidates: Vec<_> = rows
            .iter()
            .filter(|r| {
                r.OperStatus == 1
                    && r.Type == 53
                    && text(&r.Alias).eq_ignore_ascii_case("Tailscale")
                    && text(&r.Description).starts_with("Tailscale")
            })
            .map(|r| (r.InterfaceIndex, r.InterfaceLuid.Value))
            .collect();
        FreeMibTable(table.cast());
        let mut addresses = std::ptr::null_mut();
        ensure!(
            GetUnicastIpAddressTable(AF_UNSPEC, &mut addresses) == 0,
            "读取接口地址失败"
        );
        let rows = std::slice::from_raw_parts(
            (*addresses).Table.as_ptr(),
            (*addresses).NumEntries as usize,
        );
        let selected = candidates.into_iter().find_map(|(index, luid)| {
            let mut ips: Vec<_> = rows
                .iter()
                .filter(|r| {
                    r.InterfaceIndex == index
                        && r.InterfaceLuid.Value == luid
                        && r.ValidLifetime > 0
                })
                .filter_map(|r| sockaddr(&r.Address))
                .filter(|ip| is_overlay(*ip))
                .collect();
            ips.sort();
            ips.dedup();
            self_ips
                .iter()
                .all(|ip| ips.contains(ip))
                .then_some(Interface {
                    index,
                    luid,
                    addresses: ips,
                })
        });
        FreeMibTable(addresses.cast());
        selected.context("未找到与 Tailscale 本机地址一致的在线虚拟接口")
    }
}
pub async fn connected(id: Arc<Identity>) -> Result<Permit> {
    let mut cmd = tokio::process::Command::new(&id.cli);
    cmd.args(["status", "--json"])
        .creation_flags(0x0800_0000)
        .kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(2), cmd.output())
        .await
        .context("Tailscale 状态查询超时")??;
    let v: Value = serde_json::from_slice(&out.stdout)?;
    ensure!(
        v["BackendState"] == "Running" && v["Self"]["Online"].as_bool() == Some(true),
        "Tailscale 未连接"
    );
    let self_ips: Vec<_> = v["Self"]["TailscaleIPs"]
        .as_array()
        .context("本机地址缺失")?
        .iter()
        .filter_map(|s| s.as_str()?.parse::<IpAddr>().ok())
        .filter(|ip| is_overlay(*ip))
        .collect();
    let iface = interface(&self_ips)?;
    let mut peers = self_ips;
    if let Some(all) = v["Peer"].as_object() {
        for peer in all.values() {
            if let Some(ips) = peer["TailscaleIPs"].as_array() {
                peers.extend(
                    ips.iter()
                        .filter_map(|s| s.as_str()?.parse::<IpAddr>().ok())
                        .filter(|ip| is_overlay(*ip)),
                );
            }
        }
    }
    peers.push("100.100.100.100".parse().unwrap());
    peers.sort();
    peers.dedup();
    ensure!(peers.len() <= 4096, "Tailscale 地址数量超出保护规则上限");
    let (pid, _) = service_pid_and_path()?;
    ensure!(
        pid == id.pid && process_image(pid).is_some_and(|p| same_image(&p, &id.daemon)),
        "Tailscale 进程在验证期间已更换"
    );
    Ok(Permit {
        identity: id,
        interface: Some(iface),
        peers,
    })
}

#[cfg(test)]
pub(crate) fn fixture_identity(path: &Path) -> Arc<Identity> {
    Arc::new(Identity {
        pid: std::process::id(),
        daemon: path.into(),
        cli: path.into(),
        _daemon_file: File::open(path).unwrap(),
        _cli_file: File::open(path).unwrap(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_lease_is_bounded_and_disconnect_cancels_it() {
        let now = std::time::Instant::now();
        let mut c = Control::default();
        c.request(true);
        let initial = c.generation;
        c.connect(true, now);
        assert_ne!(initial, c.generation);
        assert_eq!(c.lease_left(now), 60);
        assert_eq!(c.lease_left(now + Duration::from_secs(61)), 0);
        c.connect(false, now);
        assert!(c.suspended);
        assert_eq!(c.lease_left(now), 0);
        c.request(false);
        assert!(!c.requested);
        assert!(c.lease_until.is_none());
    }
    #[test]
    fn installed_official_signature_verifies_without_network_changes() {
        let Some(root) = std::env::var_os("ProgramFiles") else {
            return;
        };
        let path = PathBuf::from(root).join("Tailscale/tailscaled.exe");
        if path.exists() {
            let _guard = trusted_file(&path)
                .expect("installed Tailscale must have a valid Tailscale Inc. signature");
        }
    }
    #[test]
    fn rejects_unsigned_or_wrong_publisher_without_install_changes() {
        assert!(trusted_file(&std::env::current_exe().unwrap()).is_err());
        if let Some(root)=std::env::var_os("SystemRoot") {
            let microsoft=PathBuf::from(root).join("System32/cmd.exe");
            if microsoft.exists() {assert!(trusted_file(&microsoft).is_err(),"another trusted publisher must not be accepted");}
        }
        assert!(!is_overlay("8.8.8.8".parse().unwrap()));
        assert!(is_overlay("100.100.100.100".parse().unwrap()));
    }
}
