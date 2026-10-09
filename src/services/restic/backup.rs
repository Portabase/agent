use super::command::{ResticRepo, short};
use super::excludes::backup_excludes;
use super::json::{BackupSummary, Snapshot, files_removed};
use crate::core::context::Context as CoreContext;
use crate::domain::files::matcher::{exclude_patterns, one_file_system};
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::logger::JobLogger;
use crate::services::backup::models::UploadResult;
use crate::services::config::DatabaseConfig;
use crate::utils::locks::{DbOpLock, FileLock};
use anyhow::{Context, Result, bail};

/// One snapshot of `cfg.path`, tagged with its `backup_storage` id.
/// Exit 3 (unreadable files) fails the run and forgets the snapshot: a mirror
/// restore of it would delete the files it is missing (P0 rule R21). On an
/// append-only repository the agent cannot forget it, so the dashboard does.
pub async fn snapshot(
    repo: &ResticRepo,
    cfg: &DatabaseConfig,
    backup_storage_id: &str,
    logger: &JobLogger,
) -> Result<BackupSummary> {
    let root = std::fs::canonicalize(&cfg.path)
        .with_context(|| format!("cannot read source directory {}", cfg.path))?;
    if !root.is_dir() {
        bail!("source path {} is not a directory", root.display());
    }
    repo.ensure_initialized(logger).await?;

    // Stable host: container hostnames change on recreate and would break
    // parent-snapshot detection (full rescan every time).
    let host = format!("portabase-{}", cfg.generated_id);
    let mut args: Vec<String> = vec![
        "backup".into(),
        root.to_string_lossy().into_owned(),
        "--json".into(),
        "--host".into(),
        host.clone(),
        "--tag".into(),
        "portabase".into(),
        "--tag".into(),
        format!("bs:{backup_storage_id}"),
    ];
    for pattern in backup_excludes(&root, &exclude_patterns(cfg))? {
        args.push("--exclude".into());
        args.push(pattern);
    }
    if one_file_system(cfg) {
        args.push("--one-file-system".into());
    }

    logger.log("info", format!("Snapshotting {} into {}", root.display(), repo.repository()));
    let run = repo.run(&args, logger).await?;
    for error in &run.errors {
        logger.log("warn", format!("Unreadable: {error}"));
    }
    let summary: Option<BackupSummary> = run
        .summary
        .clone()
        .map(serde_json::from_value)
        .transpose()
        .context("unexpected restic backup summary")?;

    match (run.code, summary) {
        (0, Some(mut s)) => {
            s.files_removed = match removed(repo, &host, &s, logger).await {
                Ok(removed) => removed,
                Err(e) => {
                    logger.log("warn", format!("Could not count removed files: {e:#}"));
                    None
                }
            };
            let removed = s.files_removed.map(|n| format!(", {n} removed")).unwrap_or_default();
            logger.log(
                "info",
                // data_added_packed: new file and directory blobs, compressed and encrypted. The
                // repository grows a little more (pack headers, index and snapshot files).
                format!(
                    "Snapshot {}: {} new, {} changed, {} unmodified{removed} file(s); {} byte(s) scanned, {} byte(s) of new data",
                    short(&s.snapshot_id), s.files_new, s.files_changed, s.files_unmodified, s.total_bytes_processed, s.data_added_packed
                ),
            );
            Ok(s)
        }
        (0, None) => bail!("restic backup returned no summary"),
        (3, summary) => {
            let mut fate = "snapshot discarded";
            if let Some(s) = summary {
                if repo.is_append_only() {
                    logger.log(
                        "warn",
                        format!("Incomplete snapshot {} left for the dashboard to forget", short(&s.snapshot_id)),
                    );
                    fate = "snapshot not restorable (the dashboard forgets it)";
                } else {
                    let forget = repo.run(["forget", s.snapshot_id.as_str()], logger).await?;
                    if forget.code != 0 {
                        logger.log(
                            "error",
                            format!("Could not forget incomplete snapshot {}: {}", s.snapshot_id, forget.error("forget")),
                        );
                    }
                }
            }
            bail!(
                "{} file(s) could not be read; {fate} so a mirror restore cannot delete them (fix permissions or add an exclude)",
                run.errors.len()
            )
        }
        _ => Err(run.error("backup")),
    }
}

/// Removed files of snapshot `s`, against the parent restic actually used (one listing of
/// the source's snapshots, no index load).
async fn removed(repo: &ResticRepo, host: &str, s: &BackupSummary, logger: &JobLogger) -> Result<Option<u64>> {
    let listed = repo.run(["snapshots", "--json", "--host", host], logger).await?;
    if listed.code != 0 {
        return Err(listed.error("snapshots"));
    }
    let snapshots: Vec<Snapshot> =
        serde_json::from_str(listed.stdout.trim()).context("unexpected `restic snapshots` output")?;
    Ok(files_removed(&snapshots, &s.snapshot_id, s.files_changed, s.files_unmodified))
}

/// Snapshots every storage in turn (one scan each, v1). Never errors: each
/// storage reports its own `backup_storage` row, like the archive uploader.
pub async fn run(
    ctx: &CoreContext,
    cfg: &DatabaseConfig,
    storages: &[DatabaseStorage],
    backup_id: &str,
    logger: &JobLogger,
) -> Vec<UploadResult> {
    if let Err(e) = FileLock::acquire(&cfg.generated_id, DbOpLock::Backup.as_str()).await {
        logger.log("error", format!("Snapshot aborted: {e}"));
        return Vec::new();
    }
    let mut results = Vec::with_capacity(storages.len());
    for storage in storages {
        results.push(one_storage(ctx, cfg, storage, backup_id, logger).await);
    }
    if let Err(e) = FileLock::release(&cfg.generated_id).await {
        logger.log("warn", format!("Failed to release the backup lock: {e}"));
    }
    results
}

pub(crate) async fn one_storage(
    ctx: &CoreContext,
    cfg: &DatabaseConfig,
    storage: &DatabaseStorage,
    backup_id: &str,
    logger: &JobLogger,
) -> UploadResult {
    let agent_id = ctx.edge_key.agent_id.clone();
    let failed = |error: String| UploadResult {
        storage_id: storage.id.clone(),
        success: false,
        error: Some(error),
        remote_file_path: None,
        total_size: None,
    };

    let backup_storage_id = match ctx
        .api
        .backup_upload_init(agent_id.clone(), cfg.generated_id.clone(), storage.id.clone(), backup_id, Some("restic"))
        .await
    {
        Ok(Some(response)) => response.backup_storage.id,
        Ok(None) => {
            logger.log("error", "Upload init returned empty response");
            return failed("backup_upload_init returned empty response".into());
        }
        Err(e) => {
            logger.log("error", format!("Upload init failed: {e}"));
            return failed("backup_upload_init failed".into());
        }
    };

    let outcome = match ResticRepo::open(storage, &cfg.generated_id, &ctx.edge_key) {
        Ok(repo) => snapshot(&repo, cfg, &backup_storage_id, logger).await,
        Err(e) => Err(e),
    };

    match outcome {
        Ok(summary) => match ctx
            .api
            .backup_upload_success(
                agent_id,
                cfg.generated_id.clone(),
                backup_storage_id,
                summary.snapshot_id.clone(),
                summary.data_added_packed,
                summary.report(),
                backup_id,
            )
            .await
        {
            Ok(_) => UploadResult {
                storage_id: storage.id.clone(),
                success: true,
                error: None,
                remote_file_path: Some(summary.snapshot_id),
                total_size: Some(summary.total_bytes_processed),
            },
            Err(e) => {
                logger.log("error", format!("Upload status update failed for {}: {e}", storage.id));
                failed(e.to_string())
            }
        },
        Err(e) => {
            logger.log("error", format!("Snapshot to storage {} failed: {e:#}", storage.id));
            if let Err(err) = ctx
                .api
                .backup_upload_status(agent_id, cfg.generated_id.clone(), backup_storage_id, "failed", String::new(), 0u64, backup_id)
                .await
            {
                logger.log("error", format!("Failed-status update failed for {}: {err}", storage.id));
            }
            failed(format!("{e:#}"))
        }
    }
}
