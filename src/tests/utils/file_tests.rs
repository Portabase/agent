use anyhow::Result;
use base64::{Engine as _, engine::general_purpose};
use futures::StreamExt;
use serde_json::Value;
use tempfile::tempdir;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::utils::file::{
    decrypt_file_stream_gcm, encrypt_file_stream_gcm, full_extension, full_file_name,
    full_file_path,
};

#[test]
fn full_extension_returns_everything_after_first_dot() {
    assert_eq!(
        full_extension(std::path::Path::new("archive.tar.gz")),
        ".tar.gz"
    );
    assert_eq!(full_extension(std::path::Path::new("README")), "");
}

#[test]
fn full_file_name_matches_expected_suffix() {
    let unencrypted = full_file_name(false);
    let encrypted = full_file_name(true);

    assert!(unencrypted.ends_with(".tar.gz"));
    assert!(encrypted.ends_with(".tar.gz.enc"));
}

#[test]
fn full_file_path_prefixes_backups_directory_and_date() {
    let file_name = "backup.tar.gz".to_string();
    let full_path = full_file_path(&file_name);

    assert!(full_path.starts_with("backups/"));
    assert!(full_path.ends_with("/backup.tar.gz"));
}

#[tokio::test]
async fn encrypt_and_decrypt_round_trip_restores_original_bytes() -> Result<()> {
    let tmp = tempdir()?;
    let input_path = tmp.path().join("plain.txt");
    let encrypted_path = tmp.path().join("cipher.bin");
    let decrypted_path = tmp.path().join("plain.out.txt");
    let original = b"encryption round-trip payload";

    fs::write(&input_path, original).await?;

    let key = general_purpose::STANDARD.encode([7u8; 32]);
    let mut encrypted_stream = encrypt_file_stream_gcm(input_path.clone(), key.clone()).await?;
    let mut encrypted_file = fs::File::create(&encrypted_path).await?;

    while let Some(chunk) = encrypted_stream.next().await {
        encrypted_file.write_all(&chunk?).await?;
    }

    decrypt_file_stream_gcm(encrypted_path, decrypted_path.clone(), key).await?;

    let decrypted = fs::read(decrypted_path).await?;
    assert_eq!(decrypted, original);

    Ok(())
}

#[tokio::test]
async fn encrypt_stream_starts_with_json_header_line() -> Result<()> {
    let tmp = tempdir()?;
    let input_path = tmp.path().join("plain.txt");
    fs::write(&input_path, b"header test").await?;

    let key = general_purpose::STANDARD.encode([9u8; 32]);
    let mut encrypted_stream = encrypt_file_stream_gcm(input_path, key).await?;
    let first_chunk = encrypted_stream.next().await.unwrap()?;

    let header_end = first_chunk
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("missing header newline");
    let header: Value = serde_json::from_slice(&first_chunk[..header_end])?;

    assert_eq!(header["version"], 1);
    assert_eq!(header["cipher"], "AES-256-GCM");
    assert_eq!(header["chunk_size"], 16 * 1024 * 1024);

    Ok(())
}
