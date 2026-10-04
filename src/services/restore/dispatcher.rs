use super::service::RestoreService;
use crate::services::api::models::agent::status::DatabaseStatus;
use crate::services::config::DatabasesConfig;

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

        if db.data.restore.engine.as_deref() == Some("restic") {
            let snapshot_id = db.data.restore.snapshot_id.clone();
            let storage = db.data.restore.storage.clone();
            let service = Self { ctx: self.ctx.clone() };
            let db_cfg = cfg.clone();
            tokio::spawn(async move {
                if let Err(e) = service.execute_restic_restore(db_cfg, snapshot_id, storage).await {
                    error!("Snapshot restore failed: {}", e);
                }
            });
            return;
        }

        let Some(file_to_restore) = db.data.restore.file.clone() else {
            error!("restore file not found");
            return;
        };

        let expected_size = db.data.restore.size.clone();

        let service = Self {
            ctx: self.ctx.clone(),
        };

        let db_cfg = cfg.clone();

        tokio::spawn(async move {
            if let Err(e) = service
                .execute_restore(db_cfg, file_to_restore, expected_size)
                .await
            {
                error!("Restore failed: {}", e);
            }
        });
    }
}
