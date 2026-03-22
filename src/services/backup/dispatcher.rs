use super::service::BackupService;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::config::DatabasesConfig;
use crate::utils::common::BackupMethod;
use tracing::error;

impl BackupService {
    pub async fn dispatch(
        &self,
        generated_id: &String,
        config: &DatabasesConfig,
        method: BackupMethod,
        storages: &Vec<DatabaseStorage>,
        encrypt: bool,
    ) {
        let Some(cfg) = config
            .databases
            .iter()
            .find(|c| c.generated_id == generated_id.as_str())
        else {
            error!("Database config not found for {}", generated_id);
            return;
        };

        let service = Self {
            ctx: self.ctx.clone(),
        };

        let db_cfg = cfg.clone();
        let storages = storages.clone();
        let generated_id = generated_id.clone();

        tokio::spawn(async move {
            if let Err(e) = service
                .execute_backup(generated_id, db_cfg, method, storages, encrypt)
                .await
            {
                error!("Backup execution failed: {}", e);
            }
        });
    }
}
