//! Daily traffic totals (traffic.json), sampled from the running core's
//! `/connections` counters. Those count from the core's start, so each
//! sample adds the difference to today's bucket.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;

/// Days kept in traffic.json.
const KEEP_DAYS: usize = 400;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Day {
    /// Local date, YYYY-MM-DD.
    pub date: String,
    pub upload: u64,
    pub download: u64,
}

pub struct Traffic {
    path: PathBuf,
    days: Mutex<BTreeMap<String, (u64, u64)>>,
    /// Core totals at the last sample.
    last: Mutex<Option<(u64, u64)>>,
}

impl Traffic {
    pub fn load(path: PathBuf) -> Self {
        let days = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Traffic { path, days: Mutex::new(days), last: Mutex::new(None) }
    }

    /// Adds what the core moved since the last sample to `date`.
    pub fn record(&self, date: &str, up: u64, down: u64) {
        let mut last = self.last.lock().unwrap();
        // Totals going down means the core restarted and counts from zero.
        let (du, dd) = match *last {
            Some((lu, ld)) if up >= lu && down >= ld => (up - lu, down - ld),
            Some(_) => (up, down),
            // First sample: what moved before Prism watched isn't today's to claim twice.
            None => (0, 0),
        };
        *last = Some((up, down));
        if du == 0 && dd == 0 {
            return;
        }
        let mut days = self.days.lock().unwrap();
        let e = days.entry(date.to_string()).or_default();
        e.0 += du;
        e.1 += dd;
        while days.len() > KEEP_DAYS {
            let first = days.keys().next().cloned().unwrap();
            days.remove(&first);
        }
    }

    /// The core stopped: its next totals start over.
    pub fn reset_baseline(&self) {
        *self.last.lock().unwrap() = None;
    }

    pub fn save(&self) {
        let days = self.days.lock().unwrap().clone();
        let tmp = self.path.with_extension("tmp");
        if serde_json::to_vec(&days).ok().and_then(|b| std::fs::write(&tmp, b).ok()).is_some() {
            let _ = std::fs::rename(tmp, &self.path);
        }
    }

    pub fn history(&self) -> Vec<Day> {
        self.days.lock().unwrap().iter().map(|(d, (u, dn))| Day { date: d.clone(), upload: *u, download: *dn }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_survive_core_restarts() {
        let path = std::env::temp_dir().join(format!("prism-traffic-{}.json", std::process::id()));
        let t = Traffic::load(path.clone());
        t.record("2026-10-25", 100, 1000); // baseline only
        t.record("2026-10-25", 150, 1500);
        t.record("2026-10-25", 20, 30); // restarted core
        t.record("2026-10-26", 30, 50);
        t.reset_baseline();
        t.record("2026-10-26", 999, 999); // new baseline
        assert_eq!(
            t.history(),
            vec![
                Day { date: "2026-10-25".into(), upload: 70, download: 530 },
                Day { date: "2026-10-26".into(), upload: 10, download: 20 },
            ]
        );
        t.save();
        assert_eq!(Traffic::load(path.clone()).history().len(), 2);
        let _ = std::fs::remove_file(path);
    }
}
