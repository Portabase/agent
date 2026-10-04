#![allow(dead_code)]

use crate::core::context::Context;
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use toml;
use tracing::info;
use uuid::Uuid;

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
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct InputDatabasesConfig {
    pub databases: Vec<InputDatabaseConfig>,
}

fn required<T: Clone>(opt: &Option<T>, db_name: &str, field_name: &str) -> Result<T, String> {
    match opt {
        Some(v) => Ok(v.clone()),
        None => Err(format!(
            "Missing required field '{}' for database '{}'",
            field_name, db_name
        )),
    }
}

fn optional<T: Clone + Default>(opt: &Option<T>) -> T {
    opt.clone().unwrap_or_default()
}

pub const FILES_METHODS: [&str; 3] = ["archive", "snapshot", "sync"];

/// The method a files source declares in `options.method` (`archive` when absent).
pub fn files_method(cfg: &DatabaseConfig) -> &'static str {
    match cfg.options.get("method").and_then(|v| v.as_str()) {
        Some("snapshot") => "snapshot",
        Some("sync") => "sync",
        _ => "archive",
    }
}

pub fn build_config(db: InputDatabaseConfig) -> Result<DatabaseConfig, String> {
    if Uuid::parse_str(&db.generated_id).is_err() {
        return Err(format!("Invalid UUID for database '{}'", db.name));
    }

    let username = match db.db_type {
        DbType::Postgresql
        | DbType::PostgresqlCluster
        | DbType::Mysql
        | DbType::Mariadb
        | DbType::Mssql => required(&db.username, &db.name, "username")?,
        _ => optional(&db.username),
    };
    let password = match db.db_type {
        DbType::Postgresql
        | DbType::PostgresqlCluster
        | DbType::Mysql
        | DbType::Mariadb
        | DbType::Mssql => required(&db.password, &db.name, "password")?,
        _ => optional(&db.password),
    };
    let host = match db.db_type {
        DbType::Postgresql
        | DbType::PostgresqlCluster
        | DbType::Mysql
        | DbType::Mariadb
        | DbType::MongoDB
        | DbType::Redis
        | DbType::Firebird
        | DbType::Valkey
        | DbType::Mssql => required(&db.host, &db.name, "host")?,
        DbType::Sqlite | DbType::DockerVolume | DbType::Files => optional(&db.host),
    };

    let port = match db.db_type {
        DbType::Postgresql
        | DbType::PostgresqlCluster
        | DbType::Mysql
        | DbType::Mariadb
        | DbType::Redis
        | DbType::Firebird
        | DbType::Valkey
        | DbType::Mssql => required(&db.port, &db.name, "port")?,
        DbType::MongoDB | DbType::Sqlite | DbType::DockerVolume | DbType::Files => {
            db.port.unwrap_or(0)
        }
    };

    let database_name = match db.db_type {
        DbType::Sqlite | DbType::Redis | DbType::Valkey | DbType::DockerVolume | DbType::Files => {
            optional(&db.database)
        }
        DbType::PostgresqlCluster => db
            .database
            .clone()
            .unwrap_or_else(|| "postgres".to_string()),
        _ => required(&db.database, &db.name, "database")?,
    };
    let path_val = match db.db_type {
        DbType::Sqlite => required(&db.path, &db.name, "path")?,
        DbType::Files => {
            let p = required(&db.path, &db.name, "path")?;
            if !p.starts_with('/') || p.trim_end_matches('/').is_empty() {
                return Err(format!(
                    "Path for files source '{}' must be absolute and not '/'",
                    db.name
                ));
            }
            p
        }
        _ => optional(&db.path),
    };
    if matches!(db.db_type, DbType::Files) {
        if let Some(method) = db.options.as_ref().and_then(|o| o.get("method")) {
            if !method.as_str().is_some_and(|m| FILES_METHODS.contains(&m)) {
                return Err(format!(
                    "Unknown method {method} for files source '{}' (expected archive, snapshot or sync)",
                    db.name
                ));
            }
        }
    }
    let max_packet_size = match db.db_type {
        DbType::Mysql | DbType::Mariadb => db.max_packet_size.unwrap_or_else(|| "512M".to_string()),
        _ => String::new(),
    };
    let volume_name = match db.db_type {
        DbType::DockerVolume => required(&db.volume_name, &db.name, "volume_name")?,
        _ => optional(&db.volume_name),
    };

    Ok(DatabaseConfig {
        name: db.name,
        database: database_name,
        db_type: db.db_type,
        username,
        password,
        host,
        port,
        generated_id: db.generated_id,
        path: path_val,
        max_packet_size,
        volume_name,
        container_name: db.container_name.clone(),
        options: db.options.unwrap_or_default(),
    })
}

pub struct ConfigService {
    ctx: Arc<Context>,
}

impl ConfigService {
    pub fn new(ctx: Arc<Context>) -> Self {
        ConfigService { ctx }
    }

    fn resolve_path(file_path: Option<&str>) -> String {
        match file_path {
            Some(fp) => fp.to_string(),
            None => format!(
                "{}/{}",
                crate::settings::CONFIG.data_path,
                crate::settings::CONFIG.databases_config_file
            ),
        }
    }

    pub fn load(&self, file_path: Option<&str>) -> Result<DatabasesConfig, String> {
        let path = Self::resolve_path(file_path);

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

        let input_config: InputDatabasesConfig = match extension {
            "json" => {
                serde_json::from_str(&contents).map_err(|e| format!("JSON parsing error: {}", e))?
            }
            "toml" => {
                toml::from_str(&contents).map_err(|e| format!("TOML parsing error: {}", e))?
            }
            _ => return Err("Unsupported config file format. Use .json or .toml".to_string()),
        };

        let mut databases = Vec::with_capacity(input_config.databases.len());
        for db in input_config.databases {
            databases.push(build_config(db)?);
        }

        info!("Databases: {} instances loaded", databases.len());
        Ok(DatabasesConfig { databases })
    }

    pub fn load_optional(&self, file_path: Option<&str>) -> DatabasesConfig {
        let path = Self::resolve_path(file_path);

        if !Path::new(&path).exists() {
            info!(
                "No local databases config at {}; using dashboard-defined databases only",
                path
            );
            return DatabasesConfig {
                databases: Vec::new(),
            };
        }

        self.load(file_path).unwrap_or_else(|e| {
            tracing::warn!(
                    "Local databases config unavailable ({e}); continuing with dashboard-defined databases only"
                );
            DatabasesConfig { databases: Vec::new() }
        })
    }
}
