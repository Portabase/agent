use crate::utils::task_manager::models;
use crate::utils::task_manager::tasks::{remove_task, upsert_task};
use crate::utils::text::normalize_cron;
use chrono::Local;
use cron::Schedule;
use redis::AsyncCommands;
use redis::aio::MultiplexedConnection;
use serde_json::Value;
use std::str::FromStr;
use tracing::debug;
use tracing::info;

pub fn next_run_timestamp(expr: &str) -> i64 {
    let schedule = Schedule::from_str(expr).unwrap();
    schedule.upcoming(Local).next().unwrap().timestamp()
}

pub async fn check_and_update_cron(
    conn: &mut MultiplexedConnection,
    cron_value: Option<String>,
    args: Vec<String>,
    task: &str,
    task_name: String,
    metadata: Option<Value>,
) {
    let redis_key = format!("redbeat:{}", task_name);

    let exists: bool = conn.exists(&redis_key).await.unwrap_or(false);

    match cron_value {
        None => {
            if exists {
                remove_task(conn, &task_name).await.unwrap_or_else(|e| {
                    tracing::error!("Failed to remove task {}: {:?}", task_name, e);
                });
                info!("Task {} removed", task_name);
            }
        }

        Some(cron) => {
            let cron = normalize_cron(&cron);
            debug!("Task cron (normalized): {:?}", cron);

            if exists {
                let raw: String = conn.hget(&redis_key, "data").await.unwrap();
                let stored: models::PeriodicTask = serde_json::from_str(&raw).unwrap();

                let cron_changed = stored.cron != cron;
                let args_changed = stored.args != args;
                let metadata_changed = stored.metadata != metadata;

                if cron_changed || args_changed || metadata_changed {
                    upsert_task(
                        conn,
                        &task_name,
                        task,
                        &cron,
                        args.clone(),
                        metadata,
                    )
                    .await
                    .unwrap_or_else(|e| {
                        tracing::error!("Failed to update task {}: {:?}", task_name, e);
                    });

                    info!(
                        "Task {} updated (cron: {}, args: {}, metadata: {})",
                        task_name, cron_changed, args_changed, metadata_changed
                    );
                }
            } else {
                upsert_task(conn, &task_name, task, &cron, args, metadata)
                    .await
                    .unwrap_or_else(|e| {
                        tracing::error!("Failed to create task {}: {:?}", task_name, e);
                    });
                info!("Task {} created", task_name);
            }
        }
    }
}
