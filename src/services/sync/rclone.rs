use super::excludes::sync_excludes;
use super::stats::{SyncStats, parse_log};
use crate::domain::files::matcher::{exclude_patterns, one_file_system};
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use crate::services::storage::providers::rclone::helpers::{remote_target, write_config};
use anyhow::{Context, Result, anyhow, bail};
use std::process::Stdio;
use tokio::process::Command;

/// `<remote>:<base>/<folder>/sync/<generated_id>/current`
pub fn replica_target(remote_name: &str, base_path: &str, folder: &str, generated_id: &str) -> String {
    remote_target(remote_name, base_path, &format!("{folder}/sync/{generated_id}/current"))
}

/// Mirrors `cfg.path` onto `dest` (`<remote>:<path>`). rclone skips its deletions when an error
/// happens before the delete phase ("not deleting files as there were IO errors");
/// an error during the delete phase can leave the replica partially updated.
/// An empty source is refused so an unmounted volume cannot wipe the replica.
pub async fn sync_dir(config_text: &str, dest: &str, cfg: &DatabaseConfig, logger: &JobLogger) -> Result<SyncStats> {
    let root = std::fs::canonicalize(&cfg.path)
        .with_context(|| format!("cannot read source directory {}", cfg.path))?;
    if !root.is_dir() {
        bail!("source path {} is not a directory", root.display());
    }
    if std::fs::read_dir(&root)
        .with_context(|| format!("cannot read source directory {}", root.display()))?
        .next()
        .is_none()
    {
        bail!(
            "source directory {} is empty; refusing to sync so an unmounted volume cannot wipe the replica",
            root.display()
        );
    }
    let config = write_config(config_text)?;

    let mut cmd = Command::new("rclone");
    cmd.arg("--config")
        .arg(config.path())
        .arg("sync")
        .arg(&root)
        .arg(dest)
        .args([
            "--links", "--use-json-log", "--stats", "1h", "--stats-log-level", "NOTICE", "--retries", "1",
            // An unreachable endpoint fails in ~1–2 min instead of blocking the next runs.
            "--contimeout", "30s", "--low-level-retries", "3",
        ]);
    if one_file_system(cfg) {
        cmd.arg("--one-file-system");
    }
    for pattern in sync_excludes(&exclude_patterns(cfg)) {
        cmd.arg("--exclude").arg(pattern);
    }

    logger.log("info", format!("Syncing {} to {dest}", root.display()));
    let out = cmd
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => anyhow!("rclone binary not found"),
            _ => anyhow!(e).context("failed to start rclone"),
        })?;

    let log = parse_log(&String::from_utf8_lossy(&out.stderr));
    for error in &log.errors {
        logger.log("warn", format!("rclone: {error}"));
    }
    if !out.status.success() {
        let reason = log
            .stats
            .as_ref()
            .and_then(|s| s.last_error.clone())
            .or_else(|| log.errors.last().cloned())
            .unwrap_or_else(|| "no error message".to_string());
        bail!("rclone sync failed ({}): {reason}; the replica may be partially updated", out.status);
    }
    let stats = log.stats.unwrap_or_default();
    logger.log(
        "info",
        format!("Synced: {} file(s) transferred ({} bytes), {} deleted", stats.transfers, stats.bytes, stats.deletes),
    );
    Ok(stats)
}
