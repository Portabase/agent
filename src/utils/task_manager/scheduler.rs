use crate::core::context::Context;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::BackupService;
use crate::services::config::ConfigService;
use crate::utils::common::BackupMethod;
use crate::utils::task_manager::cron::next_run_timestamp;
use crate::utils::task_manager::models::PeriodicTask;
use crate::utils::task_manager::tasks::SCHEDULE_KEY;
use redis::AsyncCommands;
use redis::aio::MultiplexedConnection;
use serde_json::Value;
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
                // Never log metadata: it carries decrypted storage channels (secrets).
                info!("Executing task={} args={:?}", task_clone.task, task_clone.args);
                if let Err(e) = execute_task(
                    task_clone.task.as_str(),
                    task_clone.args,
                    task_clone.metadata,
                )
                .await
                {
                    error!(
                        "An error occurred while executing task={} : {:?}",
                        task_clone.task, e
                    );
                }
                match next_run_timestamp(&task_clone.cron) {
                    Some(next_ts) => {
                        let _: () = conn_clone.zadd(SCHEDULE_KEY, &key, next_ts).await.unwrap();
                    }
                    None => {
                        error!(
                            "Invalid cron expression for task={}: {}",
                            task_clone.task, task_clone.cron
                        );
                    }
                }
            });
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

/// Local config merged with the dashboard-pushed sources, as the agent loop sees them.
fn merged_config(ctx: &Arc<Context>) -> crate::services::config::DatabasesConfig {
    let local = ConfigService::new(ctx.clone()).load_optional(None);
    let cache_path = std::path::PathBuf::from(&crate::settings::CONFIG.data_path).join("dashboard_databases.json");
    let dashboard = crate::services::dashboard_config::load_cache(&cache_path);
    crate::services::dashboard_config::merge(&local.databases, &dashboard)
}

pub async fn execute_task(
    task: &str,
    args: Vec<String>,
    metadata: Option<Value>,
) -> Result<(), anyhow::Error> {
    match task {
        "tasks.database.periodic_backup" => {
            let generated_id = &args[0];
            let dbms = &args[1];
            info!("{} | {}", generated_id, dbms);

            let ctx = Arc::new(Context::new());
            let backup_service = BackupService::new(ctx.clone());

            let config = merged_config(&ctx);

            let metadata_obj = metadata
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("Metadata missing"))?;

            let storages_value: &Value = metadata_obj
                .get("storages")
                .ok_or_else(|| anyhow::anyhow!("storages key missing"))?;

            let encrypt_value: &Value = metadata_obj
                .get("encrypt")
                .ok_or_else(|| anyhow::anyhow!("encrypt key missing"))?;

            let storages: Vec<DatabaseStorage> = serde_json::from_value(storages_value.clone())?;
            let encrypt: bool = serde_json::from_value(encrypt_value.clone())?;
            // Tasks stored before P2 have no engine: archive.
            let engine = metadata_obj.get("engine").and_then(Value::as_str).unwrap_or("archive");

            backup_service
                .dispatch(
                    generated_id,
                    &config,
                    BackupMethod::Automatic,
                    &storages,
                    encrypt,
                    engine,
                )
                .await;

            Ok(())
        }

        _ => {
            anyhow::bail!("Unknown task: {}", task)
        }
    }
}
