//! Traffic per program per day (app_traffic.json), from the bytes each
//! connection moved between two `/connections` samples.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Days kept.
const KEEP_DAYS: usize = 90;
/// Exits remembered per program per day.
const MAX_EXITS: usize = 8;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    pub upload: u64,
    pub download: u64,
    /// Exit (the node, or DIRECT) → bytes.
    #[serde(default)]
    pub exits: BTreeMap<String, u64>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct AppTotal {
    pub name: String,
    pub path: String,
    pub upload: u64,
    pub download: u64,
    /// Busiest exits first.
    pub exits: Vec<(String, u64)>,
}

#[derive(Default, Serialize, Deserialize)]
struct Data {
    /// date → program name → usage
    days: BTreeMap<String, BTreeMap<String, Usage>>,
    /// program name → last seen full path
    paths: BTreeMap<String, String>,
}

pub struct AppTraffic {
    path: PathBuf,
    data: Mutex<Data>,
    /// Connection id → (upload, download) at the last sample.
    last: Mutex<HashMap<String, (u64, u64)>>,
}

impl AppTraffic {
    pub fn load(path: PathBuf) -> Self {
        let data = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        AppTraffic { path, data: Mutex::new(data), last: Default::default() }
    }

    pub fn save(&self) {
        let tmp = self.path.with_extension("tmp");
        let bytes = serde_json::to_vec(&*self.data.lock().unwrap());
        if bytes.ok().and_then(|b| std::fs::write(&tmp, b).ok()).is_some() {
            let _ = std::fs::rename(tmp, &self.path);
        }
    }

    /// Adds what each connection in a `/connections` answer moved since the
    /// previous sample. A connection seen for the first time counts in full,
    /// except on the first sample after a (re)start of watching.
    pub fn record(&self, date: &str, conns: &Value) {
        let mut last = self.last.lock().unwrap();
        let first = last.is_empty();
        let mut next = HashMap::new();
        let mut data = self.data.lock().unwrap();
        for c in conns["connections"].as_array().into_iter().flatten() {
            let Some(id) = c["id"].as_str() else { continue };
            let (up, down) = (c["upload"].as_u64().unwrap_or(0), c["download"].as_u64().unwrap_or(0));
            next.insert(id.to_string(), (up, down));
            let (du, dd) = match last.get(id) {
                Some(&(lu, ld)) => (up.saturating_sub(lu), down.saturating_sub(ld)),
                None if first => continue,
                None => (up, down),
            };
            if du == 0 && dd == 0 {
                continue;
            }
            let m = &c["metadata"];
            // sing-box reports only the path (see fill_process).
            let name = match m["process"].as_str().filter(|p| !p.is_empty()) {
                Some(p) => p.to_string(),
                None => m["processPath"].as_str().map(prism_core::process::file_name).unwrap_or_default(),
            };
            if let Some(p) = m["processPath"].as_str().filter(|p| !p.is_empty()) {
                data.paths.insert(name.clone(), p.to_string());
            }
            let exit = c["chains"].as_array().and_then(|ch| ch.first()).and_then(|x| x.as_str()).unwrap_or("DIRECT").to_string();
            let u = data.days.entry(date.to_string()).or_default().entry(name).or_default();
            u.upload += du;
            u.download += dd;
            *u.exits.entry(exit).or_default() += du + dd;
            if u.exits.len() > MAX_EXITS {
                let smallest = u.exits.iter().min_by_key(|(_, b)| **b).map(|(k, _)| k.clone()).unwrap();
                u.exits.remove(&smallest);
            }
        }
        *last = next;
        while data.days.len() > KEEP_DAYS {
            let oldest = data.days.keys().next().cloned().unwrap();
            data.days.remove(&oldest);
        }
    }

    /// The core stopped: its connection ids are gone.
    pub fn reset(&self) {
        self.last.lock().unwrap().clear();
    }

    /// Totals per program over the days from `since` (YYYY-MM-DD) on, biggest first.
    pub fn totals(&self, since: &str) -> Vec<AppTotal> {
        let data = self.data.lock().unwrap();
        let mut sum: BTreeMap<&str, Usage> = BTreeMap::new();
        for (_, apps) in data.days.range(since.to_string()..) {
            for (name, u) in apps {
                let s = sum.entry(name).or_default();
                s.upload += u.upload;
                s.download += u.download;
                for (e, b) in &u.exits {
                    *s.exits.entry(e.clone()).or_default() += b;
                }
            }
        }
        let mut out: Vec<AppTotal> = sum
            .into_iter()
            .map(|(name, u)| {
                let mut exits: Vec<(String, u64)> = u.exits.into_iter().collect();
                exits.sort_by(|a, b| b.1.cmp(&a.1));
                AppTotal { name: name.to_string(), path: data.paths.get(name).cloned().unwrap_or_default(), upload: u.upload, download: u.download, exits }
            })
            .collect();
        out.sort_by(|a, b| (b.upload + b.download).cmp(&(a.upload + a.download)));
        out
    }
}

/// Gives sing-box's connections (`GET /connections`) a `process` like
/// prism-core's and mihomo's, and a `processPath` that is just the path,
/// so the Connections page, its app rules and app traffic see one name per
/// program.
pub fn fill_process(conns: &mut Value) {
    for c in conns["connections"].as_array_mut().into_iter().flatten() {
        let m = &mut c["metadata"];
        if m["process"].as_str().is_some_and(|p| !p.is_empty()) {
            continue;
        }
        let Some(raw) = m["processPath"].as_str() else { continue };
        let path = singbox_path(raw).unwrap_or_default().to_string();
        m["process"] = Value::String(prism_core::process::file_name(&path));
        m["processPath"] = Value::String(path);
    }
}

/// sing-box's `processPath` without what it appends: ` (<user>)` or
/// ` (<uid>)` after the path, or only the uid when it has no path (None).
/// Program names can end in `(...)` too (`Chrome Helper (GPU)`), so only a
/// real account name or a number is taken off.
fn singbox_path(p: &str) -> Option<&str> {
    #[cfg(unix)]
    if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    #[cfg(unix)]
    if let Some((path, who)) = p.strip_suffix(')').and_then(|s| s.rsplit_once(" (")) {
        if !path.is_empty() && (who.bytes().all(|b| b.is_ascii_digit()) || is_user(who)) {
            return Some(path);
        }
    }
    Some(p)
}

/// Remembered: every `GET /connections` asks, and getpwnam is a directory
/// service lookup. Under the lock, as getpwnam isn't reentrant.
#[cfg(unix)]
fn is_user(name: &str) -> bool {
    static KNOWN: Mutex<BTreeMap<String, bool>> = Mutex::new(BTreeMap::new());
    let mut known = KNOWN.lock().unwrap();
    if let Some(&k) = known.get(name) {
        return k;
    }
    let Ok(c) = std::ffi::CString::new(name) else { return false };
    let found = !name.is_empty() && unsafe { !libc::getpwnam(c.as_ptr()).is_null() };
    if known.len() < 256 {
        known.insert(name.to_string(), found);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[cfg(unix)]
    #[test]
    fn singbox_process_paths() {
        assert_eq!(singbox_path("/Applications/Mail.app/Contents/MacOS/Mail (root)"), Some("/Applications/Mail.app/Contents/MacOS/Mail"));
        assert_eq!(singbox_path("/usr/libexec/NRDUpdated (501)"), Some("/usr/libexec/NRDUpdated"));
        assert_eq!(singbox_path("/x/Google Chrome Helper (GPU)"), Some("/x/Google Chrome Helper (GPU)"));
        assert_eq!(singbox_path("/x/Google Chrome Helper (GPU) (root)"), Some("/x/Google Chrome Helper (GPU)"));
        assert_eq!(singbox_path("/x/Mail"), Some("/x/Mail"));
        assert_eq!(singbox_path("503"), None);
        assert_eq!(singbox_path(""), Some(""));

        let path = std::env::temp_dir().join(format!("prism-apptraffic-sb-{}.json", std::process::id()));
        let t = AppTraffic::load(path.clone());
        let sb = |id: &str, p: &str, down: u64| json!({ "id": id, "upload": 0, "download": down, "chains": ["DIRECT"], "metadata": { "processPath": p } });
        let filled = |mut v: Value| {
            fill_process(&mut v);
            v
        };
        t.record("2026-11-01", &filled(json!({ "connections": [sb("z", "/x/z", 0)] })));
        let mut second = json!({ "connections": [sb("a", "/x/Mail (root)", 10), sb("b", "/x/Mail", 5), sb("c", "0", 1)] });
        fill_process(&mut second);
        let m = &second["connections"][0]["metadata"];
        assert_eq!((m["process"].as_str(), m["processPath"].as_str()), (Some("Mail"), Some("/x/Mail")));
        assert_eq!(second["connections"][2]["metadata"]["process"], "");
        t.record("2026-11-01", &second);
        let all = t.totals("2026-11-01");
        assert_eq!(all.iter().map(|a| (a.name.as_str(), a.download)).collect::<Vec<_>>(), [("Mail", 15), ("", 1)]);
        assert_eq!(all[0].path, "/x/Mail");
        let _ = std::fs::remove_file(path);
    }

    fn conn(id: &str, proc_: &str, up: u64, down: u64, exit: &str) -> Value {
        json!({ "id": id, "upload": up, "download": down, "chains": [exit, "节点选择"],
                "metadata": { "process": proc_, "processPath": format!("C:\\x\\{proc_}") } })
    }

    #[test]
    fn counts_per_app_and_exit() {
        let path = std::env::temp_dir().join(format!("prism-apptraffic-{}.json", std::process::id()));
        let t = AppTraffic::load(path.clone());
        // First sample: a baseline, nothing counted.
        t.record("2026-11-01", &json!({ "connections": [conn("a", "chrome.exe", 100, 1000, "东京")] }));
        t.record("2026-11-01", &json!({ "connections": [
            conn("a", "chrome.exe", 150, 3000, "东京"),
            conn("b", "chrome.exe", 10, 20, "DIRECT"),
            conn("c", "", 1, 2, "DIRECT"),
        ]}));
        // `a` closed; `d` is new.
        t.record("2026-11-02", &json!({ "connections": [conn("d", "steam.exe", 0, 5000, "DIRECT")] }));
        let all = t.totals("2026-11-01");
        assert_eq!(all.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["steam.exe", "chrome.exe", ""]);
        let chrome = &all[1];
        assert_eq!((chrome.upload, chrome.download), (60, 2020));
        assert_eq!(chrome.exits, vec![("东京".to_string(), 2050), ("DIRECT".to_string(), 30)]);
        assert_eq!(chrome.path, "C:\\x\\chrome.exe");
        assert_eq!(t.totals("2026-11-02").len(), 1);
        t.save();
        assert_eq!(AppTraffic::load(path.clone()).totals("2026-11-01").len(), 3);
        let _ = std::fs::remove_file(path);
    }
}
