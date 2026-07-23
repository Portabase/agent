use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::domain::postgres::connection::sniff_format;
use crate::domain::postgres::format::PostgresDumpFormat;
use crate::services::backup::logger::JobLogger;

pub(crate) struct PreparedArchive {
    path: PathBuf,
    _tmp: Option<tempfile::TempDir>,
    toc: String,
}

impl PreparedArchive {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn toc(&self) -> &str {
        &self.toc
    }
}

pub(crate) fn prepare_archive(
    format: PostgresDumpFormat,
    restore_file: &Path,
    pg_restore: &Path,
    logger: &JobLogger,
) -> Result<PreparedArchive> {
    let sniffed = sniff_format(restore_file)?;
    if sniffed != format {
        logger.log("warn", format!("Declared format {:?} != sniffed {:?}; using sniffed", format, sniffed));
    }
    let format = sniffed;

    let (path, tmp) = match format {
        PostgresDumpFormat::Fc => (restore_file.to_path_buf(), None),
        PostgresDumpFormat::Fd => {
            let tar_gz = std::fs::File::open(restore_file)?;
            let dec = flate2::read::GzDecoder::new(tar_gz);
            let mut archive = tar::Archive::new(dec);
            let tmp_dir = tempfile::TempDir::new()?;
            archive.unpack(tmp_dir.path())?;

            let dump_dir = if tmp_dir.path().join("toc.dat").exists() {
                tmp_dir.path().to_path_buf()
            } else {
                std::fs::read_dir(tmp_dir.path())?
                    .filter_map(|e| e.ok())
                    .find(|entry| entry.path().join("toc.dat").exists())
                    .map(|e| e.path())
                    .ok_or_else(|| anyhow::anyhow!("Invalid FD archive: toc.dat not found"))?
            };
            (dump_dir, Some(tmp_dir))
        }
    };

    let toc_out = Command::new(pg_restore).arg("-l").arg(&path).output()?;
    if !toc_out.status.success() {
        let stderr = String::from_utf8_lossy(&toc_out.stderr).to_string();
        logger.log("error", format!("pg_restore -l failed: {}", stderr));
        anyhow::bail!("Archive validation failed (pg_restore -l): {}", stderr);
    }
    let toc = String::from_utf8_lossy(&toc_out.stdout).to_string();

    Ok(PreparedArchive { path, _tmp: tmp, toc })
}
