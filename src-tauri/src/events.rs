//! Bounded persistent runtime and security history, independent of live logs.
use std::path::Path;
use std::sync::Mutex;
use anyhow::Result;

static LOCK: Mutex<()> = Mutex::new(());
const LIMIT: usize = 2000;
// Refused/background IPv6 connections must not erase the incident history.
const RUNTIME_RESERVE: usize = 500;

fn runtime_event(line: &str) -> bool {
    ["自动恢复", "断网保护", "异常退出", "持续运行", "启动", "网络恢复", "tailscale 保护", "Tailscale 保护"]
        .iter().any(|kind| line.contains(kind))
}

fn retain(events: &mut Vec<String>) {
    while events.len() > LIMIT {
        let runtime = events.iter().filter(|line| runtime_event(line)).count();
        let remove = if runtime <= RUNTIME_RESERVE {
            events.iter().position(|line| !runtime_event(line)).unwrap_or(0)
        } else {
            0
        };
        events.remove(remove);
    }
}

fn read_unlocked(path: &Path) -> Result<Vec<String>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

pub fn read(path: &Path) -> Result<Vec<String>> {
    let _guard = LOCK.lock().unwrap();
    read_unlocked(path)
}

pub fn record(path: &Path, line: &str) -> Result<()> {
    let _guard = LOCK.lock().unwrap();
    let mut events = read_unlocked(path)?;
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    events.push(format!("{timestamp} {line}"));
    retain(&mut events);
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, serde_json::to_vec(&events)?)?;
    std::fs::rename(temp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persists_and_retains_latest_events() {
        let dir = std::env::temp_dir().join(format!("prism-events-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("security-events.json");
        let old: Vec<String> = (0..LIMIT).map(|i| format!("old {i}")).collect();
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        record(&path, "断网保护已触发").unwrap();
        record(&path, "REJECT").unwrap();
        let saved = read(&path).unwrap();
        assert_eq!(saved.len(), LIMIT);
        assert_eq!(saved[0], "old 2");
        assert!(saved[LIMIT - 2].ends_with("断网保护已触发"));
        assert!(saved[LIMIT - 1].ends_with("REJECT"));
        assert!(!path.with_extension("tmp").exists());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn reject_flood_cannot_evict_recent_recovery_and_protection_events() {
        let mut events = vec![
            "1 [prism] 自动恢复监测：TUN 通路失败".into(),
            "2 [service] 断网保护已启用".into(),
        ];
        for n in 0..LIMIT * 2 {
            events.push(format!("{} IPv6Blocked using REJECT", n + 3));
            retain(&mut events);
        }
        assert_eq!(events.len(), LIMIT);
        assert!(events[0].contains("自动恢复"));
        assert!(events[1].contains("断网保护"));
        // Runtime history is also bounded and rotates as newer incidents arrive.
        for n in 0..LIMIT {
            events.push(format!("{} [prism] 自动恢复：重新启动内核", n + 5000));
            retain(&mut events);
        }
        assert_eq!(events.len(), LIMIT);
        assert!(!events.iter().any(|line| line.starts_with("1 ")));
    }
}
