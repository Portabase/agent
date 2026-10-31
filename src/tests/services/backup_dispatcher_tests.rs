use crate::services::backup::dispatcher::{InFlightBackup, run_with_permit};
use crate::tests::init_tracing_for_test;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Semaphore;
use tokio::time::{Duration, sleep};

#[test]
fn in_flight_backup_rejects_duplicate_until_released() {
    let first = InFlightBackup::claim("in-flight-test-db");
    assert!(first.is_some());
    assert!(InFlightBackup::claim("in-flight-test-db").is_none());

    drop(first);
    assert!(InFlightBackup::claim("in-flight-test-db").is_some());
}

#[tokio::test]
async fn semaphore_gated_execution_never_exceeds_configured_limit() {
    init_tracing_for_test();

    let limit = 2;
    let semaphore = Arc::new(Semaphore::new(limit));
    let concurrent = Arc::new(AtomicUsize::new(0));
    let max_observed = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::new();
    for _ in 0..6 {
        let semaphore = semaphore.clone();
        let concurrent = concurrent.clone();
        let max_observed = max_observed.clone();

        handles.push(tokio::spawn(async move {
            run_with_permit(Some(semaphore.as_ref()), async move {
                let now = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                max_observed.fetch_max(now, Ordering::SeqCst);

                sleep(Duration::from_millis(50)).await;

                concurrent.fetch_sub(1, Ordering::SeqCst);
            })
            .await;
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }

    assert!(
        max_observed.load(Ordering::SeqCst) <= limit,
        "observed {} concurrent jobs, expected at most {}",
        max_observed.load(Ordering::SeqCst),
        limit
    );
}
