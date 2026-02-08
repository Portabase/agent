use anyhow::Result;
use async_compression::tokio::bufread::GzipDecoder;
use async_compression::tokio::write::GzipEncoder as AsyncGzipEncoder;
use futures::StreamExt;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::fs::{create_dir_all};
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio_tar::Archive;
use tokio_tar::Builder as TokioTarBuilder;
use tracing::info;

#[allow(dead_code)]
pub struct CompressionResult {
    pub compressed_path: PathBuf,
}

pub async fn compress_to_tar_gz_large(file: &PathBuf) -> Result<CompressionResult> {
    if file
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.ends_with(".tar.gz"))
        .unwrap_or(false)
    {
        info!("File {:?} is already a tar.gz, skipping compression", file);
        return Ok(CompressionResult {
            compressed_path: file.clone(),
        });
    }

    let tar_gz_path = file.with_extension("").with_extension("tar.gz");

    let output_file = File::create(&tar_gz_path).await?;
    let gzip_writer = AsyncGzipEncoder::new(output_file);
    let mut tar_builder = TokioTarBuilder::new(gzip_writer);

    let file_name = file
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Cannot get file name for {:?}", file))?;

    tar_builder
        .append_path_with_name(file, file_name)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to append path: {}", e))?;

    tar_builder
        .finish()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to finish tar: {}", e))?;

    let mut gzip = tar_builder
        .into_inner()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to extract gzip encoder: {}", e))?;

    gzip.shutdown()
        .await
        .map_err(|e| anyhow::anyhow!("Gzip shutdown failed: {}", e))?;

    info!("Compressing {:?} to {:?}", &file, &tar_gz_path);

    Ok(CompressionResult {
        compressed_path: tar_gz_path,
    })
}

pub async fn decompress_large_tar_gz(
    tar_gz_path: &Path,
    output_dir: &Path,
) -> Result<Vec<PathBuf>> {
    let file = File::open(tar_gz_path).await?;
    let buf_reader = BufReader::with_capacity(8 * 1024 * 1024, file);
    let decoder = GzipDecoder::new(buf_reader);
    let mut archive = Archive::new(decoder);

    let mut extracted_files = Vec::new();
    let mut entries = archive.entries()?;

    while let Some(entry) = entries.next().await {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let full_path = output_dir.join(&path);

        if let Some(parent) = full_path.parent() {
            create_dir_all(parent).await?;
        }

        entry.unpack(&full_path).await?;
        extracted_files.push(full_path);
    }

    // remove_file(tar_gz_path).await?;
    info!("Decompressed {:?} into {:?}", tar_gz_path, output_dir);

    Ok(extracted_files)
}
