use crate::core::context::Context;
use crate::services::backup::BackupService;
use crate::services::config::ConfigService;
use crate::utils::common::BackupMethod;
use crate::utils::task_manager::cron::next_run_timestamp;
use crate::utils::task_manager::models::PeriodicTask;
use crate::utils::task_manager::tasks::SCHEDULE_KEY;
use redis::AsyncCommands;
use redis::aio::MultiplexedConnection;
use std::sync::Arc;
use tracing::error;
use tracing::info;

pub async fn scheduler_loop(mut conn: MultiplexedConnection) {
    loop {
        let now = chrono::Local::now().timestamp();

        let due: Vec<String> = conn
            .zrangebyscore(SCHEDULE_KEY, 0, now)
            .await
            .unwrap_or_default();
        for key in due {
            let raw: String = conn.hget(&key, "data").await.unwrap();
            let task: PeriodicTask = serde_json::from_str(&raw).unwrap();

            if !task.enabled {
                continue;
            }
            let task_clone = task.clone();
            let mut conn_clone = conn.clone();

            tokio::spawn(async move {
                info!(
                    "Executing task={} args={:?} metadata={:?}",
                    task_clone.task, task_clone.args, task_clone.metadata
                );
                if let Err(e) = execute_task(task_clone.task.as_str(), task_clone.args).await {
                    error!(
                        "An error occurred while executing task={} : {:?}",
                        task_clone.task, e
                    );
                }
                let next_ts = next_run_timestamp(&task_clone.cron);
                let _: () = conn_clone.zadd(SCHEDULE_KEY, &key, next_ts).await.unwrap();
            });
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

pub async fn execute_task(task: &str, args: Vec<String>) -> Result<(), anyhow::Error> {
    match task {
        "tasks.database.periodic_backup" => {
            let generated_id = &args[0];
            let dbms = &args[1];
            info!("{} | {}", generated_id, dbms);

            let ctx = Arc::new(Context::new());
            let config_service = ConfigService::new(ctx.clone());
            let backup_service = BackupService::new(ctx.clone());
            let config = config_service.load(None).unwrap();

            backup_service
                .dispatch(generated_id, &config, BackupMethod::Automatic)
                .await;

            Ok(())
        }

        _ => {
            anyhow::bail!("Unknown task: {}", task)
        }
    }
}
