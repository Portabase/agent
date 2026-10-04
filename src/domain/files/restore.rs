use super::matcher::ExcludeMatcher;
use super::platform::{dev, is_root};
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use std::fs::{self, File};
use std::io::{self, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub async fn run(cfg: DatabaseConfig, archive: PathBuf, logger: Arc<JobLogger>) -> Result<()> {
    tokio::task::spawn_blocking(move || restore(&cfg, &archive, &logger)).await?
}

fn restore(cfg: &DatabaseConfig, archive: &Path, logger: &JobLogger) -> Result<()> {
    // Everything below works on the canonical path, like backup.rs.
    let root = ensure_restorable(Path::new(&cfg.path))?;
    let mut matcher = ExcludeMatcher::from_config(cfg)?;
    let one_file_system = cfg
        .options
        .get("one_file_system")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Opened once: the wipe may delete the archive's path, never an open handle.
    let file = File::open(archive)
        .with_context(|| format!("cannot open archive {}", archive.display()))?;

    // The wipe is irreversible: prove the whole archive reads back before touching data.
    logger.log("info", format!("Validating archive {}", archive.display()));
    validate_archive(&file).context("archive is corrupt or truncated, nothing was changed")?;

    // Keep the archive's own directory (or the archive itself when it sits directly
    // in the target), like backup.rs prunes its output.
    let archive_real = fs::canonicalize(archive)?;
    let keep = match archive_real.parent() {
        Some(dir) if dir != root => dir,
        _ => archive_real.as_path(),
    };
    if let Ok(rel) = keep.strip_prefix(&root) {
        matcher.exclude_path(rel.to_path_buf());
    }

    logger.log(
        "info",
        format!("Removing non-excluded content of {}", root.display()),
    );
    // Best effort: whatever could not be removed is kept, and extraction always runs.
    let failures = wipe_except_excluded(&root, &matcher, one_file_system, logger)?;

    logger.log(
        "info",
        format!("Extracting archive into {}", root.display()),
    );
    unpack(&file, &root, is_root(), logger)?;

    if failures > 0 {
        let noun = if failures == 1 { "entry" } else { "entries" };
        bail!(
            "restore extracted the archive but {failures} existing {noun} could not be removed; see warnings"
        );
    }
    logger.log("info", "Files restore finished".to_string());
    Ok(())
}

/// Extract the whole archive into `root`. Filesystems that refuse chown/chmod/utime
/// (NFS root_squash, CIFS, NFSv4 idmap mismatch) fail the first pass, after the wipe,
/// so retry once without metadata: the content matters more than owner and mode.
pub fn unpack(
    file: &File,
    root: &Path,
    preserve_ownerships: bool,
    logger: &JobLogger,
) -> Result<()> {
    if let Err(e) = unpack_once(file, root, true, preserve_ownerships) {
        logger.log(
            "warn",
            format!("extraction with ownership/permissions failed: {e:#}; retrying without metadata preservation"),
        );
        // ponytail: tar-rs still chmods to `mode & 0o777` with preservation off, so a
        // filesystem that rejects chmod on files the agent just created fails this pass
        // too; per-entry extraction with deferred directories if that shows up.
        unpack_once(file, root, false, false)?;
    }
    Ok(())
}

fn unpack_once(
    mut file: &File,
    root: &Path,
    preserve: bool,
    preserve_ownerships: bool,
) -> Result<()> {
    file.rewind()?;
    let mut ar = tar::Archive::new(GzDecoder::new(file));
    ar.set_preserve_permissions(preserve);
    ar.set_preserve_mtime(preserve);
    ar.set_preserve_ownerships(preserve_ownerships);
    ar.set_overwrite(true);
    ar.unpack(root)?;
    Ok(())
}

/// Read every entry to the end, then drain the gzip stream so its CRC is checked.
fn validate_archive(file: &File) -> Result<()> {
    let mut gz = GzDecoder::new(file);
    let mut ar = tar::Archive::new(&mut gz);
    for entry in ar.entries()? {
        io::copy(&mut entry?, &mut io::sink())?;
    }
    drop(ar);
    // tar stops at its end marker, before the gzip trailer.
    gz.read_to_end(&mut Vec::new())?;
    Ok(())
}

/// Refuse targets a mirror restore must never touch, and return the canonical path to
/// restore into. The canonical path is checked too, because `/data/..` style escapes
/// resolve to `/`.
pub fn ensure_restorable(root: &Path) -> Result<PathBuf> {
    if root.as_os_str().is_empty() || !root.is_absolute() || root == Path::new("/") {
        bail!(
            "refusing to restore into {:?}: path must be absolute and not /",
            root
        );
    }
    if !root.is_dir() {
        bail!(
            "restore target {} is not an existing directory",
            root.display()
        );
    }
    let canonical = fs::canonicalize(root)
        .with_context(|| format!("cannot resolve restore target {}", root.display()))?;
    if canonical == Path::new("/") {
        bail!("refusing to restore into {:?}: it resolves to /", root);
    }
    Ok(canonical)
}

/// Delete everything under `root` that does not match an exclude pattern.
/// Excluded entries (and their subtrees) are kept; `root` itself is kept.
///
/// Mount points (a directory on a different device than its parent): with
/// `one_file_system` they are kept untouched like an excluded path, matching what
/// backup never archived. Without it their contents are wiped but the mount point
/// itself is never removed (`remove_dir` on it fails with EBUSY) and counts as kept.
/// Special files (sockets, fifos, devices) are kept: backup never archives them.
///
/// Best effort: an entry that cannot be read or removed is logged, kept (so its
/// parents are kept too) and counted; the returned count is how many failed.
/// Only an unreadable `root` is an error, and then nothing was deleted.
pub fn wipe_except_excluded(
    root: &Path,
    matcher: &ExcludeMatcher,
    one_file_system: bool,
    logger: &JobLogger,
) -> Result<usize> {
    let meta = fs::symlink_metadata(root).with_context(|| format!("reading {}", root.display()))?;
    let mut wipe = Wipe {
        matcher,
        one_file_system,
        logger,
        failures: 0,
    };
    wipe.dir(root, Path::new(""), dev(&meta));
    Ok(wipe.failures)
}

struct Wipe<'a> {
    matcher: &'a ExcludeMatcher,
    one_file_system: bool,
    logger: &'a JobLogger,
    failures: usize,
}

impl Wipe<'_> {
    fn fail(&mut self, what: &str, path: &Path, e: io::Error) {
        self.logger
            .log("warn", format!("Could not {what} {}: {e}", path.display()));
        self.failures += 1;
    }

    /// Returns true when something was kept inside `dir`. `rel` is `dir` relative to
    /// the root and `dir_dev` its device.
    fn dir(&mut self, dir: &Path, rel: &Path, dir_dev: u64) -> bool {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                self.fail("read", dir, e);
                return true;
            }
        };
        let mut kept = false;
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    self.fail("read", dir, e);
                    kept = true;
                    continue;
                }
            };
            let path = entry.path();
            let rel = rel.join(entry.file_name());
            if self.matcher.is_excluded(&rel) {
                kept = true;
                continue;
            }
            // symlink_metadata: a symlinked directory is removed as a link, never followed.
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(e) => {
                    self.fail("read", &path, e);
                    kept = true;
                    continue;
                }
            };
            let ft = meta.file_type();
            let removed = if ft.is_dir() {
                let mount_point = dev(&meta) != dir_dev;
                if mount_point && self.one_file_system {
                    kept = true;
                    continue;
                }
                if self.dir(&path, &rel, dev(&meta)) || mount_point {
                    kept = true;
                    continue;
                }
                fs::remove_dir(&path)
            } else if ft.is_file() || ft.is_symlink() {
                fs::remove_file(&path)
            } else {
                // Socket, fifo or device: backup never archives them, so keep them.
                kept = true;
                continue;
            };
            if let Err(e) = removed {
                self.fail("remove", &path, e);
                kept = true;
            }
        }
        kept
    }
}
