use super::service::BackupService;
use super::models::{BackupResult, UploadResult};

use crate::services::storage;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::utils::common::BackupMethod;

use futures::future::join_all;
use anyhow::{Result, bail};
use tracing::{info, error};

impl BackupService {

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
            bail!("backup_already_in_progress");
        }

        let ctx = self.ctx.clone();

        let futures = storages.into_iter().map(|storage| {

            let ctx_clone = ctx.clone();
            let result_clone = result.clone();
            let provider = storage::get_provider(&storage);

            let storage_id = storage.id.clone();
            let generated_id = result_clone.generated_id.clone();

            async move {

                info!("Uploading storage -> {:?} for {:?}", storage.provider, storage_id);

                /*
                 INIT STEP
                */
                let init = match ctx_clone.api
                    .backup_upload_init(
                        ctx_clone.edge_key.agent_id.clone(),
                        generated_id.clone(),
                        storage_id.clone(),
                        backup_id,
                    )
                    .await
                {
                    Ok(v) => v,
                    Err(e) => {
                        error!("backup_upload_init failed: {}", e);

                        return UploadResult {
                            storage_id,
                            success: false,
                            error: Some("backup_upload_init failed".into()),
                            remote_file_path: None,
                            total_size: None,
                        };
                    }
                };

                let backup_storage_id = match init {
                    Some(v) => v.backup_storage.id,
                    None => {
                        return UploadResult {
                            storage_id,
                            success: false,
                            error: Some("backup_upload_init returned empty response".into()),
                            remote_file_path: None,
                            total_size: None,
                        };
                    }
                };

                /*
                 PROVIDER CHECK
                */
                let Some(provider) = provider else {
                    error!("Skipping storage due to missing provider");

                    return UploadResult {
                        storage_id,
                        success: false,
                        error: Some("missing provider".into()),
                        remote_file_path: None,
                        total_size: None,
                    };
                };

                /*
                 STORAGE UPLOAD
                */
                let upload_result = provider
                    .upload(
                        ctx_clone.clone(),
                        result_clone,
                        method,
                        &storage,
                        Some(encrypt),
                    )
                    .await;

                let status = if upload_result.success { "success" } else { "failed" };

                if status != "success" {
                    return upload_result;
                }

                info!(
                    "Storage {} uploaded to remote path {:?}",
                    storage_id,
                    upload_result.remote_file_path
                );

                /*
                 METADATA VALIDATION
                */
                let (remote_path, total_size) = match (
                    &upload_result.remote_file_path,
                    upload_result.total_size,
                ) {
                    (Some(path), Some(size)) => (path.clone(), size),
                    _ => {
                        return UploadResult {
                            storage_id,
                            success: false,
                            error: Some("remote_file_path or total_size missing".into()),
                            remote_file_path: None,
                            total_size: None,
                        };
                    }
                };

                /*
                 STATUS UPDATE
                */
                match ctx_clone.api.backup_upload_status(
                    ctx_clone.edge_key.agent_id.clone(),
                    generated_id,
                    backup_storage_id,
                    status,
                    remote_path,
                    total_size,
                    backup_id,
                ).await {

                    Ok(_) => upload_result,

                    Err(err) => {
                        error!(
                            "backup_upload_status failed (storage_id={}): {}",
                            storage_id,
                            err
                        );

                        UploadResult {
                            storage_id,
                            success: false,
                            error: Some(err.to_string()),
                            remote_file_path: None,
                            total_size: None,
                        }
                    }
                }
            }
        });

        let results: Vec<UploadResult> = join_all(futures).await;

        info!("Upload results: {:#?}", results);

        Ok(results)
    }
}