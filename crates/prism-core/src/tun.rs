//! TUN inbound: a virtual adapter (wintun on Windows, utun on macOS) feeding
//! a smoltcp userspace stack.
//!
//! IP packets from the adapter are fed into netstack-smoltcp, which hands back
//! TCP streams and UDP datagrams:
//!   * TCP: the first bytes are sniffed (TLS SNI / HTTP Host) so rules can
//!     match domains, then the connection goes through the normal tunnel.
//!   * TCP/UDP 53: queries are handled by the configured DNS resolver.
//!   * Other UDP follows the selected outbound through the UDP relay.
//! Windows also prevents ordinary apps sending port 53 outside the TUN.
//!
//! auto-route adds 0.0.0.0/1 + 128.0.0.0/1 on the adapter (and ::/1 +
//! 8000::/1 with `inet6-address`, so IPv6 can't bypass the rules); the
//! core's own sockets are pinned to the previous default interface (see
//! net.rs). The routes disappear together with the adapter when the process
//! exits. IPv6 setup is best-effort: if it fails, TUN runs IPv4-only.

#[cfg(not(any(windows, target_os = "macos")))]
pub async fn start(
    _: &crate::config::TunConfig,
    _: &std::path::Path,
    _: std::sync::Arc<crate::hub::Hub>,
) -> anyhow::Result<()> {
    anyhow::bail!("TUN mode is only implemented on Windows and macOS")
}

#[cfg(windows)]
pub use win::start;

#[cfg(target_os = "macos")]
pub use mac::start;

#[cfg(target_os = "android")]
#[path = "tun_android.rs"]
mod android;
#[cfg(target_os = "android")]
pub use android::attach as attach_android;
#[cfg(target_os = "android")]
pub use android::packet_counts as android_packet_counts;

/// The adapter's address. DNS on port 53 is always intercepted.
#[cfg(any(windows, target_os = "macos"))]
const ADDR: std::net::Ipv4Addr = std::net::Ipv4Addr::new(172, 19, 0, 1);
/// Peer address required by macOS point-to-point utun configuration.
#[cfg(target_os = "macos")]
const DNS: std::net::Ipv4Addr = std::net::Ipv4Addr::new(172, 19, 0, 2);
/// The adapter's IPv6 address (unique local, /126).
#[cfg(any(windows, target_os = "macos"))]
const ADDR6: &str = "fdfe:dcba:9876::1";
/// IPv6 split default routes.
#[cfg(any(windows, target_os = "macos"))]
const ROUTES6: [&str; 2] = ["::/1", "8000::/1"];
#[cfg(any(windows, target_os = "macos", target_os = "android"))]
const MTU: usize = if cfg!(target_os = "android") { 1500 } else { 9000 };

/// The platform-independent part: the userspace stack and what happens to
/// the connections it produces.
#[cfg(any(windows, target_os = "macos", target_os = "android"))]
mod stack {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::{anyhow, Result};
    use futures::{SinkExt, StreamExt};
    use netstack_smoltcp::{StackBuilder, UdpSocket as StackUdp};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::mpsc;

    use super::MTU;
    use crate::common::Address;
    use crate::config::TunConfig;
    use crate::hub::Hub;
    use crate::sniff;
    use crate::tunnel::{Metadata, Network, Tunnel, UdpNat, UdpPacket};

    /// Runs the stack over a device: `from_dev` delivers IP packets read
    /// from the adapter, `to_dev` takes the packets to write to it.
    pub fn run(_cfg: &TunConfig, hub: Arc<Hub>, mut from_dev: mpsc::Receiver<Vec<u8>>, to_dev: mpsc::Sender<Vec<u8>>) -> Result<()> {
        use crate::tun_health::Pump;
        let health = hub.tun_health.clone();
        let (stack, runner, udp, tcp) = StackBuilder::default()
            .enable_tcp(true)
            .enable_udp(true)
            .enable_icmp(true)
            .mtu(MTU)
            .build()
            .map_err(|e| anyhow!("netstack: {e}"))?;
        if let Some(r) = runner {
            let guard = health.guard(Pump::Runner);
            let runner_health = health.clone();
            tokio::spawn(async move { let _guard = guard; if let Err(error) = r.await { tracing::error!("TUN stack runner: {error}"); runner_health.fail(Pump::Runner, "stack_runner_error", None); } });
        }
        let (mut stack_sink, mut stack_stream) = stack.split();
        let input_health = health.clone();
        let input_guard = health.guard(Pump::ToStack);
        tokio::spawn(async move {
            let _guard = input_guard;
            while let Some(p) = from_dev.recv().await {
                if let Err(error) = stack_sink.send(p).await {
                    tracing::error!("TUN stack input: {error}");
                    input_health.fail(Pump::ToStack, "stack_input_error", None);
                    break;
                }
            }
        });
        let output_health = health.clone();
        let output_guard = health.guard(Pump::FromStack);
        tokio::spawn(async move {
            let _guard = output_guard;
            while let Some(result) = stack_stream.next().await {
                let p = match result {
                    Ok(p) => p,
                    Err(error) => { tracing::error!("TUN stack output: {error}"); output_health.fail(Pump::FromStack, "stack_output_error", None); break; }
                };
                if p.is_empty() || p.len() > u16::MAX as usize {
                    continue;
                }
                let queued = (to_dev.max_capacity() - to_dev.capacity() + 1).min(to_dev.max_capacity());
                let wait = std::time::Instant::now();
                output_health.queue(false, queued, std::time::Duration::ZERO);
                if to_dev.send(p).await.is_err() {
                    break;
                }
                output_health.queue(false, queued, wait.elapsed());
            }
        });

        if let Some(mut tcp) = tcp {
            let h = hub.clone();
            let guard = health.guard(Pump::Tcp);
            tokio::spawn(async move {
                let _guard = guard;
                while let Some((stream, src, dst)) = tcp.next().await {
                    tokio::spawn(handle_tcp(stream, src, dst, h.core().tunnel.clone()));
                }
            });
        }
        if let Some(udp) = udp {
            let guard = health.guard(Pump::Udp);
            tokio::spawn(async move { let _guard = guard; handle_udp(udp, hub).await; });
        }
        Ok(())
    }

    async fn handle_tcp(mut s: netstack_smoltcp::TcpStream, src: SocketAddr, dst: SocketAddr, tunnel: Arc<Tunnel>) {
        // Intercept DNS over TCP before sniffing or applying routing rules.
        if dst.port() == 53 {
            let _ = tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    let len = s.read_u16().await? as usize;
                    if len < 12 { return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "short DNS query")); }
                    let mut query = vec![0; len];
                    s.read_exact(&mut query).await?;
                    let Some(resolver) = tunnel.dns.clone() else {
                        tracing::warn!("DNS REJECT: TCP resolver unavailable; refusing direct upstream fallback");
                        return Ok::<(), std::io::Error>(());
                    };
                    let Some(answer) = resolver.handle(&query).await else { return Ok(()); };
                    let size = u16::try_from(answer.len()).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "DNS response too large"))?;
                    s.write_u16(size).await?;
                    s.write_all(&answer).await?;
                }
            }).await;
            return;
        }
        // Sniff: wait briefly for the client's first bytes (server-first
        // protocols simply time out and are routed by IP).
        let mut buf = vec![0u8; 16 * 1024];
        let mut n = 0;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
        loop {
            match tokio::time::timeout_at(deadline, s.read(&mut buf[n..])).await {
                Ok(Ok(0)) | Ok(Err(_)) => return,
                Ok(Ok(k)) => {
                    n += k;
                    let want = sniff::tls_record_len(&buf[..n]).unwrap_or(0);
                    if want <= n || n == buf.len() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let domain = sniff::domain(&buf[..n]);
        let target = match domain {
            Some(d) => Address::Domain(d, dst.port()),
            None => Address::Ip(dst),
        };
        let mut meta = Metadata::new(Network::Tcp, "Tun", src, target, Some(dst.ip()));
        if let Ok(d) = tunnel.dial(&mut meta).await {
            tunnel.relay(s, d, meta, &buf[..n]).await;
        }
    }

    async fn handle_udp(udp: StackUdp, hub: Arc<Hub>) {
        let (mut rd, mut wr) = udp.split();
        let (reply_tx, mut reply_rx) = mpsc::channel::<UdpPacket>(4096);
        tokio::spawn(async move {
            while let Some(p) = reply_rx.recv().await {
                let Address::Ip(from) = p.dst else { continue };
                if wr.send((p.data, from, p.src)).await.is_err() {
                    break;
                }
            }
        });
        let nat = UdpNat::new("Tun", reply_tx.clone());
        while let Some((data, src, dst)) = rd.next().await {
            if dst.port() == 53 {
                tokio::spawn(dns_query(hub.clone(), data, src, dst, reply_tx.clone()));
                continue;
            }
            if dst.ip().is_multicast() || dst.ip() == IpAddr::V4(Ipv4Addr::BROADCAST) {
                continue;
            }
            nat.send(&hub.core().tunnel, UdpPacket { src, dst: Address::Ip(dst), data });
        }
    }

    /// DNS hijack fails closed if the configured resolver is unavailable.
    async fn dns_query(hub: Arc<Hub>, query: Vec<u8>, src: SocketAddr, dst: SocketAddr, reply: mpsc::Sender<UdpPacket>) {
        let resp = match hub.core().dns.clone() {
            Some(r) => r.handle(&query).await,
            None => {
                tracing::warn!("DNS REJECT: resolver unavailable; refusing direct upstream fallback");
                None
            }
        };
        if let Some(data) = resp {
            // The reply appears to come from the address the client asked.
            let _ = reply.send(UdpPacket { src, dst: Address::Ip(dst), data }).await;
        }
    }
}

#[cfg(windows)]
mod win {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use anyhow::{anyhow, bail, Context, Result};
    use tokio::sync::mpsc;

    use super::{ADDR, ADDR6, MTU, ROUTES6};
    use crate::config::TunConfig;
    use crate::hub::Hub;
    use crate::net;

    const GUID: u128 = 0x5072_6973_6d54_554e_8000_0000_0000_0001;

    fn netsh(args: &[&str]) -> Result<()> {
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new("netsh")
            .args(args)
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output()
            .context("run netsh")?;
        if !out.status.success() {
            let msg = String::from_utf8_lossy(&out.stdout).trim().to_string();
            bail!("netsh {}: {msg}", args.join(" "));
        }
        Ok(())
    }

    /// IPv6 address and split default routes on the adapter.
    fn ipv6(index: u32, auto_route: bool) -> Result<()> {
        let iface = format!("interface={index}");
        netsh(&["interface", "ipv6", "add", "address", &iface, &format!("address={ADDR6}/126"), "store=active"])?;
        if auto_route {
            for prefix in ROUTES6 {
                netsh(&["interface", "ipv6", "add", "route", &format!("prefix={prefix}"), &iface, "metric=1", "store=active"])?;
            }
        }
        Ok(())
    }

    fn monitor_uplink(hub: &Arc<Hub>, index: u32, initial: crate::network::Uplink) {
        let weak = Arc::downgrade(hub);
        let changes = match crate::network_changes::Changes::register(index, initial.index) {
            Ok(changes) => Some(changes),
            Err(error) => { tracing::warn!("网络恢复: {error}; using periodic route scans"); None }
        };
        tokio::spawn(async move {
            let mut recovery = crate::network::Recovery::new(initial);
            let mut last_attempt: Option<tokio::time::Instant> = None;
            loop {
                // Measure only time waiting for a scan. A slow reload/cooldown
                // must not masquerade as another resume on the next iteration.
                let waiting = std::time::SystemTime::now();
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => {}
                    _ = async { match &changes { Some(c) => c.changed().await, None => std::future::pending::<()>().await } } => {
                        // Fixed coalescing window: an event storm cannot keep
                        // postponing the scan, nor cause a reload per callback.
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                }
                if let Some(last) = last_attempt { tokio::time::sleep_until(last + std::time::Duration::from_secs(3)).await; }
                let Some(hub) = weak.upgrade() else { break };
                let resumed = waiting.elapsed().unwrap_or_default().as_secs() > 15;
                let invalidated = changes.as_ref().is_some_and(|c| c.take_invalidation());
                let next = crate::network::uplink(index).ok();
                if !recovery.needed(next.as_ref(), resumed || invalidated) { continue; }
                let next = next.unwrap();
                last_attempt = Some(tokio::time::Instant::now());
                if let Some(changes) = &changes { changes.applied(next.index); }
                net::set_bind_interface(next.index);
                tracing::info!("网络恢复: uplink changed or resumed, rebinding to interface {}", next.index);
                // Rebuild protocol pools and DNS caches, keeping fake-IP mappings,
                // selector choices, mode, API, TUN device and firewall protection.
                let started = std::time::Instant::now();
                match tokio::time::timeout(std::time::Duration::from_secs(30), hub.repair()).await {
                    Ok(Ok(())) => {
                        hub.tun_health.network_repaired(true, started.elapsed());
                        hub.core().tunnel.stats.close_all();
                        recovery.applied(next);
                        tracing::info!("网络恢复: DNS and outbound sessions rebuilt successfully");
                    }
                    result => {
                        hub.tun_health.network_repaired(false, started.elapsed());
                        tracing::warn!("网络恢复: rebuild failed ({result:?}), retrying after cooldown");
                        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    }
                }
            }
        });
    }

    fn find_wintun(home: &Path) -> Result<PathBuf> {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
        [Some(home.join("wintun.dll")), exe_dir.map(|d| d.join("wintun.dll"))]
            .into_iter()
            .flatten()
            .find(|p| p.exists())
            .ok_or_else(|| anyhow!("wintun.dll not found (put it in {} or next to prism-core)", home.display()))
    }

    fn ready_addresses(index: u32) -> Result<(bool, bool)> {
        use windows_sys::Win32::{NetworkManagement::IpHelper::*, Networking::WinSock::*};
        let v6 = ADDR6.parse::<std::net::Ipv6Addr>().unwrap().octets();
        unsafe {
            let mut table = std::ptr::null_mut();
            let code = GetUnicastIpAddressTable(AF_UNSPEC, &mut table);
            if code != 0 { bail!("read TUN address state: Windows error {code}"); }
            let mut ready = (false, false);
            let mut duplicate = false;
            for row in std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize) {
                if row.InterfaceIndex != index { continue; }
                let target = match row.Address.si_family {
                    AF_INET if row.Address.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes() == ADDR.octets() => Some(&mut ready.0),
                    AF_INET6 if row.Address.Ipv6.sin6_addr.u.Byte == v6 => Some(&mut ready.1),
                    _ => None,
                };
                if let Some(target) = target {
                    duplicate |= row.DadState == IpDadStateDuplicate;
                    *target = row.DadState == IpDadStatePreferred;
                }
            }
            FreeMibTable(table.cast());
            if duplicate { bail!("TUN address conflict detected on interface {index}"); }
            Ok(ready)
        }
    }

    async fn wait_addresses(index: u32, ipv6: bool) -> Result<()> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let (v4_ready, v6_ready) = ready_addresses(index)?;
            if v4_ready && (!ipv6 || v6_ready) { return Ok(()); }
            if tokio::time::Instant::now() >= deadline { bail!("TUN addresses not ready after 10s (IPv4={v4_ready}, IPv6={v6_ready})"); }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    pub async fn start(cfg: &TunConfig, home: &Path, hub: Arc<Hub>) -> Result<()> {
        let initial = crate::network::uplink(0)?;
        let phys = initial.index;
        let wintun = unsafe { wintun::load_from_path(find_wintun(home)?) }.context("load wintun.dll")?;
        let adapter = wintun::Adapter::create(&wintun, "Prism", "Prism", Some(GUID))
            .map_err(|e| anyhow!("create TUN adapter: {e} (TUN mode needs administrator rights)"))?;
        let index = adapter.get_adapter_index().map_err(|e| anyhow!("adapter index: {e}"))?;
        // Install before publishing TUN routes/DNS. Windows may otherwise
        // query a LAN DNS server directly through a more-specific route.
        let dns_guard = if cfg.auto_route { Some(crate::dns_guard::DnsGuard::enable(index)?) } else { None };
        // Bring the link up before configuring addresses so Windows can
        // perform duplicate-address detection while configuration proceeds.
        let session = Arc::new(adapter.start_session(wintun::MAX_RING_CAPACITY).map_err(|e| anyhow!("TUN session: {e}"))?);
        let name = format!("name={index}");
        netsh(&["interface", "ipv4", "set", "address", &name, "source=static", "address=172.19.0.1", "mask=255.255.255.252"])?;
        netsh(&["interface", "ipv4", "set", "subinterface", &index.to_string(), &format!("mtu={MTU}"), "store=active"])?;
        netsh(&["interface", "ipv4", "set", "interface", &index.to_string(), "metric=1"])?;
        netsh(&["interface", "ipv4", "set", "dnsservers", &name, "source=static", "address=172.19.0.2", "register=none", "validate=no"])?;

        // Pin our own sockets to the physical interface before routing
        // everything else into the adapter.
        net::set_bind_interface(phys);
        if cfg.auto_route {
            let iface = format!("interface={index}");
            for prefix in ["0.0.0.0/1", "128.0.0.0/1"] {
                netsh(&["interface", "ipv4", "add", "route", &format!("prefix={prefix}"), &iface, "nexthop=0.0.0.0", "metric=1", "store=active"])?;
            }
        }
        let ipv6_ready_required = if cfg.ipv6 {
            match ipv6(index, cfg.auto_route) {
                Ok(()) => true,
                Err(e) => {
                    if hub.core().general.block_ipv6 { bail!("IPv6 protection requires TUN IPv6 routes: {e:#}"); }
                    tracing::warn!("TUN: IPv6 not set up, IPv6 traffic bypasses the TUN: {e:#}");
                    false
                }
            }
        } else { false };
        // A live Wintun session is insufficient: a Tentative address makes
        // Windows DNS queries fail and may cause fallback to another adapter.
        wait_addresses(index, ipv6_ready_required).await?;
        tracing::info!("TUN adapter Prism (if {index}) up at {ADDR}/30, outbound bound to if {phys}, auto-route={}", cfg.auto_route);
        use crate::tun_health::Pump;
        hub.tun_health.enable();

        // adapter -> stack (wintun reads block, so use a thread)
        let (tx, rx) = mpsc::channel::<Vec<u8>>(4096);
        let rs = session.clone();
        let receive_health = hub.tun_health.clone();
        let receive_guard = receive_health.guard(Pump::Receive);
        std::thread::Builder::new().name("tun-rx".into()).spawn(move || {
            let _guard = receive_guard;
            loop {
                match rs.receive_blocking() {
                    Ok(packet) => {
                        let bytes = packet.bytes().to_vec();
                        drop(packet); // Release the ring slot BEFORE a possibly blocked queue send.
                        receive_health.received(bytes.len());
                        let queued = (tx.max_capacity() - tx.capacity() + 1).min(tx.max_capacity());
                        let wait = std::time::Instant::now();
                        receive_health.queue(true, queued, std::time::Duration::ZERO);
                        if tx.blocking_send(bytes).is_err() { break; }
                        receive_health.queue(true, queued, wait.elapsed());
                    }
                    Err(error) => {
                        let code = match &error { wintun::Error::Io(e) => e.raw_os_error(), _ => None };
                        tracing::error!("TUN receive failed: {error}");
                        receive_health.fail(Pump::Receive, "device_receive_error", code);
                        break;
                    }
                }
            }
        })?;
        // stack -> adapter
        let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(4096);
        let ws = session.clone();
        let send_health = hub.tun_health.clone();
        let send_guard = send_health.guard(Pump::Send);
        tokio::spawn(async move {
            let _guard = send_guard;
            let _keep = adapter; // adapter lives as long as the pump
            let _dns_guard = dns_guard; // survives reloads, removed on TUN shutdown
            while let Some(p) = out_rx.recv().await {
                match ws.allocate_send_packet(p.len() as u16) {
                    Ok(mut pkt) => {
                        pkt.bytes_mut().copy_from_slice(&p);
                        ws.send_packet(pkt);
                        send_health.sent(p.len());
                    }
                    Err(wintun::Error::Io(error)) if error.raw_os_error() == Some(111) => {
                        // ERROR_BUFFER_OVERFLOW: congestion, not a dead adapter.
                        send_health.ring_full();
                    }
                    Err(error) => {
                        let code = match &error { wintun::Error::Io(e) => e.raw_os_error(), _ => None };
                        tracing::error!("TUN send failed: {error}");
                        send_health.fail(Pump::Send, "device_send_error", code);
                        break;
                    }
                }
            }
        });
        super::stack::run(cfg, hub.clone(), rx, out_tx)?;
        monitor_uplink(&hub, index, initial);
        Ok(())
    }
}

/// macOS: a utun interface (needs root), configured with ifconfig / route.
/// The system resolver's DNS server is usually the router, on the local
/// network and so outside the 0/1 + 128/1 routes (a host route for it would
/// also capture the gateway itself); the Prism app points the system DNS at
/// public servers while TUN runs instead.
#[cfg(target_os = "macos")]
mod mac {
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::path::Path;
    use std::sync::Arc;

    use anyhow::{anyhow, bail, Context, Result};
    use tokio::io::unix::AsyncFd;
    use tokio::sync::mpsc;

    use super::{ADDR, ADDR6, DNS, MTU, ROUTES6};
    use crate::config::TunConfig;
    use crate::hub::Hub;
    use crate::net;

    const UTUN_CONTROL: &[u8] = b"com.apple.net.utun_control";
    const UTUN_OPT_IFNAME: libc::c_int = 2;

    fn run(cmd: &str, args: &[&str]) -> Result<String> {
        let out = std::process::Command::new(cmd).args(args).output().with_context(|| format!("run {cmd}"))?;
        if !out.status.success() {
            bail!("{cmd} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Opens a new utun device; returns it and its name (utunN).
    fn open_utun() -> Result<(OwnedFd, String)> {
        unsafe {
            let fd = libc::socket(libc::PF_SYSTEM, libc::SOCK_DGRAM, libc::SYSPROTO_CONTROL);
            if fd < 0 {
                return Err(io::Error::last_os_error()).context("open utun control socket (TUN mode needs root)");
            }
            let fd = OwnedFd::from_raw_fd(fd);
            let mut info: libc::ctl_info = std::mem::zeroed();
            for (i, b) in UTUN_CONTROL.iter().enumerate() {
                info.ctl_name[i] = *b as libc::c_char;
            }
            if libc::ioctl(fd.as_raw_fd(), libc::CTLIOCGINFO, &mut info) < 0 {
                return Err(io::Error::last_os_error()).context("utun control info");
            }
            let addr = libc::sockaddr_ctl {
                sc_len: std::mem::size_of::<libc::sockaddr_ctl>() as u8,
                sc_family: libc::AF_SYSTEM as u8,
                ss_sysaddr: libc::AF_SYS_CONTROL as u16,
                sc_id: info.ctl_id,
                sc_unit: 0, // the next free utunN
                sc_reserved: [0; 5],
            };
            if libc::connect(fd.as_raw_fd(), &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_ctl>() as u32) < 0 {
                return Err(io::Error::last_os_error()).context("create utun (TUN mode needs root)");
            }
            let mut name = [0u8; 32];
            let mut len = name.len() as libc::socklen_t;
            if libc::getsockopt(fd.as_raw_fd(), libc::SYSPROTO_CONTROL, UTUN_OPT_IFNAME, name.as_mut_ptr() as *mut _, &mut len) < 0 {
                return Err(io::Error::last_os_error()).context("utun name");
            }
            let name = String::from_utf8_lossy(&name[..(len as usize).saturating_sub(1)]).into_owned();
            let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
            libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
            libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
            Ok((fd, name))
        }
    }

    /// The interface of the current default route (e.g. en0), its index and
    /// gateway.
    pub fn default_interface() -> Result<(String, u32, Option<String>)> {
        let out = run("/sbin/route", &["-n", "get", "default"])?;
        let field = |k: &str| out.lines().find_map(|l| l.trim().strip_prefix(k).map(|n| n.trim().to_string()));
        let name = field("interface:").ok_or_else(|| anyhow!("no default route"))?;
        let gateway = field("gateway:").filter(|g| g.parse::<std::net::IpAddr>().is_ok());
        let c = std::ffi::CString::new(name.clone())?;
        let index = unsafe { libc::if_nametoindex(c.as_ptr()) };
        if index == 0 {
            bail!("interface {name} not found");
        }
        Ok((name, index, gateway))
    }

    /// Sockets bound to an interface (IP_BOUND_IF) only see routes scoped
    /// to it. For the primary interface macOS usually has no scoped default
    /// route and falls back to the unscoped table, which with our 0/1 +
    /// 128/1 routes points into the tunnel: the lookup then fails. So add
    /// the scoped default route explicitly (it applies to bound sockets
    /// only, so other traffic is unaffected).
    fn ensure_scoped_default(iface: &str, gateway: Option<&str>) {
        let r = match gateway {
            Some(gw) => run("/sbin/route", &["-n", "add", "-ifscope", iface, "default", gw]),
            None => run("/sbin/route", &["-n", "add", "-ifscope", iface, "default", "-interface", iface]),
        };
        match r {
            Ok(_) => tracing::info!("added a scoped default route on {iface} for the core's own connections"),
            Err(e) if format!("{e:#}").contains("File exists") => {}
            Err(e) => tracing::warn!("no scoped default route on {iface}, the core's own connections may fail: {e:#}"),
        }
    }

    /// IPv6 address and split default routes on the utun; the physical
    /// interface gets a scoped IPv6 default route (when it has IPv6) for the
    /// core's own pinned sockets, like IPv4.
    fn ipv6(name: &str, phys: &str, auto_route: bool) -> Result<()> {
        run("/sbin/ifconfig", &[name, "inet6", ADDR6, "prefixlen", "126"])?;
        if let Ok(out) = run("/sbin/route", &["-n", "get", "-inet6", "default"]) {
            let field = |k: &str| out.lines().find_map(|l| l.trim().strip_prefix(k).map(|n| n.trim().to_string()));
            if let (Some(gw), Some(iface)) = (field("gateway:"), field("interface:")) {
                if iface == phys {
                    match run("/sbin/route", &["-n", "add", "-inet6", "-ifscope", phys, "default", &gw]) {
                        Err(e) if !format!("{e:#}").contains("File exists") => tracing::warn!("no scoped IPv6 default route on {phys}: {e:#}"),
                        _ => {}
                    }
                }
            }
        }
        if auto_route {
            for net in ROUTES6 {
                run("/sbin/route", &["-n", "add", "-inet6", "-net", net, "-interface", name])?;
            }
        }
        Ok(())
    }

    /// Strips / adds utun's 4-byte protocol family header.
    pub fn frame(p: &[u8]) -> Vec<u8> {
        let af = if p.first().is_some_and(|b| b >> 4 == 6) { libc::AF_INET6 } else { libc::AF_INET } as u32;
        let mut out = Vec::with_capacity(p.len() + 4);
        out.extend_from_slice(&af.to_be_bytes());
        out.extend_from_slice(p);
        out
    }

    pub async fn start(cfg: &TunConfig, _home: &Path, hub: Arc<Hub>) -> Result<()> {
        let (phys_name, phys, gateway) = default_interface()?;
        ensure_scoped_default(&phys_name, gateway.as_deref());
        let (fd, name) = open_utun()?;
        run("/sbin/ifconfig", &[&name, &ADDR.to_string(), &DNS.to_string(), "mtu", &MTU.to_string(), "up"])?;
        // Pin our own sockets to the physical interface before routing
        // everything else into the tunnel.
        net::set_bind_interface(phys);
        if cfg.auto_route {
            for net in ["0.0.0.0/1", "128.0.0.0/1"] {
                run("/sbin/route", &["-n", "add", "-net", net, "-interface", &name])?;
            }
        }
        if cfg.ipv6 {
            if let Err(e) = ipv6(&name, &phys_name, cfg.auto_route) {
                if hub.core().general.block_ipv6 { anyhow::bail!("IPv6 protection requires TUN IPv6 routes: {e:#}"); }
                tracing::warn!("TUN: IPv6 not set up, IPv6 traffic bypasses the TUN: {e:#}");
            }
        }
        tracing::info!("TUN {name} up at {ADDR}, outbound bound to {phys_name} (if {phys}), auto-route={}, ipv6={}", cfg.auto_route, cfg.ipv6);

        // Safety: `fd` is owned by the AsyncFd from here on and closed only
        // when it is dropped, after deregistration.
        let dev = Arc::new(unsafe { AsyncFd::register(fd) }.map_err(|e| anyhow!("register utun: {}", e.into_parts().1))?);
        use crate::tun_health::Pump;
        hub.tun_health.enable();
        let (tx, rx) = mpsc::channel::<Vec<u8>>(4096);
        let rd = dev.clone();
        let receive_health = hub.tun_health.clone();
        let receive_guard = receive_health.guard(Pump::Receive);
        tokio::spawn(async move {
            let _guard = receive_guard;
            let mut buf = vec![0u8; MTU + 4];
            loop {
                let Ok(mut guard) = rd.readable().await else { return };
                match guard.try_io(|fd| {
                    let n = unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr() as *mut _, buf.len()) };
                    if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
                }) {
                    Ok(Ok(n)) if n > 4 => {
                        receive_health.received(n - 4);
                        let queued = (tx.max_capacity() - tx.capacity() + 1).min(tx.max_capacity());
                        let wait = std::time::Instant::now();
                        receive_health.queue(true, queued, std::time::Duration::ZERO);
                        if tx.send(buf[4..n].to_vec()).await.is_err() {
                            return;
                        }
                        receive_health.queue(true, queued, wait.elapsed());
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        tracing::warn!("utun read: {e}");
                        receive_health.fail(Pump::Receive, "device_receive_error", e.raw_os_error());
                        return;
                    }
                    Err(_would_block) => {}
                }
            }
        });
        let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(4096);
        let send_health = hub.tun_health.clone();
        let send_guard = send_health.guard(Pump::Send);
        tokio::spawn(async move {
            let _guard = send_guard;
            while let Some(p) = out_rx.recv().await {
                let pkt = frame(&p);
                loop {
                    let Ok(mut guard) = dev.writable().await else { return };
                    match guard.try_io(|fd| {
                        let n = unsafe { libc::write(fd.as_raw_fd(), pkt.as_ptr() as *const _, pkt.len()) };
                        if n < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
                    }) {
                        Ok(Ok(())) => { send_health.sent(p.len()); break; }
                        Ok(Err(error)) => {
                            send_health.fail(Pump::Send, "device_send_error", error.raw_os_error());
                            return;
                        }
                        Err(_would_block) => continue,
                    }
                }
            }
        });
        super::stack::run(cfg, hub, rx, out_tx)
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn utun_header() {
            assert_eq!(&super::frame(&[0x45, 0, 0, 20])[..4], &[0, 0, 0, 2]);
            assert_eq!(&super::frame(&[0x60, 0, 0, 0])[..4], &[0, 0, 0, 30]);
        }
    }
}
