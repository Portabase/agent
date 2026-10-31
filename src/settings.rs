use dotenvy::dotenv;
use once_cell::sync::Lazy;
use std::env;
use tokio::sync::Semaphore;

#[derive(Debug)]
#[allow(dead_code)]
pub struct Settings {
    pub app_env: String,
    pub app_version: String,
    pub redis_url: String,
    pub edge_key: String,
    pub databases_config_file: String,
    pub data_path: String,
    pub pg_bin_dir: String,
    pub pooling: usize,
    pub timezone: String,
    pub log: String,
    pub chunk_size: usize, // bytes
    pub retry_attempts: u32,
    pub retry_backoff_ms: u64,
    pub max_concurrent_backups: Option<usize>, // None = unlimited
}

impl Settings {
    fn from_env() -> Self {
        dotenv().ok();

        let pooling_seconds = env::var("POOLING")
            .unwrap_or_else(|_| "5".to_string())
            .parse::<usize>()
            .expect("POOLING must be a valid positive integer");

        if pooling_seconds < 1 || pooling_seconds > 600 {
            panic!("POOLING must be between 1 second and 600 seconds (10 minutes)");
        }

        if pooling_seconds < 5 {
            eprintln!(
                "[WARNING] POOLING is set to {}s. Values under 5s are not recommended for production.",
                pooling_seconds
            );
        }

        let chunk_size_mb = env::var("CHUNK_SIZE_MB")
            .unwrap_or_else(|_| "1".to_string())
            .parse::<usize>()
            .expect("CHUNK_SIZE_MB must be a valid positive integer");

        if chunk_size_mb == 0 || chunk_size_mb > 10 {
            panic!("CHUNK_SIZE_MB must be between 1 and 10 MB");
        }

        let chunk_size = chunk_size_mb * 1024 * 1024;

        let retry_attempts = env::var("RETRY_ATTEMPTS")
            .unwrap_or_else(|_| "3".to_string())
            .parse::<u32>()
            .expect("RETRY_ATTEMPTS must be a valid positive integer");

        if retry_attempts < 3 || retry_attempts > 5 {
            panic!("RETRY_ATTEMPTS must be between 3 and 5");
        }

        let retry_backoff_ms = env::var("RETRY_BACKOFF_MS")
            .unwrap_or_else(|_| "1000".to_string())
            .parse::<u64>()
            .expect("RETRY_BACKOFF_MS must be a valid positive integer");

        if retry_backoff_ms < 100 || retry_backoff_ms > 30_000 {
            panic!("RETRY_BACKOFF_MS must be between 100 and 30000 milliseconds");
        }

        let max_concurrent_backups =
            parse_max_concurrent_backups(env::var("MAX_CONCURRENT_BACKUPS"));

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
            pg_bin_dir: env::var("PG_BIN_DIR").unwrap_or_default(),
            pooling: pooling_seconds,
            timezone: tz,
            log: env::var("LOG").unwrap_or_else(|_| "info".into()),
            chunk_size,
            retry_attempts,
            retry_backoff_ms,
            max_concurrent_backups,
        }
    }
}

pub static CONFIG: Lazy<Settings> = Lazy::new(Settings::from_env);

pub(crate) fn parse_max_concurrent_backups(value: Result<String, env::VarError>) -> Option<usize> {
    match value {
        Ok(val) if val.trim().is_empty() => None,
        Ok(val) => {
            let parsed = val
                .trim()
                .parse::<usize>()
                .expect("MAX_CONCURRENT_BACKUPS must be a valid positive integer");

            if parsed == 0 {
                panic!("MAX_CONCURRENT_BACKUPS must be at least 1");
            }
            if parsed > Semaphore::MAX_PERMITS {
                panic!(
                    "MAX_CONCURRENT_BACKUPS must not exceed {}",
                    Semaphore::MAX_PERMITS
                );
            }

            Some(parsed)
        }
        Err(env::VarError::NotPresent) => None,
        Err(env::VarError::NotUnicode(_)) => panic!("MAX_CONCURRENT_BACKUPS must be valid Unicode"),
    }
}
