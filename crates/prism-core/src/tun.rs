//! TUN inbound (Windows): wintun adapter + smoltcp userspace stack.
//!
//! IP packets from the adapter are fed into netstack-smoltcp, which hands back
//! TCP streams and UDP datagrams:
//!   * TCP: the first bytes are sniffed (TLS SNI / HTTP Host) so rules can
//!     match domains, then the connection goes through the normal tunnel.
//!   * UDP 53: DNS queries are forwarded upstream over pinned sockets.
//!   * UDP 443 (QUIC) is dropped so browsers fall back to TCP.
//!   * Other UDP is relayed only when the rules send it to DIRECT.
//!
//! auto-route adds 0.0.0.0/1 + 128.0.0.0/1 on the adapter; the core's own
//! sockets are pinned to the previous default interface (see net.rs). The
//! routes disappear together with the adapter when the process exits.

#[cfg(not(windows))]
pub async fn start(
    _: &crate::config::TunConfig,
    _: &std::path::Path,
    _: std::sync::Arc<crate::hub::Hub>,
) -> anyhow::Result<()> {
    anyhow::bail!("TUN mode is only implemented on Windows")
}

#[cfg(windows)]
pub use imp::start;

#[cfg(windows)]
mod imp {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::{anyhow, bail, Context, Result};
    use futures::{SinkExt, StreamExt};
    use netstack_smoltcp::{StackBuilder, UdpSocket as StackUdp};
    use tokio::io::AsyncReadExt;
    use tokio::sync::mpsc;

    use crate::common::Address;
    use crate::config::TunConfig;
    use crate::hub::Hub;
    use crate::tunnel::{Metadata, Network, Tunnel, UdpNat, UdpPacket};
    use crate::{net, sniff};

    const ADDR: Ipv4Addr = Ipv4Addr::new(172, 19, 0, 1);
    /// Advertised as the adapter's DNS server; queries to it are hijacked.
    const DNS: Ipv4Addr = Ipv4Addr::new(172, 19, 0, 2);
    const MTU: usize = 9000;
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

    /// Interface currently used for the default route (before we add ours).
    fn default_interface() -> Result<u32> {
        let mut index = 0u32;
        let dest = u32::from_ne_bytes([1, 1, 1, 1]);
        let r = unsafe { windows_sys::Win32::NetworkManagement::IpHelper::GetBestInterface(dest, &mut index) };
        if r != 0 {
            bail!("no default network interface (GetBestInterface error {r})");
        }
        Ok(index)
    }

    fn find_wintun(home: &Path) -> Result<PathBuf> {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
        [Some(home.join("wintun.dll")), exe_dir.map(|d| d.join("wintun.dll"))]
            .into_iter()
            .flatten()
            .find(|p| p.exists())
            .ok_or_else(|| anyhow!("wintun.dll not found (put it in {} or next to prism-core)", home.display()))
    }

    pub async fn start(cfg: &TunConfig, home: &Path, hub: Arc<Hub>) -> Result<()> {
        let phys = default_interface()?;
        let wintun = unsafe { wintun::load_from_path(find_wintun(home)?) }.context("load wintun.dll")?;
        let adapter = wintun::Adapter::create(&wintun, "Prism", "Prism", Some(GUID))
            .map_err(|e| anyhow!("create TUN adapter: {e} (TUN mode needs administrator rights)"))?;
        let index = adapter.get_adapter_index().map_err(|e| anyhow!("adapter index: {e}"))?;
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
        let session = Arc::new(adapter.start_session(wintun::MAX_RING_CAPACITY).map_err(|e| anyhow!("TUN session: {e}"))?);
        tracing::info!("TUN adapter Prism (if {index}) up at {ADDR}/30, outbound bound to if {phys}, auto-route={}", cfg.auto_route);

        let (stack, runner, udp, tcp) = StackBuilder::default()
            .enable_tcp(true)
            .enable_udp(true)
            .enable_icmp(true)
            .mtu(MTU)
            .build()
            .map_err(|e| anyhow!("netstack: {e}"))?;
        if let Some(r) = runner {
            tokio::spawn(r);
        }
        let (mut stack_sink, mut stack_stream) = stack.split();

        // adapter -> stack (wintun reads block, so use a thread)
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(4096);
        let rs = session.clone();
        std::thread::Builder::new().name("tun-rx".into()).spawn(move || {
            while let Ok(p) = rs.receive_blocking() {
                if tx.blocking_send(p.bytes().to_vec()).is_err() {
                    break;
                }
            }
        })?;
        tokio::spawn(async move {
            while let Some(p) = rx.recv().await {
                if stack_sink.send(p).await.is_err() {
                    break;
                }
            }
        });
        // stack -> adapter
        let ws = session.clone();
        tokio::spawn(async move {
            let _keep = adapter; // adapter lives as long as the pump
            while let Some(Ok(p)) = stack_stream.next().await {
                if p.is_empty() || p.len() > u16::MAX as usize {
                    continue;
                }
                if let Ok(mut pkt) = ws.allocate_send_packet(p.len() as u16) {
                    pkt.bytes_mut().copy_from_slice(&p);
                    ws.send_packet(pkt);
                }
            }
        });

        if let Some(mut tcp) = tcp {
            let h = hub.clone();
            tokio::spawn(async move {
                while let Some((stream, src, dst)) = tcp.next().await {
                    tokio::spawn(handle_tcp(stream, src, dst, h.core().tunnel.clone()));
                }
            });
        }
        if let Some(udp) = udp {
            let upstream = cfg.dns_upstream();
            tokio::spawn(handle_udp(udp, hub, upstream));
        }
        Ok(())
    }

    async fn handle_tcp(mut s: netstack_smoltcp::TcpStream, src: SocketAddr, dst: SocketAddr, tunnel: Arc<Tunnel>) {
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
        let mut meta = Metadata {
            network: Network::Tcp,
            inbound: "Tun",
            src,
            dst: match domain {
                Some(d) => Address::Domain(d, dst.port()),
                None => Address::Ip(dst),
            },
            dst_ip: Some(dst.ip()),
        };
        if let Ok(d) = tunnel.dial(&mut meta).await {
            tunnel.relay(s, d, meta, &buf[..n]).await;
        }
    }

    async fn handle_udp(udp: StackUdp, hub: Arc<Hub>, upstream: Vec<IpAddr>) {
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
                tokio::spawn(dns_query(hub.clone(), data, src, dst, upstream.clone(), reply_tx.clone()));
                continue;
            }
            if dst.ip().is_multicast() || dst.ip() == IpAddr::V4(Ipv4Addr::BROADCAST) {
                continue;
            }
            nat.send(&hub.core().tunnel, UdpPacket { src, dst: Address::Ip(dst), data });
        }
    }

    /// DNS hijack: the DNS module answers when enabled (fake-ip etc.);
    /// otherwise the query is forwarded to the server the client asked (or
    /// the default upstream for the TUN's own DNS address).
    async fn dns_query(hub: Arc<Hub>, query: Vec<u8>, src: SocketAddr, dst: SocketAddr, upstream: Vec<IpAddr>, reply: mpsc::Sender<UdpPacket>) {
        let resp = match hub.core().dns.clone() {
            Some(r) => r.handle(&query).await,
            None => {
                let target = if dst.ip() == IpAddr::V4(DNS) || dst.ip() == IpAddr::V4(ADDR) {
                    SocketAddr::new(upstream.first().copied().unwrap_or(IpAddr::V4(Ipv4Addr::new(223, 5, 5, 5))), 53)
                } else {
                    dst
                };
                match tokio::time::timeout(Duration::from_secs(5), crate::dns::upstream::udp_query(target, &query)).await {
                    Ok(Ok(r)) => Some(r),
                    Ok(Err(e)) => {
                        tracing::debug!("dns {target}: {e}");
                        None
                    }
                    Err(_) => None,
                }
            }
        };
        if let Some(data) = resp {
            // The reply appears to come from the address the client asked.
            let _ = reply.send(UdpPacket { src, dst: Address::Ip(dst), data }).await;
        }
    }
}
