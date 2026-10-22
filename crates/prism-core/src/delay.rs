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
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: prism\r\nConnection: close\r\n\r\n");

    let start = Instant::now();
    let (out, _) = reg.resolve(name, &target)?;
    let stream = out.connect(&target).await?;
    let mut buf = [0u8; 12];
    if https {
        let mut s = Tls::new(&host, false, &["http/1.1".to_string()])?.wrap(stream).await?;
        s.write_all(req.as_bytes()).await?;
        s.flush().await?;
        s.read_exact(&mut buf).await?;
    } else {
        let mut s = stream;
        s.write_all(req.as_bytes()).await?;
        s.flush().await?;
        s.read_exact(&mut buf).await?;
    }
    if !buf.starts_with(b"HTTP/") {
        return Err(other("unexpected response"));
    }
    Ok(start.elapsed().as_millis())
}
