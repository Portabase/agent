use crate::utils::task_manager::models;
use crate::utils::task_manager::tasks::{remove_task, upsert_task};
use crate::utils::text::normalize_cron;
use chrono::{Local};
use cron::Schedule;
use tracing::debug;
use redis::AsyncCommands;
use redis::aio::MultiplexedConnection;
use std::str::FromStr;
use tracing::info;

pub fn next_run_timestamp(expr: &str) -> i64 {
    let schedule = Schedule::from_str(expr).unwrap();
    // schedule.upcoming(Utc).next().unwrap().timestamp()
    schedule.upcoming(Local).next().unwrap().timestamp()
}

pub async fn check_and_update_cron(
    conn: &mut MultiplexedConnection,
    cron_value: Option<String>,
    args: Vec<String>,
    task: &str,
    task_name: String,
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

                if stored.cron != cron {
                    upsert_task(conn, &task_name, task, &cron, args.clone())
                        .await
                        .unwrap_or_else(|e| {
                            tracing::error!("Failed to update task {}: {:?}", task_name, e);
                        });
                    info!("Task {} updated", task_name);
                }
            } else {
                upsert_task(conn, &task_name, task, &cron, args)
                    .await
                    .unwrap_or_else(|e| {
                        tracing::error!("Failed to create task {}: {:?}", task_name, e);
                    });
                info!("Task {} created", task_name);
            }
        }
    }
}
