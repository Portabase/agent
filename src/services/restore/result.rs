use super::models::RestoreResult;
use super::service::RestoreService;
use crate::services::api::ApiError;
use crate::services::api::models::agent::restore::ResultRestoreResponse;

use tracing::{error, info};
use crate::services::backup::logger::JobLogEntry;

impl RestoreService {
    pub async fn send_result(
        &self,
        result: RestoreResult,
        logs: Vec<JobLogEntry>,
        duration_ms: f64,
    ) -> Result<Option<ResultRestoreResponse>, ApiError> {
        info!(
            "[RestoreService] DB: {} | Status: {} | Duration: {}ms",
            result.generated_id,
            result.status,
            duration_ms
        );

        self.ctx
            .api
            .restore_result(
                self.ctx.edge_key.agent_id.clone(),
                &result.generated_id,
                &result.status,
                logs,
                duration_ms
            )
            .await
            .map_err(|e| {
                error!("Failed to send restoration result: {}", e);
                e.into()
            })
    }
}