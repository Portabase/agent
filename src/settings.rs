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
    pub pooling: usize,
    pub timezone: String,
    pub log: String,
}

impl Settings {
    fn from_env() -> Self {
        dotenv().ok();

        let pooling_seconds = env::var("POOLING")
            .unwrap_or_else(|_| "5".to_string())
            .parse::<usize>()
            .expect("POOLING must be a valid positive integer");

        let tz = env::var("TZ").unwrap_or_else(|_| "UTC".to_string());

        Self {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            app_env: env::var("APP_ENV").unwrap_or_else(|_| "development".into()),
            redis_url: env::var("CELERY_BROKER_URL")
                .unwrap_or_else(|_| "redis://localhost:65515/".into()),
            edge_key: env::var("EDGE_KEY").unwrap_or_default(),
            databases_config_file: env::var("DATABASES_CONFIG_FILE")
                .unwrap_or_else(|_| "config.json".into()),
            data_path: env::var("DATA_PATH").unwrap_or_else(|_| "/config".into()),
            pooling: pooling_seconds,
            timezone: tz,
            log: env::var("LOG").unwrap_or_else(|_| "info".into()),
        }
    }
}

pub static CONFIG: Lazy<Settings> = Lazy::new(Settings::from_env);
