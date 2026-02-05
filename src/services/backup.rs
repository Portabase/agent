#![allow(dead_code)]

use crate::core::context::Context;
use crate::domain::factory::DatabaseFactory;
use crate::services::config::{DatabaseConfig, DatabasesConfig, DbType};
use crate::services::status::DatabaseStorage;
use crate::services::storage;
use crate::utils::common::BackupMethod;
use crate::utils::file::{encrypt_file_stream, full_extension};
use crate::utils::tus::upload_to_tus_stream_with_headers;
use anyhow::Result;
use bytes::Bytes;
use futures::future::join_all;
use futures::{Stream, StreamExt};
use hex;
use rand::RngCore;
use reqwest::header::{HeaderMap, HeaderValue};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tempfile::TempDir;
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct BackupResult {
    pub generated_id: String,
    pub db_type: DbType,
    pub status: String,
    pub backup_file: Option<PathBuf>,
    pub code: Option<String>,
}

#[derive(Debug)]
pub struct UploadResult {
    pub storage_id: String,
    pub success: bool,
    pub error: Option<String>,
}

pub struct BackupService {
    ctx: Arc<Context>,
}

impl BackupService {
    pub fn new(ctx: Arc<Context>) -> Self {
        Self { ctx }
    }

    pub async fn dispatch(
        &self,
        generated_id: &String,
        config: &DatabasesConfig,
        method: BackupMethod,
        storages: &Vec<DatabaseStorage>,
    ) {
        if let Some(cfg) = config
            .databases
            .iter()
            .find(|c| c.generated_id == generated_id.as_str())
        {
            let db_cfg = cfg.clone();
            let ctx_clone = self.ctx.clone();
            let storages_clone = storages.clone();

            tokio::spawn(async move {
                match TempDir::new() {
                    Ok(temp_dir) => {
                        let tmp_path = temp_dir.path().to_path_buf();
                        info!("Created temp directory {}", tmp_path.display());

                        match BackupService::run(db_cfg, &tmp_path).await {
                            Ok(result) => {
                                let service = BackupService { ctx: ctx_clone };
                                service.send_result(result, method, storages_clone).await;
                            }
                            Err(e) => error!("Backup error {}", e),
                        }
                        // TempDir is automatically deleted when dropped here
                    }
                    Err(e) => error!("Failed to create temp dir: {}", e),
                }
            });
        }
    }

    pub async fn run(cfg: DatabaseConfig, tmp_path: &Path) -> Result<BackupResult> {
        let db_instance = DatabaseFactory::create_for_backup(cfg.clone()).await;
        let generated_id = cfg.generated_id.clone();
        let db_type = cfg.db_type.clone();

        let reachable = db_instance.ping().await.unwrap_or(false);
        info!("Reachable: {}", reachable);
        if !reachable {
            return Ok(BackupResult {
                generated_id,
                db_type,
                status: "failed".into(),
                backup_file: None,
                code: None,
            });
        }

        match db_instance.backup(tmp_path).await {
            Ok(file) => Ok(BackupResult {
                generated_id,
                db_type,
                status: "success".into(),
                backup_file: Some(file),
                code: None,
            }),
            Err(e) => match e.to_string().as_str() {
                "backup_already_in_progress" => Ok(BackupResult {
                    generated_id,
                    db_type,
                    status: "failed".into(),
                    backup_file: None,
                    code: Some(e.to_string()),
                }),
                _ => Ok(BackupResult {
                    generated_id,
                    db_type,
                    status: "failed".into(),
                    backup_file: None,
                    code: None,
                }),
            },
        }
    }

    // pub async fn send_result(&self, result: BackupResult, method: BackupMethod) {
    //     if result.code.as_deref() == Some("backup_already_in_progress") {
    //         info!("Skipping send: backup already in progress");
    //         return;
    //     }
    //
    //     let Some(file_path) = result.backup_file else {
    //         return;
    //     };
    //
    //     let mut aes_key = [0u8; 32];
    //     rand::rng().fill_bytes(&mut aes_key);
    //
    //     let mut iv = [0u8; 16];
    //     rand::rng().fill_bytes(&mut iv);
    //
    //     let public_key_pem = self.ctx.edge_key.public_key.as_bytes().to_vec();
    //
    //     let (encrypted_stream, encrypted_key_hex) =
    //         match encrypt_file_stream(file_path.clone(), aes_key, iv, public_key_pem).await {
    //             Ok(v) => v,
    //             Err(e) => {
    //                 error!("Encryption failed: {}", e);
    //                 return;
    //             }
    //         };
    //
    //     let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
    //
    //     let body = Body::wrap_stream(
    //         encrypted_stream
    //             .map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))),
    //     );
    //
    //     let url = format!(
    //         "{}/services/v1/upload/{}",
    //         self.ctx.edge_key.server_url, self.ctx.edge_key.agent_id
    //     );
    //
    //     let client = reqwest::Client::new();
    //
    //     info!("file Size to {}", file_size);
    //
    //     let resp = client
    //         .post(&url)
    //         .header("X-Generated-Id", &result.generated_id)
    //         .header("X-Status", &result.status)
    //         .header("X-Method", method.to_string())
    //         .header("X-AES-Key", encrypted_key_hex)
    //         .header("X-IV", hex::encode(iv))
    //         .header("X-Extension", full_extension(&file_path))
    //         .header("Transfer-Encoding", "chunked")
    //         .header("X-File-Size", file_size)
    //         .body(body)
    //         .send()
    //         .await;
    //
    //     match resp {
    //         Ok(r) if r.status().is_success() => {
    //             info!("Backup uploaded successfully");
    //         }
    //         Ok(r) => {
    //             error!("Upload failed: {}", r.status());
    //         }
    //         Err(e) => {
    //             error!("Upload error: {}", e);
    //         }
    //     }
    // }

    //
    // pub async fn send_result(&self, result: BackupResult, method: BackupMethod) {
    //     // Skip if backup already in progress
    //     if result.code.as_deref() == Some("backup_already_in_progress") {
    //         info!("Skipping send: backup already in progress");
    //         return;
    //     }
    //
    //     let Some(file_path) = result.backup_file else { return; };
    //
    //     // Generate AES key and IV
    //     let mut aes_key = [0u8; 32];
    //     rand::rng().fill_bytes(&mut aes_key);
    //     let mut iv = [0u8; 16];
    //     rand::rng().fill_bytes(&mut iv);
    //
    //     let public_key_pem = self.ctx.edge_key.public_key.as_bytes().to_vec();
    //
    //     // Encrypt the file as a streaming source
    //     let (encrypted_stream, encrypted_key_hex) =
    //         match encrypt_file_stream(file_path.clone(), aes_key, iv, public_key_pem).await {
    //             Ok(v) => v,
    //             Err(e) => {
    //                 error!("Encryption failed: {}", e);
    //                 return;
    //             }
    //         };
    //
    //     // Get file size
    //     let file_size = match std::fs::metadata(&file_path) {
    //         Ok(m) => m.len(),
    //         Err(e) => {
    //             error!("Cannot get file metadata: {}", e);
    //             return;
    //         }
    //     };
    //
    //     // TUS endpoint
    //     // let tus_endpoint = format!(
    //     //     // "{}/services/v1/upload/{}",
    //     //     // "{}/tus/files",
    //     //     "http://localhost:1080/files",
    //     //     // self.ctx.edge_key.server_url,
    //     //     // self.ctx.edge_key.agent_id
    //     // );
    //
    //     let tus_endpoint = String::from("http://localhost:1080/files");
    //
    //     // Wrap the stream to handle errors
    //     let stream: Pin<Box<dyn futures::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>> =
    //         Box::pin(encrypted_stream.map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))));
    //
    //     // Build additional headers for TUS PATCH requests
    //     let extra_headers = {
    //         let mut h = HeaderMap::new();
    //         h.insert("X-AES-Key", HeaderValue::from_str(&encrypted_key_hex).unwrap());
    //         h.insert("X-IV", HeaderValue::from_str(&hex::encode(iv)).unwrap());
    //         h.insert("X-Generated-Id", HeaderValue::from_str(&result.generated_id).unwrap());
    //         h.insert("X-Status", HeaderValue::from_str(&result.status).unwrap());
    //         h.insert("X-Method", HeaderValue::from_str(&method.to_string()).unwrap());
    //         h.insert("X-Extension", HeaderValue::from_str(&full_extension(&file_path)).unwrap());
    //         // h.insert("X-File-Size", HeaderValue::from_str(&file_size.to_string()).unwrap());
    //         h
    //     };
    //
    //     match upload_to_tus_stream_with_headers(stream, file_size, &tus_endpoint, extra_headers).await {
    //         Ok(_) => info!("Backup uploaded successfully via TUS"),
    //         Err(e) => error!("TUS upload failed: {}", e),
    //     }
    // }

    pub async fn send_result(
        &self,
        result: BackupResult,
        method: BackupMethod,
        storages: Vec<DatabaseStorage>,
    ) {
        if result.code.as_deref() == Some("backup_already_in_progress") {
            info!("Skipping send: backup already in progress");
            return;
        }

        info!("Sending result: {:#?}", storages);

        let upload_futures = storages.into_iter().map(|storage| {
            info!(
                "Uploading storage -> {:?} for {:?}",
                storage.provider, storage.id
            );
            let provider = storage::get_provider(&storage);
            let result_clone = result.clone();
            let ctx_clone = self.ctx.clone();

            async move {
                match provider {
                    Some(provider) => {
                        provider
                            .upload(ctx_clone, result_clone, method, &storage)
                            .await
                    }
                    None => {
                        error!("Skipping storage due to missing provider");
                        UploadResult {
                            storage_id: storage.id.clone(),
                            success: false,
                            error: Some("Skipping storage due to missing provider".to_string()),
                        }
                    }
                }
            }
        });

        let results: Vec<UploadResult> = join_all(upload_futures).await;
        info!("Upload results: {:#?}", results);
        return;

        //
        //
        // let Some(file_path) = result.backup_file else { return; };
        //
        // let mut aes_key = [0u8; 32];
        // rand::rng().fill_bytes(&mut aes_key);
        // let mut iv = [0u8; 16];
        // rand::rng().fill_bytes(&mut iv);
        //
        // let public_key_pem = self.ctx.edge_key.public_key.as_bytes().to_vec();
        //
        // let (encrypted_stream, encrypted_key_hex) =
        //     match encrypt_file_stream(file_path.clone(), aes_key, iv, public_key_pem).await {
        //         Ok(v) => v,
        //         Err(e) => {
        //             error!("Encryption failed: {}", e);
        //             return;
        //         }
        //     };
        //
        // // let tus_endpoint = format!("{}/files", self.ctx.edge_key.server_url);
        //
        // // Loop over all storages (local : tus, s3, etc.)
        //
        // let tus_endpoint = format!(
        //         // "{}/services/v1/upload/{}",
        //         "{}/tus/files",
        //         self.ctx.edge_key.server_url,
        //         // self.ctx.edge_key.agent_id
        //     );
        //
        // // let tus_endpoint = String::from("http://localhost:1080/files");
        // // let tus_endpoint = String::from("http://localhost:1080/files");
        // let stream: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> =
        //     Box::pin(encrypted_stream.map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))));
        //
        // let mut extra_headers = HeaderMap::new();
        // extra_headers.insert("X-AES-Key", HeaderValue::from_str(&encrypted_key_hex).unwrap());
        // extra_headers.insert("X-IV", HeaderValue::from_str(&hex::encode(iv)).unwrap());
        // extra_headers.insert("X-Generated-Id", HeaderValue::from_str(&result.generated_id).unwrap());
        // extra_headers.insert("X-Status", HeaderValue::from_str(&result.status).unwrap());
        // extra_headers.insert("X-Method", HeaderValue::from_str(&method.to_string()).unwrap());
        // extra_headers.insert("X-Extension", HeaderValue::from_str(&full_extension(&file_path)).unwrap());
        //
        //
        // match upload_to_tus_stream_with_headers(stream, &tus_endpoint, extra_headers).await {
        //     Ok(_) => info!("Backup uploaded successfully via TUS"),
        //     Err(e) => error!("TUS upload failed: {}", e),
        // }
    }
}
