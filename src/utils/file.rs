#![allow(dead_code)]

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use uuid::Uuid;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use base64::engine::general_purpose;
use bytes::Bytes;
use futures::Stream;
use rand::rngs::OsRng;
use rand::TryRngCore;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::info;

#[derive(Serialize, Deserialize)]
pub struct EncryptionMetadataFile {
    pub version: u8,
    pub cipher: String,
    pub encrypted_aes_key_b64: String,
    pub iv_b64: String,
}

pub fn full_extension(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.find('.').map(|i| &n[i..]))
        .unwrap_or("")
        .to_string()
}

pub fn full_file_name(encrypt: bool) -> String {
    let uuid = Uuid::new_v4();
    let base_name = format!("{}.{}", uuid, "tar.gz");
    if encrypt {
        format!("{}.enc", base_name)
    } else {
        base_name.to_string()
    }
}

pub fn full_file_path(file_name: &String) -> String {
    format!("backups/{}/{}", Utc::now().format("%Y-%m-%d"), file_name)
}

const CHUNK_SIZE: usize = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize, Debug)]
struct FileHeader {
    version: u8,
    cipher: String,
    chunk_size: usize,
    base_nonce: Vec<u8>,
}

pub async fn encrypt_file_stream_gcm(
    file_path: PathBuf,
    master_key_b64: String,
) -> Result<impl Stream<Item = Result<Bytes>> + Send + 'static> {
    let master_key_bytes = general_purpose::STANDARD
        .decode(master_key_b64)
        .map_err(|_| anyhow::anyhow!("Invalid base64"))?;

    let (tx, rx) = mpsc::channel(8);

    tokio::spawn(async move {
        let mut rng = OsRng;
        let mut base_nonce = [0u8; 8];
        rng.try_fill_bytes(&mut base_nonce).unwrap();

        let key = Key::<Aes256Gcm>::try_from(master_key_bytes.as_slice())
            .map_err(|_| anyhow::anyhow!("Invalid AES-256 key length")).unwrap();

        let cipher = Aes256Gcm::new(&key);

        let header = FileHeader {
            version: 1,
            cipher: "AES-256-GCM".to_string(),
            chunk_size: CHUNK_SIZE,
            base_nonce: base_nonce.to_vec(),
        };
        let header_json = serde_json::to_string(&header).unwrap();
        tx.send(Ok(Bytes::from(header_json + "\n"))).await.unwrap();

        let file = File::open(&file_path).await.unwrap();
        let mut reader = BufReader::new(file);
        let mut buffer = vec![0u8; CHUNK_SIZE];
        let mut chunk_index: u32 = 0;

        loop {
            let n = reader.read(&mut buffer).await.unwrap();
            if n == 0 {
                break;
            }

            let mut nonce_bytes = [0u8; 12];
            nonce_bytes[..8].copy_from_slice(&base_nonce);
            nonce_bytes[8..].copy_from_slice(&chunk_index.to_be_bytes());
            let nonce = Nonce::try_from(&nonce_bytes[..])
                .map_err(|_| anyhow::anyhow!("Invalid nonce length")).unwrap();

            let ciphertext = cipher.encrypt(&nonce, &buffer[..n]).unwrap();
            let mut out = Vec::with_capacity(4 + ciphertext.len());
            out.extend_from_slice(&(ciphertext.len() as u32).to_be_bytes());
            out.extend_from_slice(&ciphertext);

            tx.send(Ok(Bytes::from(out))).await.unwrap();
            chunk_index += 1;
        }
    });

    Ok(ReceiverStream::new(rx))
}

pub async fn decrypt_file_stream_gcm(
    encrypted_path: PathBuf,
    decrypted_path: PathBuf,
    master_key_b64: String,
) -> Result<()> {
    info!("Decrypting {:?}", decrypted_path);

    let master_key_bytes = general_purpose::STANDARD
        .decode(master_key_b64)
        .map_err(|_| anyhow::anyhow!("Invalid base64"))?;

    let mut reader = BufReader::new(File::open(&encrypted_path).await?);

    let mut header_line = Vec::new();
    reader.read_until(b'\n', &mut header_line).await?;
    let header: FileHeader = serde_json::from_slice(&header_line)?;

    let key = Key::<Aes256Gcm>::try_from(master_key_bytes.as_slice())
        .map_err(|_| anyhow::anyhow!("Invalid AES-256 key length"))?;
    let cipher = Aes256Gcm::new(&key);

    let mut writer = BufWriter::new(File::create(&decrypted_path).await?);
    let mut chunk_index: u32 = 0;

    loop {
        let mut len_buf = [0u8; 4];
        match reader.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
        let chunk_len = u32::from_be_bytes(len_buf) as usize;

        let mut chunk_ciphertext = vec![0u8; chunk_len];
        reader.read_exact(&mut chunk_ciphertext).await?;

        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[..8].copy_from_slice(&header.base_nonce);
        nonce_bytes[8..].copy_from_slice(&chunk_index.to_be_bytes());
        let nonce = Nonce::try_from(&nonce_bytes[..])
            .map_err(|_| anyhow::anyhow!("Invalid nonce length"))?;

        let plaintext = cipher
            .decrypt(&nonce, chunk_ciphertext.as_slice())
            .map_err(|e| anyhow::anyhow!("AES-GCM decryption failed: {:?}", e))?;

        writer.write_all(&plaintext).await?;
        chunk_index += 1;
    }

    writer.flush().await?;
    Ok(())
}
