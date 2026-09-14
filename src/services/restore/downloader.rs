use super::service::RestoreService;

use crate::services::backup::logger::JobLogger;
use crate::utils::retry::{RetryPolicy, retry};
use anyhow::Result;
use futures::StreamExt;
use reqwest::{Client, Url};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::AsyncWriteExt;

fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{} MB", bytes / 1024 / 1024)
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

impl RestoreService {
    pub async fn download_backup(
        &self,
        file_url: &str,
        tmp_path: &Path,
        logger: Arc<JobLogger>,
        expected_size: Option<String>,
    ) -> Result<PathBuf> {
        let policy = RetryPolicy::default();

        let logger_ref = &logger;

        let outcome = retry("Backup download", &logger, &policy, move |_| {
            let expected = expected_size.clone();

            async move {
                self.download_once(file_url, tmp_path, Arc::clone(logger_ref), expected)
                    .await
            }
        })
        .await;

        if let Err(e) = &outcome {
            logger.log("error", format!("Download failed: {e}"));
        }

        outcome
    }

    pub async fn download_once(
        &self,
        file_url: &str,
        tmp_path: &Path,
        logger: Arc<JobLogger>,
        expected_size: Option<String>,
    ) -> Result<PathBuf> {
        logger.log("info", "Start downloading backup archive".to_string());

        let client = Client::new();

        let response = client.get(file_url).send().await?;
        let status = response.status();

        if !status.is_success() {
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

        let total = expected_size
            .as_deref()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .filter(|&n| n > 0);

        logger.log(
            "info",
            format!(
                "Downloading backup '{}' ({})",
                filename,
                total
                    .map(human_size)
                    .unwrap_or_else(|| "unknown size".to_string())
            ),
        );

        let start = Instant::now();
        let mut file = tokio::fs::File::create(&path).await?;
        let mut stream = response.bytes_stream();
        let mut downloaded: u64 = 0;
        let mut next_pct: u64 = 10;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;

            if let Some(total) = total {
                let pct = (downloaded.saturating_mul(100) / total).min(100);
                let milestone = pct / 10 * 10;
                if milestone >= next_pct {
                    logger.log(
                        "info",
                        format!(
                            "Download progress: {}% ({} / {} bytes)",
                            milestone, downloaded, total
                        ),
                    );
                    next_pct = milestone + 10;
                }
            }
        }

        file.flush().await?;

        if downloaded == 0 {
            logger.log(
                "warn",
                format!("Downloaded 0 bytes (status {status}); backup body was empty"),
            );
        }

        if let Some(total) = total
            && downloaded < total
        {
            anyhow::bail!(
                "Downloaded {} bytes but expected at least {} - backup appears truncated",
                downloaded,
                total
            );
        }

        logger.log(
            "info",
            format!(
                "Backup downloaded to {} ( {} bytes in {:.1}s)",
                path.display(),
                downloaded,
                start.elapsed().as_secs_f64()
            ),
        );

        Ok(path)
    }
}
