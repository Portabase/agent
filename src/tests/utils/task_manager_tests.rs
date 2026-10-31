use crate::tests::init_tracing_for_test;
use crate::utils::task_manager::cron::check_and_update_cron;
use crate::utils::task_manager::models::PeriodicTask;
use crate::utils::task_manager::scheduler::{execute_task, scheduler_loop};
use crate::utils::task_manager::tasks::SCHEDULE_KEY;
use redis::AsyncCommands;
use redis::aio::MultiplexedConnection;
use std::time::Duration;
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::redis::Redis;
use url::Host;

#[tokio::test]
async fn execute_task_errors_instead_of_panicking_on_missing_generated_id() {
    let result = execute_task("tasks.database.periodic_backup", vec![], None).await;

    let err = result.expect_err("expected an error, not a panic, for empty args");
    assert!(
        err.to_string().contains("generated_id"),
        "expected error to mention missing generated_id, got: {err}"
    );
}

#[tokio::test]
async fn execute_task_errors_instead_of_panicking_on_missing_dbms() {
    let args = vec!["some-generated-id".to_string()];
    let result = execute_task("tasks.database.periodic_backup", args, None).await;

    let err = result.expect_err("expected an error, not a panic, for a single arg");
    assert!(
        err.to_string().contains("dbms"),
        "expected error to mention missing dbms, got: {err}"
    );
}

async fn start_redis() -> (ContainerAsync<Redis>, MultiplexedConnection) {
    let container = Redis::default().start().await.unwrap();

    let host = container
        .get_host()
        .await
        .unwrap_or(Host::parse("127.0.0.1").unwrap());
    let port = container.get_host_port_ipv4(6379).await.unwrap_or(6379);

    let client = redis::Client::open(format!("redis://{host}:{port}")).unwrap();
    let conn = client.get_multiplexed_async_connection().await.unwrap();

    (container, conn)
}

#[tokio::test]
async fn check_and_update_cron_rewrites_malformed_stored_data() {
    init_tracing_for_test();
    let (_container, mut conn) = start_redis().await;

    let task_name = "malformed-task";
    let redis_key = format!("redbeat:{task_name}");

    let _: () = conn
        .hset(&redis_key, "data", "not-valid-json")
        .await
        .unwrap();

    check_and_update_cron(
        &mut conn,
        Some("*/5 * * * *".to_string()),
        vec![],
        "tasks.database.periodic_backup",
        task_name.to_string(),
        None,
    )
    .await;

    let raw: String = conn.hget(&redis_key, "data").await.unwrap();
    assert!(serde_json::from_str::<PeriodicTask>(&raw).is_ok());
    let score: Option<i64> = conn.zscore(SCHEDULE_KEY, &redis_key).await.unwrap();
    assert!(score.is_some(), "rewritten task should be scheduled");
}

#[tokio::test]
async fn scheduler_loop_drops_unreadable_due_tasks_without_panicking() {
    init_tracing_for_test();
    let (_container, mut conn) = start_redis().await;

    let malformed_key = "redbeat:malformed-due-task";
    let missing_key = "redbeat:missing-due-task";
    let now = chrono::Local::now().timestamp();

    let _: () = conn
        .hset(malformed_key, "data", "not-valid-json")
        .await
        .unwrap();
    let _: () = conn.zadd(SCHEDULE_KEY, malformed_key, now).await.unwrap();
    let _: () = conn.zadd(SCHEDULE_KEY, missing_key, now).await.unwrap();

    let handle = tokio::spawn(scheduler_loop(conn.clone()));
    let abort_handle = handle.abort_handle();

    let outcome = tokio::time::timeout(Duration::from_millis(1500), handle).await;
    abort_handle.abort();

    match outcome {
        Err(_elapsed) => {
            for key in [malformed_key, missing_key] {
                let score: Option<i64> = conn.zscore(SCHEDULE_KEY, key).await.unwrap();
                assert_eq!(score, None, "{key} should be unscheduled");
                let exists: bool = conn.exists(key).await.unwrap();
                assert!(!exists, "{key} hash should be removed");
            }
        }
        Ok(join_result) => {
            panic!("scheduler_loop exited instead of continuing to loop: {join_result:?}");
        }
    }
}
