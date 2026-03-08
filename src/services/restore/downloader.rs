use super::service::RestoreService;

use reqwest::{Client, Url};
use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::info;

impl RestoreService {

    pub async fn download_backup(
        &self,
        file_url: &str,
        tmp_path: &Path,
    ) -> Result<PathBuf> {

        let client = Client::new();

        let response = client.get(file_url).send().await?;

        if !response.status().is_success() {
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

        info!("Backup downloaded to {}", path.display());

        Ok(path)
    }
}