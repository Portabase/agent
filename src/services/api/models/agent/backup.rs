use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct BackupStorage {
    pub id: String,
}

#[derive(Serialize)]
pub struct InitUploadResponse<'a> {
    pub message: &'a str,
    #[serde(rename = "backupStorage")]
    pub backup_storage: BackupStorage,
}
