//! Trojan over TLS: hex(sha224(password)) CRLF cmd addr CRLF payload...

use std::io;

use async_trait::async_trait;
use sha2::{Digest, Sha224};
use tokio::io::AsyncWriteExt;

use super::tls::Tls;
use super::Outbound;
use crate::common::{tcp_connect, Address, BoxedStream};
use crate::config::ProxyConfig;

pub struct Trojan {
    name: String,
    server: String,
    port: u16,
    hash: String,
    tls: Tls,
}

impl Trojan {
    pub fn new(p: &ProxyConfig) -> io::Result<Self> {
        Ok(Trojan {
            name: p.name.clone(),
            server: p.server.clone(),
            port: p.port,
            hash: hex::encode(Sha224::digest(p.password.as_deref().unwrap_or("").as_bytes())),
            tls: Tls::new(p.sni.as_deref().unwrap_or(&p.server), p.skip_cert_verify, &p.alpn)?,
        })
    }
}

#[async_trait]
impl Outbound for Trojan {
    fn name(&self) -> &str {
        &self.name
    }
    fn api_type(&self) -> &'static str {
        "Trojan"
    }
    fn udp(&self) -> bool {
        false
    }
    async fn connect(&self, target: &Address) -> io::Result<BoxedStream> {
        let tcp = tcp_connect(&self.server, self.port).await?;
        let mut s = self.tls.wrap(tcp).await?;
        let mut head = Vec::with_capacity(128);
        head.extend_from_slice(self.hash.as_bytes());
        head.extend_from_slice(b"\r\n\x01");
        target.write_socks(&mut head);
        head.extend_from_slice(b"\r\n");
        s.write_all(&head).await?;
        s.flush().await?;
        Ok(Box::new(s))
    }
}
