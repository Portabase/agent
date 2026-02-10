#![allow(dead_code)]

use crate::core::context::Context;
use crate::domain::factory::DatabaseFactory;
use crate::services::config::{DatabaseConfig, DatabasesConfig};
use anyhow::Result;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;
use tracing::{error, info};
use crate::services::api::models::agent::status::DatabaseStatus;
use crate::utils::compress::decompress_large_tar_gz;

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
                        match RestoreService::run(db_cfg, &tmp_path, &file_to_restore.unwrap()).await {
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
        cfg: DatabaseConfig,
        tmp_path: &Path,
        file_url: &str,
    ) -> Result<RestoreResult> {
        let generated_id = cfg.generated_id.clone();

        info!("File url: {}", file_url);

        let client = reqwest::Client::new();
        let response = client.get(file_url).send().await?;
        if !response.status().is_success() {
            error!("Backup download failed with status {}", response.status());
            return Ok(RestoreResult {
                generated_id,
                status: "failed".into(),
            });
        }

        let bytes = response.bytes().await?;

        let compressed_archive = tmp_path.join("compressed_archive_tmp.tar.gz");
        tokio::fs::write(&compressed_archive, &bytes).await?;
        info!("Backup downloaded to {}", compressed_archive.display());

        let decompressed_files = decompress_large_tar_gz(compressed_archive.as_path(), tmp_path).await?;

        info!("Decompressed_files {:#?}", decompressed_files);

        if decompressed_files.is_empty() {
            return Ok(RestoreResult {
                generated_id,
                status: "failed".into(),
            });
        }

        let backup_file_path = if decompressed_files.len() == 1 {
            decompressed_files.get(0).unwrap()
        }else {
            compressed_archive.as_path()
        };

        info!("Decompressed file {}", backup_file_path.display());

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
                log::error!("Restore failed: {:?}", e);
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
