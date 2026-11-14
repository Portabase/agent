use super::hooks::Hooks;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum DbType {
    Mysql,
    Mariadb,
    Postgresql,
    #[serde(rename = "postgresql-cluster")]
    PostgresqlCluster,
    MongoDB,
    Sqlite,
    Redis,
    Valkey,
    Firebird,
    Mssql,
    #[serde(rename = "docker-volume")]
    DockerVolume,
    Files,
}

impl DbType {
    pub fn as_str(&self) -> &'static str {
        match self {
            DbType::Mysql => "mysql",
            DbType::Mariadb => "mariadb",
            DbType::Postgresql => "postgresql",
            DbType::PostgresqlCluster => "postgresql-cluster",
            DbType::MongoDB => "mongodb",
            DbType::Sqlite => "sqlite",
            DbType::Redis => "redis",
            DbType::Valkey => "valkey",
            DbType::Firebird => "firebird",
            DbType::Mssql => "mssql",
            DbType::DockerVolume => "docker-volume",
            DbType::Files => "files",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub name: String,
    pub database: String,
    #[serde(rename = "type")]
    pub db_type: DbType,
    pub username: String,
    pub password: String,
    pub port: u16,
    pub host: String,
    pub generated_id: String,
    pub path: String,
    pub max_packet_size: String,
    pub volume_name: String,
    pub container_name: Option<String>,
    pub options: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub hooks: Option<Hooks>,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DatabasesConfig {
    pub databases: Vec<DatabaseConfig>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct InputDatabaseConfig {
    pub name: String,
    pub database: Option<String>,
    #[serde(rename = "type")]
    pub db_type: DbType,
    pub username: Option<String>,
    pub password: Option<String>,
    pub port: Option<u16>,
    pub host: Option<String>,
    pub generated_id: String,
    pub path: Option<String>,
    pub max_packet_size: Option<String>,
    pub volume_name: Option<String>,
    pub container_name: Option<String>,
    pub options: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub hooks: Option<Hooks>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct InputDatabasesConfig {
    pub databases: Vec<InputDatabaseConfig>,
}
