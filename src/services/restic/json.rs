use serde::Deserialize;
use serde_json::Value;

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
}

/// One entry of `restic snapshots --json`.
#[derive(Debug, Deserialize)]
pub struct Snapshot {
    #[allow(dead_code)] // part of the `restic snapshots` shape; restore() only needs `paths`
    pub id: String,
    pub paths: Vec<String>,
}
