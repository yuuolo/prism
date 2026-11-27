//! Service-owned restart intent. Never serializes the launch configuration.
use std::collections::VecDeque;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct Launch {
    pub core: String, pub config: String, pub kill_switch: bool,
    pub tailscale: bool, pub apps: Vec<String>, pub recovery: bool,
    pub mutations: std::collections::BTreeMap<String, serde_json::Value>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub enabled: bool, pub phase: String, pub restarts: u32,
    pub last_exit_pid: Option<u32>, pub last_exit_code: Option<i32>,
    pub incident: Option<serde_json::Value>,
}
#[derive(Default)]
pub struct Supervisor {
    generation: u64, desired: Option<Launch>, attempts: VecDeque<u64>, pub status: Status,
    pub exit: Option<(u32, u64, Option<i32>)>,
}
impl Supervisor {
    pub fn change(&mut self) -> u64 { self.generation = self.generation.wrapping_add(1); self.generation }
    pub fn valid(&self, ticket: u64) -> bool { self.generation == ticket }
    pub fn started(&mut self, ticket: u64, launch: Launch) {
        if !self.valid(ticket) { return; }
        self.status.enabled = launch.recovery && launch.core == "prism";
        self.status.phase = "monitoring".into(); self.desired = Some(launch);
    }
    pub fn stop(&mut self) {
        self.change(); self.desired = None; self.attempts.clear();
        self.status.enabled = false; self.status.phase = "stopped".into();
    }
    pub fn enable(&mut self, enabled: bool) {
        self.change();
        if let Some(desired) = &mut self.desired {
            desired.recovery = enabled;
            self.status.enabled = enabled && desired.core == "prism";
            self.status.phase = if self.status.enabled { "monitoring" } else { "disabled" }.into();
        }
    }
    pub fn apps(&mut self, apps: Vec<String>) { if let Some(d) = &mut self.desired { d.apps = apps; } }
    pub fn desired(&self) -> Option<Launch> { self.desired.clone() }
    pub fn config(&mut self, config: String) { if let Some(d) = &mut self.desired { d.config = config; } }
    pub fn sync_state(&mut self, config: Option<String>, mutations: std::collections::BTreeMap<String, serde_json::Value>) {
        if let Some(d) = &mut self.desired {
            if let Some(config) = config { d.config = config; }
            d.mutations = mutations; self.status.enabled = d.recovery && d.core == "prism";
        }
    }
    pub fn report_exit(&mut self, pid: u32, started_at: u64, code: Option<i32>) {
        self.exit = Some((pid, started_at, code));
        self.status.last_exit_pid = Some(pid); self.status.last_exit_code = code;
        self.status.phase = "exited".into();
    }
    pub fn candidate(&mut self, now: u64) -> Option<(u64, Launch)> {
        if !self.status.enabled { return None; }
        while self.attempts.front().is_some_and(|t| now.saturating_sub(*t) >= 600) { self.attempts.pop_front(); }
        if self.attempts.len() >= 3 || self.attempts.back().is_some_and(|t| now.saturating_sub(*t) < 60) {
            self.status.phase = "cooldown".into(); return None;
        }
        let launch = self.desired.clone()?;
        self.attempts.push_back(now); self.status.phase = "restarting".into();
        Some((self.generation, launch))
    }
}

/// Export only known numerical telemetry and fixed fault codes, never configs.
pub fn project_snapshot(value: &serde_json::Value, pid: u32, since: u64) -> Option<serde_json::Value> {
    use serde_json::{json, Map, Value};
    if value["schema"].as_u64()? != 1 || value["process_id"].as_u64()? != pid as u64 || value["captured_at"].as_u64()? < since { return None; }
    let mut output = Map::new();
    for key in ["schema", "process_id", "captured_at", "uptime_ms", "running_pumps", "rx_packets", "rx_bytes", "tx_packets", "tx_bytes", "ring_full_drops", "rx_queue_high_water", "tx_queue_high_water", "max_queue_wait_ms", "last_rx_ms", "last_tx_ms", "working_set_bytes", "private_bytes", "process_handles", "cpu_time_100ns", "network_recoveries", "network_recovery_failures", "last_network_recovery_ms"] {
        if let Some(n) = value[key].as_u64() { output.insert(key.into(), json!(n)); }
    }
    if let Some(enabled) = value["enabled"].as_bool() { output.insert("enabled".into(), json!(enabled)); }
    if let Some(version) = value["version"].as_str().filter(|v| v.len() <= 64 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))) {
        output.insert("version".into(), json!(version));
    }
    let stages = ["adapter_rx", "adapter_tx", "to_stack", "from_stack", "stack_runner", "tcp_accept", "udp_accept"];
    let reasons = ["task_exited", "device_receive_error", "device_send_error", "stack_input_error", "stack_output_error", "stack_runner_error"];
    if let (Some(stage), Some(reason)) = (value["fault"]["stage"].as_str(), value["fault"]["reason"].as_str()) {
        if stages.contains(&stage) && reasons.contains(&reason) {
            output.insert("fault".into(), json!({"stage":stage,"reason":reason,"os_error":value["fault"]["os_error"].as_i64().and_then(|n| i32::try_from(n).ok())}));
        }
    }
    Some(Value::Object(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn launch() -> Launch { Launch { core:"prism".into(), config:"secret: never-export".into(), kill_switch:true, tailscale:false, apps:vec![], recovery:true, mutations:Default::default() } }
    #[test]
    fn stop_and_new_commands_cancel_old_recovery_and_budget_is_bounded() {
        let mut s = Supervisor::default(); let ticket = s.change(); s.started(ticket, launch());
        let (old, _) = s.candidate(0).unwrap(); assert!(s.candidate(59).is_none());
        assert!(s.candidate(60).is_some()); assert!(s.candidate(120).is_some()); assert!(s.candidate(180).is_none());
        assert!(s.candidate(600).is_some()); s.stop(); assert!(!s.valid(old)); assert!(s.candidate(700).is_none());
        s.started(old, launch()); assert!(!s.status.enabled);
    }
    #[test]
    fn disabling_recovery_and_snapshot_projection_do_not_leak_secrets() {
        let mut s = Supervisor::default(); let t = s.change(); s.started(t, launch()); s.enable(false);
        assert!(s.candidate(0).is_none());
        let v = serde_json::json!({"schema":1,"process_id":7,"captured_at":20,"rx_bytes":9,"secret":"no","config":"no","fault":{"stage":"adapter_rx","reason":"device_receive_error","os_error":38,"payload":"no"}});
        let out = project_snapshot(&v, 7, 10).unwrap(); assert_eq!(out["rx_bytes"], 9);
        assert!(!out.to_string().contains("secret")); assert!(!out.to_string().contains("payload"));
        assert!(project_snapshot(&v, 8, 10).is_none()); assert!(project_snapshot(&v, 7, 21).is_none());
        assert!(!serde_json::to_string(&s.status).unwrap().contains("never-export"));
    }
}
