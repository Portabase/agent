use crate::core::context::Context;
use crate::services::backup::{BackupResult, UploadResult};
use crate::services::providers::StorageProvider;
use crate::services::status::DatabaseStorage;
use crate::utils::common::BackupMethod;
use crate::utils::file::{encrypt_file_stream, full_extension};
use crate::utils::tus::upload_to_tus_stream_with_headers;
use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use hex;
use rand::RngCore;
use reqwest::header::{HeaderMap, HeaderValue};
use std::pin::Pin;
use std::sync::Arc;
use tracing::{error, info};

pub struct TusProvider;

#[async_trait]
impl StorageProvider for TusProvider {
    async fn upload(
        &self,
        ctx: Arc<Context>,
        result: BackupResult,
        method: BackupMethod,
        storage: &DatabaseStorage,
    ) -> UploadResult {
        let Some(file_path) = result.backup_file else {
            return UploadResult { storage_id: storage.id.clone(), success: false, error: Some("File path error".to_string()) } ;
        };

        let mut aes_key = [0u8; 32];
        rand::rng().fill_bytes(&mut aes_key);
        let mut iv = [0u8; 16];
        rand::rng().fill_bytes(&mut iv);

        let public_key_pem = ctx.edge_key.public_key.as_bytes().to_vec();

        let (encrypted_stream, encrypted_key_hex) =
            match encrypt_file_stream(file_path.clone(), aes_key, iv, public_key_pem).await {
                Ok(v) => v,
                Err(e) => {
                    error!("Encryption failed: {}", e);
                    return UploadResult { storage_id: storage.id.clone(), success: false, error: Some(e.to_string()) } ;
                }
            };

        let stream: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> = Box::pin(
            encrypted_stream
                .map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))),
        );

        let mut extra_headers = HeaderMap::new();
        extra_headers.insert("X-AES-Key", HeaderValue::from_str(&encrypted_key_hex).unwrap());
        extra_headers.insert("X-IV", HeaderValue::from_str(&hex::encode(iv)).unwrap());
        extra_headers.insert(
            "X-Generated-Id",
            HeaderValue::from_str(&result.generated_id).unwrap(),
        );
        extra_headers.insert("X-Status", HeaderValue::from_str(&result.status).unwrap());
        extra_headers.insert("X-Method", HeaderValue::from_str(&method.to_string()).unwrap());
        extra_headers.insert(
            "X-Extension",
            HeaderValue::from_str(&full_extension(&file_path)).unwrap(),
        );

        let tus_endpoint = format!("{}/tus/files", ctx.edge_key.server_url);

        // upload_to_tus_stream_with_headers(stream, &tus_endpoint, extra_headers).await?;
        // Ok(())

        match upload_to_tus_stream_with_headers(stream, &tus_endpoint, extra_headers).await {
            Ok(_) => UploadResult { storage_id: storage.id.clone(), success: true, error: None },
            Err(e) => {
                error!("TUS upload failed: {}", e);
                UploadResult { storage_id: storage.id.clone(), success: false, error: Some(e.to_string()) }
            }
        }
    }
}
