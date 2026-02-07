use std::path::{Path, PathBuf};
use anyhow::{anyhow, Result};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, BufReader};
use futures::{Stream};
use bytes::Bytes;
use openssl::symm::{Cipher, Crypter, Mode};
use openssl::pkey::PKey;
use openssl::encrypt::Encrypter;
use openssl::rsa::Padding;
use openssl::hash::MessageDigest;
use async_stream::try_stream;
use serde::{Deserialize, Serialize};


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

pub fn full_file_name(path: &Path, encrypt: bool) -> String {
        let base_name = path
            .file_name()
            .and_then(|e| e.to_str())
            .ok_or_else(|| anyhow!("Missing or invalid file name")).unwrap();
        if encrypt {
            format!("{}.enc", base_name)
        } else {
            base_name.to_string()
        }
}



pub async fn encrypt_file_stream(
    file_path: PathBuf,
    aes_key: [u8; 32],
    iv: [u8; 16],
    pub_key_pem: Vec<u8>,
) -> Result<(impl Stream<Item = Result<Bytes>> + Send + 'static, Vec<u8>)> {
    // ---------- Encrypt AES key with RSA ----------
    let pkey = PKey::public_key_from_pem(&pub_key_pem)?;
    let mut rsa = Encrypter::new(&pkey)?;
    rsa.set_rsa_padding(Padding::PKCS1_OAEP)?;
    rsa.set_rsa_oaep_md(MessageDigest::sha256())?;
    rsa.set_rsa_mgf1_md(MessageDigest::sha256())?;

    let mut encrypted_key = vec![0u8; rsa.encrypt_len(&aes_key)?];
    let len = rsa.encrypt(&aes_key, &mut encrypted_key)?;
    encrypted_key.truncate(len);

    // ---------- Streaming AES encryption ----------
    let stream = try_stream! {
        let file = File::open(&file_path).await?;
        let mut reader = BufReader::new(file);

        let cipher = Cipher::aes_256_cbc();
        let mut crypter = Crypter::new(cipher, Mode::Encrypt, &aes_key, Some(&iv))?;
        crypter.pad(true);

        let mut buffer = vec![0u8; 1024 * 1024];

        loop {
            let n = reader.read(&mut buffer).await?;
            if n == 0 {
                break;
            }

            let mut out = vec![0u8; n + cipher.block_size()];
            let count = crypter.update(&buffer[..n], &mut out)?;
            out.truncate(count);

            yield Bytes::from(out);
        }

        let mut final_block = vec![0u8; cipher.block_size()];
        let rest = crypter.finalize(&mut final_block)?;
        final_block.truncate(rest);

        if !final_block.is_empty() {
            yield Bytes::from(final_block);
        }
    };

    Ok((stream, encrypted_key))
}

