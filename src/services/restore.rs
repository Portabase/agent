#![allow(dead_code)]
#![warn(unused_assignments)]
use crate::core::context::Context;
use crate::domain::factory::DatabaseFactory;
use crate::services::api::models::agent::status::DatabaseStatus;
use crate::services::config::{DatabaseConfig, DatabasesConfig};
use crate::utils::compress::decompress_large_tar_gz;
use crate::utils::file::decrypt_file_stream_gcm;
use anyhow::Result;
use reqwest::{Client, Url};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use tracing::{error, info};

#[derive(Debug, Serialize)]
pub struct RestoreResult {
    #[serde(rename = "generatedId")]
    pub generated_id: String,
    pub status: String,
}

pub struct RestoreService {
    ctx: Arc<Context>,
}

impl RestoreService {
    pub fn new(ctx: Arc<Context>) -> Self {
        Self { ctx }
    }

    pub async fn dispatch(&self, db: &DatabaseStatus, config: &DatabasesConfig) {
        if let Some(cfg) = config
            .databases
            .iter()
            .find(|c| c.generated_id == db.generated_id)
        {
            let db_cfg = cfg.clone();
            let ctx_clone = self.ctx.clone();
            let file_to_restore = db.data.restore.file.clone();
            if file_to_restore.is_none() {
                error!("restore file not found");
                return;
            }
            tokio::spawn(async move {
                match TempDir::new() {
                    Ok(temp_dir) => {
                        let tmp_path = temp_dir.path().to_path_buf();
                        info!("Created temp directory {}", tmp_path.display());
                        match RestoreService::run(
                            &ctx_clone,
                            db_cfg,
                            &tmp_path,
                            &file_to_restore.unwrap(),
                        )
                        .await
                        {
                            Ok(result) => {
                                let service = RestoreService { ctx: ctx_clone };
                                service.send_result(result).await;
                            }
                            Err(e) => error!("Restoration error {}", e),
                        }
                        // TempDir is automatically deleted when dropped
                    }
                    Err(e) => error!("Failed to create temp dir: {}", e),
                }
            });
        }
    }

    pub async fn run(
        ctx: &Arc<Context>,
        cfg: DatabaseConfig,
        tmp_path: &Path,
        file_url: &str,
    ) -> Result<RestoreResult> {
        let generated_id = cfg.generated_id.clone();

        info!("File url: {}", file_url);

        let client = Client::new();
        let response = client.get(file_url).send().await?;

        if !response.status().is_success() {
            error!("Backup download failed with status {}", response.status());
            return Ok(RestoreResult {
                generated_id,
                status: "failed".into(),
            });
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

        info!("File name: {}", filename);

        let bytes = response.bytes().await?;

        let is_legacy_file = if filename.ends_with(".sql") {
            true
        } else if filename.ends_with(".dump") {
            true
        } else {
            false
        };
        let downloaded_file = tmp_path.join(&filename);
        tokio::fs::write(&downloaded_file, &bytes).await?;
        info!("Backup downloaded to {}", downloaded_file.display());

        let backup_file_path: PathBuf = if !is_legacy_file {
            let encrypted = if filename.ends_with(".tar.gz") {
                false
            } else if filename.ends_with(".tar.gz.enc") {
                true
            } else {
                return Ok(RestoreResult {
                    generated_id,
                    status: "failed".into(),
                });
            };

            info!("Encrypted: {}", encrypted);

            let mut compressed_archive = downloaded_file.clone();

            if encrypted {
                let new_name = downloaded_file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_suffix(".enc"))
                    .ok_or_else(|| anyhow::anyhow!("Invalid encrypted filename"))?;

                let new_compressed_archive = tmp_path.join(new_name);

                decrypt_file_stream_gcm(
                    downloaded_file,
                    new_compressed_archive.clone(),
                    ctx.edge_key.master_key_b64.clone(),
                )
                .await
                .map_err(|e| {
                    error!("Failed to decrypt file: {}", e);
                    e
                })?;

                compressed_archive = new_compressed_archive;
            }

            let decompressed_files =
                decompress_large_tar_gz(compressed_archive.as_path(), tmp_path).await?;

            if decompressed_files.is_empty() {
                return Ok(RestoreResult {
                    generated_id,
                    status: "failed".into(),
                });
            }

            if decompressed_files.len() == 1 {
                decompressed_files[0].clone()
            } else {
                compressed_archive
            }
        } else {
            downloaded_file.clone()
        };

        let db_instance = DatabaseFactory::create_for_restore(cfg.clone(), &backup_file_path).await;
        let reachable = db_instance.ping().await.unwrap_or(false);
        info!("Reachable: {}", reachable);
        if !reachable {
            return Ok(RestoreResult {
                generated_id,
                status: "failed".into(),
            });
        }

        match db_instance.restore(&backup_file_path).await {
            Ok(_) => Ok(RestoreResult {
                generated_id,
                status: "success".into(),
            }),
            Err(e) => {
                error!("Restore failed: {:?}", e);
                Ok(RestoreResult {
                    generated_id,
                    status: "failed".into(),
                })
            }
        }
    }

    // TODO : update with ctx api manager
    pub async fn send_result(&self, result: RestoreResult) {
        info!(
            "[RestoreService] DB: {} | Status: {}",
            result.generated_id, result.status,
        );

        let client = reqwest::Client::new();
        let url = format!(
            "{}/api/agent/{}/restore",
            self.ctx.edge_key.server_url, self.ctx.edge_key.agent_id
        );

        let body = RestoreResult {
            generated_id: result.generated_id,
            status: result.status,
        };

        match client.post(&url).json(&body).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    info!("Restoration result sent successfully");
                } else {
                    let text = resp.text().await.unwrap_or_default();
                    error!(
                        "Restoration result failed, status: {}, body: {}",
                        status, text
                    );
                }
            }
            Err(e) => {
                error!("Failed to send restoration result: {}", e);
            }
        }
    }
}
