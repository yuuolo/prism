//! VLESS over TCP, TLS or REALITY, with optional XTLS Vision
//! (`flow: xtls-rprx-vision`, TCP only).
//!
//! Request:  ver(0) uuid[16] addons_len addons cmd(1=tcp, 2=udp) port u16 BE atyp addr
//!           addons: protobuf, field 1 = flow
//! Response: ver addons_len addons... then payload (Vision: padded at first).

use std::io;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::ctls::{self, Reality};
use super::tls::Tls;
use super::vision::VisionStream;
use super::Outbound;
use crate::common::{pump, tcp_connect, udp_over_stream, Address, BoxedStream, Framing, UdpPipe};
use crate::config::ProxyConfig;

const VISION: &str = "xtls-rprx-vision";

enum Transport {
    Plain,
    Tls(Tls),
    /// The Chrome-style client: REALITY, or TLS when Vision needs to reach
    /// the raw socket.
    Ctls(ctls::Config),
}

pub struct Vless {
    name: String,
    server: String,
    port: u16,
    uuid: [u8; 16],
    vision: bool,
    transport: Transport,
}

impl Vless {
    pub fn new(p: &ProxyConfig) -> anyhow::Result<Self> {
        let vision = match p.flow.as_deref() {
            None | Some("") => false,
            Some(f) if f == VISION || f == "xtls-rprx-vision-udp443" => true,
            Some(f) => anyhow::bail!("{}: VLESS flow {f} is not supported (only {VISION})", p.name),
        };
        let uuid = uuid::Uuid::parse_str(p.uuid.as_deref().unwrap_or("")).map_err(|_| anyhow::anyhow!("{}: invalid uuid", p.name))?;
        let sni = p.sni.clone().unwrap_or_else(|| p.server.clone());
        let alpn = if p.alpn.is_empty() { vec!["h2".to_string(), "http/1.1".to_string()] } else { p.alpn.clone() };
        let transport = if let Some(pk) = &p.reality_public_key {
            let reality = Reality::parse(pk, p.reality_short_id.as_deref().unwrap_or(""))
                .ok_or_else(|| anyhow::anyhow!("{}: invalid reality-opts public-key / short-id", p.name))?;
            Transport::Ctls(ctls::Config { server_name: sni, alpn, skip_verify: true, reality: Some(reality) })
        } else if p.tls && vision {
            Transport::Ctls(ctls::Config { server_name: sni, alpn, skip_verify: p.skip_cert_verify, reality: None })
        } else if p.tls {
            Transport::Tls(Tls::new(&sni, p.skip_cert_verify, &p.alpn, p.fingerprint.as_deref())?)
        } else {
            if vision {
                anyhow::bail!("{}: {VISION} needs TLS or REALITY", p.name);
            }
            Transport::Plain
        };
        Ok(Vless { name: p.name.clone(), server: p.server.clone(), port: p.port, uuid: *uuid.as_bytes(), vision, transport })
    }

    async fn dial(&self) -> io::Result<BoxedStream> {
        match &self.transport {
            Transport::Plain => Ok(Box::new(tcp_connect(&self.server, self.port).await?)),
            Transport::Tls(t) => t.connect(&self.server, self.port).await,
            Transport::Ctls(cfg) => Ok(Box::new(ctls::connect(tcp_connect(&self.server, self.port).await?, cfg).await?)),
        }
    }
}

/// Request header; `cmd` 1 = TCP, 2 = UDP.
fn header(uuid: &[u8; 16], flow: Option<&str>, cmd: u8, target: &Address) -> Vec<u8> {
    let mut h = vec![0u8];
    h.extend_from_slice(uuid);
    match flow {
        Some(f) => {
            let addons_len = 2 + f.len();
            h.push(addons_len as u8);
            h.push(0x0a); // field 1, length-delimited
            h.push(f.len() as u8);
            h.extend_from_slice(f.as_bytes());
        }
        None => h.push(0),
    }
    h.push(cmd);
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

async fn finish<S>(mut s: S, head: Vec<u8>, skip_response: bool) -> io::Result<BoxedStream>
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
            if skip_response {
                let mut resp = [0u8; 2];
                rd.read_exact(&mut resp).await?;
                let mut addons = vec![0u8; resp[1] as usize];
                rd.read_exact(&mut addons).await?;
            }
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
    /// Vision servers refuse plain UDP requests (they expect XUDP over mux,
    /// which prism-core doesn't speak).
    fn udp(&self) -> bool {
        !self.vision
    }
    async fn connect(&self, target: &Address) -> io::Result<BoxedStream> {
        if self.vision {
            let Transport::Ctls(cfg) = &self.transport else { unreachable!() };
            let tls = ctls::connect(tcp_connect(&self.server, self.port).await?, cfg).await?;
            // The header goes out plain (Vision pads what follows); the
            // stream skips the response header itself.
            let mut v = VisionStream::new(tls, self.uuid);
            v.write_header(&header(&self.uuid, Some(VISION), 1, target)).await?;
            return finish(v, Vec::new(), false).await;
        }
        let s = self.dial().await?;
        finish(s, header(&self.uuid, None, 1, target), true).await
    }
    async fn connect_udp(&self, target: &Address) -> io::Result<UdpPipe> {
        if self.vision {
            return Err(io::Error::new(io::ErrorKind::Unsupported, format!("{}: {VISION} doesn't carry UDP", self.name)));
        }
        let mut s = match &self.transport {
            Transport::Ctls(cfg) => Box::new(ctls::connect(tcp_connect(&self.server, self.port).await?, cfg).await?) as BoxedStream,
            _ => self.dial().await?,
        };
        s.write_all(&header(&self.uuid, None, 2, target)).await?;
        s.flush().await?;
        Ok(udp_over_stream(s, Framing::Vless))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_with_flow() {
        let h = header(&[1; 16], Some(VISION), 1, &Address::Domain("a.com".into(), 443));
        assert_eq!(h[17], 18);
        assert_eq!(&h[18..20], &[0x0a, 16]);
        assert_eq!(&h[20..36], VISION.as_bytes());
        assert_eq!(&h[36..40], &[1, 1, 187, 2]);
        let plain = header(&[1; 16], None, 2, &Address::Domain("a.com".into(), 53));
        assert_eq!(plain[17], 0);
    }
}
