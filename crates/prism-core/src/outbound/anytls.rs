//! AnyTLS client (protocol v2, compatible with anytls-go and sing-box).
//!
//! Wire format after the TLS handshake:
//!   auth:  sha256(password)[32] | padding_len u16 BE | padding
//!   frame: cmd u8 | stream_id u32 BE | data_len u16 BE | data
//!
//! Sessions (TLS connections) are reused like anytls-go does: a session
//! carries one stream at a time, goes back to an idle pool when its stream
//! ends, and is closed after sitting idle for [`IDLE_TIMEOUT`]. A new
//! connection on a pooled session costs no handshake round trips: SYN and
//! the destination are written immediately. Padding is optional for clients
//! and not used here.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use md5::{Digest as _, Md5};
use sha2::Sha256;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use super::tls::Tls;
use super::Outbound;
use crate::common::{other, pump, udp_over_stream, Address, BoxedStream, Framing, UdpPipe};
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

const MAX_FRAME: usize = 16 * 1024;
/// sing-box UDP-over-TCP v2: a stream to this address carries datagrams.
const UOT_MAGIC: &str = "sp.v2.udp-over-tcp.arpa";
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// anytls-go's default padding scheme; we only report its md5 so the server
/// does not need to push an update.
const DEFAULT_PADDING_SCHEME: &str = "stop=8\n0=30-30\n1=100-400\n2=400-500,c,500-1000,c,500-1000,c,500-1000,c,500-1000\n3=9-9,500-1000\n4=500-1000\n5=500-1000\n6=500-1000\n7=500-1000";

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

/// What the session reader delivers to a stream.
enum Event {
    Data(Vec<u8>),
    Error(String),
}

/// One TLS connection to the server.
struct Session {
    /// Frames to the writer task; taken (dropped) on close.
    out: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    /// Open streams; removing a sender signals FIN to that stream.
    streams: Mutex<HashMap<u32, mpsc::Sender<Event>>>,
    next_sid: AtomicU32,
    closed: AtomicBool,
    idle_since: Mutex<Instant>,
}

impl Session {
    fn sender(&self) -> Option<mpsc::Sender<Vec<u8>>> {
        if self.closed.load(Ordering::Acquire) {
            return None;
        }
        self.out.lock().unwrap().clone()
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Ends the session: the writer sends close_notify, the server then
    /// closes the connection and the reader exits.
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.out.lock().unwrap().take();
        self.streams.lock().unwrap().clear();
    }
}

type Pool = Arc<Mutex<Vec<Arc<Session>>>>;

pub struct AnyTls {
    name: String,
    server: String,
    port: u16,
    password_hash: [u8; 32],
    tls: Tls,
    idle: Pool,
    reaper_started: AtomicBool,
}

impl AnyTls {
    pub fn new(p: &ProxyConfig) -> io::Result<Self> {
        let password = p.password.clone().unwrap_or_default();
        Ok(AnyTls {
            name: p.name.clone(),
            server: p.server.clone(),
            port: p.port,
            password_hash: Sha256::digest(password.as_bytes()).into(),
            tls: Tls::new(p.sni.as_deref().unwrap_or(&p.server), p.skip_cert_verify, &p.alpn, p.fingerprint.as_deref())?,
            idle: Arc::new(Mutex::new(Vec::new())),
            reaper_started: AtomicBool::new(false),
        })
    }

    /// Most recently used idle session that is still open.
    fn take_idle(&self) -> Option<Arc<Session>> {
        let mut idle = self.idle.lock().unwrap();
        while let Some(s) = idle.pop() {
            let fresh = s.idle_since.lock().unwrap().elapsed() < IDLE_TIMEOUT;
            if !s.is_closed() && fresh {
                return Some(s);
            }
            s.close();
        }
        None
    }

    /// Periodically closes sessions that sat idle too long (servers and
    /// NATs drop quiet connections; a dead pooled session would stall the
    /// next stream).
    fn start_reaper(&self) {
        if self.reaper_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let pool: Weak<Mutex<Vec<Arc<Session>>>> = Arc::downgrade(&self.idle);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                let Some(pool) = pool.upgrade() else { return };
                pool.lock().unwrap().retain(|s| {
                    let keep = !s.is_closed() && s.idle_since.lock().unwrap().elapsed() < IDLE_TIMEOUT;
                    if !keep {
                        s.close();
                    }
                    keep
                });
            }
        });
    }

    async fn new_session(&self) -> io::Result<Arc<Session>> {
        self.start_reaper();
        let mut tls = self.tls.connect(&self.server, self.port).await?;
        let mut hello = Vec::with_capacity(160);
        hello.extend_from_slice(&self.password_hash);
        hello.extend_from_slice(&0u16.to_be_bytes());
        frame(&mut hello, CMD_SETTINGS, 0, client_settings().as_bytes());
        tls.write_all(&hello).await?;

        let (mut rd, mut wr) = tokio::io::split(tls);
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(256);
        let session = Arc::new(Session {
            out: Mutex::new(Some(tx)),
            streams: Mutex::new(HashMap::new()),
            next_sid: AtomicU32::new(1),
            closed: AtomicBool::new(false),
            idle_since: Mutex::new(Instant::now()),
        });

        // Writer: batches whatever is queued, flushes once per batch.
        let ws = Arc::downgrade(&session);
        tokio::spawn(async move {
            let res: io::Result<()> = async {
                while let Some(buf) = rx.recv().await {
                    wr.write_all(&buf).await?;
                    while let Ok(more) = rx.try_recv() {
                        wr.write_all(&more).await?;
                    }
                    wr.flush().await?;
                }
                wr.shutdown().await
            }
            .await;
            if res.is_err() {
                if let Some(s) = ws.upgrade() {
                    s.close();
                }
            }
        });

        // Reader: demultiplexes frames to streams.
        let rs = session.clone();
        tokio::spawn(async move {
            let res: io::Result<()> = async {
                let mut head = [0u8; 7];
                let mut data = vec![0u8; 65535];
                loop {
                    rd.read_exact(&mut head).await?;
                    let cmd = head[0];
                    let sid = u32::from_be_bytes([head[1], head[2], head[3], head[4]]);
                    let len = u16::from_be_bytes([head[5], head[6]]) as usize;
                    rd.read_exact(&mut data[..len]).await?;
                    let body = &data[..len];
                    match cmd {
                        CMD_PSH => {
                            let tx = rs.streams.lock().unwrap().get(&sid).cloned();
                            if let Some(tx) = tx {
                                // Backpressure: wait for the stream to take it.
                                let _ = tx.send(Event::Data(body.to_vec())).await;
                            }
                        }
                        CMD_FIN => {
                            rs.streams.lock().unwrap().remove(&sid);
                        }
                        CMD_SYNACK if !body.is_empty() => {
                            let tx = rs.streams.lock().unwrap().remove(&sid);
                            if let Some(tx) = tx {
                                let msg = format!("anytls: server dial failed: {}", String::from_utf8_lossy(body));
                                let _ = tx.send(Event::Error(msg)).await;
                            }
                        }
                        CMD_ALERT => {
                            return Err(other(format!("anytls: alert: {}", String::from_utf8_lossy(body))));
                        }
                        CMD_HEART_REQUEST => {
                            if let Some(out) = rs.sender() {
                                let mut f = Vec::with_capacity(7);
                                frame(&mut f, CMD_HEART_RESPONSE, sid, &[]);
                                let _ = out.send(f).await;
                            }
                        }
                        CMD_WASTE | CMD_SYNACK | CMD_UPDATE_PADDING | CMD_SERVER_SETTINGS | CMD_HEART_RESPONSE => {}
                        _ => {}
                    }
                }
            }
            .await;
            if let Err(e) = res {
                if e.kind() != io::ErrorKind::UnexpectedEof {
                    tracing::debug!("anytls session closed: {e}");
                }
            }
            rs.close();
        });
        Ok(session)
    }

    /// Opens stream `sid` on `session`: SYN + destination in one write.
    async fn open(session: &Arc<Session>, target: &Address) -> io::Result<(u32, mpsc::Receiver<Event>)> {
        let out = session.sender().ok_or_else(|| other("anytls: session closed"))?;
        let sid = session.next_sid.fetch_add(1, Ordering::Relaxed);
        let (etx, erx) = mpsc::channel(64);
        session.streams.lock().unwrap().insert(sid, etx);
        let mut dest = Vec::new();
        target.write_socks(&mut dest);
        let mut f = Vec::with_capacity(32 + dest.len());
        frame(&mut f, CMD_SYN, sid, &[]);
        frame(&mut f, CMD_PSH, sid, &dest);
        out.send(f).await.map_err(|_| other("anytls: session closed"))?;
        Ok((sid, erx))
    }
}

#[async_trait]
impl Outbound for AnyTls {
    fn name(&self) -> &str {
        &self.name
    }
    fn api_type(&self) -> &'static str {
        "AnyTLS"
    }
    fn udp(&self) -> bool {
        true
    }
    /// UDP via sing-box UDP-over-TCP v2 (what sing-box / mihomo AnyTLS
    /// clients use): request `is_connect u8 | socks addr`, then
    /// `u16 len | data` both ways.
    async fn connect_udp(&self, target: &Address) -> io::Result<UdpPipe> {
        let mut s = self.connect(&Address::Domain(UOT_MAGIC.into(), 0)).await?;
        let mut req = vec![1u8];
        target.write_socks(&mut req);
        s.write_all(&req).await?;
        Ok(udp_over_stream(s, Framing::Length))
    }
    async fn connect(&self, target: &Address) -> io::Result<BoxedStream> {
        // Prefer a pooled session; fall back to a fresh one if it turns out
        // to be closed.
        let (session, sid, mut events) = loop {
            match self.take_idle() {
                Some(s) => match Self::open(&s, target).await {
                    Ok((sid, ev)) => break (s, sid, ev),
                    Err(_) => s.close(),
                },
                None => {
                    let s = self.new_session().await?;
                    let (sid, ev) = Self::open(&s, target).await?;
                    break (s, sid, ev);
                }
            }
        };
        let pool = self.idle.clone();
        Ok(pump(move |local| async move {
            let (mut lrd, mut lwr) = tokio::io::split(local);
            let out = session.sender().ok_or_else(|| other("anytls: session closed"))?;
            let upload = async move {
                let mut buf = vec![0u8; MAX_FRAME];
                loop {
                    let n = lrd.read(&mut buf).await?;
                    let mut f = Vec::with_capacity(n + 7);
                    frame(&mut f, if n == 0 { CMD_FIN } else { CMD_PSH }, sid, &buf[..n]);
                    out.send(f).await.map_err(|_| other("anytls: session closed"))?;
                    if n == 0 {
                        return Ok::<_, io::Error>(());
                    }
                }
            };
            let download = async {
                // If the local side goes away, keep draining until the
                // server's FIN so the session stays clean for reuse.
                let mut local_open = true;
                while let Some(ev) = events.recv().await {
                    match ev {
                        Event::Data(d) => {
                            if local_open && lwr.write_all(&d).await.is_err() {
                                local_open = false;
                            }
                        }
                        // Stream-level failure; the session itself is fine.
                        Event::Error(e) => {
                            tracing::debug!("{e}");
                            break;
                        }
                    }
                }
                // Sender removed: FIN from the server, or the session died.
                if local_open {
                    let _ = lwr.shutdown().await;
                }
                if session.is_closed() {
                    return Err(other("anytls: session closed"));
                }
                Ok(())
            };
            let res = tokio::try_join!(upload, download);
            session.streams.lock().unwrap().remove(&sid);
            if res.is_ok() && !session.is_closed() {
                *session.idle_since.lock().unwrap() = Instant::now();
                pool.lock().unwrap().push(session);
            } else {
                session.close();
            }
            res.map(|_| ())
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
