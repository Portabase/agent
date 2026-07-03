use crate::utils::compress::{compress_to_tar_gz_large, decompress_large_tar_gz};
use anyhow::Result;
use tempfile::tempdir;
use tokio::fs::{read, write};

#[tokio::test]
async fn compress_creates_tar_gz() -> Result<()> {
    let tmp = tempdir()?;
    let file_path = tmp.path().join("test.txt");
    write(&file_path, b"hello world").await?;

    let result = compress_to_tar_gz_large(&file_path, std::sync::Arc::new(crate::services::backup::logger::JobLogger::new())).await?;
    assert!(result.compressed_path.exists());
    assert_eq!(result.compressed_path.extension().unwrap(), "gz");

    Ok(())
}

#[tokio::test]
async fn compress_skips_existing_tar_gz() -> Result<()> {
    let tmp = tempdir()?;
    let file_path = tmp.path().join("already.tar.gz");
    write(&file_path, b"compressed").await?;

    let result = compress_to_tar_gz_large(&file_path, std::sync::Arc::new(crate::services::backup::logger::JobLogger::new())).await?;
    assert_eq!(result.compressed_path, file_path);

    Ok(())
}

#[tokio::test]
async fn decompress_restores_file() -> Result<()> {
    let tmp = tempdir()?;
    let file_path = tmp.path().join("file.txt");
    write(&file_path, b"data for decompress").await?;

    let compress_result = compress_to_tar_gz_large(&file_path, std::sync::Arc::new(crate::services::backup::logger::JobLogger::new())).await?;
    let output_dir = tmp.path().join("out");
    tokio::fs::create_dir_all(&output_dir).await?;

    let extracted_files =
        decompress_large_tar_gz(&compress_result.compressed_path, &output_dir).await?;
    assert_eq!(extracted_files.len(), 1);

    let extracted_content = read(&extracted_files[0]).await?;
    assert_eq!(extracted_content, b"data for decompress");

    Ok(())
}

#[tokio::test]
async fn decompress_multiple_files() -> Result<()> {
    let tmp = tempdir()?;
    let file1 = tmp.path().join("file1.txt");
    let file2 = tmp.path().join("file2.txt");
    write(&file1, b"file1").await?;
    write(&file2, b"file2").await?;

    // Compress both files individually (for simplicity in this test)
    let compress1 = compress_to_tar_gz_large(&file1, std::sync::Arc::new(crate::services::backup::logger::JobLogger::new())).await?;
    let compress2 = compress_to_tar_gz_large(&file2, std::sync::Arc::new(crate::services::backup::logger::JobLogger::new())).await?;

    let output_dir = tmp.path().join("out_multi");
    tokio::fs::create_dir_all(&output_dir).await?;

    let extracted1 = decompress_large_tar_gz(&compress1.compressed_path, &output_dir).await?;
    let extracted2 = decompress_large_tar_gz(&compress2.compressed_path, &output_dir).await?;

    assert_eq!(extracted1.len(), 1);
    assert_eq!(extracted2.len(), 1);

    Ok(())
}

#[tokio::test]
async fn compress_tar_is_not_double_wrapped() -> Result<()> {
    use tokio_tar::Builder as TarBuilder;

    let tmp = tempdir()?;
    // Build a real tar containing a single entry "payload.txt".
    let payload = tmp.path().join("payload.txt");
    write(&payload, b"volume-bytes").await?;
    let tar_path = tmp.path().join("volume.tar");
    {
        let f = tokio::fs::File::create(&tar_path).await?;
        let mut b = TarBuilder::new(f);
        b.append_path_with_name(&payload, "payload.txt").await?;
        b.finish().await?;
    }

    let result = compress_to_tar_gz_large(
        &tar_path,
        std::sync::Arc::new(crate::services::backup::logger::JobLogger::new()),
    )
    .await?;
    assert_eq!(result.compressed_path, tmp.path().join("volume.tar.gz"));

    // Decompress and confirm the FIRST tar entry is "payload.txt" — i.e. our tar
    // was gzipped directly, not wrapped inside another tar named "volume.tar".
    let out = tmp.path().join("out");
    tokio::fs::create_dir_all(&out).await?;
    let files = decompress_large_tar_gz(&result.compressed_path, &out).await?;
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_name().unwrap(), "payload.txt");
    assert_eq!(read(&files[0]).await?, b"volume-bytes");

    Ok(())
}

#[tokio::test]
async fn gunzip_to_file_restores_tar_byte_for_byte() -> Result<()> {
    use crate::utils::compress::gunzip_to_file;

    let tmp = tempdir()?;
    let tar_path = tmp.path().join("input.tar");
    let original: Vec<u8> = (0u32..50_000).map(|n| (n % 256) as u8).collect();
    write(&tar_path, &original).await?;

    let gz = compress_to_tar_gz_large(&tar_path, std::sync::Arc::new(crate::services::backup::logger::JobLogger::new())).await?;

    let out_tar = tmp.path().join("out.tar");
    gunzip_to_file(&gz.compressed_path, &out_tar).await?;

    assert_eq!(read(&out_tar).await?, original);

    Ok(())
}
