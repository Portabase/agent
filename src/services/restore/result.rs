use super::service::RestoreService;
use super::models::RestoreResult;

use tracing::{info, error};

impl RestoreService {
    pub async fn send_result(&self, result: RestoreResult) {

        info!(
            "[RestoreService] DB: {} | Status: {}",
            result.generated_id, result.status
        );

        match self.ctx
            .api
            .restore_result(
                self.ctx.edge_key.agent_id.clone(),
                &result.generated_id,
                &result.status,
            )
            .await {
            Ok(_) => {
                info!("Restoration result sent successfully");
            }
            Err(e) => {
                error!("Failed to send restoration result: {}", e);
            }
        }
    }
}