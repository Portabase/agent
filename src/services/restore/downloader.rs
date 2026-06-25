use super::service::RestoreService;

use anyhow::Result;
use reqwest::{Client, Url};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use crate::services::backup::logger::JobLogger;

/// Fallback progress cadence when the server sends no Content-Length.
pub(crate) const BYTE_STEP: u64 = 512 * 1024 * 1024; // 512 MB

/// Tracks streamed download progress and emits milestone log lines.
/// Pure arithmetic, no I/O — unit-tested in
/// `src/tests/services/restore_downloader_tests.rs`.
pub(crate) struct ProgressTracker {
    total: Option<u64>,
    downloaded: u64,
    next_pct: u64,
    next_bytes: u64,
}

impl ProgressTracker {
    pub(crate) fn new(total: Option<u64>) -> Self {
        Self {
            total,
            downloaded: 0,
            next_pct: 10,
            next_bytes: BYTE_STEP,
        }
    }

    /// Record `n` more downloaded bytes; return any milestone messages crossed
    /// (normally 0 or 1; more only if one chunk spans several milestones).
    pub(crate) fn advance(&mut self, n: usize) -> Vec<String> {
        self.downloaded += n as u64;
        let mut msgs = Vec::new();

        match self.total {
            Some(total) if total > 0 => {
                let pct = self.downloaded.saturating_mul(100) / total;
                while pct >= self.next_pct && self.next_pct <= 100 {
                    msgs.push(format!(
                        "Download progress: {}% ({}/{} MB)",
                        self.next_pct,
                        mb(self.downloaded),
                        mb(total),
                    ));
                    self.next_pct += 10;
                }
            }
            _ => {
                while self.downloaded >= self.next_bytes {
                    msgs.push(format!("Downloaded {} MB", mb(self.downloaded)));
                    self.next_bytes += BYTE_STEP;
                }
            }
        }

        msgs
    }

    /// Human-readable total for the start log line.
    pub(crate) fn fmt_total(total: Option<u64>) -> String {
        match total {
            Some(t) if t > 0 => format!("{} MB", mb(t)),
            _ => "unknown size".to_string(),
        }
    }
}

fn mb(bytes: u64) -> u64 {
    bytes / 1024 / 1024
}

impl RestoreService {
    pub async fn download_backup(&self, file_url: &str, tmp_path: &Path, logger: Arc<JobLogger>) -> Result<PathBuf> {
        logger.log("info", "Start downloading backup archive".to_string());

        let client = Client::new();

        let response = client.get(file_url).send().await?;

        if !response.status().is_success() {
            logger.log("error", "Failed to download".to_string());
            anyhow::bail!("download failed");
        }

        let filename_from_header = response
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split("filename=").nth(1))
            .map(|f| f.trim_matches('"').to_string());

        let filename_from_url = Url::parse(file_url).ok().and_then(|u| {
            u.path_segments()?
                .last()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        });

        let filename = filename_from_header
            .or(filename_from_url)
            .unwrap_or_else(|| "downloaded_file".to_string());

        let path = tmp_path.join(&filename);

        let bytes = response.bytes().await?;

        tokio::fs::write(&path, &bytes).await?;

        logger.log("info", format!("Backup downloaded to {}", path.display()));
        Ok(path)
    }
}
