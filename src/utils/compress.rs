use anyhow::Result;
use async_compression::tokio::write::GzipEncoder as AsyncGzipEncoder;
use std::ffi::OsString;
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio_tar::Builder as TokioTarBuilder;
use tracing::info;

pub struct CompressionResult {
    pub compressed_path: PathBuf,
    pub original_extension: Option<OsString>,
}

pub async fn compress_to_tar_gz_large(file: &PathBuf) -> Result<CompressionResult> {
    let original_extension = file.extension().map(|e| e.to_os_string());
    let tar_gz_path = file.with_extension("tar.gz");

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
        original_extension,
    })
}
