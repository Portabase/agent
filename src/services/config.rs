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
use uuid::Uuid;

#[derive(Debug, Deserialize, Clone)]
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
    pub path: String,
    pub max_packet_size: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
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
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct InputDatabasesConfig {
    pub databases: Vec<InputDatabaseConfig>,
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

        let input_config: InputDatabasesConfig = match extension {
            "json" => {
                serde_json::from_str(&contents).map_err(|e| format!("JSON parsing error: {}", e))?
            }
            "toml" => {
                toml::from_str(&contents).map_err(|e| format!("TOML parsing error: {}", e))?
            }
            _ => return Err("Unsupported config file format. Use .json or .toml".to_string()),
        };

        fn required<T: Clone>(
            opt: &Option<T>,
            db_name: &str,
            field_name: &str,
        ) -> Result<T, String> {
            match opt {
                Some(v) => Ok(v.clone()),
                None => {
                    let msg = format!(
                        "Missing required field '{}' for database '{}'",
                        field_name, db_name
                    );
                    Err(msg)
                }
            }
        }

        fn optional<T: Clone>(opt: &Option<T>) -> T
        where
            T: Default,
        {
            opt.clone().unwrap_or_default()
        }

        let mut databases = Vec::with_capacity(input_config.databases.len());

        for db in input_config.databases {
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
                DbType::Sqlite => optional(&db.host),
            };

            let port = match db.db_type {
                DbType::Postgresql
                | DbType::PostgresqlCluster
                | DbType::Mysql
                | DbType::Mariadb
                | DbType::MongoDB
                | DbType::Redis
                | DbType::Firebird
                | DbType::Valkey
                | DbType::Mssql => required(&db.port, &db.name, "port")?,
                DbType::Sqlite => db.port.unwrap_or(0),
            };

            let database_name = match db.db_type {
                DbType::Sqlite | DbType::Redis | DbType::Valkey => optional(&db.database),
                DbType::PostgresqlCluster => db
                    .database
                    .clone()
                    .unwrap_or_else(|| "postgres".to_string()),
                _ => required(&db.database, &db.name, "database")?,
            };

            let path_val = match db.db_type {
                DbType::Sqlite => required(&db.path, &db.name, "path")?,
                _ => optional(&db.path),
            };

            let max_packet_size = match db.db_type {
                DbType::Mysql | DbType::Mariadb => {
                    db.max_packet_size.unwrap_or_else(|| "512M".to_string())
                }
                _ => String::new(),
            };

            databases.push(DatabaseConfig {
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
            });
        }

        info!("Databases: {} instances loaded", databases.len());
        Ok(DatabasesConfig { databases })
    }
}
