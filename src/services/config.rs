#![allow(dead_code)]

use crate::core::context::Context;
use serde::Deserialize;
use serde_json;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use toml;
use tracing::info;

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum DbType {
    Mysql,
    Mariadb,
    Postgresql,
    MongoDB
    // Sqlite,
    // Add other DB types if needed
}

impl DbType {
    pub fn as_str(&self) -> &'static str {
        match self {
            DbType::Mysql => "mysql",
            DbType::Mariadb => "mysql",
            DbType::Postgresql => "postgresql",
            DbType::MongoDB => "mongodb",
            // DbType::Sqlite => "sqlite",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
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
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct DatabasesConfig {
    pub databases: Vec<DatabaseConfig>,
}

pub struct ConfigService {
    ctx: Arc<Context>,
}

impl ConfigService {
    pub fn new(ctx: Arc<Context>) -> Self {
        ConfigService { ctx }
    }

    pub fn load(&self, file_path: Option<&str>) -> Result<DatabasesConfig, String> {
        let path: String = if let Some(fp) = file_path {
            fp.to_string()
        } else {
            format!(
                "{}/{}",
                crate::settings::CONFIG.data_path,
                crate::settings::CONFIG.databases_config_file
            )
        };

        info!("Loading databases config from: {}", path);

        let path_obj = Path::new(&path);

        if !path_obj.exists() {
            return Err(format!(
                "Config file not found: {}, check documentation and add config file.",
                &path
            ));
        }

        let extension = path_obj
            .extension()
            .and_then(|s| s.to_str())
            .ok_or_else(|| "Failed to determine config file extension".to_string())?;

        let mut file =
            File::open(path_obj).map_err(|e| format!("Failed to open config file: {}", e))?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .map_err(|e| format!("Failed to read config file: {}", e))?;

        let config: DatabasesConfig = match extension {
            "json" => {
                serde_json::from_str(&contents).map_err(|e| format!("JSON parsing error: {}", e))?
            }
            "toml" => {
                toml::from_str(&contents).map_err(|e| format!("TOML parsing error: {}", e))?
            }
            _ => return Err("Unsupported config file format. Use .json or .toml".to_string()),
        };

        info!("Databases : {:?} instances loaded", config.databases.len());

        Ok(config)
    }
}
