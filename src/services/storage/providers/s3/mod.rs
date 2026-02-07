mod models;

use crate::core::context::Context;
use crate::services::backup::{BackupResult, UploadResult};
use crate::services::storage::StorageProvider;
use crate::utils::common::BackupMethod;
use crate::utils::file::{EncryptionMetadataFile, full_extension, full_file_name};
use base64::{engine::general_purpose, Engine as _};
use crate::services::storage::providers::s3::models::S3ProviderConfig;
use crate::utils::stream::build_stream;
use anyhow::anyhow;
use async_trait::async_trait;
use aws_sdk_s3 as s3;
use aws_sdk_s3::config::BehaviorVersion;
use aws_sdk_s3::config::Region;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use chrono::Utc;
use futures::StreamExt;
use std::pin::Pin;
use std::sync::Arc;
use tracing::{error, info};
use crate::services::api::models::agent::status::DatabaseStorage;

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
            };
        };
        info!("{:?}", file_path);
        info!("{:?}", file_path.extension());
        info!("{:?}", file_path.file_name());

        let encrypt = encrypt.unwrap_or(false);

        let upload = build_stream(
            &file_path,
            encrypt,
            encrypt.then(|| ctx.edge_key.public_key.as_bytes().to_vec()),
        )
            .await
            .unwrap();

        let config: S3ProviderConfig = match storage.clone().config.try_into() {
            Ok(c) => c,
            Err(e) => {
                return UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some(e.to_string()),
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

        info!("Credential {:#?}", credentials);

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


        let file_name = full_file_name(&file_path, encrypt);

        info!("Uploading file {}", file_name);

        let bucket = &config.bucket_name;
        let key = format!(
            "backups/{}/{}",
            Utc::now().format("%Y-%m-%d"),
            file_name
        );

        info!("S3 key {:}", key);
        info!("Starting multipart upload to s3://{}/{}", bucket, key);

        let create_resp = match client
            .create_multipart_upload()
            .bucket(bucket)
            .key(&key)
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
                        .key(&key)
                        .upload_id(&upload_id)
                        .send()
                        .await;
                    return UploadResult {
                        storage_id: storage.id.clone(),
                        success: false,
                        error: Some(format!("Stream error: {}", e)),
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
                    .key(&key)
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
                            .key(&key)
                            .upload_id(&upload_id)
                            .send()
                            .await;
                        return UploadResult {
                            storage_id: storage.id.clone(),
                            success: false,
                            error: Some(e.to_string()),
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
                .key(&key)
                .upload_id(&upload_id)
                .send()
                .await;
            return UploadResult {
                storage_id: storage.id.clone(),
                success: false,
                error: Some("No parts were uploaded".to_string()),
            };
        }

        let completed = CompletedMultipartUpload::builder()
            .set_parts(Some(parts))
            .build();

        match client
            .complete_multipart_upload()
            .bucket(bucket)
            .key(&key)
            .upload_id(&upload_id)
            .multipart_upload(completed)
            .send()
            .await
        {
            Ok(_) => {
                info!("Successfully completed multipart upload: {}", key);

                if let Some(enc) = upload.encryption {
                    let meta = EncryptionMetadataFile {
                        version: 1,
                        cipher: "AES-256-CBC+RSA-OAEP-SHA256".to_string(),
                        encrypted_aes_key_b64: general_purpose::STANDARD.encode(enc.encrypted_aes_key),
                        iv_b64: general_purpose::STANDARD.encode(enc.iv),
                    };

                    let meta_toml = toml::to_string(&meta).expect("Serialization error");

                    let meta_key = format!("{}.meta", key);

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
                }
            }
            Err(e) => {
                error!("Failed to complete multipart upload: {}", e);
                let _ = client
                    .abort_multipart_upload()
                    .bucket(bucket)
                    .key(&key)
                    .upload_id(&upload_id)
                    .send()
                    .await;
                UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some(e.to_string()),
                }
            }
        }
    }
}
