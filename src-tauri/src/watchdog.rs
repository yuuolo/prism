//! Background recovery watches data/API health independently of the connections UI.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub stall_secs: u64,
    pub probe_urls: Vec<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            stall_secs: 120,
            probe_urls: vec![
                "https://www.gstatic.com/generate_204".into(),
                "https://www.cloudflare.com/cdn-cgi/trace".into(),
            ],
        }
    }
}
impl Settings {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (60..=3600).contains(&self.stall_secs),
            "无变化检测时间必须在 60–3600 秒之间"
        );
        let urls: Vec<_> = self
            .probe_urls
            .iter()
            .map(|u| u.trim())
            .filter(|u| !u.is_empty())
            .collect();
        anyhow::ensure!(
            !urls.is_empty() && urls.len() <= 4,
            "请配置 1–4 个联网探测地址"
        );
        for url in urls {
            let u = reqwest::Url::parse(url)?;
            anyhow::ensure!(
                matches!(u.scheme(), "https" | "http")
                    && u.host_str().is_some()
                    && u.username().is_empty()
                    && u.password().is_none(),
                "联网探测必须使用 HTTP(S) 地址且不能包含账户信息"
            );
        }
        Ok(())
    }
}
#[derive(Default)]
pub struct Intent {
    state: AtomicU64,
}
impl Intent {
    pub fn start(&self) -> u64 {
        let old = self
            .state
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                Some(v.wrapping_add(2) | 1)
            })
            .unwrap();
        old.wrapping_add(2) | 1
    }
    pub fn stop(&self) {
        let _ = self
            .state
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                Some(v.wrapping_add(2) & !1)
            });
    }
    pub fn ticket(&self) -> Option<u64> {
        let n = self.state.load(Ordering::SeqCst);
        (n & 1 == 1).then_some(n)
    }
    pub fn valid(&self, n: u64) -> bool {
        self.ticket() == Some(n)
    }
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Status {
    pub phase: String,
    pub message: String,
    pub restarts: u32,
    pub checked_at: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Sample {
    up: u64,
    down: u64,
    connections: Vec<(String, u64, u64)>,
}
impl Sample {
    fn parse(v: &Value) -> Option<Self> {
        let value = v.get("connections")?;
        let empty = Vec::new();
        let connections = if value.is_null() {
            &empty
        } else {
            value.as_array()?
        };
        let mut list = Vec::new();
        for c in connections {
            list.push((
                c.get("id")?.as_str()?.into(),
                c.get("upload")?.as_u64()?,
                c.get("download")?.as_u64()?,
            ));
        }
        list.sort();
        Some(Self {
            up: v.get("uploadTotal")?.as_u64()?,
            down: v.get("downloadTotal")?.as_u64()?,
            connections: list,
        })
    }
}
#[derive(Default)]
struct Policy {
    sample: Option<Sample>,
    unchanged_since: u64,
    dead: u32,
    api_failures: u32,
    probe_failures: u32,
    last_probe: Option<u64>,
    restarts: VecDeque<u64>,
}
impl Policy {
    fn observe(
        &mut self,
        now: u64,
        alive: bool,
        sample: Option<Sample>,
        stall: u64,
    ) -> Option<&'static str> {
        if !alive {
            self.dead += 1;
            return (self.dead >= 2).then_some("内核异常退出");
        }
        self.dead = 0;
        match sample {
            None => {
                self.api_failures += 1;
                (self.api_failures >= 2).then_some("内核 API 无响应")
            }
            Some(s) => {
                self.api_failures = 0;
                if self.sample.as_ref() != Some(&s) {
                    self.sample = Some(s);
                    self.unchanged_since = now;
                    return None;
                }
                (now.saturating_sub(self.unchanged_since) >= stall)
                    .then_some("连接和流量长时间无变化")
            }
        }
    }
    fn probe_due(&mut self, now: u64) -> bool {
        if self
            .last_probe
            .is_some_and(|last| now.saturating_sub(last) < 30)
        {
            return false;
        }
        self.last_probe = Some(now);
        true
    }
    fn probe_result(&mut self, healthy: bool) -> bool {
        if healthy {
            self.probe_failures = 0;
            false
        } else {
            self.probe_failures += 1;
            self.probe_failures >= 3
        }
    }
    fn restart_allowed(&mut self, now: u64) -> bool {
        while self
            .restarts
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= 600)
        {
            self.restarts.pop_front();
        }
        self.restarts.len() < 3
            && self
                .restarts
                .back()
                .is_none_or(|t| now.saturating_sub(*t) >= 60)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProbeResult {
    Healthy,
    ProxyFailed,
    TunFailed,
}
impl ProbeResult {
    fn reason(self) -> &'static str {
        match self {
            Self::Healthy => "联网探测正常",
            Self::ProxyFailed => "代理数据通路联网探测失败",
            Self::TunFailed => "代理端口正常，但系统 TUN/DNS 通路联网探测失败",
        }
    }
}

async fn probe_path(s: &crate::store::Settings, via_proxy: bool) -> bool {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(4));
    if via_proxy {
        let Ok(proxy) = reqwest::Proxy::all(format!("http://127.0.0.1:{}", s.mixed_port)) else {
            return false;
        };
        builder = builder.proxy(proxy);
    }
    let Ok(client) = builder.build() else {
        return false;
    };
    for url in s
        .recovery
        .probe_urls
        .iter()
        .map(|u| u.trim())
        .filter(|u| !u.is_empty())
    {
        // Receiving an HTTP response proves the data path is alive; a blocked
        // probe site's status alone is not evidence that the core is broken.
        if let Ok(response) = client.get(url).send().await {
            if response.status().as_u16() < 500 && response.status().as_u16() != 407 {
                return true;
            }
        }
    }
    false
}
async fn probe(s: &crate::store::Settings) -> ProbeResult {
    if !probe_path(s, true).await {
        return ProbeResult::ProxyFailed;
    }
    // A working mixed port does not prove the TUN routes, adapter or system DNS
    // work. This request uses system DNS/routing, with environment proxies off.
    // It never changes routes or relaxes the service's kill-switch filters.
    if s.tun && !probe_path(s, false).await {
        return ProbeResult::TunFailed;
    }
    ProbeResult::Healthy
}
fn status(st: &crate::AppState, phase: &str, message: &str) {
    let mut s = st.recovery_status.lock().unwrap();
    s.phase = phase.into();
    s.message = message.into();
    s.checked_at = chrono::Utc::now().timestamp();
}
fn diagnostic(app: &AppHandle, st: &crate::AppState, phase: &str, message: &str) {
    let changed = {
        let old = st.recovery_status.lock().unwrap();
        old.phase != phase || old.message != message
    };
    status(st, phase, message);
    if changed {
        st.core.push_log(app, format!("[prism] 自动恢复监测 [{phase}]：{message}"));
    }
}
pub async fn run(app: AppHandle) {
    let clock = Instant::now();
    let mut policy = Policy::default();
    let mut generation = None;
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let st = app.state::<crate::AppState>();
        let s = st.settings();
        let ticket = st.core_intent.ticket();
        if !s.recovery.enabled || ticket.is_none() || s.active_profile.is_none() {
            policy = Policy::default();
            generation = None;
            diagnostic(&app, &st, "idle", "未启用或已手动停止");
            continue;
        }
        let ticket = ticket.unwrap();
        let Ok(_guard) = st.lifecycle.try_lock() else {
            continue;
        };
        if !st.core_intent.valid(ticket) {
            continue;
        }
        let now = clock.elapsed().as_secs();
        let next_generation = (ticket, s.recovery.clone());
        if generation.as_ref() != Some(&next_generation) {
            generation = Some(next_generation);
            policy = Policy::default();
            st.core.push_log(&app, format!("[prism] 自动恢复监测：已启用，30 秒周期探测；TUN={}，断网保护={}，连续 3 次失败后恢复", s.tun, s.kill_switch));
        }
        let alive = tokio::time::timeout(Duration::from_secs(4), st.core.running())
            .await
            .ok()
            .flatten()
            .is_some();
        if alive && st.core.uptime_secs() < 30 {
            status(&st, "warming", "内核启动宽限期");
            continue;
        }
        let sample = if alive {
            tokio::time::timeout(
                Duration::from_secs(4),
                crate::core::api(&s, "GET", "/connections", None),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|v| Sample::parse(&v))
        } else {
            None
        };
        let observed = policy.observe(now, alive, sample, s.recovery.stall_secs);
        if !alive && observed.is_none() {
            diagnostic(&app, &st, "suspect", "内核状态不可用，等待下一轮确认");
            continue;
        }
        // Probe periodically even while counters/connections change: retries
        // and background traffic can keep a broken data path looking active.
        if alive && !policy.probe_due(now) {
            continue;
        }
        let health = probe(&s).await;
        if !st.core_intent.valid(ticket) || st.settings().recovery != s.recovery {
            continue;
        }
        let healthy = health == ProbeResult::Healthy;
        let confirmed = policy.probe_result(healthy);
        if healthy {
            let message = if observed == Some("连接和流量长时间无变化") {
                "连接和流量无变化，但联网探测正常"
            } else {
                "联网探测正常"
            };
            let recovering = st.recovery_status.lock().unwrap().phase != "healthy";
            if recovering {
                diagnostic(&app, &st, "healthy", "联网探测恢复正常");
            } else {
                status(&st, "healthy", message);
            }
            continue;
        }
        let reason = if alive { health.reason() } else { observed.unwrap() };
        if alive && !confirmed {
            diagnostic(&app, &st, "suspect", &format!("{}（连续 {} 次，等待确认）", reason, policy.probe_failures));
            continue;
        }
        if !st.core_intent.valid(ticket) {
            continue;
        }
        if !policy.restart_allowed(now) {
            diagnostic(&app, &st, "cooldown", "自动恢复已达频率限制，冷却后重试");
            continue;
        }
        policy.restarts.push_back(now);
        policy.probe_failures = 0;
        policy.sample = None;
        {
            st.recovery_status.lock().unwrap().restarts += 1;
        }
        status(&st, "restarting", reason);
        st.core
            .push_log(&app, format!("[prism] 自动恢复：{reason}，重新启动内核"));
        match crate::start_locked(&app, &st).await {
            Ok(()) => {
                status(&st, "warming", "内核已重启，等待恢复检测");
                st.core
                    .push_log(&app, "[prism] 自动恢复：内核重启成功".into());
            }
            Err(e) => {
                status(&st, "failed", &e);
                st.core.push_log(&app, format!("[prism] 自动恢复失败：{e}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Sample {
        Sample {
            up: 0,
            down: 0,
            connections: vec![],
        }
    }
    #[test]
    fn idle_requires_failed_probes_and_rate_is_bounded() {
        let mut p = Policy::default();
        assert!(p.observe(0, true, Some(sample()), 120).is_none());
        assert!(p.observe(119, true, Some(sample()), 120).is_none());
        assert!(p.observe(120, true, Some(sample()), 120).is_some());
        for _ in 0..10 {
            assert!(!p.probe_result(true));
        }
        assert!(!p.probe_result(false));
        assert!(!p.probe_result(false));
        assert!(p.probe_result(false));
        assert!(p.restart_allowed(120));
        p.restarts.push_back(120);
        assert!(!p.restart_allowed(179));
        assert!(p.restart_allowed(180));
        p.restarts.extend([180, 240]);
        assert!(!p.restart_allowed(400));
        assert!(p.restart_allowed(720));
    }
    #[test]
    fn null_connections_are_normal_idle_not_an_api_failure() {
        let s = Sample::parse(
            &serde_json::json!({"uploadTotal": 0, "downloadTotal": 0, "connections": null}),
        )
        .unwrap();
        assert_eq!(s, sample());
    }
    #[test]
    fn activity_and_manual_stop_cancel_stale_recovery() {
        let i = Intent::default();
        assert!(i.ticket().is_none());
        let ticket = i.start();
        assert!(i.valid(ticket));
        i.stop();
        assert!(!i.valid(ticket));
        let next = i.start();
        assert!(!i.valid(ticket));
        assert!(i.valid(next));
        let mut p = Policy::default();
        assert!(p.observe(0, false, None, 120).is_none());
        assert!(p.observe(10, false, None, 120).is_some());
        let mut s = sample();
        s.up = 1;
        assert!(p.observe(20, true, Some(s), 120).is_none());
    }
    #[tokio::test]
    async fn probes_use_the_local_proxy_and_server_errors_do_not_count_as_healthy() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (code, expected) in [(403, true), (503, false), (407, false)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let task = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 4096];
                let n = socket.read(&mut request).await.unwrap();
                assert!(
                    String::from_utf8_lossy(&request[..n]).starts_with("GET http://probe.invalid/")
                );
                socket.write_all(format!("HTTP/1.1 {code} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            });
            let mut s = crate::store::Settings::default();
            s.mixed_port = port;
            s.recovery.probe_urls = vec!["http://probe.invalid/".into()];
            assert_eq!(probe_path(&s, true).await, expected);
            task.await.unwrap();
        }
    }
    #[test]
    fn failed_probe_confirmation_survives_its_own_counter_changes() {
        let mut p = Policy::default();
        p.observe(0, true, Some(sample()), 120);
        assert!(!p.probe_result(false));
        let mut changed = sample();
        changed.up = 1;
        p.observe(130, true, Some(changed), 120);
        assert_eq!(p.probe_failures, 1);
        assert!(!p.probe_result(false));
        assert!(p.probe_result(false));
        assert!(!p.probe_result(true));
        assert_eq!(p.probe_failures, 0);
    }
    #[test]
    fn changing_connections_do_not_postpone_periodic_failure_detection() {
        let mut p = Policy::default();
        let mut confirmed_at = None;
        for now in (0..=60).step_by(10) {
            let mut s = sample();
            s.up = now + 1;
            s.connections.push((format!("retry-{now}"), 1, 0));
            assert!(p.observe(now, true, Some(s), 120).is_none());
            if p.probe_due(now) && p.probe_result(false) {
                confirmed_at = Some(now);
            }
        }
        assert_eq!(confirmed_at, Some(60));
    }
    #[tokio::test]
    async fn tun_health_requires_system_path_even_when_proxy_port_works() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        async fn responder(code: u16) -> (u16, tokio::task::JoinHandle<()>) {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let task = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                socket.read(&mut request).await.unwrap();
                socket.write_all(format!("HTTP/1.1 {code} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            });
            (port, task)
        }
        for (tun, system_code, expected) in [
            (false, 503, ProbeResult::Healthy),
            (true, 503, ProbeResult::TunFailed),
            (true, 204, ProbeResult::Healthy),
        ] {
            let (proxy_port, proxy) = responder(204).await;
            let (system_port, system) = responder(system_code).await;
            let mut s = crate::store::Settings::default();
            s.tun = tun;
            s.mixed_port = proxy_port;
            s.recovery.probe_urls = vec![format!("http://127.0.0.1:{system_port}/health")];
            assert_eq!(probe(&s).await, expected);
            proxy.await.unwrap();
            if tun { system.await.unwrap(); } else { system.abort(); }
        }
    }
}
