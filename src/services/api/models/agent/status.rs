use serde::{Deserialize, Serialize};
use toml::Value;
use crate::utils::deserializer::deserialize_snake_case;

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
}

#[derive(Debug, Deserialize)]
pub struct DatabaseStatus {
    pub dbms: String,
    #[serde(rename = "generatedId")]
    pub generated_id: String,
    pub storages: Vec<DatabaseStorage>,
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
}

#[derive(Debug, Deserialize)]
pub struct RestoreInfo {
    pub action: bool,
    pub file: Option<String>,
    #[serde(rename = "metaFile")]
    pub meta_file: Option<String>,
}
