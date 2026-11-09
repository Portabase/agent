use super::checks::ServerFacts;
use super::decision::Decision;
use crate::domain::postgres::connection::select_pg_path;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result, anyhow, bail};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

const COMPRESSION_METHODS: [&str; 3] = ["server-zstd", "server-gzip", "none"];

pub struct PhysicalBackup {
    pub bundle: PathBuf,
    pub parent_backup_id: Option<String>,
    pub manifest: Vec<u8>,
}

pub fn is_compression_unsupported(stderr: &str) -> bool {
    stderr.contains("compression is not supported by this build")
        || stderr.contains("does not support compression with")
}

fn failure_reason(stderr: &str) -> &str {
    stderr
        .lines()
        .rfind(|line| line.starts_with("pg_basebackup: error:"))
        .or_else(|| stderr.lines().rfind(|line| !line.trim().is_empty()))
        .unwrap_or_default()
        .trim()
}

pub async fn run(
    cfg: &DatabaseConfig,
    facts: &ServerFacts,
    decision: Decision,
    work_dir: &Path,
    logger: Arc<JobLogger>,
) -> Result<PhysicalBackup> {
    let (cfg, major, work_dir) = (cfg.clone(), facts.major, work_dir.to_path_buf());
    tokio::task::spawn_blocking(move || run_blocking(&cfg, major, decision, &work_dir, &logger))
        .await?
}

fn run_blocking(
    cfg: &DatabaseConfig,
    major: u32,
    decision: Decision,
    work_dir: &Path,
    logger: &JobLogger,
) -> Result<PhysicalBackup> {
    let bin_dir = select_pg_path(&major.to_string());
    let backup_dir = work_dir.join("backup");
    let verify_dir = work_dir.join("verify");
    let full_backup = || -> Result<()> {
        base_backup(cfg, &bin_dir, &backup_dir, None, logger)?.map_err(|stderr| {
            anyhow!(
                "pg_basebackup failed for {}: {}",
                cfg.name,
                failure_reason(&stderr)
            )
        })
    };

    let parent_backup_id = match decision {
        Decision::Full { .. } => {
            full_backup()?;
            None
        }
        Decision::Incremental {
            parent_backup_id,
            parent_manifest,
        } => {
            let parent_manifest_path = work_dir.join("parent_backup_manifest");
            fs::write(&parent_manifest_path, parent_manifest)?;
            let attempt = base_backup(
                cfg,
                &bin_dir,
                &backup_dir,
                Some(&parent_manifest_path),
                logger,
            );
            fs::remove_file(&parent_manifest_path)?;
            match attempt? {
                Ok(()) => Some(parent_backup_id),
                Err(stderr) => {
                    logger.log(
                        "warn",
                        format!(
                            "Incremental backup failed ({}); taking a full backup instead",
                            failure_reason(&stderr)
                        ),
                    );
                    full_backup()?;
                    None
                }
            }
        }
    };

    if parent_backup_id.is_some() {
        extract_backup(&backup_dir, &verify_dir)?;
        verify(&backup_dir, &verify_dir, major, logger)?;
        fs::remove_dir_all(&verify_dir)?;
    }

    let raw_wal = backup_dir.join("pg_wal.tar");
    if raw_wal.is_file() {
        zstd::stream::copy_encode(
            File::open(&raw_wal)?,
            File::create(backup_dir.join("pg_wal.tar.zst"))?,
            0,
        )?;
        fs::remove_file(&raw_wal)?;
    }

    let manifest = fs::read(backup_dir.join("backup_manifest"))?;
    let bundle = work_dir.join(format!("{}.tar.gz", cfg.generated_id));
    let mut bundle_builder =
        tar::Builder::new(GzEncoder::new(File::create(&bundle)?, Compression::none()));
    bundle_builder.append_dir_all("", &backup_dir)?;
    bundle_builder.into_inner()?.finish()?;

    fs::remove_dir_all(&backup_dir)?;

    Ok(PhysicalBackup {
        bundle,
        parent_backup_id,
        manifest,
    })
}

fn base_backup(
    cfg: &DatabaseConfig,
    bin_dir: &Path,
    backup_dir: &Path,
    parent_manifest: Option<&Path>,
    logger: &JobLogger,
) -> Result<Result<(), String>> {
    let mut stderr = String::new();
    for compression in COMPRESSION_METHODS {
        if backup_dir.exists() {
            fs::remove_dir_all(backup_dir)
                .with_context(|| format!("could not remove {}", backup_dir.display()))?;
        }
        let mut command = Command::new(bin_dir.join("pg_basebackup"));
        command
            .arg("--host")
            .arg(&cfg.host)
            .arg("--port")
            .arg(cfg.port.to_string())
            .arg("--username")
            .arg(&cfg.username)
            .arg("-D")
            .arg(backup_dir)
            .args(["-Ft", "-X", "stream"])
            .arg(format!("--compress={compression}"))
            .args(["-w", "-v"])
            .env("PGPASSWORD", &cfg.password);
        let mut label = format!("pg_basebackup --compress={compression}");
        if let Some(parent_manifest) = parent_manifest {
            let mut incremental = OsString::from("--incremental=");
            incremental.push(parent_manifest);
            command.arg(incremental);
            label.push_str(" --incremental");
        }

        let (succeeded, output) = run_logged(command, &label, logger)?;
        if succeeded {
            return Ok(Ok(()));
        }
        stderr = output;
        if !is_compression_unsupported(&stderr) {
            break;
        }
        logger.log(
            "warn",
            format!(
                "Compression {compression} is not supported by the server; trying the next method"
            ),
        );
    }
    Ok(Err(stderr))
}

pub fn verify(
    backup_dir: &Path,
    extracted_dir: &Path,
    major: u32,
    logger: &JobLogger,
) -> Result<()> {
    let mut command = Command::new(select_pg_path(&major.to_string()).join("pg_verifybackup"));
    command
        .arg("--no-parse-wal")
        .arg("--manifest-path")
        .arg(backup_dir.join("backup_manifest"))
        .arg(extracted_dir);
    let (succeeded, output) = run_logged(command, "pg_verifybackup --no-parse-wal", logger)?;
    if !succeeded {
        bail!("pg_verifybackup failed: {}", output.trim());
    }
    Ok(())
}

pub fn extract_backup(backup_dir: &Path, target: &Path) -> Result<()> {
    unpack(&archive_named(backup_dir, "base")?, target)?;
    unpack(
        &archive_named(backup_dir, "pg_wal")?,
        &target.join("pg_wal"),
    )
}

fn archive_named(backup_dir: &Path, stem: &str) -> Result<PathBuf> {
    ["tar.zst", "tar.gz", "tar"]
        .into_iter()
        .map(|extension| backup_dir.join(format!("{stem}.{extension}")))
        .find(|path| path.is_file())
        .with_context(|| format!("{stem}.tar not found in {}", backup_dir.display()))
}

fn unpack(archive: &Path, target: &Path) -> Result<()> {
    let file =
        File::open(archive).with_context(|| format!("could not open {}", archive.display()))?;
    let reader: Box<dyn Read> = match archive.extension().and_then(|extension| extension.to_str()) {
        Some("zst") => Box::new(zstd::Decoder::new(file)?),
        Some("gz") => Box::new(GzDecoder::new(BufReader::new(file))),
        _ => Box::new(BufReader::new(file)),
    };
    fs::create_dir_all(target)?;
    tar::Archive::new(reader)
        .unpack(target)
        .with_context(|| format!("could not extract {}", archive.display()))
}

fn run_logged(mut command: Command, label: &str, logger: &JobLogger) -> Result<(bool, String)> {
    let start = Instant::now();
    let result = command.output();
    let duration_ms = start.elapsed().as_millis() as f64;
    let output = result
        .inspect_err(|error| {
            logger.log_command(label, Some(error.to_string()), Some(-1), Some(duration_ms))
        })
        .with_context(|| format!("could not run {label}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    logger.log_command(
        label,
        (!text.trim().is_empty()).then(|| text.clone()),
        Some(output.status.code().unwrap_or(-1)),
        Some(duration_ms),
    );
    Ok((output.status.success(), text))
}
