#![allow(dead_code)]

use crate::utils::task_manager::cron::next_run_timestamp;
use crate::utils::task_manager::models::PeriodicTask;
use redis::aio::MultiplexedConnection;
use serde_json::Value;

pub const SCHEDULE_KEY: &str = "redbeat:schedule";

pub async fn upsert_task(
    conn: &mut MultiplexedConnection,
    name: &str,
    task: &str,
    cron: &str,
    args: Vec<String>,
    metadata: Option<Value>,
) -> redis::RedisResult<()> {
    let key = format!("redbeat:{}", name);
    let next_ts = next_run_timestamp(cron);

    let entry = PeriodicTask {
        task: task.to_string(),
        cron: cron.to_string(),
        args,
        enabled: true,
        metadata
    };

    let payload = serde_json::to_string(&entry).unwrap();

    let mut pipe = redis::pipe();
    pipe.atomic()
        .hset(&key, "data", payload)
        .zadd(SCHEDULE_KEY, &key, next_ts);

    pipe.query_async(conn).await
}

pub async fn remove_task(conn: &mut MultiplexedConnection, name: &str) -> redis::RedisResult<()> {
    let key = format!("redbeat:{}", name);

    let mut pipe = redis::pipe();
    pipe.atomic().zrem(SCHEDULE_KEY, &key).del(&key);

    pipe.query_async(conn).await
}
