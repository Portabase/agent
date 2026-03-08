use super::service::RestoreService;
use crate::services::config::DatabasesConfig;
use crate::services::api::models::agent::status::DatabaseStatus;

use tracing::error;

impl RestoreService {

    pub async fn dispatch(&self, db: &DatabaseStatus, config: &DatabasesConfig) {

        let Some(cfg) = config
            .databases
            .iter()
            .find(|c| c.generated_id == db.generated_id)
        else {
            error!("Database config not found");
            return;
        };

        let Some(file_to_restore) = db.data.restore.file.clone() else {
            error!("restore file not found");
            return;
        };

        let service = Self {
            ctx: self.ctx.clone(),
        };

        let db_cfg = cfg.clone();

        tokio::spawn(async move {

            if let Err(e) = service
                .execute_restore(db_cfg, file_to_restore)
                .await
            {
                error!("Restore failed: {}", e);
            }

        });
    }
}