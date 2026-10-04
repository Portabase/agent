#![allow(dead_code)]

use crate::core::context::Context;
use crate::services::backup::BackupService;
use crate::services::config::{ConfigService, DatabaseConfig};
use crate::services::cron::CronService;
use crate::services::dashboard_config::{collect_configs, load_cache, local_only_ids, merge, persist_cache};
use crate::services::restore::RestoreService;
use crate::services::status::StatusService;
use crate::settings::CONFIG;
use crate::utils::common::BackupMethod;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info, warn};

pub struct Agent {
    ctx: Arc<Context>,
    config_service: ConfigService,
    status_service: StatusService,
    cron_service: CronService,
    backup_service: BackupService,
    restore_service: RestoreService,
    dashboard_cache: Vec<DatabaseConfig>,
    cache_path: PathBuf,
}

impl Agent {
    pub async fn new(ctx: Arc<Context>) -> Self {
        let config_service = ConfigService::new(ctx.clone());
        let status_service = StatusService::new(ctx.clone());
        let cron_service = CronService::new(ctx.clone()).await;

        let backup_service = BackupService::new(ctx.clone());
        let restore_service = RestoreService::new(ctx.clone());

        let cache_path = PathBuf::from(&CONFIG.data_path).join("dashboard_databases.json");
        let dashboard_cache = load_cache(&cache_path);

        Agent {
            ctx,
            config_service,
            status_service,
            cron_service,
            backup_service,
            restore_service,
            dashboard_cache,
            cache_path,
        }
    }

    pub async fn run(&mut self, method: BackupMethod) -> Result<(), Box<dyn std::error::Error>> {
        let local = self.config_service.load_optional(None);
        let local_ids = local_only_ids(&local.databases, &self.dashboard_cache);

        let merged_in = merge(&local.databases, &self.dashboard_cache);
        let ping_result = self.status_service.ping(&merged_in.databases, &local_ids).await?;

        self.dashboard_cache = collect_configs(&ping_result);
        if let Err(e) = persist_cache(&self.cache_path, &self.dashboard_cache) {
            error!("Failed to persist dashboard cache: {e}");
        }

        let merged = merge(&local.databases, &self.dashboard_cache);

        for db in ping_result.databases.iter() {
            let Some(database) = merged
                .databases
                .iter()
                .find(|cfg_db| cfg_db.generated_id == db.generated_id)
            else {
                warn!("No config for returned database {}; skipping", db.generated_id);
                continue;
            };
            info!(
                "Generated Id: {} | backup action: {} | restore action: {} | Database Name: {}",
                db.generated_id, db.data.backup.action, db.data.restore.action, database.name,
            );
            let _ = self.cron_service.sync(db).await;

            if db.data.backup.action {
                let _ = self
                    .backup_service
                    .dispatch(
                        &db.generated_id,
                        &merged,
                        method.clone(),
                        &db.storages,
                        db.encrypt,
                        db.data.backup.engine.as_deref().unwrap_or("archive"),
                    )
                    .await;
            } else if db.data.restore.action {
                let _ = self.restore_service.dispatch(db, &merged).await;
            }
        }

        Ok(())
    }
}
