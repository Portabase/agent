#![allow(dead_code)]

use crate::core::context::Context;
use crate::domain::factory::DatabaseFactory;
use crate::services::config::{DatabaseConfig, DatabasesConfig, DbType};
use crate::utils::common::BackupMethod;
use crate::utils::file::{encrypt_file_stream, full_extension};
use anyhow::Result;
use hex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use rand::RngCore;
use tempfile::TempDir;
use tracing::{error, info};
use reqwest::Body;
use futures::StreamExt;

#[derive(Debug)]
pub struct BackupResult {
    pub generated_id: String,
    pub db_type: DbType,
    pub status: String,
    pub backup_file: Option<PathBuf>,
    pub code: Option<String>,
}

pub struct BackupService {
    ctx: Arc<Context>,
}

impl BackupService {
    pub fn new(ctx: Arc<Context>) -> Self {
        Self { ctx }
    }

    pub async fn dispatch(
        &self,
        generated_id: &String,
        config: &DatabasesConfig,
        method: BackupMethod,
    ) {
        if let Some(cfg) = config
            .databases
            .iter()
            .find(|c| c.generated_id == generated_id.as_str())
        {
            let db_cfg = cfg.clone();
            let ctx_clone = self.ctx.clone();

            tokio::spawn(async move {
                match TempDir::new() {
                    Ok(temp_dir) => {
                        let tmp_path = temp_dir.path().to_path_buf();
                        info!("Created temp directory {}", tmp_path.display());

                        match BackupService::run(db_cfg, &tmp_path).await {
                            Ok(result) => {
                                let service = BackupService { ctx: ctx_clone };
                                service.send_result(result, method).await;
                            }
                            Err(e) => error!("Backup error {}", e),
                        }
                        // TempDir is automatically deleted when dropped here
                    }
                    Err(e) => error!("Failed to create temp dir: {}", e),
                }
            });
        }
    }

    pub async fn run(cfg: DatabaseConfig, tmp_path: &Path) -> Result<BackupResult> {
        let db_instance = DatabaseFactory::create_for_backup(cfg.clone()).await;
        let generated_id = cfg.generated_id.clone();
        let db_type = cfg.db_type.clone();

        let reachable = db_instance.ping().await.unwrap_or(false);
        info!("Reachable: {}", reachable);
        if !reachable {
            return Ok(BackupResult {
                generated_id,
                db_type,
                status: "failed".into(),
                backup_file: None,
                code: None,
            });
        }

        match db_instance.backup(tmp_path).await {
            Ok(file) => Ok(BackupResult {
                generated_id,
                db_type,
                status: "success".into(),
                backup_file: Some(file),
                code: None,
            }),
            Err(e) => match e.to_string().as_str() {
                "backup_already_in_progress" => Ok(BackupResult {
                    generated_id,
                    db_type,
                    status: "failed".into(),
                    backup_file: None,
                    code: Some(e.to_string()),
                }),
                _ => Ok(BackupResult {
                    generated_id,
                    db_type,
                    status: "failed".into(),
                    backup_file: None,
                    code: None,
                }),
            },
        }
    }

    pub async fn send_result(&self, result: BackupResult, method: BackupMethod) {
        if result.code.as_deref() == Some("backup_already_in_progress") {
            info!("Skipping send: backup already in progress");
            return;
        }

        let Some(file_path) = result.backup_file else {
            return;
        };

        let mut aes_key = [0u8; 32];
        rand::rng().fill_bytes(&mut aes_key);

        let mut iv = [0u8; 16];
        rand::rng().fill_bytes(&mut iv);

        let public_key_pem = self.ctx.edge_key.public_key.as_bytes().to_vec();


        let (encrypted_stream, encrypted_key_hex) =
            match encrypt_file_stream(
                file_path.clone(),
                aes_key,
                iv,
                public_key_pem,
            )
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    error!("Encryption failed: {}", e);
                    return;
                }
            };

        let file_size = std::fs::metadata(&file_path)
            .map(|m| m.len())
            .unwrap_or(0);

        let body = Body::wrap_stream(
            encrypted_stream.map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)))
        );

        let url = format!(
            "{}/services/v1/upload/{}",
            self.ctx.edge_key.server_url,
            self.ctx.edge_key.agent_id
        );

        let client = reqwest::Client::new();


        info!("file Size to {}", file_size);

        let resp = client
            .post(&url)
            .header("X-Generated-Id", &result.generated_id)
            .header("X-Status", &result.status)
            .header("X-Method", method.to_string())
            .header("X-AES-Key", encrypted_key_hex)
            .header("X-IV", hex::encode(iv))
            .header("X-Extension", full_extension(&file_path))
            .header("Transfer-Encoding", "chunked")
            .header("X-File-Size", file_size)
            .body(body)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                info!("Backup uploaded successfully");
            }
            Ok(r) => {
                error!("Upload failed: {}", r.status());
            }
            Err(e) => {
                error!("Upload error: {}", e);
            }
        }
    }
}
