//! VLESS over TCP (+TLS). No flow / REALITY / transports yet.
//!
//! Request:  ver(0) uuid[16] addons_len(0) cmd(1=tcp) port u16 BE atyp addr
//! Response: ver addons_len addons... then raw payload.

use std::io;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::tls::Tls;
use super::Outbound;
use crate::common::{pump, tcp_connect, Address, BoxedStream};
use crate::config::ProxyConfig;

pub struct Vless {
    name: String,
    server: String,
    port: u16,
    uuid: [u8; 16],
    tls: Option<Tls>,
}

impl Vless {
    pub fn new(p: &ProxyConfig) -> anyhow::Result<Self> {
        if p.flow.is_some() || p.reality_public_key.is_some() {
            anyhow::bail!("{}: VLESS flow/REALITY is not supported by prism-core yet", p.name);
        }
        let uuid = uuid::Uuid::parse_str(p.uuid.as_deref().unwrap_or(""))
            .map_err(|_| anyhow::anyhow!("{}: invalid uuid", p.name))?;
        Ok(Vless {
            name: p.name.clone(),
            server: p.server.clone(),
            port: p.port,
            uuid: *uuid.as_bytes(),
            tls: if p.tls {
                Some(Tls::new(p.sni.as_deref().unwrap_or(&p.server), p.skip_cert_verify, &p.alpn)?)
            } else {
                None
            },
        })
    }
}

fn header(uuid: &[u8; 16], target: &Address) -> Vec<u8> {
    let mut h = vec![0u8];
    h.extend_from_slice(uuid);
    h.push(0); // addons
    h.push(1); // tcp
    h.extend_from_slice(&target.port().to_be_bytes());
    match target {
        Address::Ip(std::net::SocketAddr::V4(sa)) => {
            h.push(1);
            h.extend_from_slice(&sa.ip().octets());
        }
        Address::Domain(d, _) => {
            h.push(2);
            h.push(d.len() as u8);
            h.extend_from_slice(d.as_bytes());
        }
        Address::Ip(std::net::SocketAddr::V6(sa)) => {
            h.push(3);
            h.extend_from_slice(&sa.ip().octets());
        }
    }
    h
}

async fn finish<S>(mut s: S, head: Vec<u8>) -> io::Result<BoxedStream>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    s.write_all(&head).await?;
    s.flush().await?;
    let (mut rd, mut wr) = tokio::io::split(s);
    Ok(pump(move |local| async move {
        let (mut lrd, mut lwr) = tokio::io::split(local);
        let upload = async move {
            tokio::io::copy(&mut lrd, &mut wr).await?;
            wr.shutdown().await
        };
        let download = async move {
            let mut resp = [0u8; 2];
            rd.read_exact(&mut resp).await?;
            let mut addons = vec![0u8; resp[1] as usize];
            rd.read_exact(&mut addons).await?;
            tokio::io::copy(&mut rd, &mut lwr).await?;
            lwr.shutdown().await
        };
        tokio::try_join!(upload, download)?;
        Ok(())
    }))
}

#[async_trait]
impl Outbound for Vless {
    fn name(&self) -> &str {
        &self.name
    }
    fn api_type(&self) -> &'static str {
        "Vless"
    }
    async fn connect(&self, target: &Address) -> io::Result<BoxedStream> {
        let tcp = tcp_connect(&self.server, self.port).await?;
        let head = header(&self.uuid, target);
        match &self.tls {
            Some(t) => finish(t.wrap(tcp).await?, head).await,
            None => finish(tcp, head).await,
        }
    }
}
