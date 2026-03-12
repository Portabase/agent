use tempfile::tempdir;
use tokio::fs::{write, read};
use anyhow::Result;
use crate::utils::compress::{compress_to_tar_gz_large, decompress_large_tar_gz};

#[tokio::test]
async fn compress_creates_tar_gz() -> Result<()> {
    let tmp = tempdir()?;
    let file_path = tmp.path().join("test.txt");
    write(&file_path, b"hello world").await?;

    let result = compress_to_tar_gz_large(&file_path).await?;
    assert!(result.compressed_path.exists());
    assert_eq!(result.compressed_path.extension().unwrap(), "gz");

    Ok(())
}

#[tokio::test]
async fn compress_skips_existing_tar_gz() -> Result<()> {
    let tmp = tempdir()?;
    let file_path = tmp.path().join("already.tar.gz");
    write(&file_path, b"compressed").await?;

    let result = compress_to_tar_gz_large(&file_path).await?;
    // Should return same path without creating a new file
    assert_eq!(result.compressed_path, file_path);

    Ok(())
}

#[tokio::test]
async fn decompress_restores_file() -> Result<()> {
    let tmp = tempdir()?;
    let file_path = tmp.path().join("file.txt");
    write(&file_path, b"data for decompress").await?;

    let compress_result = compress_to_tar_gz_large(&file_path).await?;
    let output_dir = tmp.path().join("out");
    tokio::fs::create_dir_all(&output_dir).await?;

    let extracted_files = decompress_large_tar_gz(&compress_result.compressed_path, &output_dir).await?;
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
    let compress1 = compress_to_tar_gz_large(&file1).await?;
    let compress2 = compress_to_tar_gz_large(&file2).await?;

    let output_dir = tmp.path().join("out_multi");
    tokio::fs::create_dir_all(&output_dir).await?;

    let extracted1 = decompress_large_tar_gz(&compress1.compressed_path, &output_dir).await?;
    let extracted2 = decompress_large_tar_gz(&compress2.compressed_path, &output_dir).await?;

    assert_eq!(extracted1.len(), 1);
    assert_eq!(extracted2.len(), 1);

    Ok(())
}
