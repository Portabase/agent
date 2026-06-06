use chrono::Utc;
use serde::Serialize;
use std::sync::Mutex;
use tracing::{event, Level};

#[derive(Serialize, Clone, Debug)]
pub struct JobLogEntry {
    pub timestamp: String,
    #[serde(rename = "type")]
    pub entry_type: &'static str,
    pub level: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
}

#[derive(Default, Debug)]
pub struct JobLogger {
    entries: Mutex<Vec<JobLogEntry>>,
}

impl JobLogger {
    pub fn new() -> Self {
        Self::default()
    }

    fn trace_log(level: &'static str, message: &str) {
        match level {
            "trace" => event!(Level::TRACE, "{message}"),
            "debug" => event!(Level::DEBUG, "{message}"),
            "info" => event!(Level::INFO, "{message}"),
            "warn" | "warning" => event!(Level::WARN, "{message}"),
            "error" => event!(Level::ERROR, "{message}"),
            _ => event!(Level::INFO, "{message}"),
        }
    }


    #[allow(dead_code)]
    pub fn log(&self, level: &'static str, message: impl Into<String>) {
        let message = message.into();

        Self::trace_log(level, &message);

        let mut entries = self.entries.lock().unwrap();
        entries.push(JobLogEntry {
            timestamp: Utc::now().to_rfc3339(),
            entry_type: "log",
            level,
            message,
            command: None,
            output: None,
            exit_code: None,
            duration_ms: None,
        });
    }

    pub fn log_command(
        &self,
        command: impl Into<String>,
        output: Option<String>,
        exit_code: Option<i32>,
        duration_ms: Option<f64>,
    ) {
        let level = match exit_code {
            Some(0) | None => "debug",
            _ => "error",
        };


        let cmd = command.into();
        let message = format!("Executed: {}", cmd);

        match level {
            "debug" => {
                tracing::debug!(
                command = %cmd,
                exit_code = ?exit_code,
                duration_ms = ?duration_ms,
                "Command executed"
            );
            }
            "error" => {
                tracing::error!(
                command = %cmd,
                exit_code = ?exit_code,
                duration_ms = ?duration_ms,
                "Command failed"
            );
            }
            _ => {
                tracing::info!(
                command = %cmd,
                exit_code = ?exit_code,
                duration_ms = ?duration_ms,
                "Command executed"
            );
            }
        }

        if let Some(output_text) = output.as_deref() {
            for line in output_text.lines() {
                if line.trim().is_empty() {
                    continue;
                }

                match level {
                    "debug" => tracing::debug!("{line}"),
                    "error" => tracing::error!("{line}"),
                    _ => tracing::info!("{line}"),
                }
            }
        }

        let mut entries = self.entries.lock().unwrap();
        entries.push(JobLogEntry {
            timestamp: Utc::now().to_rfc3339(),
            entry_type: "command",
            level,
            message,
            command: Some(cmd),
            output,
            exit_code,
            duration_ms,
        });
    }

    pub fn into_entries(self) -> Vec<JobLogEntry> {
        self.entries.into_inner().unwrap()
    }
}
