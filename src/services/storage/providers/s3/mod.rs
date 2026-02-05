mod models;

use crate::core::context::Context;
use crate::services::backup::{BackupResult, UploadResult};
use crate::services::status::DatabaseStorage;
use crate::services::storage::StorageProvider;
use crate::utils::common::BackupMethod;
use crate::utils::file::{encrypt_file_stream, full_extension};

use anyhow::Result;
use async_trait::async_trait;
use aws_sdk_s3 as s3;
use aws_sdk_s3::config::Region;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use aws_sdk_s3::primitives::ByteStream;
use aws_config::{meta::region::RegionProviderChain};
use bytes::Bytes;
use chrono::Utc;
use futures::{Stream, StreamExt};
use hex;
use rand::RngCore;
use std::pin::Pin;
use std::sync::Arc;
use aws_sdk_s3::config::endpoint::Endpoint;
use tracing::{error, info};
use uuid::{Uuid};
use crate::services::storage::providers::s3::models::S3ProviderConfig;
use aws_sdk_s3::config::BehaviorVersion;



pub struct S3Provider {}

#[async_trait]
impl StorageProvider for S3Provider {
    async fn upload(
        &self,
        ctx: Arc<Context>,
        result: BackupResult,
        _method: BackupMethod,
        storage: &DatabaseStorage,
    ) -> UploadResult {


        let Some(file_path) = result.backup_file else {
            return UploadResult {
                storage_id: storage.id.clone(),
                success: false,
                error: Some("Missing backup file path".to_string()),
            };
        };

        // Generate encryption materials
        let mut aes_key = [0u8; 32];
        rand::rng().fill_bytes(&mut aes_key);
        let mut iv = [0u8; 16];
        rand::rng().fill_bytes(&mut iv);


        let public_key_pem = ctx.edge_key.public_key.as_bytes().to_vec();

        let (encrypted_stream, encrypted_key_hex) = match encrypt_file_stream(
            file_path.clone(),
            aes_key,
            iv,
            public_key_pem,
        )
            .await
        {
            Ok(v) => v,
            Err(e) => {
                error!("Encryption failed: {}", e);
                return UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some(format!("Encryption failed: {}", e)),
                };
            }
        };

        let stream: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> = Box::pin(
            encrypted_stream.map(|r| {
                r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
            }),
        );


        // --- Configuration S3 ---
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



        // Build region provider chain
        let region_provider = if let Some(r) = config.region.as_ref() {
            RegionProviderChain::default_provider().or_else(Region::new(r.clone()))
        } else {
            RegionProviderChain::default_provider()
        };

        // Static credentials (suitable for MinIO, custom S3, etc.)
        let credentials = s3::config::Credentials::new(
            config.access_key.clone(),
            config.secret_key.clone(),
            None,
            None,
            "static-creds",
        );

        let region = Region::new(config.region.clone().unwrap_or("eu-central-3".to_string()));

        info!("Credential {:#?}", credentials);


        let sdk_config =s3::config::Builder::new()
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




        // ────────────────────────────────────────────────────────────────
        //                     MULTIPART UPLOAD LOGIC
        // ────────────────────────────────────────────────────────────────

        const PART_SIZE: usize = 100 * 1024 * 1024; // 100 MiB

        let bucket = &config.bucket_name;
        let key = format!(
            "backups/{}/{}.enc",
            Utc::now().format("%Y-%m-%d"),
            Uuid::new_v4().to_string()
        );

        info!("Starting multipart upload to s3://{}/{}", bucket, key);

        let create_resp = match
        client
            .create_multipart_upload()
            .bucket(bucket)
            .key(&key)
            .metadata("x-encrypted-key", encrypted_key_hex)
            .metadata("x-encryption-iv", hex::encode(iv))
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

        // Make the stream peekable **once**
        let mut peekable = stream.peekable();

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