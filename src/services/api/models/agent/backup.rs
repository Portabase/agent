use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct BackupStorage {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupUploadResponse {
    pub message: String,
    #[serde(rename = "backupStorage")]
    pub backup_storage: BackupStorage,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Backup {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupResponse {
    pub message: String,
    pub backup: Backup,
}
