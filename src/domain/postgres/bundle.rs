use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::format::PostgresDumpFormat;
use super::globals;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;

const MANIFEST_FILENAME: &str = "manifest.json";

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct BundleManifest {
    pub format: String,
    pub has_globals: bool,
    pub dump_path: String,
}

impl BundleManifest {
    pub fn write(&self, dir: &Path) -> Result<()> {
        let path = dir.join(MANIFEST_FILENAME);
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn read(dir: &Path) -> Result<Option<Self>> {
        let path = dir.join(MANIFEST_FILENAME);
        if !path.is_file() {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(path)?;
        Ok(Some(serde_json::from_str(&contents)?))
    }
}

/// Packs an already-produced dump artifact (a `.dump` file for `Fc`, or the
/// raw `pg_dump -Fd` output directory for `Fd`) together with a fresh
/// `globals.sql` and a `manifest.json` into one tar.gz. The returned path
/// already ends in `.tar.gz` so `compress_backup`'s "already compressed"
/// check short-circuits and never re-wraps it.
pub fn build(
    cfg: &DatabaseConfig,
    format: PostgresDumpFormat,
    dump_artifact: &Path,
    backup_dir: &Path,
    pg_version: &str,
    env: &HashMap<String, String>,
    logger: Arc<JobLogger>,
) -> Result<PathBuf> {
    let bundle_dir = backup_dir.join(format!("{}_bundle", cfg.generated_id));
    std::fs::create_dir_all(&bundle_dir)?;

    let dump_path_in_bundle = if dump_artifact.is_dir() {
        let dest = bundle_dir.join("dump_dir");
        std::fs::rename(dump_artifact, &dest)?;
        "dump_dir".to_string()
    } else {
        let dest = bundle_dir.join("dump.dump");
        std::fs::rename(dump_artifact, &dest)?;
        "dump.dump".to_string()
    };

    globals::dump(cfg, pg_version, &bundle_dir, env, &logger)?;

    BundleManifest {
        format: format.as_str().to_string(),
        has_globals: true,
        dump_path: dump_path_in_bundle,
    }
    .write(&bundle_dir)?;

    let tar_path = backup_dir.join(format!("{}.tar.gz", cfg.generated_id));
    let tar_gz = std::fs::File::create(&tar_path)?;
    let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
    let mut tar = tar::Builder::new(enc);
    tar.append_dir_all(".", &bundle_dir)?;
    tar.finish()?;

    logger.log("info", format!("Globals bundle created at {:?}", tar_path));

    Ok(tar_path)
}

pub struct ResolvedRestore {
    pub dump_path: PathBuf,
    pub globals_path: Option<PathBuf>,
    pub format_override: Option<PostgresDumpFormat>,
    _tmp_dir: Option<tempfile::TempDir>,
}

/// Peeks `restore_file` for a bundle produced by `build()`. Any archive
/// without a `manifest.json` at its root — every backup taken before this
/// feature existed, and every backup taken with `include_globals: false` —
/// passes through with `dump_path` unchanged and `format_override: None`,
/// leaving `restore.rs`'s existing Fc/Fd handling exactly as it was.
pub fn resolve(restore_file: &Path) -> Result<ResolvedRestore> {
    let passthrough = || ResolvedRestore {
        dump_path: restore_file.to_path_buf(),
        globals_path: None,
        format_override: None,
        _tmp_dir: None,
    };

    let file = match std::fs::File::open(restore_file) {
        Ok(f) => f,
        Err(_) => return Ok(passthrough()),
    };

    let dec = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(dec);
    let tmp_dir = tempfile::TempDir::new()?;

    if archive.unpack(tmp_dir.path()).is_err() {
        return Ok(passthrough());
    }

    let manifest = match BundleManifest::read(tmp_dir.path())? {
        Some(m) => m,
        None => return Ok(passthrough()),
    };

    let format_override = PostgresDumpFormat::from_str(&manifest.format).ok_or_else(|| {
        anyhow::anyhow!("Unknown format '{}' in bundle manifest", manifest.format)
    })?;

    let dump_path = tmp_dir.path().join(&manifest.dump_path);
    let globals_path = if manifest.has_globals {
        let p = tmp_dir.path().join("globals.sql");
        if p.is_file() { Some(p) } else { None }
    } else {
        None
    };

    Ok(ResolvedRestore {
        dump_path,
        globals_path,
        format_override: Some(format_override),
        _tmp_dir: Some(tmp_dir),
    })
}
