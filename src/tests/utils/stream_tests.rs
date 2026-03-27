use anyhow::Result;
use base64::{Engine as _, engine::general_purpose};
use futures::StreamExt;
use tempfile::tempdir;
use tokio::fs;

use crate::utils::file::decrypt_file_stream_gcm;
use crate::utils::stream::build_stream;

#[tokio::test]
async fn build_stream_without_encryption_yields_original_bytes() -> Result<()> {
    let tmp = tempdir()?;
    let file_path = tmp.path().join("plain.txt");
    let content = b"plain upload stream";
    fs::write(&file_path, content).await?;

    let mut upload_stream = build_stream(&file_path, false, &String::new())
        .await?
        .stream;
    let mut collected = Vec::new();

    while let Some(chunk) = upload_stream.next().await {
        collected.extend_from_slice(&chunk?);
    }

    assert_eq!(collected, content);

    Ok(())
}

#[tokio::test]
async fn build_stream_with_encryption_produces_decryptable_content() -> Result<()> {
    let tmp = tempdir()?;
    let input_path = tmp.path().join("plain.txt");
    let encrypted_path = tmp.path().join("encrypted.bin");
    let decrypted_path = tmp.path().join("decrypted.txt");
    let content = b"encrypted upload stream";
    fs::write(&input_path, content).await?;

    let key = general_purpose::STANDARD.encode([5u8; 32]);
    let mut upload_stream = build_stream(&input_path, true, &key).await?.stream;
    let mut encrypted_bytes = Vec::new();

    while let Some(chunk) = upload_stream.next().await {
        encrypted_bytes.extend_from_slice(&chunk?);
    }

    fs::write(&encrypted_path, encrypted_bytes).await?;
    decrypt_file_stream_gcm(encrypted_path, decrypted_path.clone(), key).await?;

    assert_eq!(fs::read(decrypted_path).await?, content);

    Ok(())
}
