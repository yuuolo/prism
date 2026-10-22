//! AnyTLS client (protocol v2, compatible with anytls-go and sing-box).
//!
//! Wire format after the TLS handshake:
//!   auth:  sha256(password)[32] | padding_len u16 BE | padding
//!   frame: cmd u8 | stream_id u32 BE | data_len u16 BE | data
//!
//! We open one TLS connection per proxied stream (stream id 1): send the
//! client settings, SYN, then a PSH carrying the SOCKS-style destination,
//! followed by PSH frames with payload. Padding is optional for clients and
//! not used here.

use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use md5::{Digest as _, Md5};
use sha2::Sha256;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

use super::tls::Tls;
use super::Outbound;
use crate::common::{other, pump, tcp_connect, Address, BoxedStream};
use crate::config::ProxyConfig;

const CMD_WASTE: u8 = 0;
const CMD_SYN: u8 = 1;
const CMD_PSH: u8 = 2;
const CMD_FIN: u8 = 3;
const CMD_SETTINGS: u8 = 4;
const CMD_ALERT: u8 = 5;
const CMD_UPDATE_PADDING: u8 = 6;
const CMD_SYNACK: u8 = 7;
const CMD_HEART_REQUEST: u8 = 8;
const CMD_HEART_RESPONSE: u8 = 9;
const CMD_SERVER_SETTINGS: u8 = 10;

const SID: u32 = 1;
const MAX_FRAME: usize = 16 * 1024;

/// anytls-go's default padding scheme; we only report its md5 so the server
/// does not need to push an update.
const DEFAULT_PADDING_SCHEME: &str = "stop=8\n0=30-30\n1=100-400\n2=400-500,c,500-1000,c,500-1000,c,500-1000,c,500-1000\n3=9-9,500-1000\n4=500-1000\n5=500-1000\n6=500-1000\n7=500-1000";

pub struct AnyTls {
    name: String,
    server: String,
    port: u16,
    password_hash: [u8; 32],
    tls: Tls,
}

impl AnyTls {
    pub fn new(p: &ProxyConfig) -> io::Result<Self> {
        let password = p.password.clone().unwrap_or_default();
        let alpn = if p.alpn.is_empty() { vec![] } else { p.alpn.clone() };
        Ok(AnyTls {
            name: p.name.clone(),
            server: p.server.clone(),
            port: p.port,
            password_hash: Sha256::digest(password.as_bytes()).into(),
            tls: Tls::new(p.sni.as_deref().unwrap_or(&p.server), p.skip_cert_verify, &alpn)?,
        })
    }
}

fn frame(buf: &mut Vec<u8>, cmd: u8, sid: u32, data: &[u8]) {
    buf.push(cmd);
    buf.extend_from_slice(&sid.to_be_bytes());
    buf.extend_from_slice(&(data.len() as u16).to_be_bytes());
    buf.extend_from_slice(data);
}

pub fn client_settings() -> String {
    let md5 = hex::encode(Md5::digest(DEFAULT_PADDING_SCHEME.as_bytes()));
    format!("v=2\nclient=prism/{}\npadding-md5={md5}", env!("CARGO_PKG_VERSION"))
}

#[async_trait]
impl Outbound for AnyTls {
    fn name(&self) -> &str {
        &self.name
    }
    fn api_type(&self) -> &'static str {
        "AnyTLS"
    }
    async fn connect(&self, target: &Address) -> io::Result<BoxedStream> {
        let tcp = tcp_connect(&self.server, self.port).await?;
        let mut tls = self.tls.wrap(tcp).await?;

        // auth + settings + SYN + destination in a single write
        let mut hello = Vec::with_capacity(256);
        hello.extend_from_slice(&self.password_hash);
        hello.extend_from_slice(&0u16.to_be_bytes());
        frame(&mut hello, CMD_SETTINGS, 0, client_settings().as_bytes());
        frame(&mut hello, CMD_SYN, SID, &[]);
        let mut dest = Vec::new();
        target.write_socks(&mut dest);
        frame(&mut hello, CMD_PSH, SID, &dest);
        tls.write_all(&hello).await?;
        tls.flush().await?;

        let (mut rd, wr) = tokio::io::split(tls);
        let wr = Arc::new(Mutex::new(wr));
        Ok(pump(move |local| async move {
            let (mut lrd, mut lwr) = tokio::io::split(local);
            let wr_up = wr.clone();
            let upload = async move {
                let mut buf = vec![0u8; MAX_FRAME];
                let mut out = Vec::with_capacity(MAX_FRAME + 7);
                loop {
                    let n = lrd.read(&mut buf).await?;
                    out.clear();
                    if n == 0 {
                        frame(&mut out, CMD_FIN, SID, &[]);
                    } else {
                        frame(&mut out, CMD_PSH, SID, &buf[..n]);
                    }
                    let mut w = wr_up.lock().await;
                    w.write_all(&out).await?;
                    w.flush().await?;
                    if n == 0 {
                        return Ok::<_, io::Error>(());
                    }
                }
            };
            let download = async move {
                let mut head = [0u8; 7];
                let mut data = vec![0u8; 65535];
                loop {
                    if let Err(e) = rd.read_exact(&mut head).await {
                        let _ = lwr.shutdown().await;
                        return if e.kind() == io::ErrorKind::UnexpectedEof { Ok(()) } else { Err(e) };
                    }
                    let cmd = head[0];
                    let sid = u32::from_be_bytes([head[1], head[2], head[3], head[4]]);
                    let len = u16::from_be_bytes([head[5], head[6]]) as usize;
                    rd.read_exact(&mut data[..len]).await?;
                    let body = &data[..len];
                    match cmd {
                        CMD_PSH if sid == SID => {
                            lwr.write_all(body).await?;
                        }
                        CMD_FIN if sid == SID => {
                            let _ = lwr.shutdown().await;
                            return Ok(());
                        }
                        CMD_SYNACK if !body.is_empty() => {
                            return Err(other(format!("anytls: server dial failed: {}", String::from_utf8_lossy(body))));
                        }
                        CMD_ALERT => {
                            return Err(other(format!("anytls: alert: {}", String::from_utf8_lossy(body))));
                        }
                        CMD_HEART_REQUEST => {
                            let mut out = Vec::new();
                            frame(&mut out, CMD_HEART_RESPONSE, sid, &[]);
                            let mut w = wr.lock().await;
                            w.write_all(&out).await?;
                            w.flush().await?;
                        }
                        CMD_WASTE | CMD_SYNACK | CMD_UPDATE_PADDING | CMD_SERVER_SETTINGS | CMD_HEART_RESPONSE => {}
                        _ => {}
                    }
                }
            };
            tokio::try_join!(upload, download)?;
            Ok(())
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_layout() {
        let mut b = Vec::new();
        frame(&mut b, CMD_PSH, 1, b"hi");
        assert_eq!(b, [2, 0, 0, 0, 1, 0, 2, b'h', b'i']);
        assert!(client_settings().starts_with("v=2\nclient=prism/"));
    }
}
