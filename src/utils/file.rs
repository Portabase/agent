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
use rand::{RngCore, TryRngCore};
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
//
// pub async fn encrypt_file_stream(
//     file_path: PathBuf,
//     aes_key: [u8; 32],
//     iv: [u8; 16],
//     pub_key_pem: Vec<u8>,
// ) -> Result<(impl Stream<Item = Result<Bytes>> + Send + 'static, Vec<u8>)> {
//     // ---------- Encrypt AES key with RSA ----------
//     let pkey = PKey::public_key_from_pem(&pub_key_pem)?;
//     let mut rsa = Encrypter::new(&pkey)?;
//     rsa.set_rsa_padding(Padding::PKCS1_OAEP)?;
//     rsa.set_rsa_oaep_md(MessageDigest::sha256())?;
//     rsa.set_rsa_mgf1_md(MessageDigest::sha256())?;
//
//     let mut encrypted_key = vec![0u8; rsa.encrypt_len(&aes_key)?];
//     let len = rsa.encrypt(&aes_key, &mut encrypted_key)?;
//     encrypted_key.truncate(len);
//
//     // ---------- Streaming AES encryption ----------
//     let stream = try_stream! {
//         let file = File::open(&file_path).await?;
//         let mut reader = BufReader::new(file);
//
//         let cipher = Cipher::aes_256_cbc();
//         let mut crypter = Crypter::new(cipher, Mode::Encrypt, &aes_key, Some(&iv))?;
//         crypter.pad(true);
//
//         let mut buffer = vec![0u8; 1024 * 1024];
//
//         loop {
//             let n = reader.read(&mut buffer).await?;
//             if n == 0 {
//                 break;
//             }
//
//             let mut out = vec![0u8; n + cipher.block_size()];
//             let count = crypter.update(&buffer[..n], &mut out)?;
//             out.truncate(count);
//
//             yield Bytes::from(out);
//         }
//
//         let mut final_block = vec![0u8; cipher.block_size()];
//         let rest = crypter.finalize(&mut final_block)?;
//         final_block.truncate(rest);
//
//         if !final_block.is_empty() {
//             yield Bytes::from(final_block);
//         }
//     };
//
//     Ok((stream, encrypted_key))
// }

// const CHUNK_SIZE: usize = 16 * 1024 * 1024;
//
// pub async fn encrypt_file_stream_gcm(
//     file_path: PathBuf,
//     master_key: [u8; 32],
// ) -> Result<(impl Stream<Item = Result<Bytes>> + Send + 'static, Vec<u8>)> {
//     // ---------- generate base nonce ----------
//     let mut base_nonce_bytes = [0u8; 8];
//     let mut rng = OsRng;
//     rng.try_fill_bytes(&mut base_nonce_bytes)?;
//
//     // AES-256 key from slice using TryFrom
//     let key = Key::<Aes256Gcm>::try_from(&master_key[..])
//         .map_err(|_| anyhow::anyhow!("Invalid AES-256 key length"))?;
//
//     let cipher = Aes256Gcm::new(&key);
//
//     let stream = try_stream! {
//         let file = File::open(&file_path).await?;
//         let mut reader = BufReader::new(file);
//         let mut buffer = vec![0u8; CHUNK_SIZE];
//         let mut chunk_index: u32 = 0;
//
//         loop {
//             let n = reader.read(&mut buffer).await?;
//             if n == 0 { break; }
//
//             // Construct nonce: base 8 bytes + 4-byte counter
//             let mut nonce_bytes = [0u8; 12];
//             nonce_bytes[..8].copy_from_slice(&base_nonce_bytes);
//             nonce_bytes[8..].copy_from_slice(&chunk_index.to_be_bytes());
//             let nonce = Nonce::from_slice(&nonce_bytes);
//
//             let ciphertext = cipher.encrypt(nonce, &buffer[..n])
//                 .map_err(|e| anyhow::anyhow!("AES-GCM encryption failed: {:?}", e))?;
//
//             yield Bytes::from(ciphertext);
//
//             chunk_index += 1;
//         }
//     };
//
//     Ok((stream, base_nonce_bytes.to_vec()))
// }
//
// // pub fn write_metadata(base_nonce_bytes: &[u8], path: &str) -> anyhow::Result<()> {
// //     let meta = json!({
// //         "version": 1,
// //         "cipher": "AES-256-GCM",
// //         "chunk_size": 16 * 1024 * 1024,
// //         "base_nonce_b64": general_purpose::STANDARD.encode(base_nonce_bytes),
// //         "key_info": "The file was encrypted with a shared 256-bit AES key. Keep it safe.",
// //         "notes": "Each file chunk is encrypted with AES-GCM using base_nonce + chunk counter. Store this metadata alongside the encrypted file to allow streaming decryption."
// //     });
// //
// //     let mut file = File::create(path)?;
// //     file.write_all(meta.to_string().as_bytes())?;
// //     Ok(())
// // }
// // {
// // "version": 1,
// // "cipher": "AES-256-GCM",
// // "chunk_size": 16777216,
// // "base_nonce_b64": "BASE_NONCE_HERE",
// // "key_info": "The file was encrypted with a shared 256-bit AES key. Keep it safe.",
// // "notes": "Each file chunk is encrypted with AES-GCM using base_nonce + chunk counter. Store this metadata alongside the encrypted file to allow streaming decryption."
// // }
//
// /// Decrypt a large file that was encrypted with `encrypt_file_stream_gcm`.
// /// `base_nonce_bytes` is read from the `.meta` file.
// pub async fn decrypt_file_stream_gcm(
//     encrypted_path: PathBuf,
//     decrypted_path: PathBuf,
//     master_key: [u8; 32],
//     base_nonce_bytes: &[u8],
// ) -> Result<()> {
//     // AES-256 key from slice
//     let key = Key::<Aes256Gcm>::try_from(&master_key[..])
//         .map_err(|_| anyhow::anyhow!("Invalid AES-256 key length"))?;
//     let cipher = Aes256Gcm::new(&key);
//
//     let mut reader = BufReader::new(File::open(encrypted_path).await?);
//     let mut writer = BufWriter::new(File::create(decrypted_path).await?);
//
//     let mut chunk_index: u32 = 0;
//
//     loop {
//         // Read chunk length first (u32, big-endian)
//         let mut len_buf = [0u8; 4];
//         match reader.read_exact(&mut len_buf).await {
//             Ok(_) => {},
//             Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break, // end of file
//             Err(e) => return Err(e.into()),
//         }
//         let chunk_len = u32::from_be_bytes(len_buf) as usize;
//
//         // Read ciphertext for this chunk
//         let mut chunk_ciphertext = vec![0u8; chunk_len];
//         reader.read_exact(&mut chunk_ciphertext).await?;
//
//         // Construct per-chunk nonce: base 8 bytes + counter 4 bytes
//         let mut nonce_bytes = [0u8; 12];
//         nonce_bytes[..8].copy_from_slice(&base_nonce_bytes);
//         nonce_bytes[8..].copy_from_slice(&chunk_index.to_be_bytes());
//         let nonce = Nonce::from_slice(&nonce_bytes);
//
//         // Decrypt chunk
//         let plaintext = cipher
//             .decrypt(nonce, chunk_ciphertext.as_ref())
//             .map_err(|e| anyhow::anyhow!("AES-GCM decryption failed: {:?}", e))?;
//
//         // Write decrypted chunk
//         writer.write_all(&plaintext).await?;
//
//         chunk_index += 1;
//     }
//
//     writer.flush().await?;
//     Ok(())
// }

const CHUNK_SIZE: usize = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize, Debug)]
struct FileHeader {
    version: u8,
    cipher: String,
    chunk_size: usize,
    base_nonce: Vec<u8>,
}

/// Encrypt a file with AES-256-GCM streaming, writing metadata in the file header.
/// Returns a Stream of ciphertext chunks.
// pub async fn encrypt_file_stream_gcm_with_header(
//     input_path: PathBuf,
//     output_path: PathBuf,
//     master_key_b64: String,
// ) -> Result<()> {
//     let mut rng = OsRng;
//     let mut base_nonce = [0u8; 8];
//     rng.try_fill_bytes(&mut base_nonce)?;
//
//     let master_key_bytes = general_purpose::STANDARD
//         .decode(master_key_b64)
//         .map_err(|_| anyhow::anyhow!("Invalid base64"))?;
//
//     let key = Key::<Aes256Gcm>::try_from(master_key_bytes.as_slice())
//         .map_err(|_| anyhow::anyhow!("Invalid AES-256 key length"))?;
//     let cipher = Aes256Gcm::new(&key);
//
//     let mut writer = BufWriter::new(File::create(&output_path).await?);
//
//     let header = FileHeader {
//         version: 1,
//         cipher: "AES-256-GCM".to_string(),
//         chunk_size: CHUNK_SIZE,
//         base_nonce: base_nonce.to_vec(),
//     };
//     let header_json = serde_json::to_string(&header)?;
//     writer.write_all(header_json.as_bytes()).await?;
//     writer.write_all(b"\n").await?; // newline as delimiter
//
//     let mut reader = BufReader::new(File::open(&input_path).await?);
//     let mut buffer = vec![0u8; CHUNK_SIZE];
//     let mut chunk_index: u32 = 0;
//
//     loop {
//         let n = reader.read(&mut buffer).await?;
//         if n == 0 {
//             break;
//         }
//
//         // Per-chunk nonce: base 8 bytes + 4-byte counter
//         let mut nonce_bytes = [0u8; 12];
//         nonce_bytes[..8].copy_from_slice(&base_nonce);
//         nonce_bytes[8..].copy_from_slice(&chunk_index.to_be_bytes());
//         let nonce = Nonce::from_slice(&nonce_bytes);
//
//         let ciphertext = cipher
//             .encrypt(nonce, &buffer[..n])
//             .map_err(|e| anyhow::anyhow!("AES-GCM encryption failed: {:?}", e))?;
//
//         let chunk_len = (ciphertext.len() as u32).to_be_bytes();
//         writer.write_all(&chunk_len).await?;
//         writer.write_all(&ciphertext).await?;
//
//         chunk_index += 1;
//     }
//
//     writer.flush().await?;
//     Ok(())
// }

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

        let key = Key::<Aes256Gcm>::from_slice(master_key_bytes.as_slice());
        let cipher = Aes256Gcm::new(key);

        // Send header as first chunk
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
            let nonce = Nonce::from_slice(&nonce_bytes);

            let ciphertext = cipher.encrypt(nonce, &buffer[..n]).unwrap();
            let mut out = Vec::with_capacity(4 + ciphertext.len());
            out.extend_from_slice(&(ciphertext.len() as u32).to_be_bytes());
            out.extend_from_slice(&ciphertext);

            tx.send(Ok(Bytes::from(out))).await.unwrap();
            chunk_index += 1;
        }
    });

    Ok(ReceiverStream::new(rx))
}

/// Decrypt file encrypted with `encrypt_file_stream_gcm`
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
        let nonce = Nonce::from_slice(&nonce_bytes);

        let plaintext = cipher
            .decrypt(nonce, chunk_ciphertext.as_slice())
            .map_err(|e| anyhow::anyhow!("AES-GCM decryption failed: {:?}", e))?;

        writer.write_all(&plaintext).await?;
        chunk_index += 1;
    }

    writer.flush().await?;
    Ok(())
}
