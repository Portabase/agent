use super::service::RestoreService;

use anyhow::Result;
use futures::StreamExt;
use reqwest::{Client, Url};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use crate::services::backup::logger::JobLogger;

/// Human-readable byte count for the end-of-download log (avoids "0 MB" for small files).
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
    pub async fn download_backup(&self, file_url: &str, tmp_path: &Path, logger: Arc<JobLogger>) -> Result<PathBuf> {
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

        // Stream the body to disk in constant memory (was: response.bytes() buffered
        // the whole file in RAM, hanging on >5GB downloads).
        let start = Instant::now();
        let mut file = tokio::fs::File::create(&path).await?;
        let mut stream = response.bytes_stream();
        let mut downloaded: u64 = 0;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
        }

        file.flush().await?;

        if downloaded == 0 {
            logger.log(
                "warn",
                format!("Downloaded 0 bytes (status {status}); backup body was empty"),
            );
        }

        logger.log(
            "info",
            format!(
                "Backup downloaded to {} ({} / {} bytes in {:.1}s)",
                path.display(),
                human_size(downloaded),
                downloaded,
                start.elapsed().as_secs_f64()
            ),
        );

        Ok(path)
    }
}
