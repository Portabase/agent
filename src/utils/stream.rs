use crate::utils::file::encrypt_file_stream;
use anyhow::Result;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use rand::RngCore;
use std::pin::Pin;
use tokio_util::io::ReaderStream;

pub struct EncryptionMetadata {
    pub encrypted_aes_key: Vec<u8>,
    pub iv: [u8; 16],
}

pub struct UploadStream {
    pub stream: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    pub encryption: Option<EncryptionMetadata>,
}

pub async fn build_stream(
    file_path: &std::path::Path,
    encrypt: bool,
    public_key_pem: Option<Vec<u8>>,
) -> Result<UploadStream> {
    if encrypt {
        let public_key =
            public_key_pem.ok_or_else(|| anyhow::anyhow!("Missing public key for encryption"))?;

        let mut aes_key = [0u8; 32];
        rand::rng().fill_bytes(&mut aes_key);

        let mut iv = [0u8; 16];
        rand::rng().fill_bytes(&mut iv);

        let (encrypted_stream, encrypted_aes_key) =
            encrypt_file_stream(file_path.to_path_buf(), aes_key, iv, public_key).await?;

        let stream = Box::pin(
            encrypted_stream
                .map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))),
        );

        Ok(UploadStream {
            stream,
            encryption: Some(EncryptionMetadata {
                encrypted_aes_key,
                iv,
            }),
        })
    } else {
        let file = tokio::fs::File::open(file_path).await?;
        let reader = ReaderStream::new(file);

        let stream = Box::pin(
            reader.map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))),
        );

        Ok(UploadStream {
            stream,
            encryption: None,
        })
    }
}
