use crate::utils::text::human_size;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Default, Deserialize, PartialEq)]
pub struct SyncStats {
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub transfers: u64,
    #[serde(default)]
    pub deletes: u64,
    #[serde(default)]
    pub errors: u64,
    #[serde(default, rename = "lastError")]
    pub last_error: Option<String>,
    #[serde(skip)]
    pub replica_bytes: Option<u64>,
}

impl SyncStats {
    pub fn report(&self) -> Value {
        json!({
            "filesTransferred": self.transfers,
            "filesDeleted": self.deletes,
            "replicaBytes": self.replica_bytes,
        })
    }
}

#[derive(Debug, Default)]
pub struct SyncLog {
    pub stats: Option<SyncStats>,
    pub errors: Vec<String>,
}

pub fn parse_log(stderr: &str) -> SyncLog {
    let mut log = SyncLog::default();
    for line in stderr.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            if !log.errors.contains(&line.to_string()) {
                log.errors.push(line.to_string());
            }
            continue;
        };
        if let Some(stats) = value.get("stats").and_then(|s| serde_json::from_value(s.clone()).ok()) {
            log.stats = Some(stats);
        }
        if let Some(level) = value.get("level").and_then(Value::as_str) {
            if matches!(level, "error" | "critical" | "alert" | "emergency") {
                if let Some(msg) = value.get("msg").and_then(Value::as_str) {
                    let msg_str = msg.to_string();
                    if !log.errors.contains(&msg_str) {
                        log.errors.push(msg_str);
                    }
                }
            }
        }
    }
    if log.errors.len() > 20 {
        log.errors = log.errors.into_iter().rev().take(20).collect::<Vec<_>>();
        log.errors.reverse();
    }
    log
}

pub fn progress(line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line).ok()?;
    let stats = value.get("stats")?;
    let n = |key: &str| stats[key].as_u64().unwrap_or(0);
    let (bytes, total) = (n("bytes"), n("totalBytes"));
    let percent = if total > 0 { bytes * 100 / total } else { 100 };
    let eta = stats["eta"].as_u64().map(|s| format!(", ETA {s}s")).unwrap_or_default();
    Some(format!(
        "{} / {} ({percent}%), {}/{} transferred, {}/{} checked, {} deleted, {}/s{eta}",
        human_size(bytes),
        human_size(total),
        n("transfers"),
        n("totalTransfers"),
        n("checks"),
        n("totalChecks"),
        n("deletes"),
        human_size(stats["speed"].as_f64().unwrap_or(0.0) as u64),
    ))
}
