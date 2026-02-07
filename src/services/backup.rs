#![allow(dead_code)]

use crate::core::context::Context;
use crate::domain::factory::DatabaseFactory;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::config::{DatabaseConfig, DatabasesConfig, DbType};
use crate::services::storage;
use crate::utils::common::BackupMethod;
use crate::utils::compress::compress_to_tar_gz_large;
use anyhow::Result;
use futures::future::join_all;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct BackupResult {
    pub generated_id: String,
    pub db_type: DbType,
    pub status: String,
    pub backup_file: Option<PathBuf>,
    pub code: Option<String>,
}

#[derive(Debug)]
pub struct UploadResult {
    pub storage_id: String,
    pub success: bool,
    pub error: Option<String>,
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
        storages: &Vec<DatabaseStorage>,
        encrypt: bool,
    ) {
        if let Some(cfg) = config
            .databases
            .iter()
            .find(|c| c.generated_id == generated_id.as_str())
        {
            let db_cfg = cfg.clone();
            let ctx_clone = self.ctx.clone();
            let storages_clone = storages.clone();

            tokio::spawn(async move {
                match TempDir::new() {
                    Ok(temp_dir) => {
                        // trigger backup POST

                        let tmp_path = temp_dir.path().to_path_buf();
                        info!("Created temp directory {}", tmp_path.display());
                        match BackupService::run(db_cfg, &tmp_path).await {
                            Ok(mut result) => {
                                if let Some(backup_file) = result.backup_file.take() {
                                    match compress_to_tar_gz_large(&backup_file).await {
                                        Ok(compression_result) => {
                                            result.backup_file =
                                                Some(compression_result.compressed_path);
                                            let service = BackupService { ctx: ctx_clone };
                                            match service
                                                .upload(
                                                    result.clone(),
                                                    method,
                                                    storages_clone.clone(),
                                                    encrypt,
                                                )
                                                .await
                                            {
                                                Ok(upload_result) => {
                                                    match service
                                                        .send_result(result, method, upload_result)
                                                        .await
                                                    {
                                                        Ok(_) => {
                                                            return;
                                                        }
                                                        Err(e) => {
                                                            error!(
                                                                "Failed to send backup result: {}",
                                                                e
                                                            );
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    error!("Failed to upload backup files: {}", e);
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            error!("Failed to compress backup file : {}", e);
                                        }
                                    }
                                } else {
                                    error!("No backup file generated");
                                }
                            }
                            Err(e) => error!("BackupService run failed: {}", e),
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

    pub async fn upload(
        &self,
        result: BackupResult,
        method: BackupMethod,
        storages: Vec<DatabaseStorage>,
        encrypt: bool,
    ) -> Result<Vec<UploadResult>> {
        if result.code.as_deref() == Some("backup_already_in_progress") {
            info!("Skipping send: backup already in progress");
            anyhow::bail!("backup_already_in_progres");
        }

        info!("Storages : {:#?}", storages);

        let upload_futures = storages.into_iter().map(|storage| {
            info!(
                "Uploading storage -> {:?} for {:?}",
                storage.provider, storage.id
            );
            let provider = storage::get_provider(&storage);
            let result_clone = result.clone();
            let ctx_clone = self.ctx.clone();
            let storages_clone = storage.clone();
            let storage_id = storages_clone.id;

            async move {
                match self
                    .ctx
                    .api
                    .backup_upload_init(self.ctx.edge_key.agent_id.clone(), "", "", storage_id.clone())
                    .await
                {
                    Ok(_) => match provider {
                        Some(provider) => {
                            provider
                                .upload(ctx_clone, result_clone, method, &storage, Some(encrypt))
                                .await
                        }
                        None => {
                            error!("Skipping storage due to missing provider");
                            UploadResult {
                                storage_id: storage_id.clone(),
                                success: false,
                                error: Some("Skipping storage due to missing provider".to_string()),
                            }
                        }
                    },
                    Err(e) => {
                        error!(
                            "Unable to create the storage backup on remote server : {}",
                            e
                        );
                        UploadResult {
                            storage_id: storage_id.clone(),
                            success: false,
                            error: Some(
                                "Unable to create the storage backup on remote server".to_string(),
                            ),
                        }
                    }
                }
            }
        });

        let results: Vec<UploadResult> = join_all(upload_futures).await;
        info!("Upload results: {:#?}", results);

        Ok(results)
    }

    pub async fn send_result(
        &self,
        result: BackupResult,
        method: BackupMethod,
        upload_results: Vec<UploadResult>,
    ) -> Result<()> {
        Ok(())
    }
}
