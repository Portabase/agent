use super::matcher::ExcludeMatcher;
use super::platform::dev;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result, bail};
use flate2::Compression;
use flate2::write::GzEncoder;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use walkdir::WalkDir;

pub async fn run(
    cfg: DatabaseConfig,
    backup_dir: PathBuf,
    logger: Arc<JobLogger>,
) -> Result<PathBuf> {
    tokio::task::spawn_blocking(move || archive(&cfg, &backup_dir, &logger)).await?
}

fn archive(cfg: &DatabaseConfig, backup_dir: &Path, logger: &JobLogger) -> Result<PathBuf> {
    // Canonical paths so the output location can be recognised while walking.
    let root = std::fs::canonicalize(&cfg.path)
        .with_context(|| format!("cannot read source directory {}", cfg.path))?;
    if !root.is_dir() {
        bail!("source path {} is not a directory", root.display());
    }
    let backup_real = std::fs::canonicalize(backup_dir)
        .with_context(|| format!("cannot read backup directory {}", backup_dir.display()))?;
    let matcher = ExcludeMatcher::from_config(cfg)?;
    let one_file_system = cfg
        .options
        .get("one_file_system")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let root_dev = dev(&std::fs::metadata(&root)
        .with_context(|| format!("cannot read source directory {}", root.display()))?);

    let file_name = format!("{}.tar.gz", cfg.generated_id);
    let out = backup_dir.join(&file_name);
    let out_real = backup_real.join(&file_name);
    let gz = GzEncoder::new(File::create(&out)?, Compression::default());
    let mut tar = tar::Builder::new(gz);
    tar.follow_symlinks(false);

    logger.log("info", format!("Archiving {}", root.display()));
    let mut files = 0u64;
    // Entries that could not be read: a mirror restore of this archive would delete them.
    let mut unreadable = 0usize;
    let mut walker = WalkDir::new(&root)
        .follow_links(false)
        .min_depth(1)
        .into_iter();

    while let Some(entry) = walker.next() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                logger.log("warn", format!("Skipping unreadable entry: {e}"));
                unreadable += 1;
                continue;
            }
        };
        let rel = entry.path().strip_prefix(&root)?.to_path_buf();
        let ft = entry.file_type();

        // Never archive our own output (backup dir under the source, or the archive itself).
        let mut skip =
            matcher.is_excluded(&rel) || entry.path() == backup_real || entry.path() == out_real;
        if !skip && one_file_system && ft.is_dir() {
            match entry.metadata() {
                Ok(m) => skip = dev(&m) != root_dev,
                Err(e) => {
                    logger.log("warn", format!("Skipping {}: {e}", rel.display()));
                    unreadable += 1;
                    skip = true;
                }
            }
        }
        if skip {
            if ft.is_dir() {
                walker.skip_current_dir();
            }
            continue;
        }

        if ft.is_file() {
            // Only a failure to open is skippable: it happens before anything is
            // written. Every later error leaves the tar stream misaligned and must abort.
            let mut f = match File::open(entry.path()) {
                Ok(f) => f,
                Err(e) => {
                    logger.log("warn", format!("Skipping {}: {e}", rel.display()));
                    unreadable += 1;
                    continue;
                }
            };
            let meta = f.metadata()?;
            let size = meta.len();
            let mut header = tar::Header::new_gnu();
            header.set_metadata(&meta);
            // The header size is fixed up front: cap the copy if the file grew and
            // zero-pad if it shrank, so the entry always matches its header.
            let reader = (&mut f).take(size).chain(std::io::repeat(0)).take(size);
            tar.append_data(&mut header, &rel, reader)
                .with_context(|| format!("failed to archive {}", rel.display()))?;
            files += 1;
        } else if ft.is_dir() || ft.is_symlink() {
            // These fail on lstat/readlink before the header is written.
            if let Err(e) = tar.append_path_with_name(entry.path(), &rel) {
                logger.log("warn", format!("Skipping {}: {e}", rel.display()));
                unreadable += 1;
            }
        } else {
            // Sockets, fifos and devices hold no data to restore; restore keeps them too.
            logger.log("warn", format!("Skipping special file {}", rel.display()));
        }
    }

    if unreadable > 0 {
        let noun = if unreadable == 1 { "entry" } else { "entries" };
        bail!(
            "{unreadable} {noun} could not be read; backup aborted so a mirror restore cannot delete unarchived data (fix permissions or add an exclude)"
        );
    }
    tar.into_inner()?.finish()?;
    logger.log(
        "info",
        format!("Archived {files} file(s) into {}", out.display()),
    );
    Ok(out)
}
