#![allow(dead_code)]

use std::path::PathBuf;
use crate::services::config::DbType;

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
    pub remote_file_path: Option<String>,
    pub total_size: Option<u64>,
}