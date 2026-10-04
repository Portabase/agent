use super::command::{ResticRepo, is_snapshot_id, short};
use super::excludes::{glob_escape, restore_excludes};
use super::json::Snapshot;
use crate::domain::files::matcher::{exclude_patterns, one_file_system};
use crate::domain::files::platform::dev;
use crate::domain::files::restore::ensure_restorable;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::logger::JobLogger;
use crate::services::config::{DatabaseConfig, DbType};
use crate::utils::edge_key::EdgeKey;
use crate::utils::locks::{DbOpLock, FileLock};
use anyhow::{Context, Result, bail};
use std::path::Path;
use walkdir::WalkDir;

/// Mirror restore of `snapshot_id` into `cfg.path`: files created since are
/// deleted, paths matching the current exclude patterns are kept (spike S1).
pub async fn restore(
    repo: &ResticRepo,
    cfg: &DatabaseConfig,
    snapshot_id: &str,
    logger: &JobLogger,
) -> Result<()> {
    if !is_snapshot_id(snapshot_id) {
        bail!("invalid snapshot id in the restore payload");
    }
    let target = ensure_restorable(Path::new(&cfg.path))?;

    let listed = repo.run(["snapshots", snapshot_id, "--json"], logger).await?;
    if listed.code != 0 {
        return Err(listed.error("snapshots"));
    }
    let snapshots: Vec<Snapshot> = serde_json::from_str(listed.stdout.trim())
        .context("unexpected `restic snapshots` output")?;
    // The original path inside the snapshot; the mount point may have moved since.
    let Some(original) = snapshots.first().and_then(|s| s.paths.first()) else {
        bail!("snapshot {} not found in {}", short(snapshot_id), repo.repository());
    };

    let mut excludes = restore_excludes(&exclude_patterns(cfg))?;
    if one_file_system(cfg) {
        excludes.extend(mount_points(&target).into_iter().map(|rel| format!("/{}", glob_escape(&rel))));
    }
    let mut args: Vec<String> = vec![
        "restore".into(),
        format!("{snapshot_id}:{original}"),
        "--target".into(),
        target.to_string_lossy().into_owned(),
        "--delete".into(),
        "--json".into(),
    ];
    for pattern in excludes {
        args.push("--exclude".into());
        args.push(pattern);
    }

    logger.log(
        "info",
        format!("Restoring snapshot {} ({original}) into {}", short(snapshot_id), target.display()),
    );
    let run = repo.run(&args, logger).await?;
    for error in &run.errors {
        logger.log("warn", format!("Restore error: {error}"));
    }
    if run.code != 0 {
        return Err(run.error("restore"));
    }
    if let Some(s) = &run.summary {
        logger.log(
            "info",
            format!(
                "Restored {} file(s), deleted {} file(s)",
                s["files_restored"].as_u64().unwrap_or(0),
                s["files_deleted"].as_u64().unwrap_or(0)
            ),
        );
    }
    Ok(())
}

/// Directories under `root` on another device. With `one_file_system` backup
/// never entered them, so the mirror restore must not delete their content.
fn mount_points(root: &Path) -> Vec<String> {
    let Ok(meta) = std::fs::metadata(root) else { return Vec::new() };
    let root_dev = dev(&meta);
    let mut out = Vec::new();
    let mut walker = WalkDir::new(root).min_depth(1).follow_links(false).into_iter();
    while let Some(entry) = walker.next() {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_dir() {
            continue;
        }
        if matches!(entry.metadata(), Ok(m) if dev(&m) != root_dev) {
            if let Ok(rel) = entry.path().strip_prefix(root) {
                out.push(rel.to_string_lossy().into_owned());
            }
            walker.skip_current_dir();
        }
    }
    out
}

/// Snapshot restore job: the source's FileLock is held for the whole restore.
pub async fn run(
    edge_key: &EdgeKey,
    cfg: &DatabaseConfig,
    snapshot_id: &str,
    storage: &DatabaseStorage,
    logger: &JobLogger,
) -> Result<()> {
    if !matches!(cfg.db_type, DbType::Files) {
        bail!("snapshot restore needs a files source, got {}", cfg.db_type.as_str());
    }
    FileLock::acquire(&cfg.generated_id, DbOpLock::Restore.as_str())
        .await
        .context("another backup or restore of this source is running")?;
    let result = async {
        let repo = ResticRepo::open(storage, &cfg.generated_id, edge_key)?;
        restore(&repo, cfg, snapshot_id, logger).await
    }
    .await;
    FileLock::release(&cfg.generated_id).await?;
    result
}
