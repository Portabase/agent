use serde::Deserialize;
use serde_json::{Value, json};

/// One line of `restic --json` output. stdout carries `status` / `summary`,
/// stderr carries `error` / `exit_error` and plain-text fatal messages.
#[derive(Debug, PartialEq)]
pub enum Line {
    Status { percent_done: f64 },
    Summary(Value),
    Error { message: String, item: Option<String> },
    ExitError { code: i32, message: String },
    Text(String),
}

pub fn parse(line: &str) -> Line {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Line::Text(line.to_string());
    };
    match value.get("message_type").and_then(Value::as_str) {
        Some("status") => Line::Status {
            percent_done: value["percent_done"].as_f64().unwrap_or(0.0),
        },
        Some("summary") => Line::Summary(value),
        Some("error") => Line::Error {
            message: value["error"]["message"].as_str().unwrap_or("unknown error").to_string(),
            item: value["item"].as_str().map(str::to_string),
        },
        Some("exit_error") => Line::ExitError {
            code: value["code"].as_i64().unwrap_or(1) as i32,
            message: value["message"].as_str().unwrap_or_default().to_string(),
        },
        _ => Line::Text(line.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct BackupSummary {
    pub snapshot_id: String,
    pub data_added_packed: u64,
    pub total_bytes_processed: u64,
    pub files_new: u64,
    pub files_changed: u64,
    pub files_unmodified: u64,
    /// Files of the parent snapshot this one no longer has: restic does not count them
    /// (see `files_removed`). None when unknown.
    #[serde(skip)]
    pub files_removed: Option<u64>,
}

impl BackupSummary {
    /// Counters the dashboard shows for this snapshot (`size` carries `data_added_packed`).
    pub fn report(&self) -> Value {
        json!({
            "filesNew": self.files_new,
            "filesChanged": self.files_changed,
            "filesUnmodified": self.files_unmodified,
            "filesRemoved": self.files_removed,
        })
    }
}

/// One entry of `restic snapshots --json`.
#[derive(Debug, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub parent: Option<String>,
    /// Written by restic >= 0.17.
    #[serde(default)]
    pub summary: Option<SnapshotSummary>,
}

#[derive(Debug, Deserialize)]
pub struct SnapshotSummary {
    pub total_files_processed: u64,
}

/// Files of the parent missing from `snapshot_id`, from a `restic snapshots --json` listing:
/// restic counts a file as changed or unmodified only when the parent has it, so the parent's
/// file count minus those is what was removed. Some(0) without a parent; None when the parent
/// is not listed or has no summary.
pub fn files_removed(snapshots: &[Snapshot], snapshot_id: &str, changed: u64, unmodified: u64) -> Option<u64> {
    let find = |id: &str| snapshots.iter().find(|s| s.id == id);
    let Some(parent) = find(snapshot_id)?.parent.as_deref() else {
        return Some(0);
    };
    let total = find(parent)?.summary.as_ref()?.total_files_processed;
    Some(total.saturating_sub(changed + unmodified))
}
