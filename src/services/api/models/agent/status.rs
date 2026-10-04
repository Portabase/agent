#![allow(dead_code)]

use crate::services::config::DatabaseConfig;
use crate::utils::deserializer::{deserialize_snake_case, string_or_number_to_string};
use serde::{Deserialize, Serialize};
use toml::Value;

#[derive(Debug, Deserialize)]
pub struct PingResult {
    pub agent: AgentInfo,
    pub databases: Vec<DatabaseStatus>,
}

#[derive(Debug, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    #[serde(rename = "lastContact")]
    pub last_contact: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct DatabaseStorage {
    pub id: String,
    #[serde(deserialize_with = "deserialize_snake_case")]
    pub config: Value,
    pub provider: String,
    #[serde(default, rename = "folderName")]
    pub folder_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseStatus {
    pub dbms: String,
    #[serde(rename = "generatedId")]
    pub generated_id: String,
    #[serde(default)]
    pub storages: Vec<DatabaseStorage>,
    #[serde(default)]
    pub storages_encrypted: Option<bool>,
    #[serde(default)]
    pub storages_ciphertext: Option<String>,
    #[serde(default)]
    pub config_encrypted: Option<bool>,
    #[serde(default)]
    pub config_ciphertext: Option<String>,
    /// Filled in memory after decrypting `config_ciphertext`; never on the wire.
    #[serde(skip)]
    pub resolved_config: Option<DatabaseConfig>,
    pub encrypt: bool,
    pub data: DatabaseData,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseData {
    pub backup: BackupInfo,
    pub restore: RestoreInfo,
}

#[derive(Debug, Deserialize)]
pub struct BackupInfo {
    pub action: bool,
    pub cron: Option<String>,
    /// "archive" | "restic"; absent from dashboards older than P2 (→ archive).
    #[serde(default)]
    pub engine: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RestoreInfo {
    pub action: bool,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default, rename = "metaFile")]
    pub meta_file: Option<String>,
    #[serde(default, deserialize_with = "string_or_number_to_string")]
    pub size: Option<String>,
    /// "restic" for a snapshot restore; absent for archives.
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default, rename = "snapshotId")]
    pub snapshot_id: Option<String>,
    /// AES-GCM JSON array holding the snapshot's storage channel.
    #[serde(default, rename = "storageCiphertext")]
    pub storage_ciphertext: Option<String>,
    /// Filled in memory from `storage_ciphertext`; never on the wire.
    #[serde(skip)]
    pub storage: Option<DatabaseStorage>,
}
