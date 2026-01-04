use dotenvy::dotenv;
use once_cell::sync::Lazy;
use std::env;

#[derive(Debug)]
#[allow(dead_code)]
pub struct Settings {
    pub app_env: String,
    pub app_version: String,
    pub redis_url: String,
    pub edge_key: String,
    pub databases_config_file: String,
    pub data_path: String,
}

impl Settings {
    fn from_env() -> Self {
        dotenv().ok();

        Self {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            app_env: env::var("APP_ENV").unwrap_or_else(|_| "development".into()),
            redis_url: env::var("CELERY_BROKER_URL")
                .unwrap_or_else(|_| "redis://localhost:6379/".into()),
            edge_key: env::var("EDGE_KEY").unwrap_or_default(),
            databases_config_file: env::var("DATABASES_CONFIG_FILE")
                .unwrap_or_else(|_| "config.toml".into()),
            data_path: env::var("DATA_PATH").unwrap_or_else(|_| "/config".into()),
        }
    }
}

pub static CONFIG: Lazy<Settings> = Lazy::new(Settings::from_env);
