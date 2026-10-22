//! Routes inbound connections to outbounds and relays traffic, keeping the
//! per-connection statistics exposed by the Clash API.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::common::{Address, BoxedStream, ProxyStream};
use crate::config::Mode;
use crate::outbound::Registry;
use crate::rules::Router;

#[derive(Clone, Debug)]
pub struct Metadata {
    /// Inbound protocol: HTTP, HTTPS (CONNECT) or Socks5.
    pub inbound: &'static str,
    pub src: SocketAddr,
    pub dst: Address,
    /// Resolved destination IP (only when an IP rule needed it).
    pub dst_ip: Option<IpAddr>,
}

pub struct Conn {
    pub id: String,
    pub meta: Metadata,
    pub chains: Vec<String>,
    pub rule: String,
    pub rule_payload: String,
    pub start: DateTime<Utc>,
    pub upload: AtomicU64,
    pub download: AtomicU64,
    cancel: CancellationToken,
}

#[derive(Default)]
pub struct Stats {
    pub upload: AtomicU64,
    pub download: AtomicU64,
    pub conns: Mutex<HashMap<String, Arc<Conn>>>,
}

impl Stats {
    pub fn snapshot(&self) -> Vec<Arc<Conn>> {
        let mut v: Vec<_> = self.conns.lock().unwrap().values().cloned().collect();
        v.sort_by_key(|c| c.start);
        v
    }

    pub fn close(&self, id: &str) -> bool {
        match self.conns.lock().unwrap().get(id) {
            Some(c) => {
                c.cancel.cancel();
                true
            }
            None => false,
        }
    }

    pub fn close_all(&self) {
        for c in self.conns.lock().unwrap().values() {
            c.cancel.cancel();
        }
    }
}

pub struct Dialed {
    pub stream: BoxedStream,
    pub chains: Vec<String>,
    pub rule: String,
    pub rule_payload: String,
}

pub struct Tunnel {
    pub reg: Arc<Registry>,
    pub router: Router,
    pub mode: RwLock<Mode>,
    pub stats: Arc<Stats>,
}

impl Tunnel {
    pub fn mode(&self) -> Mode {
        *self.mode.read().unwrap()
    }

    /// Picks the outbound name for a connection: (target, rule, payload).
    async fn target(&self, meta: &mut Metadata) -> (String, String, String) {
        match self.mode() {
            Mode::Direct => ("DIRECT".to_string(), "Direct".to_string(), String::new()),
            Mode::Global => ("GLOBAL".to_string(), "Global".to_string(), String::new()),
            Mode::Rule => match self.router.route(meta).await {
                Some(r) => (r.target.clone(), r.display(), r.payload.clone()),
                None => ("DIRECT".to_string(), "Match".to_string(), String::new()),
            },
        }
    }

    /// UDP is only relayed for connections the rules send to DIRECT
    /// (proxy outbounds don't carry UDP yet).
    pub async fn udp_direct(&self, meta: &mut Metadata) -> bool {
        let (target, rule, _) = self.target(meta).await;
        let leaf = self.reg.resolve(&target, &meta.dst).map(|(o, _)| o.name().to_string()).unwrap_or_default();
        let ok = leaf == "DIRECT";
        tracing::debug!("[UDP] {} --> {} match {rule} using {target}{}", meta.src, meta.dst, if ok { "" } else { " (dropped: UDP proxy unsupported)" });
        ok
    }

    pub async fn dial(&self, meta: &mut Metadata) -> io::Result<Dialed> {
        let (target, rule, payload) = self.target(meta).await;
        let (out, chain) = self.reg.resolve(&target, &meta.dst)?;
        let shown = if chain.len() > 1 { format!("{target}[{}]", chain.last().unwrap()) } else { target.clone() };
        let res = tokio::time::timeout(Duration::from_secs(10), out.connect(&meta.dst)).await;
        let stream = match res {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                tracing::warn!("[TCP] {} --> {} match {rule} using {shown} failed: {e}", meta.src, meta.dst);
                return Err(e);
            }
            Err(_) => {
                tracing::warn!("[TCP] {} --> {} match {rule} using {shown} failed: timeout", meta.src, meta.dst);
                return Err(io::Error::new(io::ErrorKind::TimedOut, "dial timeout"));
            }
        };
        tracing::info!("[TCP] {} --> {} match {rule} using {shown}", meta.src, meta.dst);
        let mut chains = chain;
        chains.reverse(); // leaf first, like mihomo
        Ok(Dialed { stream, chains, rule, rule_payload: payload })
    }

    /// Relays until both directions finish or the connection is closed via API.
    pub async fn relay<C: ProxyStream>(&self, client: C, dialed: Dialed, meta: Metadata, initial: &[u8]) {
        let conn = Arc::new(Conn {
            id: uuid::Uuid::new_v4().to_string(),
            meta,
            chains: dialed.chains,
            rule: dialed.rule,
            rule_payload: dialed.rule_payload,
            start: Utc::now(),
            upload: AtomicU64::new(0),
            download: AtomicU64::new(0),
            cancel: CancellationToken::new(),
        });
        self.stats.conns.lock().unwrap().insert(conn.id.clone(), conn.clone());

        let (cr, cw) = tokio::io::split(client);
        let (rr, mut rw) = tokio::io::split(dialed.stream);
        let stats = self.stats.clone();
        let work = async {
            if !initial.is_empty() {
                rw.write_all(initial).await?;
                rw.flush().await?;
                conn.upload.fetch_add(initial.len() as u64, Ordering::Relaxed);
                stats.upload.fetch_add(initial.len() as u64, Ordering::Relaxed);
            }
            tokio::try_join!(
                copy_count(cr, rw, &conn.upload, &stats.upload),
                copy_count(rr, cw, &conn.download, &stats.download),
            )
        };
        tokio::select! {
            r = work => {
                if let Err(e) = r {
                    tracing::debug!("relay {}: {e}", conn.meta.dst);
                }
            }
            _ = conn.cancel.cancelled() => {}
        }
        self.stats.conns.lock().unwrap().remove(&conn.id);
    }
}

async fn copy_count<R, W>(mut r: R, mut w: W, conn: &AtomicU64, total: &AtomicU64) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; 32 * 1024];
    loop {
        let n = r.read(&mut buf).await?;
        if n == 0 {
            let _ = w.shutdown().await;
            return Ok(());
        }
        w.write_all(&buf[..n]).await?;
        w.flush().await?;
        conn.fetch_add(n as u64, Ordering::Relaxed);
        total.fetch_add(n as u64, Ordering::Relaxed);
    }
}
