use super::service::RestoreService;
use super::models::RestoreResult;

use tracing::{info, error};

impl RestoreService {
    // TODO : update with ctx api manager
    pub async fn send_result(&self, result: RestoreResult) {

        info!(
            "[RestoreService] DB: {} | Status: {}",
            result.generated_id, result.status
        );

        let client = reqwest::Client::new();

        let url = format!(
            "{}/api/agent/{}/restore",
            self.ctx.edge_key.server_url,
            self.ctx.edge_key.agent_id
        );

        match client.post(&url).json(&result).send().await {

            Ok(resp) => {
                if resp.status().is_success() {
                    info!("Restoration result sent successfully");
                } else {
                    let text = resp.text().await.unwrap_or_default();

                    error!("Restore result failed: {}", text);
                }
            }

            Err(e) => {
                error!("Failed to send restoration result: {}", e);
            }
        }
    }
}