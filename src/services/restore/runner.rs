use super::models::RestoreResult;
use super::service::RestoreService;

use crate::domain::factory::DatabaseFactory;
use crate::services::config::DatabaseConfig;

use anyhow::Result;
use std::path::PathBuf;
use tracing::{error, info};

impl RestoreService {
    pub async fn run_restore(
        &self,
        cfg: DatabaseConfig,
        backup_file: PathBuf,
    ) -> Result<RestoreResult> {
        let generated_id = cfg.generated_id.clone();

        let db = DatabaseFactory::create_for_restore(cfg.clone(), &backup_file).await;

        let reachable = db.ping().await.unwrap_or(false);

        info!("Reachable: {}", reachable);

        if !reachable {
            return Ok(RestoreResult {
                generated_id,
                status: "failed".into(),
            });
        }

        match db.restore(&backup_file, Some(false)).await {
            Ok(_) => Ok(RestoreResult {
                generated_id,
                status: "success".into(),
            }),

            Err(e) => {
                error!("Restore failed: {:?}", e);

                Ok(RestoreResult {
                    generated_id,
                    status: "failed".into(),
                })
            }
        }
    }
}
