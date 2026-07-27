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
    pub chunk_size: usize,                     // bytes
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
            max_concurrent_backups,
        }
    }
}

pub static CONFIG: Lazy<Settings> = Lazy::new(Settings::from_env);

/// None = unset/blank (unlimited). Panics on a malformed or zero value rather than
/// silently falling back to unlimited, matching this file's other env-parsed fields.
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
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_max_concurrent_backups;
    use std::env::VarError;

    #[test]
    fn unset_means_unlimited() {
        assert_eq!(
            parse_max_concurrent_backups(Err(VarError::NotPresent)),
            None
        );
    }

    #[test]
    fn blank_means_unlimited() {
        assert_eq!(parse_max_concurrent_backups(Ok("".to_string())), None);
        assert_eq!(parse_max_concurrent_backups(Ok("   ".to_string())), None);
    }

    #[test]
    fn valid_value_is_parsed() {
        assert_eq!(parse_max_concurrent_backups(Ok("2".to_string())), Some(2));
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(parse_max_concurrent_backups(Ok(" 2 ".to_string())), Some(2));
    }

    #[test]
    #[should_panic(expected = "must be at least 1")]
    fn zero_panics() {
        parse_max_concurrent_backups(Ok("0".to_string()));
    }

    #[test]
    #[should_panic(expected = "must be a valid positive integer")]
    fn malformed_value_panics() {
        parse_max_concurrent_backups(Ok("not-a-number".to_string()));
    }

    #[test]
    #[should_panic(expected = "must not exceed")]
    fn value_exceeding_semaphore_max_permits_panics() {
        let over_limit = (tokio::sync::Semaphore::MAX_PERMITS as u128 + 1).to_string();
        parse_max_concurrent_backups(Ok(over_limit));
    }
}
