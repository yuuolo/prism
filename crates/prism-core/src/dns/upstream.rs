//! DNS upstream servers, in mihomo's URL syntax:
//!
//! `223.5.5.5`, `udp://1.1.1.1:53`, `tcp://8.8.8.8`, `tls://dns.alidns.com`,
//! `https://doh.pub/dns-query`, optionally followed by `#<proxy or group>`
//! to send the queries through that outbound.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::wire;
use crate::common::{other, Address, BoxedStream};
use crate::outbound::tls::Tls;
use crate::outbound::RegistryCell;

const TIMEOUT: Duration = Duration::from_secs(5);
const POOL: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Udp,
    Tcp,
    Tls,
    Https,
}

pub struct Upstream {
    pub url: String,
    kind: Kind,
    host: String,
    port: u16,
    path: String,
    /// Outbound (proxy or group) the queries go through; None = direct.
    pub detour: Option<String>,
    tls: Option<Tls>,
    bootstrap: Arc<Vec<SocketAddr>>,
    reg: RegistryCell,
    pool: Mutex<Vec<BoxedStream>>,
    addr: Mutex<Option<(IpAddr, Instant)>>,
}

impl Upstream {
    /// Parses one nameserver entry. `bootstrap` (plain-IP default
    /// nameservers) resolves hostnames such as `dns.alidns.com`.
    pub fn parse(spec: &str, bootstrap: Arc<Vec<SocketAddr>>, reg: RegistryCell) -> Result<Self, String> {
        let (main, detour) = match spec.split_once('#') {
            Some((m, d)) => {
                let d = d.split('&').next().unwrap_or("").trim();
                (m.trim(), if d.is_empty() || d.eq_ignore_ascii_case("DIRECT") || d.contains('=') { None } else { Some(d.to_string()) })
            }
            None => (spec.trim(), None),
        };
        let (kind, rest) = match main.split_once("://") {
            Some(("udp", r)) => (Kind::Udp, r),
            Some(("tcp", r)) => (Kind::Tcp, r),
            Some(("tls", r)) => (Kind::Tls, r),
            Some(("https", r)) => (Kind::Https, r),
            Some((s, _)) => return Err(format!("dns server {spec}: {s}:// is not supported")),
            None if main == "system" || main.starts_with("dhcp") => return Err(format!("dns server {spec} is not supported")),
            None => (Kind::Udp, main),
        };
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], rest[i..].to_string()),
            None => (rest, "/dns-query".to_string()),
        };
        let default_port = match kind {
            Kind::Udp | Kind::Tcp => 53,
            Kind::Tls => 853,
            Kind::Https => 443,
        };
        let addr = Address::parse_authority(authority).unwrap_or_else(|| Address::new(authority, default_port));
        let (host, port) = (addr.host(), addr.port());
        if host.is_empty() {
            return Err(format!("dns server {spec}: missing host"));
        }
        let tls = match kind {
            Kind::Tls | Kind::Https => {
                let alpn = if kind == Kind::Https { vec!["http/1.1".to_string()] } else { vec![] };
                Some(Tls::new(&host, false, &alpn).map_err(|e| format!("dns server {spec}: {e}"))?)
            }
            _ => None,
        };
        Ok(Upstream {
            url: spec.to_string(),
            kind,
            host,
            port,
            path,
            detour,
            tls,
            bootstrap,
            reg,
            pool: Mutex::new(Vec::new()),
            addr: Mutex::new(None),
        })
    }

    pub fn is_direct(&self) -> bool {
        self.detour.is_none()
    }

    /// Sends `query` and returns the response (any rcode).
    pub async fn exchange(&self, query: &[u8]) -> io::Result<Vec<u8>> {
        let res = tokio::time::timeout(TIMEOUT, async {
            match self.kind {
                Kind::Udp => self.exchange_udp(query).await,
                _ => self.exchange_stream(query).await,
            }
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, format!("dns {}: timeout", self.url)))??;
        if res.len() < 12 || wire::id(&res) != wire::id(query) {
            return Err(other(format!("dns {}: mismatched response", self.url)));
        }
        Ok(res)
    }

    fn target(&self) -> Address {
        Address::new(&self.host, self.port)
    }

    /// Server IP for direct queries (hostnames go through the bootstrap
    /// servers, cached for 10 minutes).
    async fn server_ip(&self) -> io::Result<IpAddr> {
        if let Ok(ip) = self.host.parse::<IpAddr>() {
            return Ok(ip);
        }
        if let Some((ip, at)) = *self.addr.lock().unwrap() {
            if at.elapsed() < Duration::from_secs(600) {
                return Ok(ip);
            }
        }
        let ip = bootstrap_lookup(&self.bootstrap, &self.host).await?;
        *self.addr.lock().unwrap() = Some((ip, Instant::now()));
        Ok(ip)
    }

    async fn exchange_udp(&self, query: &[u8]) -> io::Result<Vec<u8>> {
        if let Some(d) = &self.detour {
            let reg = self.reg.get().ok_or_else(|| other("registry not ready"))?;
            let (out, _) = reg.resolve(d, &self.target())?;
            let mut pipe = out.connect_udp(&self.target()).await?;
            pipe.tx.send(query.to_vec()).await.map_err(|_| other("dns: udp session closed"))?;
            return pipe.rx.recv().await.ok_or_else(|| other("dns: no response"));
        }
        let server = SocketAddr::new(self.server_ip().await?, self.port);
        udp_query(server, query).await
    }

    async fn connect(&self) -> io::Result<BoxedStream> {
        let raw: BoxedStream = match &self.detour {
            Some(d) => {
                let reg = self.reg.get().ok_or_else(|| other("registry not ready"))?;
                let (out, _) = reg.resolve(d, &self.target())?;
                out.connect(&self.target()).await?
            }
            None => {
                let ip = self.server_ip().await?;
                Box::new(crate::net::tcp_connect(&ip.to_string(), self.port).await?)
            }
        };
        Ok(match &self.tls {
            Some(t) => Box::new(t.wrap(raw).await?),
            None => raw,
        })
    }

    /// TCP / DoT / DoH over pooled keep-alive connections; a pooled
    /// connection that turns out to be dead is retried once on a new one.
    async fn exchange_stream(&self, query: &[u8]) -> io::Result<Vec<u8>> {
        for _ in 0..2 {
            let pooled = self.pool.lock().unwrap().pop();
            let fresh = pooled.is_none();
            let mut s = match pooled {
                Some(s) => s,
                None => self.connect().await?,
            };
            match self.one(&mut s, query).await {
                Ok(r) => {
                    let mut pool = self.pool.lock().unwrap();
                    if pool.len() < POOL {
                        pool.push(s);
                    }
                    return Ok(r);
                }
                Err(e) if fresh => return Err(e),
                Err(_) => continue,
            }
        }
        Err(other(format!("dns {}: failed", self.url)))
    }

    async fn one(&self, s: &mut BoxedStream, query: &[u8]) -> io::Result<Vec<u8>> {
        if self.kind == Kind::Https {
            let host = if self.port == 443 { self.host.clone() } else { format!("{}:{}", self.host, self.port) };
            let mut req = format!(
                "POST {} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/dns-message\r\nAccept: application/dns-message\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                self.path,
                query.len()
            )
            .into_bytes();
            req.extend_from_slice(query);
            crate::http::write_all(s, &req).await?;
            let resp = crate::http::read_response(s).await?;
            if resp.status != 200 {
                return Err(other(format!("dns {}: HTTP {}", self.url, resp.status)));
            }
            if resp.header("connection").is_some_and(|c| c.eq_ignore_ascii_case("close")) {
                return Err(other("connection closed")); // don't pool it
            }
            return Ok(resp.body);
        }
        let mut msg = (query.len() as u16).to_be_bytes().to_vec();
        msg.extend_from_slice(query);
        s.write_all(&msg).await?;
        s.flush().await?;
        let len = s.read_u16().await? as usize;
        let mut resp = vec![0u8; len];
        s.read_exact(&mut resp).await?;
        Ok(resp)
    }
}

/// One UDP query to `server` (over a pinned socket), waiting for the
/// matching id.
pub async fn udp_query(server: SocketAddr, query: &[u8]) -> io::Result<Vec<u8>> {
    let sock = crate::net::udp_socket(server).await?;
    sock.send_to(query, server).await?;
    let mut buf = vec![0u8; 4096];
    loop {
        let (n, from) = sock.recv_from(&mut buf).await?;
        if from.ip() == server.ip() && n >= 12 && wire::id(&buf[..n]) == wire::id(query) {
            return Ok(buf[..n].to_vec());
        }
    }
}

/// Resolves a DNS server hostname with the bootstrap (default) nameservers,
/// or the system resolver when none are configured.
pub async fn bootstrap_lookup(servers: &[SocketAddr], host: &str) -> io::Result<IpAddr> {
    if servers.is_empty() {
        let mut it = tokio::net::lookup_host((host, 0)).await?;
        return it.next().map(|a| a.ip()).ok_or_else(|| other(format!("cannot resolve {host}")));
    }
    let q = wire::query(rand::random(), host, wire::TYPE_A);
    let tries = servers.iter().map(|s| {
        let q = q.clone();
        Box::pin(async move {
            let r = tokio::time::timeout(TIMEOUT, udp_query(*s, &q)).await.map_err(|_| other("timeout"))??;
            let (ips, _) = wire::answers(&r).ok_or_else(|| other("bad response"))?;
            ips.first().map(|(ip, _)| *ip).ok_or_else(|| other(format!("{host}: no address")))
        })
    });
    futures::future::select_ok(tries).await.map(|(ip, _)| ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(spec: &str) -> Upstream {
        Upstream::parse(spec, Arc::new(vec![]), Arc::new(std::sync::OnceLock::new())).unwrap()
    }

    #[test]
    fn parses_mihomo_syntax() {
        let u = up("223.5.5.5");
        assert_eq!((u.kind, u.host.as_str(), u.port, u.detour.as_deref()), (Kind::Udp, "223.5.5.5", 53, None));
        let u = up("https://1.1.1.1/dns-query#PROXY");
        assert_eq!((u.kind, u.port, u.path.as_str(), u.detour.as_deref()), (Kind::Https, 443, "/dns-query", Some("PROXY")));
        let u = up("tls://dns.alidns.com");
        assert_eq!((u.kind, u.host.as_str(), u.port), (Kind::Tls, "dns.alidns.com", 853));
        let u = up("tcp://8.8.8.8:5353#DIRECT");
        assert_eq!((u.kind, u.port, u.detour.as_deref()), (Kind::Tcp, 5353, None));
        let u = up("https://doh.pub/dns-query#节点选择&h3=false");
        assert_eq!(u.detour.as_deref(), Some("节点选择"));
        assert!(Upstream::parse("quic://dns.adguard.com", Arc::new(vec![]), Arc::new(std::sync::OnceLock::new())).is_err());
    }
}
