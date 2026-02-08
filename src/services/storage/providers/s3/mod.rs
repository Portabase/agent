mod models;

use crate::core::context::Context;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::{BackupResult, UploadResult};
use crate::services::storage::StorageProvider;
use crate::services::storage::providers::s3::models::S3ProviderConfig;
use crate::utils::common::BackupMethod;
use crate::utils::file::{EncryptionMetadataFile, full_file_name, full_file_path};
use crate::utils::stream::build_stream;
use async_trait::async_trait;
use aws_sdk_s3 as s3;
use aws_sdk_s3::config::BehaviorVersion;
use aws_sdk_s3::config::Region;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use base64::{Engine as _, engine::general_purpose};
use futures::StreamExt;
use std::pin::Pin;
use std::sync::Arc;
use tokio::fs;
use tracing::{error, info};

pub struct S3Provider {}

#[async_trait]
impl StorageProvider for S3Provider {
    async fn upload(
        &self,
        ctx: Arc<Context>,
        result: BackupResult,
        _method: BackupMethod,
        storage: &DatabaseStorage,
        encrypt: Option<bool>,
    ) -> UploadResult {
        let Some(file_path) = result.backup_file else {
            return UploadResult {
                storage_id: storage.id.clone(),
                success: false,
                error: Some("Missing backup file path".to_string()),
                remote_file_path: None,
                total_size: None,
            };
        };

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

        let encrypt = encrypt.unwrap_or(false);

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
                    total_size: None,
                };
            }
        };

        let config: S3ProviderConfig = match storage.clone().config.try_into() {
            Ok(c) => c,
            Err(e) => {
                return UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some(e.to_string()),
                    remote_file_path: None,
                    total_size: None,
                };
            }
        };

        let credentials = s3::config::Credentials::new(
            config.access_key.clone(),
            config.secret_key.clone(),
            None,
            None,
            "static-creds",
        );

        let region = Region::new(config.region.clone().unwrap_or("eu-central-3".to_string()));

        let sdk_config = s3::config::Builder::new()
            .credentials_provider(credentials)
            .region(region)
            .force_path_style(true)
            .endpoint_url(format!(
                "{}://{}",
                if config.ssl { "https" } else { "http" },
                config.end_point_url
            ))
            .behavior_version(BehaviorVersion::latest())
            .build();

        let client = s3::Client::from_conf(sdk_config);

        const PART_SIZE: usize = 100 * 1024 * 1024; // 100 MiB

        let file_name = full_file_name(encrypt);

        info!("Uploading file {}", file_name);

        let bucket = &config.bucket_name;
        let remote_file_path = full_file_path(&file_name);
        info!("S3 key {:}", remote_file_path);
        info!(
            "Starting multipart upload to s3://{}/{}",
            bucket, remote_file_path
        );

        let create_resp = match client
            .create_multipart_upload()
            .bucket(bucket)
            .key(&remote_file_path)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to create multipart upload: {}", e);
                return UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some(e.to_string()),
                    remote_file_path: None,
                    total_size: None,
                };
            }
        };

        let upload_id = match create_resp.upload_id {
            Some(id) => id,
            None => {
                return UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some("No upload ID returned".to_string()),
                    remote_file_path: None,
                    total_size: None,
                };
            }
        };

        let mut parts: Vec<CompletedPart> = Vec::new();
        let mut part_number: i32 = 1;
        let mut buffer: Vec<u8> = Vec::with_capacity(PART_SIZE);

        let mut peekable = upload.stream.peekable();

        while let Some(item) = peekable.next().await {
            let bytes = match item {
                Ok(b) => b,
                Err(e) => {
                    error!("Stream error during upload: {}", e);
                    let _ = client
                        .abort_multipart_upload()
                        .bucket(bucket)
                        .key(&remote_file_path)
                        .upload_id(&upload_id)
                        .send()
                        .await;
                    return UploadResult {
                        storage_id: storage.id.clone(),
                        success: false,
                        error: Some(format!("Stream error: {}", e)),
                        remote_file_path: None,
                        total_size: None,
                    };
                }
            };

            buffer.extend_from_slice(&bytes);

            let is_last = {
                let pinned = Pin::new(&mut peekable);
                let peek_future = pinned.peek();
                peek_future.await.is_none()
            };

            let should_upload = buffer.len() >= PART_SIZE || is_last;

            if should_upload && !buffer.is_empty() {
                let body = ByteStream::from(buffer.clone());

                match client
                    .upload_part()
                    .bucket(bucket)
                    .key(&remote_file_path)
                    .upload_id(&upload_id)
                    .part_number(part_number)
                    .body(body)
                    .send()
                    .await
                {
                    Ok(resp) => {
                        if let Some(etag) = resp.e_tag {
                            parts.push(
                                CompletedPart::builder()
                                    .part_number(part_number)
                                    .e_tag(etag)
                                    .build(),
                            );
                            info!("Uploaded part {} ({} bytes)", part_number, buffer.len());
                        }
                    }
                    Err(e) => {
                        error!("Failed to upload part {}: {}", part_number, e);
                        let _ = client
                            .abort_multipart_upload()
                            .bucket(bucket)
                            .key(&remote_file_path)
                            .upload_id(&upload_id)
                            .send()
                            .await;
                        return UploadResult {
                            storage_id: storage.id.clone(),
                            success: false,
                            error: Some(e.to_string()),
                            remote_file_path: None,
                            total_size: None,
                        };
                    }
                }
                buffer.clear();
                part_number += 1;
            }
        }

        if !buffer.is_empty() {
            let _ = client
                .abort_multipart_upload()
                .bucket(bucket)
                .key(&remote_file_path)
                .upload_id(&upload_id)
                .send()
                .await;
            return UploadResult {
                storage_id: storage.id.clone(),
                success: false,
                error: Some("No parts were uploaded".to_string()),
                remote_file_path: None,
                total_size: None,
            };
        }

        let completed = CompletedMultipartUpload::builder()
            .set_parts(Some(parts))
            .build();

        match client
            .complete_multipart_upload()
            .bucket(bucket)
            .key(&remote_file_path)
            .upload_id(&upload_id)
            .multipart_upload(completed)
            .send()
            .await
        {
            Ok(_) => {
                info!(
                    "Successfully completed multipart upload: {}",
                    remote_file_path
                );

                if let Some(enc) = upload.encryption {
                    let meta = EncryptionMetadataFile {
                        version: 1,
                        cipher: "AES-256-CBC+RSA-OAEP-SHA256".to_string(),
                        encrypted_aes_key_b64: general_purpose::STANDARD
                            .encode(enc.encrypted_aes_key),
                        iv_b64: general_purpose::STANDARD.encode(enc.iv),
                    };

                    let meta_toml = toml::to_string(&meta).expect("Serialization error");

                    let meta_key = format!("{}.meta", remote_file_path);

                    client
                        .put_object()
                        .bucket(bucket)
                        .key(&meta_key)
                        .body(ByteStream::from(meta_toml.into_bytes()))
                        .content_type("application/toml")
                        .send()
                        .await
                        .map_err(|e| {
                            error!("Metadata upload failed: {}", e);
                            e
                        })
                        .unwrap();
                }

                UploadResult {
                    storage_id: storage.id.clone(),
                    success: true,
                    error: None,
                    remote_file_path: Some(remote_file_path),
                    total_size: Some(total_size),
                }
            }
            Err(e) => {
                error!("Failed to complete multipart upload: {}", e);
                let _ = client
                    .abort_multipart_upload()
                    .bucket(bucket)
                    .key(&remote_file_path)
                    .upload_id(&upload_id)
                    .send()
                    .await;
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
