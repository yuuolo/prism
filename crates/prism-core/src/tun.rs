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
    _: std::sync::Arc<crate::tunnel::Tunnel>,
) -> anyhow::Result<()> {
    anyhow::bail!("TUN mode is only implemented on Windows")
}

#[cfg(windows)]
pub use imp::start;

#[cfg(windows)]
mod imp {
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use anyhow::{anyhow, bail, Context, Result};
    use futures::{SinkExt, StreamExt};
    use netstack_smoltcp::{StackBuilder, UdpSocket as StackUdp};
    use tokio::io::AsyncReadExt;
    use tokio::sync::mpsc;

    use crate::common::Address;
    use crate::config::TunConfig;
    use crate::tunnel::{Metadata, Tunnel};
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

    pub async fn start(cfg: &TunConfig, home: &Path, tunnel: Arc<Tunnel>) -> Result<()> {
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
            let t = tunnel.clone();
            tokio::spawn(async move {
                while let Some((stream, src, dst)) = tcp.next().await {
                    tokio::spawn(handle_tcp(stream, src, dst, t.clone()));
                }
            });
        }
        if let Some(udp) = udp {
            let upstream = cfg.dns_upstream();
            tokio::spawn(handle_udp(udp, tunnel, upstream));
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

    type Msg = (Vec<u8>, SocketAddr, SocketAddr);

    async fn handle_udp(udp: StackUdp, tunnel: Arc<Tunnel>, upstream: Vec<IpAddr>) {
        let (mut rd, mut wr) = udp.split();
        let (wtx, mut wrx) = mpsc::channel::<Msg>(4096);
        tokio::spawn(async move {
            while let Some(m) = wrx.recv().await {
                if wr.send(m).await.is_err() {
                    break;
                }
            }
        });
        let sessions: Arc<Mutex<HashMap<(SocketAddr, SocketAddr), Arc<tokio::net::UdpSocket>>>> = Default::default();
        while let Some((data, src, dst)) = rd.next().await {
            if dst.port() == 53 {
                let target = if dst.ip() == IpAddr::V4(DNS) || dst.ip() == IpAddr::V4(ADDR) {
                    SocketAddr::new(upstream.first().copied().unwrap_or(IpAddr::V4(Ipv4Addr::new(223, 5, 5, 5))), 53)
                } else {
                    dst
                };
                tokio::spawn(dns_forward(data, src, dst, target, wtx.clone()));
                continue;
            }
            if dst.port() == 443 || dst.ip().is_multicast() || dst.ip() == IpAddr::V4(Ipv4Addr::BROADCAST) {
                continue; // QUIC -> force TCP fallback; no multicast/broadcast
            }
            let existing = sessions.lock().unwrap().get(&(src, dst)).cloned();
            if let Some(sock) = existing {
                let _ = sock.send(&data).await;
                continue;
            }
            let mut meta = Metadata { inbound: "Tun", src, dst: Address::Ip(dst), dst_ip: Some(dst.ip()) };
            if !tunnel.udp_direct(&mut meta).await {
                continue;
            }
            let Ok(sock) = net::udp_socket(dst).await else { continue };
            if sock.connect(dst).await.is_err() {
                continue;
            }
            let sock = Arc::new(sock);
            sessions.lock().unwrap().insert((src, dst), sock.clone());
            let _ = sock.send(&data).await;
            let (sessions, wtx) = (sessions.clone(), wtx.clone());
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65535];
                while let Ok(Ok(n)) = tokio::time::timeout(Duration::from_secs(60), sock.recv(&mut buf)).await {
                    if wtx.send((buf[..n].to_vec(), dst, src)).await.is_err() {
                        break;
                    }
                }
                sessions.lock().unwrap().remove(&(src, dst));
            });
        }
    }

    async fn dns_forward(query: Vec<u8>, src: SocketAddr, dst: SocketAddr, upstream: SocketAddr, wtx: mpsc::Sender<Msg>) {
        let res = async {
            let sock = net::udp_socket(upstream).await?;
            sock.send_to(&query, upstream).await?;
            let mut buf = vec![0u8; 4096];
            let (n, _) = tokio::time::timeout(Duration::from_secs(5), sock.recv_from(&mut buf))
                .await
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "dns timeout"))??;
            buf.truncate(n);
            Ok::<_, std::io::Error>(buf)
        }
        .await;
        match res {
            // Reply appears to come from the address the client asked.
            Ok(resp) => {
                let _ = wtx.send((resp, dst, src)).await;
            }
            Err(e) => tracing::debug!("dns {upstream}: {e}"),
        }
    }
}
