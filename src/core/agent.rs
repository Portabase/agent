#![allow(dead_code)]

use crate::core::context::Context;
use crate::services::backup::BackupService;
use crate::services::config::ConfigService;
use crate::services::cron::CronService;
use crate::services::status::StatusService;
use crate::utils::common::BackupMethod;
use std::sync::Arc;
use tracing::info;
use crate::services::restore::RestoreService;

pub struct Agent {
    ctx: Arc<Context>,
    config_service: ConfigService,
    status_service: StatusService,
    cron_service: CronService,
    backup_service: BackupService,
    restore_service: RestoreService,
}

impl Agent {
    pub async fn new(ctx: Arc<Context>) -> Self {
        let config_service = ConfigService::new(ctx.clone());
        let status_service = StatusService::new(ctx.clone());
        let cron_service = CronService::new(ctx.clone()).await;

        let backup_service = BackupService::new(ctx.clone());
        let restore_service = RestoreService::new(ctx.clone());

        Agent {
            ctx,
            config_service,
            status_service,
            cron_service,
            backup_service,
            restore_service,
        }
    }

    pub async fn run(&mut self, method: BackupMethod) -> Result<(), Box<dyn std::error::Error>> {
        let config = self.config_service.load(None)?;
        let ping_result = self.status_service.ping(&config.databases).await?;

        for db in ping_result.databases.iter() {
            info!(
                "Generated Id: {} | backup action: {} | restore action: {}",
                db.generated_id, db.data.backup.action, db.data.restore.action
            );
            let _ = self.cron_service.sync(db).await;

            if db.data.backup.action {
                let _ = self
                    .backup_service
                    .dispatch(&db.generated_id, &config, method.clone(), &db.storages, db.encrypt)
                    .await;
            } else if db.data.restore.action {
                let _ = self
                    .restore_service
                    .dispatch(db, &config)
                    .await;
            }
        }

        Ok(())
    }
}
