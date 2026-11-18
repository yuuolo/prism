//! Process-lifetime TUN telemetry. No addresses, domains, packets or secrets.
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

pub const SNAPSHOT_FILE: &str = "tun-health.json";
#[derive(Clone, Copy)]
pub enum Pump { Receive = 0, ToStack = 1, FromStack = 2, Send = 3, Runner = 4, Tcp = 5, Udp = 6 }
impl Pump {
    fn name(self) -> &'static str { ["adapter_rx", "to_stack", "from_stack", "adapter_tx", "stack_runner", "tcp_accept", "udp_accept"][self as usize] }
}
#[derive(Clone, Serialize)]
pub struct Fault { pub stage: &'static str, pub reason: &'static str, pub os_error: Option<i32> }
#[derive(Serialize)]
pub struct Snapshot {
    pub schema: u32, pub version: &'static str, pub process_id: u32,
    pub captured_at: u64, pub uptime_ms: u64, pub enabled: bool,
    pub running_pumps: u64, pub rx_packets: u64, pub rx_bytes: u64,
    pub tx_packets: u64, pub tx_bytes: u64, pub ring_full_drops: u64,
    pub rx_queue_high_water: u64, pub tx_queue_high_water: u64,
    pub max_queue_wait_ms: u64, pub last_rx_ms: u64, pub last_tx_ms: u64,
    pub fault: Option<Fault>,
    pub working_set_bytes: Option<u64>, pub private_bytes: Option<u64>,
    pub process_handles: Option<u32>, pub cpu_time_100ns: Option<u64>,
}
pub struct Health {
    started: Instant, enabled: AtomicBool, stopping: AtomicBool,
    running: AtomicU64, rx_packets: AtomicU64, rx_bytes: AtomicU64,
    tx_packets: AtomicU64, tx_bytes: AtomicU64, ring_full: AtomicU64,
    rx_high: AtomicU64, tx_high: AtomicU64, wait_max: AtomicU64,
    last_rx: AtomicU64, last_tx: AtomicU64, fault: Mutex<Option<Fault>>,
    pub failed: CancellationToken,
}
impl Default for Health {
    fn default() -> Self { Self {
        started: Instant::now(), enabled: false.into(), stopping: false.into(),
        running: 0.into(), rx_packets: 0.into(), rx_bytes: 0.into(), tx_packets: 0.into(), tx_bytes: 0.into(),
        ring_full: 0.into(), rx_high: 0.into(), tx_high: 0.into(), wait_max: 0.into(),
        last_rx: 0.into(), last_tx: 0.into(), fault: Mutex::new(None), failed: CancellationToken::new(),
    } }
}
impl Health {
    pub fn enable(&self) { self.enabled.store(true, Ordering::Release); }
    pub fn stopping(&self) { self.stopping.store(true, Ordering::Release); }
    pub fn guard(self: &Arc<Self>, pump: Pump) -> Guard {
        self.running.fetch_or(1 << pump as u8, Ordering::Relaxed);
        Guard { health: self.clone(), pump }
    }
    pub fn fail(&self, pump: Pump, reason: &'static str, os_error: Option<i32>) {
        if self.stopping.load(Ordering::Acquire) { return; }
        let mut fault = self.fault.lock().unwrap_or_else(|e| e.into_inner());
        if fault.is_none() {
            *fault = Some(Fault { stage: pump.name(), reason, os_error });
            tracing::error!("TUN_HEALTH_FATAL stage={} reason={reason} os_error={os_error:?}", pump.name());
            self.failed.cancel();
        }
    }
    pub fn received(&self, bytes: usize) {
        self.rx_packets.fetch_add(1, Ordering::Relaxed); self.rx_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        self.last_rx.store(self.elapsed(), Ordering::Relaxed);
    }
    pub fn sent(&self, bytes: usize) {
        self.tx_packets.fetch_add(1, Ordering::Relaxed); self.tx_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        self.last_tx.store(self.elapsed(), Ordering::Relaxed);
    }
    pub fn ring_full(&self) { self.ring_full.fetch_add(1, Ordering::Relaxed); }
    pub fn queue(&self, receive: bool, depth: usize, waited: std::time::Duration) {
        let high = if receive { &self.rx_high } else { &self.tx_high };
        high.fetch_max(depth as u64, Ordering::Relaxed);
        self.wait_max.fetch_max(waited.as_millis().min(u64::MAX as u128) as u64, Ordering::Relaxed);
    }
    fn elapsed(&self) -> u64 { self.started.elapsed().as_millis().min(u64::MAX as u128) as u64 }
    pub fn snapshot(&self) -> Snapshot {
        let get = |a: &AtomicU64| a.load(Ordering::Relaxed);
        let (working_set_bytes, private_bytes, process_handles, cpu_time_100ns) = resources();
        Snapshot { schema: 1, version: env!("CARGO_PKG_VERSION"), process_id: std::process::id(),
            captured_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
            uptime_ms: self.elapsed(), enabled: self.enabled.load(Ordering::Acquire), running_pumps: get(&self.running),
            rx_packets: get(&self.rx_packets), rx_bytes: get(&self.rx_bytes), tx_packets: get(&self.tx_packets), tx_bytes: get(&self.tx_bytes),
            ring_full_drops: get(&self.ring_full), rx_queue_high_water: get(&self.rx_high), tx_queue_high_water: get(&self.tx_high),
            max_queue_wait_ms: get(&self.wait_max), last_rx_ms: get(&self.last_rx), last_tx_ms: get(&self.last_tx),
            fault: self.fault.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            working_set_bytes, private_bytes, process_handles, cpu_time_100ns,
        }
    }
    pub fn persist(&self, home: &std::path::Path) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.snapshot())?;
        let temp = home.join("tun-health.tmp");
        std::fs::write(&temp, bytes)?;
        std::fs::rename(temp, home.join(SNAPSHOT_FILE))?;
        Ok(())
    }
}

#[cfg(windows)]
fn resources() -> (Option<u64>, Option<u64>, Option<u32>, Option<u64>) {
    use windows_sys::Win32::{Foundation::FILETIME, System::{Threading::{GetCurrentProcess, GetProcessHandleCount, GetProcessTimes}, ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX}}};
    unsafe {
        let process = GetCurrentProcess();
        let mut memory: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed(); memory.cb = std::mem::size_of_val(&memory) as u32;
        let memory_ok = GetProcessMemoryInfo(process, (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(), memory.cb) != 0;
        let mut handles = 0; let handles_ok = GetProcessHandleCount(process, &mut handles) != 0;
        let mut created: FILETIME = std::mem::zeroed(); let mut ended = created; let mut kernel = created; let mut user = created;
        let times_ok = GetProcessTimes(process, &mut created, &mut ended, &mut kernel, &mut user) != 0;
        let ticks = |v: FILETIME| ((v.dwHighDateTime as u64) << 32) | v.dwLowDateTime as u64;
        (memory_ok.then_some(memory.WorkingSetSize as u64), memory_ok.then_some(memory.PrivateUsage as u64), handles_ok.then_some(handles), times_ok.then_some(ticks(kernel).saturating_add(ticks(user))))
    }
}
#[cfg(not(windows))]
fn resources() -> (Option<u64>, Option<u64>, Option<u32>, Option<u64>) { (None, None, None, None) }
pub struct Guard { health: Arc<Health>, pump: Pump }
impl Drop for Guard {
    fn drop(&mut self) {
        self.health.running.fetch_and(!(1 << self.pump as u8), Ordering::Relaxed);
        self.health.fail(self.pump, "task_exited", None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_and_pressure_are_not_fatal_but_task_exit_is() {
        let h = Arc::new(Health::default()); h.enable();
        let guard = h.guard(Pump::Receive);
        h.ring_full(); h.received(64); h.sent(32);
        h.queue(true, 4096, std::time::Duration::from_millis(12));
        assert!(!h.failed.is_cancelled());
        let s = h.snapshot(); assert_eq!(s.rx_bytes, 64); assert_eq!(s.tx_bytes, 32);
        #[cfg(windows)] {
            assert!(s.working_set_bytes.is_some_and(|n| n > 0));
            assert!(s.private_bytes.is_some_and(|n| n > 0));
            assert!(s.process_handles.is_some_and(|n| n > 0));
            assert!(s.cpu_time_100ns.is_some());
        }
        assert_eq!(s.rx_queue_high_water, 4096); assert_eq!(s.ring_full_drops, 1);
        drop(guard); assert!(h.failed.is_cancelled());
        assert_eq!(h.snapshot().fault.unwrap().stage, "adapter_rx");
    }
    #[test]
    fn deliberate_shutdown_is_not_an_incident_and_first_error_survives() {
        let h = Arc::new(Health::default()); let guard = h.guard(Pump::Send);
        h.stopping(); drop(guard); assert!(!h.failed.is_cancelled());
        let h = Health::default(); h.fail(Pump::Send, "device_error", Some(38));
        h.fail(Pump::FromStack, "task_exited", None);
        let json = serde_json::to_value(h.snapshot()).unwrap();
        assert_eq!(json["fault"]["reason"], "device_error");
        assert_eq!(json["fault"]["os_error"], 38);
        for key in ["secret", "config", "password", "domain", "address"] { assert!(json.get(key).is_none()); }
    }
}
