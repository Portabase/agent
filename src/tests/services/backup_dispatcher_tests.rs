//! Regression coverage for the concurrency cap added alongside the #94 panic fixes.
//! `BackupService::dispatch` gates `execute_backup` behind `BACKUP_SEMAPHORE`
//! (`src/services/backup/dispatcher.rs`), configurable via `MAX_CONCURRENT_BACKUPS`.

use crate::services::backup::dispatcher::BACKUP_SEMAPHORE;
use crate::tests::init_tracing_for_test;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Semaphore;
use tokio::time::{Duration, sleep};

#[tokio::test]
async fn backup_semaphore_defaults_to_unlimited_when_max_concurrent_backups_unset() {
    init_tracing_for_test();

    // Neither this test suite nor docker-compose.test.yml sets MAX_CONCURRENT_BACKUPS,
    // so the real, process-wide BACKUP_SEMAPHORE must be None: dispatch() must not
    // throttle backups unless an operator explicitly opts in.
    assert!(
        BACKUP_SEMAPHORE.is_none(),
        "expected no concurrency cap by default (MAX_CONCURRENT_BACKUPS unset)"
    );
}

#[tokio::test]
async fn semaphore_gated_execution_never_exceeds_configured_limit() {
    init_tracing_for_test();

    // Exercises the exact acquire-hold-release pattern dispatch() wraps around
    // execute_backup: a fresh Semaphore rather than the global BACKUP_SEMAPHORE,
    // since CONFIG.max_concurrent_backups is a process-wide singleton read once
    // from the environment and can't be reconfigured per-test.
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
            let _permit = semaphore.acquire().await.unwrap();

            let now = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            max_observed.fetch_max(now, Ordering::SeqCst);

            sleep(Duration::from_millis(50)).await;

            concurrent.fetch_sub(1, Ordering::SeqCst);
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
