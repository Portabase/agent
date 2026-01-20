#![allow(dead_code)]

use crate::core::context::Context;
use crate::domain::factory::DatabaseFactory;
use crate::services::config::{DatabaseConfig, DatabasesConfig, DbType};
use crate::utils::common::BackupMethod;
use crate::utils::file::full_extension;
use anyhow::Result;
use hex;
use openssl::encrypt::Encrypter;
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::rand::rand_bytes;
use openssl::rsa::Padding;
use openssl::symm::{Cipher, Crypter, Mode};
use reqwest::multipart::{Form, Part};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::fs;
use tracing::{error, info};

#[derive(Debug)]
pub struct BackupResult {
    pub generated_id: String,
    pub db_type: DbType,
    pub status: String,
    pub backup_file: Option<PathBuf>,
    pub code: Option<String>,
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
    ) {
        if let Some(cfg) = config
            .databases
            .iter()
            .find(|c| c.generated_id == generated_id.as_str())
        {
            let db_cfg = cfg.clone();
            let ctx_clone = self.ctx.clone();

            tokio::spawn(async move {
                match TempDir::new() {
                    Ok(temp_dir) => {
                        let tmp_path = temp_dir.path().to_path_buf();
                        info!("Created temp directory {}", tmp_path.display());

                        match BackupService::run(db_cfg, &tmp_path).await {
                            Ok(result) => {
                                let service = BackupService { ctx: ctx_clone };
                                service.send_result(result, method).await;
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

    pub async fn send_result(&self, result: BackupResult, method: BackupMethod) {
        if result.code.as_deref() == Some("backup_already_in_progress") {
            info!(
                "[BackupService] Skipping send for DB {}: backup already in progress",
                result.generated_id
            );
            return;
        }

        info!(
            "[BackupService] DB: {} Type: {} Status: {} File: {:?}",
            result.generated_id,
            result.db_type.as_str(),
            result.status,
            result.backup_file
        );

        let client = reqwest::Client::new();
        let url = format!(
            "{}/api/agent/{}/backup",
            self.ctx.edge_key.server_url, self.ctx.edge_key.agent_id
        );

        let mut form = Form::new()
            .text("generatedId", result.generated_id.clone())
            .text("status", result.status.clone())
            .text("method", method.to_string());

        if let Some(file_path) = result.backup_file {
            match fs::read(&file_path).await {
                Ok(raw_data) => {
                    // AES key + IV
                    let mut aes_key = [0u8; 32];
                    rand_bytes(&mut aes_key).unwrap();

                    let mut iv = [0u8; 16];
                    rand_bytes(&mut iv).unwrap();

                    // AES CBC PKCS7 encryption
                    let cipher = Cipher::aes_256_cbc();
                    let mut encrypter =
                        Crypter::new(cipher, Mode::Encrypt, &aes_key, Some(&iv)).unwrap();
                    encrypter.pad(true);
                    let mut encrypted = vec![0u8; raw_data.len() + cipher.block_size()];
                    let count = encrypter.update(&raw_data, &mut encrypted).unwrap();
                    let rest = encrypter.finalize(&mut encrypted[count..]).unwrap();
                    encrypted.truncate(count + rest);

                    // Encrypt AES key with RSA public key
                    let pub_key_pem = self.ctx.edge_key.public_key.as_bytes();
                    let pkey = PKey::public_key_from_pem(pub_key_pem).unwrap();

                    let mut encrypter = Encrypter::new(&pkey).unwrap();
                    // Set OAEP padding (default OAEP uses SHA1, so override)
                    encrypter.set_rsa_padding(Padding::PKCS1_OAEP).unwrap();
                    // Set OAEP hash to SHA‑256
                    encrypter.set_rsa_oaep_md(MessageDigest::sha256()).unwrap();
                    encrypter.set_rsa_mgf1_md(MessageDigest::sha256()).unwrap();

                    let mut encrypted_key = vec![0u8; encrypter.encrypt_len(&aes_key).unwrap()];
                    let encrypted_len = encrypter.encrypt(&aes_key, &mut encrypted_key).unwrap();
                    encrypted_key.truncate(encrypted_len);

                    let extension = full_extension(&file_path);

                    // Attach file and AES info to multipart form
                    form = form
                        .part(
                            "file",
                            Part::bytes(encrypted)
                                .file_name(format!("{}.enc", result.generated_id)),
                        )
                        .text("aes_key", hex::encode(encrypted_key))
                        .text("iv", hex::encode(iv))
                        .text("extension", extension);
                }
                Err(e) => {
                    error!("Failed to read backup file: {}", e);
                }
            }
        } else {
            form = form.text("file", "");
        }

        match client.post(&url).multipart(form).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    info!("Backup result sent successfully");
                } else {
                    let text = resp.text().await.unwrap_or_default(); // consumes resp
                    error!("Backup result failed, status: {}, body: {}", status, text);
                }
            }
            Err(e) => {
                error!("Failed to send backup result: {}", e);
            }
        }
    }
}
