use anyhow::{Context, Result, anyhow, bail};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;

use crate::services::backup::logger::JobLogger;

const BUFFER_SIZE: usize = 8 * 1024 * 1024;

#[allow(dead_code)]
pub struct CompressionResult {
    pub compressed_path: PathBuf,
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .context("compression task panicked")?
}

pub async fn compress_to_tar_gz_large(
    file: &PathBuf,
    logger: Arc<JobLogger>,
) -> Result<CompressionResult> {
    if file
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.ends_with(".tar.gz"))
        .unwrap_or(false)
    {
        logger.log(
            "info",
            format!("File {:?} is already a tar.gz, skipping compression", file),
        );
        return Ok(CompressionResult {
            compressed_path: file.clone(),
        });
    }

    if file
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.ends_with(".tar"))
        .unwrap_or(false)
    {
        let gz_path = PathBuf::from(format!("{}.gz", file.display()));
        logger.log(
            "info",
            format!("Input {:?} is a raw tar, gzipping directly", file),
        );

        let (input_path, output_path) = (file.clone(), gz_path.clone());
        blocking(move || {
            let input = File::open(&input_path)
                .map_err(|e| anyhow!("Failed to open tar {:?}: {}", input_path, e))?;
            let output = File::create(&output_path)
                .map_err(|e| anyhow!("Failed to create {:?}: {}", output_path, e))?;
            let mut encoder = GzEncoder::new(BufWriter::new(output), Compression::default());

            io::copy(
                &mut BufReader::with_capacity(BUFFER_SIZE, input),
                &mut encoder,
            )
            .map_err(|e| anyhow!("Gzip copy failed: {}", e))?;
            encoder
                .finish()
                .and_then(|mut writer| writer.flush())
                .map_err(|e| anyhow!("Gzip shutdown failed: {}", e))
        })
        .await?;

        logger.log("info", format!("Compressed {:?} to {:?}", file, gz_path));
        return Ok(CompressionResult {
            compressed_path: gz_path,
        });
    }

    let tar_gz_path = file.with_extension("").with_extension("tar.gz");
    tar_gz_file_to(file, &tar_gz_path, logger).await?;

    Ok(CompressionResult {
        compressed_path: tar_gz_path,
    })
}

pub async fn tar_gz_file_to(file: &Path, tar_gz_path: &Path, logger: Arc<JobLogger>) -> Result<()> {
    let (input_path, output_path, task_logger) = (
        file.to_path_buf(),
        tar_gz_path.to_path_buf(),
        logger.clone(),
    );
    blocking(move || {
        let fail = |message: String| {
            task_logger.log("error", message.clone());
            anyhow!(message)
        };

        let output = File::create(&output_path).map_err(|e| {
            fail(format!(
                "Failed to create output file {:?}: {}",
                output_path, e
            ))
        })?;
        let file_name = input_path
            .file_name()
            .ok_or_else(|| fail(format!("Cannot get file name for {:?}", input_path)))?;

        let mut tar_builder = tar::Builder::new(GzEncoder::new(
            BufWriter::new(output),
            Compression::default(),
        ));
        tar_builder
            .append_path_with_name(&input_path, file_name)
            .map_err(|e| fail(format!("Failed to append path: {}", e)))?;
        tar_builder
            .finish()
            .map_err(|e| fail(format!("Failed to finish tar: {}", e)))?;
        let gzip = tar_builder
            .into_inner()
            .map_err(|e| fail(format!("Failed to extract gzip encoder: {}", e)))?;
        gzip.finish()
            .and_then(|mut writer| writer.flush())
            .map_err(|e| fail(format!("Gzip shutdown failed: {}", e)))
    })
    .await?;

    logger.log(
        "info",
        format!("Compressed {:?} to {:?}", &file, &tar_gz_path),
    );

    Ok(())
}

pub async fn decompress_large_tar_gz(
    tar_gz_path: &Path,
    output_dir: &Path,
) -> Result<Vec<PathBuf>> {
    let (archive_path, target_dir) = (tar_gz_path.to_path_buf(), output_dir.to_path_buf());
    let extracted_files = blocking(move || {
        let file = File::open(&archive_path)?;
        let mut archive =
            tar::Archive::new(GzDecoder::new(BufReader::with_capacity(BUFFER_SIZE, file)));

        let mut extracted_files = Vec::new();
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.into_owned();
            // unpack_in refuses `..` and paths through symlinks outside target_dir;
            // absolute paths would be silently re-rooted, so reject them too.
            if path.has_root() || !entry.unpack_in(&target_dir)? {
                bail!("Archive entry {:?} escapes {:?}", path, target_dir);
            }
            extracted_files.push(target_dir.join(&path));
        }
        Ok(extracted_files)
    })
    .await?;

    info!("Decompressed {:?} into {:?}", tar_gz_path, output_dir);

    Ok(extracted_files)
}

pub async fn gunzip_to_file(gz_path: &Path, out_path: &Path) -> Result<()> {
    let (input_path, output_path) = (gz_path.to_path_buf(), out_path.to_path_buf());
    blocking(move || {
        let file = File::open(&input_path)
            .with_context(|| format!("Failed to open {}", input_path.display()))?;
        let out = File::create(&output_path)
            .with_context(|| format!("Failed to create {}", output_path.display()))?;
        let mut writer = BufWriter::new(out);

        io::copy(
            &mut GzDecoder::new(BufReader::with_capacity(BUFFER_SIZE, file)),
            &mut writer,
        )?;
        writer.flush()?;
        Ok(())
    })
    .await?;

    info!("Gunzipped {:?} into {:?}", gz_path, out_path);
    Ok(())
}
