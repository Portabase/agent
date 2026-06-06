use super::logger::JobLogEntry;
use super::models::{BackupResult, UploadResult};
use super::service::BackupService;
use crate::services::api::ApiError;
use crate::services::api::models::agent::backup::BackupResponse;
use anyhow::Result;
use tracing::{error, info};

impl BackupService {
    pub async fn send_result(
        &self,
        result: BackupResult,
        upload_results: Vec<UploadResult>,
        backup_id: &String,
        logs: Vec<JobLogEntry>,
        duration_ms: f64,
    ) -> Result<Option<BackupResponse>, ApiError> {
        let status = if upload_results.iter().any(|r| r.success) {
            "success"
        } else {
            "failed"
        };

        let file_size = upload_results
            .iter()
            .filter_map(|r| r.total_size)
            .reduce(|a, b| a + b)
            .map(|sum| sum / upload_results.len() as u64);

        info!("{:#?}",logs);

        self.ctx
            .api
            .backup_update(
                self.ctx.edge_key.agent_id.clone(),
                backup_id,
                status,
                file_size,
                &result.generated_id,
                logs,
                duration_ms
            )
            .await
            .map_err(|e| {
                error!(
                    "backup_update failed (generated_id={}, backup_id={}): {}",
                    result.generated_id, backup_id, e
                );
                e.into()
            })
    }
}
