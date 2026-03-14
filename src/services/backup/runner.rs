use super::models::BackupResult;
use super::service::BackupService;

use crate::domain::factory::DatabaseFactory;
use crate::services::config::DatabaseConfig;

use anyhow::Result;
use std::path::Path;
use tracing::{error, info};

impl BackupService {

    pub async fn run(
        cfg: DatabaseConfig,
        tmp_path: &Path,
    ) -> Result<BackupResult> {

        let db = DatabaseFactory::create_for_backup(cfg.clone()).await;

        let generated_id = cfg.generated_id.clone();
        let db_type = cfg.db_type.clone();

        let reachable = match db.ping().await {
            Ok(v) => v,
            Err(e) => {
                error!("Ping failed: {}", e);
                return Err(e.into());
            }
        };

        info!("Reachable: {}", reachable);

        if !reachable {
            return Ok(BackupResult {
                generated_id,
                db_type,
                status: "failed".into(),
                backup_file: None,
                code: None,
            });
        }

        match db.backup(tmp_path, Some(false)).await {

            Ok(file) => Ok(BackupResult {
                generated_id,
                db_type,
                status: "success".into(),
                backup_file: Some(file),
                code: None,
            }),

            Err(e) if e.to_string() == "backup_already_in_progress" => Ok(BackupResult {
                generated_id,
                db_type,
                status: "failed".into(),
                backup_file: None,
                code: Some("backup_already_in_progress".into()),
            }),

            Err(_) => Ok(BackupResult {
                generated_id,
                db_type,
                status: "failed".into(),
                backup_file: None,
                code: None,
            }),
        }
    }
}