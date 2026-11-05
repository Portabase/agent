use super::builder::build_config;
use super::models::{DatabasesConfig, InputDatabasesConfig};
use crate::core::context::Context;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use tracing::info;

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
