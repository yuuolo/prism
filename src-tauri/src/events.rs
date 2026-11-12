//! Bounded persistent runtime and security history, independent of live logs.
use std::path::Path;
use std::sync::Mutex;
use anyhow::Result;

static LOCK: Mutex<()> = Mutex::new(());
const LIMIT: usize = 2000;

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
    if events.len() > LIMIT { events.drain(..events.len() - LIMIT); }
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
}
