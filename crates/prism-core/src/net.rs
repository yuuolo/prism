//! Outbound sockets. While TUN mode is active every socket the core opens is
//! pinned to the physical interface (IP_UNICAST_IF), so the core's own
//! traffic bypasses the TUN routes instead of looping back into itself.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpSocket, TcpStream, UdpSocket};

/// Interface index outbound sockets are bound to; 0 = no binding.
static BIND_IF: AtomicU32 = AtomicU32::new(0);

pub fn set_bind_interface(index: u32) {
    BIND_IF.store(index, Ordering::Relaxed);
}

pub fn bind_interface() -> u32 {
    BIND_IF.load(Ordering::Relaxed)
}

#[cfg(windows)]
fn pin(sock: std::os::windows::io::RawSocket, v6: bool, index: u32) -> io::Result<()> {
    use windows_sys::Win32::Networking::WinSock::{setsockopt, IPPROTO_IP, IPPROTO_IPV6, IPV6_UNICAST_IF, IP_UNICAST_IF};
    // IPv4 takes the index in network byte order, IPv6 in host order.
    let (level, opt, val) = if v6 {
        (IPPROTO_IPV6, IPV6_UNICAST_IF, index)
    } else {
        (IPPROTO_IP, IP_UNICAST_IF, index.to_be())
    };
    let r = unsafe { setsockopt(sock as usize, level, opt, &val as *const u32 as *const u8, 4) };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn pin(_: i32, _: bool, _: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn raw<S: std::os::windows::io::AsRawSocket>(s: &S) -> std::os::windows::io::RawSocket {
    s.as_raw_socket()
}

#[cfg(not(windows))]
fn raw<S: std::os::fd::AsRawFd>(s: &S) -> i32 {
    s.as_raw_fd()
}

async fn connect_addr(addr: SocketAddr) -> io::Result<TcpStream> {
    let index = bind_interface();
    let s = if index == 0 {
        TcpStream::connect(addr).await?
    } else {
        let sock = if addr.is_ipv4() { TcpSocket::new_v4()? } else { TcpSocket::new_v6()? };
        pin(raw(&sock), addr.is_ipv6(), index)?;
        sock.connect(addr).await?
    };
    let _ = s.set_nodelay(true);
    Ok(s)
}

static RESOLVER: std::sync::RwLock<Option<Arc<crate::dns::Resolver>>> = std::sync::RwLock::new(None);

/// Name resolution for the core's own connections goes through the DNS
/// module when it is enabled.
pub fn set_resolver(r: Option<Arc<crate::dns::Resolver>>) {
    *RESOLVER.write().unwrap() = r;
}

/// Resolves `host`: DNS module when enabled, else the system resolver (in
/// TUN mode its queries are served by the DNS hijack over pinned sockets).
pub async fn resolve(host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = h.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let resolver = RESOLVER.read().unwrap().clone();
    if let Some(r) = resolver {
        let ips = r.lookup(h).await?;
        return Ok(ips.into_iter().map(|ip| SocketAddr::new(ip, port)).collect());
    }
    let addrs: Vec<SocketAddr> = tokio::time::timeout(Duration::from_secs(10), tokio::net::lookup_host((h, port)))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, format!("resolve {h}: timeout")))??
        .collect();
    if addrs.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("resolve {h}: no address")));
    }
    Ok(addrs)
}

/// Connects to `host:port`, trying IPv4 addresses first.
pub async fn tcp_connect(host: &str, port: u16) -> io::Result<TcpStream> {
    let mut addrs = resolve(host, port).await?;
    addrs.sort_by_key(|a| a.is_ipv6());
    let mut last = None;
    for a in addrs.into_iter().take(4) {
        match connect_addr(a).await {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no address")))
}

/// UDP socket for talking to `peer`, pinned like the TCP sockets.
pub async fn udp_socket(peer: SocketAddr) -> io::Result<UdpSocket> {
    let bind: SocketAddr = if peer.is_ipv4() { "0.0.0.0:0".parse().unwrap() } else { "[::]:0".parse().unwrap() };
    let s = UdpSocket::bind(bind).await?;
    let index = bind_interface();
    if index != 0 {
        pin(raw(&s), peer.is_ipv6(), index)?;
    }
    Ok(s)
}
