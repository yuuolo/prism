//! DIRECT, REJECT and upstream SOCKS5 / HTTP proxies.

use std::io;

use async_trait::async_trait;
use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::tls::Tls;
use super::Outbound;
use crate::common::{invalid, other, tcp_connect, Address, BoxedStream};
use crate::config::ProxyConfig;

pub struct Direct;

#[async_trait]
impl Outbound for Direct {
    fn name(&self) -> &str {
        "DIRECT"
    }
    fn api_type(&self) -> &'static str {
        "Direct"
    }
    fn udp(&self) -> bool {
        true
    }
    async fn connect(&self, target: &Address) -> io::Result<BoxedStream> {
        let s = match target {
            Address::Ip(sa) => {
                let s = tokio::net::TcpStream::connect(sa).await?;
                let _ = s.set_nodelay(true);
                s
            }
            Address::Domain(d, p) => tcp_connect(d, *p).await?,
        };
        Ok(Box::new(s))
    }
}

pub struct Reject;

#[async_trait]
impl Outbound for Reject {
    fn name(&self) -> &str {
        "REJECT"
    }
    fn api_type(&self) -> &'static str {
        "Reject"
    }
    async fn connect(&self, _: &Address) -> io::Result<BoxedStream> {
        Err(io::Error::new(io::ErrorKind::ConnectionRefused, "rejected by rule"))
    }
}

async fn dial(server: &str, port: u16, tls: &Option<Tls>) -> io::Result<BoxedStream> {
    let tcp = tcp_connect(server, port).await?;
    Ok(match tls {
        Some(t) => Box::new(t.wrap(tcp).await?),
        None => Box::new(tcp),
    })
}

fn tls_of(p: &ProxyConfig) -> io::Result<Option<Tls>> {
    if !p.tls {
        return Ok(None);
    }
    Tls::new(p.sni.as_deref().unwrap_or(&p.server), p.skip_cert_verify, &p.alpn).map(Some)
}

pub struct Socks5 {
    name: String,
    server: String,
    port: u16,
    auth: Option<(String, String)>,
    tls: Option<Tls>,
}

impl Socks5 {
    pub fn new(p: &ProxyConfig) -> io::Result<Self> {
        Ok(Socks5 {
            name: p.name.clone(),
            server: p.server.clone(),
            port: p.port,
            auth: p.username.clone().map(|u| (u, p.password.clone().unwrap_or_default())),
            tls: tls_of(p)?,
        })
    }
}

#[async_trait]
impl Outbound for Socks5 {
    fn name(&self) -> &str {
        &self.name
    }
    fn api_type(&self) -> &'static str {
        "Socks5"
    }
    async fn connect(&self, target: &Address) -> io::Result<BoxedStream> {
        let mut s = dial(&self.server, self.port, &self.tls).await?;
        let method = if self.auth.is_some() { 2u8 } else { 0u8 };
        s.write_all(&[5, 1, method]).await?;
        let mut rep = [0u8; 2];
        s.read_exact(&mut rep).await?;
        if rep[0] != 5 || rep[1] != method {
            return Err(other("socks5: no acceptable auth method"));
        }
        if let Some((u, pw)) = &self.auth {
            let mut b = vec![1, u.len() as u8];
            b.extend_from_slice(u.as_bytes());
            b.push(pw.len() as u8);
            b.extend_from_slice(pw.as_bytes());
            s.write_all(&b).await?;
            s.read_exact(&mut rep).await?;
            if rep[1] != 0 {
                return Err(other("socks5: authentication failed"));
            }
        }
        let mut req = vec![5, 1, 0];
        target.write_socks(&mut req);
        s.write_all(&req).await?;
        let mut head = [0u8; 3];
        s.read_exact(&mut head).await?;
        if head[1] != 0 {
            return Err(other(format!("socks5: connect failed (code {})", head[1])));
        }
        Address::read_socks(&mut s).await?; // bound address
        Ok(s)
    }
}

pub struct Http {
    name: String,
    server: String,
    port: u16,
    auth: Option<String>,
    tls: Option<Tls>,
}

impl Http {
    pub fn new(p: &ProxyConfig) -> io::Result<Self> {
        Ok(Http {
            name: p.name.clone(),
            server: p.server.clone(),
            port: p.port,
            auth: p.username.as_ref().map(|u| {
                base64::engine::general_purpose::STANDARD
                    .encode(format!("{u}:{}", p.password.clone().unwrap_or_default()))
            }),
            tls: tls_of(p)?,
        })
    }
}

#[async_trait]
impl Outbound for Http {
    fn name(&self) -> &str {
        &self.name
    }
    fn api_type(&self) -> &'static str {
        "Http"
    }
    async fn connect(&self, target: &Address) -> io::Result<BoxedStream> {
        let mut s = dial(&self.server, self.port, &self.tls).await?;
        let host = match target {
            Address::Ip(sa) => sa.to_string(),
            Address::Domain(d, p) => format!("{d}:{p}"),
        };
        let mut req = format!("CONNECT {host} HTTP/1.1\r\nHost: {host}\r\n");
        if let Some(a) = &self.auth {
            req.push_str(&format!("Proxy-Authorization: Basic {a}\r\n"));
        }
        req.push_str("\r\n");
        s.write_all(req.as_bytes()).await?;
        // Read the response head byte by byte so no tunneled data is consumed.
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            if head.len() > 8192 {
                return Err(invalid("http proxy: response too long"));
            }
            head.push(s.read_u8().await?);
        }
        let line = String::from_utf8_lossy(&head);
        let status = line.split_whitespace().nth(1).unwrap_or("");
        if status != "200" {
            return Err(other(format!("http proxy: CONNECT returned {status}")));
        }
        Ok(s)
    }
}
