//! Regression coverage for the concurrency cap added alongside the #94 panic fixes.
//! `BackupService::dispatch` gates `execute_backup` behind `BACKUP_SEMAPHORE` via the
//! shared `run_with_permit` helper (`src/services/backup/dispatcher.rs`), configurable
//! via `MAX_CONCURRENT_BACKUPS`.

use crate::services::backup::dispatcher::{BACKUP_SEMAPHORE, run_with_permit};
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
    // throttle backups unless an operator explicitly opts in. This depends on ambient
    // environment (CONFIG is a process-wide Lazy singleton) — see settings::tests for
    // hermetic coverage of the underlying parsing logic that doesn't have this caveat.
    assert!(
        BACKUP_SEMAPHORE.is_none(),
        "expected no concurrency cap by default (MAX_CONCURRENT_BACKUPS unset)"
    );
}

#[tokio::test]
async fn semaphore_gated_execution_never_exceeds_configured_limit() {
    init_tracing_for_test();

    // Calls the same run_with_permit() that dispatch() uses, so this exercises the
    // real gating code path rather than a parallel reimplementation. A local Semaphore
    // is used instead of the global BACKUP_SEMAPHORE because CONFIG.max_concurrent_backups
    // is a process-wide singleton read once from the environment and can't be
    // reconfigured per-test.
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
