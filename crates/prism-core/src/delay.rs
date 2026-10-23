//! Latency history and URL tests (`GET <url>` through an outbound).

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::common::{other, Address};
use crate::outbound::tls::Tls;
use crate::outbound::Registry;

#[derive(Clone, Serialize)]
pub struct DelayRecord {
    pub time: DateTime<Utc>,
    /// Milliseconds; 0 means the test failed.
    pub delay: u16,
}

#[derive(Default)]
pub struct DelayStore {
    map: Mutex<HashMap<String, VecDeque<DelayRecord>>>,
}

impl DelayStore {
    pub fn record(&self, name: &str, delay: u16) {
        let mut m = self.map.lock().unwrap();
        let h = m.entry(name.to_string()).or_default();
        h.push_back(DelayRecord { time: Utc::now(), delay });
        while h.len() > 10 {
            h.pop_front();
        }
    }

    pub fn history(&self, name: &str) -> Vec<DelayRecord> {
        self.map.lock().unwrap().get(name).map(|h| h.iter().cloned().collect()).unwrap_or_default()
    }

    pub fn last(&self, name: &str) -> Option<u16> {
        self.map.lock().unwrap().get(name).and_then(|h| h.back()).map(|r| r.delay)
    }

    /// Untested proxies count as alive.
    pub fn alive(&self, name: &str) -> bool {
        self.last(name) != Some(0)
    }
}

/// Measures the time for a full HTTP(S) request through `name` (including
/// the proxy handshake), records it and returns milliseconds.
pub async fn url_test(reg: &Registry, name: &str, url: &str, timeout: Duration) -> std::io::Result<u16> {
    let res = tokio::time::timeout(timeout, request(reg, name, url)).await;
    let res = match res {
        Ok(r) => r,
        Err(_) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout")),
    };
    if let Err(e) = &res {
        tracing::debug!("url test {name} {url}: {e}");
    }
    let delay = res.as_ref().map(|d| (*d).clamp(1, u16::MAX as u128) as u16).unwrap_or(0);
    reg.delays.record(name, delay);
    res.map(|_| delay)
}

async fn request(reg: &Registry, name: &str, url: &str) -> std::io::Result<u128> {
    let u = url::Url::parse(url).map_err(|e| other(e.to_string()))?;
    let host = u.host_str().ok_or_else(|| other("test url has no host"))?.to_string();
    let https = u.scheme() == "https";
    let port = u.port_or_known_default().unwrap_or(if https { 443 } else { 80 });
    let target = Address::new(&host, port);
    let path = match u.query() {
        Some(q) => format!("{}?{q}", u.path()),
        None => u.path().to_string(),
    };
    let req = |conn: &str| format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: prism\r\nConnection: {conn}\r\n\r\n");

    let start = Instant::now();
    let (out, _) = reg.resolve(name, &target)?;
    let stream = out.connect(&target).await?;
    if https {
        let s = Tls::new(&host, false, &["http/1.1".to_string()])?.wrap(stream).await?;
        exchange(s, start, reg.unified_delay, &req("keep-alive"), &req("close")).await
    } else {
        exchange(stream, start, reg.unified_delay, &req("keep-alive"), &req("close")).await
    }
}

/// Sends the test request. With `unified` (mihomo's unified-delay), a first
/// request warms the connection and only the second, keep-alive request is
/// timed, so handshakes (proxy and TLS) don't count.
async fn exchange<S>(mut s: S, start: Instant, unified: bool, warm: &str, timed: &str) -> std::io::Result<u128>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut start = start;
    if unified {
        s.write_all(warm.as_bytes()).await?;
        s.flush().await?;
        let (status, len) = read_head(&mut s).await?;
        // The body must be skippable to reuse the connection.
        let body = match (status, len) {
            (204 | 304, _) => Some(0),
            (_, Some(n)) if n <= 64 * 1024 => Some(n),
            _ => None,
        };
        match body {
            Some(n) => {
                let mut skip = vec![0u8; n];
                s.read_exact(&mut skip).await?;
                start = Instant::now();
            }
            None => return Ok(start.elapsed().as_millis()),
        }
    }
    s.write_all(timed.as_bytes()).await?;
    s.flush().await?;
    let mut buf = [0u8; 12];
    s.read_exact(&mut buf).await?;
    if !buf.starts_with(b"HTTP/") {
        return Err(other("unexpected response"));
    }
    Ok(start.elapsed().as_millis())
}

/// Reads a response head; returns (status, Content-Length).
async fn read_head<S: tokio::io::AsyncRead + Unpin>(s: &mut S) -> std::io::Result<(u16, Option<usize>)> {
    let mut head = Vec::with_capacity(512);
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 16 * 1024 {
            return Err(other("response head too long"));
        }
        head.push(s.read_u8().await?);
    }
    let text = String::from_utf8_lossy(&head);
    let mut lines = text.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    if !status_line.starts_with("HTTP/") {
        return Err(other("unexpected response"));
    }
    let status = status_line.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let len = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse().ok());
    Ok((status, len))
}
