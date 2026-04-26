use crate::utils::file::encrypt_file_stream_gcm;
use anyhow::Result;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use std::pin::Pin;
use tokio::io::{AsyncReadExt};
use crate::settings::CONFIG;

pub struct UploadStream {
    pub stream: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
}



pub async fn build_stream(
    file_path: &std::path::Path,
    encrypt: bool,
    master_key_b64: &String,
) -> Result<UploadStream> {
    if encrypt {
        let encrypted_stream =
            encrypt_file_stream_gcm(file_path.to_path_buf(), master_key_b64.to_string()).await?;

        let stream = Box::pin(
            encrypted_stream
                .map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))),
        );

        Ok(UploadStream { stream })
    } else {
        let mut file = tokio::fs::File::open(file_path).await?;

        let stream = async_stream::stream! {
            let mut buffer = vec![0u8; CONFIG.chunk_size];

            loop {
                let n = match file.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => {
                        yield Err(e);
                        break;
                    }
                };

                yield Ok(Bytes::copy_from_slice(&buffer[..n]));
            }
        };

        Ok(UploadStream {
            stream: Box::pin(stream),
        })
    }
}
