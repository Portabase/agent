use chrono::{DateTime, Utc};
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
    #[serde(default)]
    pub parent: Option<ParentOffer>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ParentOffer {
    #[serde(rename = "backupId")]
    pub backup_id: String,
    #[serde(rename = "chainStartedAt")]
    pub chain_started_at: DateTime<Utc>,
    pub manifest: String,
    #[serde(rename = "scheduledFulls", default)]
    pub scheduled_fulls: bool,
}
