use serde::Deserialize;
use serde_json::Value;

/// Final `stats` object of `rclone --use-json-log --stats 1h --stats-log-level NOTICE`.
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
}

/// What `rclone sync` printed on stderr: its last stats object and its error
/// messages (JSON `level: error|critical|alert|emergency` lines and any plain-text line).
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
    // Keep only the last 20 messages
    if log.errors.len() > 20 {
        log.errors = log.errors.into_iter().rev().take(20).collect::<Vec<_>>();
        log.errors.reverse();
    }
    log
}
