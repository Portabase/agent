use super::logger::JobLogger;
use super::models::BackupResult;
use super::service::BackupService;

use crate::domain::factory::DatabaseFactory;
use crate::services::config::DatabaseConfig;

use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info};

impl BackupService {
    pub async fn run(cfg: DatabaseConfig, tmp_path: &Path, logger: Arc<JobLogger>) -> Result<BackupResult> {
        let db = DatabaseFactory::create_for_backup(cfg.clone()).await;

        let generated_id = cfg.generated_id.clone();
        let db_type = cfg.db_type.clone();

        let reachable = match db.ping().await {
            Ok(v) => v,
            Err(e) => {
                error!("Ping failed: {}", e);
                logger.log("error", format!("Ping failed: {}", e));
                return Err(e.into());
            }
        };

        info!("Reachable: {}", reachable);
        logger.log("info", format!("Database reachable: {}", reachable));

        if !reachable {
            logger.log("error", "Database unreachable, backup aborted");
            return Ok(BackupResult {
                generated_id,
                db_type,
                status: "failed".into(),
                backup_file: None,
                code: None,
            });
        }

        match db.backup(tmp_path, Arc::clone(&logger)).await {
            Ok(file) => Ok(BackupResult {
                generated_id,
                db_type,
                status: "success".into(),
                backup_file: Some(file),
                code: None,
            }),

            Err(e) if e.to_string() == "backup_already_in_progress" => {
                logger.log("warn", "Backup already in progress");
                Ok(BackupResult {
                    generated_id,
                    db_type,
                    status: "failed".into(),
                    backup_file: None,
                    code: Some("backup_already_in_progress".into()),
                })
            }

            Err(e) => {
                error!("Backup failed for {}: {:?}", generated_id, e);
                logger.log("error", format!("Backup failed: {}", e));
                Ok(BackupResult {
                    generated_id,
                    db_type,
                    status: "failed".into(),
                    backup_file: None,
                    code: None,
                })
            }
        }
    }
}
