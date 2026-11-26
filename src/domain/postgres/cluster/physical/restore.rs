use super::backup::{archive_named, extract_backup, run_logged, verify};
use super::commands::{missing_config_files, output_dir, start_commands};
use crate::domain::postgres::connection::select_pg_path;
use crate::services::backup::logger::JobLogger;
use crate::utils::compress::decompress_large_tar_gz;
use crate::utils::text::human_size;
use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tempfile::{NamedTempFile, TempDir};
use walkdir::WalkDir;

const RESTORE_PATH_REFUSAL: &str = "RESTORE_PATH is not set or not writable; a physical restore needs a folder shared with the host (see https://portabase.io/docs/agent/db/postgresql#physical-incremental-backups)";

#[derive(Debug)]
pub struct RestoredCluster {
    pub data_dir: PathBuf,
    pub major: u32,
    pub size: u64,
}

pub fn ensure_restore_root(restore_path: Option<&str>) -> Result<PathBuf> {
    restore_path
        .map(PathBuf::from)
        .filter(|root| root.is_dir() && NamedTempFile::new_in(root).is_ok())
        .ok_or_else(|| anyhow!(RESTORE_PATH_REFUSAL))
}

pub async fn rebuild(
    bundles: &[PathBuf],
    restore_root: &Path,
    generated_id: &str,
    logger: Arc<JobLogger>,
) -> Result<RestoredCluster> {
    if bundles.is_empty() {
        bail!("no backup to restore");
    }
    let work_dir = TempDir::new_in(restore_root)?;
    let mut bundle_dirs = Vec::new();
    for (index, bundle) in bundles.iter().enumerate() {
        let bundle_dir = work_dir.path().join(index.to_string());
        fs::create_dir(&bundle_dir)?;
        decompress_large_tar_gz(bundle, &bundle_dir)
            .await
            .with_context(|| format!("could not extract {}", bundle.display()))?;
        fs::remove_file(bundle)?;
        bundle_dirs.push(bundle_dir);
    }
    let (restore_root, generated_id) = (restore_root.to_path_buf(), generated_id.to_string());
    tokio::task::spawn_blocking(move || {
        combine(&bundle_dirs, &restore_root, &generated_id, &logger)
    })
    .await?
}

fn combine(
    bundle_dirs: &[PathBuf],
    restore_root: &Path,
    generated_id: &str,
    logger: &JobLogger,
) -> Result<RestoredCluster> {
    let backup_dirs = bundle_dirs
        .iter()
        .map(|bundle_dir| {
            let backup_dir = bundle_dir.join("data");
            extract_backup(bundle_dir, &backup_dir)?;
            for stem in ["base", "pg_wal"] {
                fs::remove_file(archive_named(bundle_dir, stem)?)?;
            }
            fs::rename(
                bundle_dir.join("backup_manifest"),
                backup_dir.join("backup_manifest"),
            )
            .with_context(|| format!("backup_manifest not found in {}", bundle_dir.display()))?;
            Ok(backup_dir)
        })
        .collect::<Result<Vec<_>>>()?;

    let major: u32 = fs::read_to_string(backup_dirs[0].join("PG_VERSION"))
        .context("could not read PG_VERSION of the full backup")?
        .trim()
        .parse()
        .context("invalid PG_VERSION in the full backup")?;
    let data_dir = output_dir(restore_root, generated_id, Utc::now(), major);
    if fs::read_dir(&data_dir).is_ok_and(|mut entries| entries.next().is_some()) {
        bail!(
            "Output directory {} already exists and is not empty; nothing was overwritten",
            data_dir.display()
        );
    }
    if let Some(parent) = data_dir.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut command = Command::new(select_pg_path(&major.to_string()).join("pg_combinebackup"));
    command.arg("-o").arg(&data_dir).args(&backup_dirs);
    let (succeeded, output) = run_logged(command, "pg_combinebackup", logger)?;
    if !succeeded {
        bail!("pg_combinebackup failed: {}", output.trim());
    }
    verify(&data_dir, &data_dir, major, logger).inspect_err(|_| {
        logger.log(
            "error",
            format!(
                "The rebuilt directory is kept for inspection at {}",
                data_dir.display()
            ),
        )
    })?;

    let size = WalkDir::new(&data_dir)
        .into_iter()
        .filter_map(|entry| entry.ok()?.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum();
    logger.log(
        "info",
        format!(
            "Restored data directory ({}): {}\nThis path is inside the agent; on the host, it is under the folder mounted at RESTORE_PATH.",
            human_size(size),
            data_dir.display()
        ),
    );
    logger.log("info", start_commands(&data_dir, major).join("\n"));
    for missing in missing_config_files(&data_dir) {
        logger.log(
            "warn",
            format!(
                "{missing} is not in the restored directory: the source server keeps it elsewhere (SHOW config_file, hba_file and ident_file on the source tell where). Copy it into $PGDATA before starting."
            ),
        );
    }

    Ok(RestoredCluster {
        data_dir,
        major,
        size,
    })
}
