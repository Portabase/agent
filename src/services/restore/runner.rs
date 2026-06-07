use super::models::RestoreResult;
use super::service::RestoreService;

use crate::domain::factory::DatabaseFactory;
use crate::services::config::DatabaseConfig;

use crate::services::backup::logger::JobLogger;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

impl RestoreService {
    pub async fn run_restore(
        &self,
        cfg: DatabaseConfig,
        backup_file: PathBuf,
        logger: Arc<JobLogger>,
    ) -> Result<RestoreResult> {
        let generated_id = cfg.generated_id.clone();

        logger.log(
            "info",
            format!("Preparing restore for database {}", cfg.name),
        );

        let db = DatabaseFactory::create_for_restore(cfg.clone(), &backup_file).await;

        logger.log(
            "debug",
            format!("Checking reachability for database {}", cfg.name),
        );

        let reachable = db.ping().await.unwrap_or(false);

        logger.log("info", format!("Reachable: {}", reachable));

        if !reachable {
            logger.log(
                "error",
                format!("Database {} unreachable, aborting restore", cfg.name),
            );

            return Ok(RestoreResult {
                generated_id,
                status: "failed".into(),
            });
        }

        match db.restore(&backup_file, Arc::clone(&logger)).await {
            Ok(_) => {
                logger.log(
                    "info",
                    format!("Restore completed successfully for database {}", cfg.name),
                );
                Ok(RestoreResult {
                    generated_id,
                    status: "success".into(),
                })
            }

            Err(e) => {
                logger.log(
                    "error",
                    format!("Restore failed for database {}: {:?}", cfg.name, e),
                );

                Ok(RestoreResult {
                    generated_id,
                    status: "failed".into(),
                })
            }
        }
    }
}
