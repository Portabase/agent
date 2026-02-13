#![allow(dead_code)]

use crate::core::context::Context as CoreContext;
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
use crate::utils::locks::FileLock;

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
    pub remote_file_path: Option<String>,
    pub total_size: Option<u64>,
}

pub struct BackupService {
    ctx: Arc<CoreContext>,
}

impl BackupService {
    pub fn new(ctx: Arc<CoreContext>) -> Self {
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
            let ctx = self.ctx.clone();
            let storages_clone = storages.clone();
            let generated_id_clone = generated_id.clone();

            tokio::spawn(async move {
                match TempDir::new() {
                    Ok(temp_dir) => {
                        match FileLock::is_locked(&generated_id_clone).await {
                            Ok(true) => {
                                error!("Backup already running for {}", &generated_id_clone);
                                return;
                            }
                            Ok(false) => {

                                match ctx
                                    .api
                                    .backup_create(
                                        method.clone().to_string(),
                                        ctx.edge_key.agent_id.clone(),
                                        &generated_id_clone,
                                    )
                                    .await
                                {
                                    Ok(backup_created_result) => {
                                        info!("Backup created successfully");
                                        let tmp_path = temp_dir.path().to_path_buf();
                                        info!("Created temp directory {}", tmp_path.display());
                                        match BackupService::run(db_cfg, &tmp_path).await {
                                            Ok(mut result) => {
                                                if let Some(backup_file) = result.backup_file.take() {
                                                    match compress_to_tar_gz_large(&backup_file).await {
                                                        Ok(compression_result) => {
                                                            result.backup_file =
                                                                Some(compression_result.compressed_path);
                                                            let service = BackupService { ctx: ctx.clone() };
                                                            let backup_id = backup_created_result.unwrap().backup.id;
                                                            match service
                                                                .upload(
                                                                    result.clone(),
                                                                    method,
                                                                    storages_clone.clone(),
                                                                    encrypt,
                                                                    &backup_id,
                                                                )
                                                                .await
                                                            {
                                                                Ok(upload_result) => {
                                                                    match service
                                                                        .send_result(
                                                                            result,
                                                                            upload_result,
                                                                            &backup_id
                                                                        )
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
                                                                    error!(
                                                                "Failed to upload backup files: {}",
                                                                e
                                                            );
                                                                }
                                                            }
                                                        }
                                                        Err(e) => {
                                                            error!(
                                                        "Failed to compress backup file : {}",
                                                        e
                                                    );
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
                                    Err(e) => error!("Backup creation failed: {}", e),
                                }
                            },
                            Err(e) => error!("An error occurred while checking lock : {}", e),
                        }
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
        backup_id: &String,
    ) -> Result<Vec<UploadResult>> {
        if result.code.as_deref() == Some("backup_already_in_progress") {
            info!("Skipping send: backup already in progress");
            anyhow::bail!("backup_already_in_progres");
        }

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
            let generated_id = result_clone.generated_id.clone();

            async move {
                match self
                    .ctx
                    .api
                    .backup_upload_init(
                        self.ctx.edge_key.agent_id.clone(),
                        generated_id.clone(),
                        storage_id.clone(),
                        backup_id,
                    )
                    .await
                {
                    Ok(upload_init_result) => {
                        info!("Uploading init result: {:#?}", upload_init_result);
                        let backup_storage_id = upload_init_result.unwrap().backup_storage.id.clone();
                        match provider {
                            Some(provider) => {
                                let upload_result = provider
                                    .upload(
                                        ctx_clone,
                                        result_clone,
                                        method,
                                        &storage,
                                        Some(encrypt),
                                    )
                                    .await;

                                let status = if upload_result.success {
                                    "success"
                                } else {
                                    "failed"
                                };
                                info!("Storage {} uploaded to remote path {:?}", storage_id, upload_result.remote_file_path);


                                let (remote_path, total_size) = match (
                                    &upload_result.remote_file_path,
                                    upload_result.total_size,
                                ) {
                                    (Some(path), Some(size)) => (path.clone(), size),
                                    _ => {
                                        return UploadResult {
                                            storage_id: storage_id.clone(),
                                            success: false,
                                            error: Some("remote_file_path or total_size missing".to_string()),
                                            remote_file_path: None,
                                            total_size: None,
                                        }
                                    }
                                };
                                
                                match self.ctx.api.backup_upload_status(
                                    self.ctx.edge_key.agent_id.clone(),
                                    generated_id.clone(),
                                    backup_storage_id,
                                    status,
                                    remote_path,
                                    total_size,
                                    backup_id
                                ).await {
                                    Ok(_) => {
                                        upload_result
                                    },
                                    Err(err)=> {
                                        error!(
                                        "backup_upload_status failed (generated_id={}, storage_id={}): {}",
                                        generated_id, storage_id, err
                                    );
                                        UploadResult {
                                            storage_id: storage_id.clone(),
                                            success: false,
                                            error: Some(err.to_string()),
                                            remote_file_path: None,
                                            total_size: None,
                                        }
                                    }
                                }
                            }
                            None => {
                                error!("Skipping storage due to missing provider");
                                UploadResult {
                                    storage_id: storage_id.clone(),
                                    success: false,
                                    error: Some(
                                        "Skipping storage due to missing provider".to_string(),
                                    ),
                                    remote_file_path: None,
                                    total_size: None,
                                }
                            }
                        }
                    }
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
                            remote_file_path: None,
                            total_size: None,
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
        upload_results: Vec<UploadResult>,
        backup_id: &String,
    ) -> Result<()> {
        let status = if upload_results.iter().any(|r| r.success) {
            "success"
        } else {
            "failed"
        };

        let file_size = upload_results
            .iter()
            .map(|r| r.total_size)
            .try_fold((0u64, 0u64), |(sum, count), v| {
                match v {
                    Some(size) => Ok((sum + size, count + 1)),
                    None => Err(()), // stop and return None
                }
            })
            .ok()
            .map(|(sum, count)| sum / count);

        match self
            .ctx
            .api
            .backup_update(self.ctx.edge_key.agent_id.clone(), backup_id, status, file_size, &result.generated_id)
            .await
        {
            Ok(_result) => Ok(()),
            Err(e) => {
                error!(
                    "backup_update failed (generated_id={}, backup_id={}): {}",
                    &result.generated_id, &backup_id, e
                );
                Err(e.into())
            }
        }
    }
}
