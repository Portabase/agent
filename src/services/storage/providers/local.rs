use crate::core::context::Context;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::{BackupResult, UploadResult};
use crate::services::storage::StorageProvider;
use crate::utils::common::BackupMethod;
use crate::utils::file::{full_file_name, full_file_path};
use crate::utils::stream::build_stream;
use crate::utils::tus::upload_to_tus_stream_with_headers;
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose;
use reqwest::header::{HeaderMap, HeaderValue};
use std::sync::Arc;
use tokio::fs;
use tracing::error;

pub struct LocalProvider;

#[async_trait]
impl StorageProvider for LocalProvider {
    async fn upload(
        &self,
        ctx: Arc<Context>,
        result: BackupResult,
        method: BackupMethod,
        storage: &DatabaseStorage,
        encrypt: Option<bool>,
    ) -> UploadResult {
        let Some(file_path) = result.backup_file else {
            return UploadResult {
                storage_id: storage.id.clone(),
                success: false,
                error: Some("File path error".to_string()),
                remote_file_path: None,
                total_size: None,
            };
        };

        let encrypt = encrypt.unwrap_or(false);

        let file_name = full_file_name(encrypt);
        let remote_file_path = full_file_path(&file_name);

        let total_size = match fs::metadata(&file_path).await {
            Ok(meta) => meta.len(),
            Err(e) => {
                error!("Failed to get file size: {}", e);
                return UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some(e.to_string()),
                    remote_file_path: None,
                    total_size: None,
                };
            }
        };

        let upload = match build_stream(
            &file_path,
            encrypt,
            encrypt.then(|| ctx.edge_key.public_key.as_bytes().to_vec()),
        )
        .await
        {
            Ok(u) => u,
            Err(e) => {
                error!("Stream build failed: {}", e);
                return UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some(e.to_string()),
                    remote_file_path: None,
                    total_size: None
                };
            }
        };

        let mut extra_headers = HeaderMap::new();

        extra_headers.insert("X-File-Name", HeaderValue::from_str(&file_name).unwrap());
        extra_headers.insert("X-File-Size", HeaderValue::from_str(&total_size.to_string()).unwrap());
        extra_headers.insert(
            "X-File-Path",
            HeaderValue::from_str(&remote_file_path).unwrap(),
        );
        extra_headers.insert(
            "X-Generated-Id",
            HeaderValue::from_str(&result.generated_id).unwrap(),
        );
        extra_headers.insert("X-Status", HeaderValue::from_str(&result.status).unwrap());
        extra_headers.insert(
            "X-Method",
            HeaderValue::from_str(&method.to_string()).unwrap(),
        );

        if let Some(enc) = upload.encryption {
            let mut meta_pairs = Vec::new();

            meta_pairs.push(format!("version {}", "1"));
            meta_pairs.push(format!("cipher {}", "AES-256-CBC+RSA-OAEP-SHA256"));
            meta_pairs.push(format!(
                "encrypted_aes_key_b64 {}",
                general_purpose::STANDARD.encode(&enc.encrypted_aes_key)
            ));
            meta_pairs.push(format!(
                "iv_b64 {}",
                general_purpose::STANDARD.encode(&enc.iv)
            ));

            let metadata_header_value = meta_pairs.join(",");

            extra_headers.insert(
                "Upload-Metadata",
                HeaderValue::from_str(&*general_purpose::STANDARD.encode(metadata_header_value))
                    .unwrap(),
            );
        }

        let tus_endpoint = format!("{}/tus/files", ctx.edge_key.server_url);

        match upload_to_tus_stream_with_headers(upload.stream, &tus_endpoint, extra_headers, total_size).await {
            Ok(_) => UploadResult {
                storage_id: storage.id.clone(),
                success: true,
                error: None,
                remote_file_path: Some(remote_file_path),
                total_size: Some(total_size),
            },
            Err(e) => {
                error!("Local upload failed: {}", e);
                UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some(e.to_string()),
                    remote_file_path: None,
                    total_size: None,
                }
            }
        }
    }
}
