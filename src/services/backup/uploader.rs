use super::logger::JobLogger;
use super::models::{BackupResult, UploadResult};
use super::service::BackupService;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::storage;
use crate::utils::common::BackupMethod;
use anyhow::{Result, bail};
use futures::future::join_all;
use std::sync::Arc;
use tracing::info;

impl BackupService {
    pub async fn upload(
        &self,
        result: BackupResult,
        method: BackupMethod,
        storages: Vec<DatabaseStorage>,
        encrypt: bool,
        backup_id: &String,
        logger: Arc<JobLogger>,
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
            let logger_clone = Arc::clone(&logger);

            let storage_id = storage.id.clone();
            let generated_id = result_clone.generated_id.clone();

            async move {
                logger_clone.log("info", format!("Uploading storage {:?} (id: {})", storage.provider, storage_id));

                /*
                 INIT STEP
                */
                let init = match ctx_clone
                    .api
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
                        logger_clone.log("error", format!("Upload init failed: {}", e));

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
                        logger_clone.log("error", "Upload init returned empty response");
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
                    logger_clone.log("error", format!("Missing provider for storage {}", storage_id));

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
                    logger_clone.log("error", format!(
                        "Upload failed for storage {}: {}",
                        storage_id,
                        upload_result.error.as_deref().unwrap_or("unknown error")
                    ));
                    return upload_result;
                }

                logger_clone.log("info", format!(
                    "Storage {} uploaded to {:?} ({} bytes)",
                    storage_id,
                    upload_result.remote_file_path.clone().unwrap().to_string(),
                    upload_result.total_size.unwrap_or(0)
                ));

                /*
                 METADATA VALIDATION
                */
                let (remote_path, total_size) =
                    match (&upload_result.remote_file_path, upload_result.total_size) {
                        (Some(path), Some(size)) => (path.clone(), size),
                        _ => {
                            logger_clone.log("error", format!("Missing remote_file_path or total_size for storage {}", storage_id));
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
                match ctx_clone
                    .api
                    .backup_upload_status(
                        ctx_clone.edge_key.agent_id.clone(),
                        generated_id,
                        backup_storage_id,
                        status,
                        remote_path,
                        total_size,
                        backup_id,
                    )
                    .await
                {
                    Ok(_) => upload_result,

                    Err(err) => {
                        logger_clone.log("error", format!("Upload status update failed for {}: {}", storage_id, err));

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
