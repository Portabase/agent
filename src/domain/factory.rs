use crate::domain::mongodb::database::MongoDatabase;
use crate::domain::mysql::database::MySQLDatabase;
use crate::domain::postgres::database::PostgresDatabase;
use crate::domain::postgres::{detect_format_from_file, detect_format_from_size};
use crate::domain::redis::database::RedisDatabase;
use crate::domain::sqlite::database::SqliteDatabase;
use crate::domain::valkey::database::ValkeyDatabase;
use crate::services::config::{DatabaseConfig, DbType};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use crate::domain::firebird::database::FirebirdDatabase;
use crate::domain::mariadb::database::MariaDBDatabase;

#[async_trait::async_trait]
pub trait Database: Send + Sync {
    fn file_extension(&self) -> &'static str;
    async fn ping(&self) -> Result<bool>;
    async fn backup(&self, backup_dir: &Path) -> Result<PathBuf>;
    async fn restore(&self, restore_file: &Path) -> Result<()>;
}

pub struct DatabaseFactory;

impl DatabaseFactory {
    pub async fn create_for_backup(cfg: DatabaseConfig) -> Arc<dyn Database> {
        match cfg.db_type {
            DbType::Postgresql => {
                let format = detect_format_from_size(&cfg).await;
                Arc::new(PostgresDatabase::new(cfg, format))
            }
            DbType::Mysql => Arc::new(MySQLDatabase::new(cfg)),
            DbType::Mariadb => Arc::new(MariaDBDatabase::new(cfg)),
            DbType::MongoDB => Arc::new(MongoDatabase::new(cfg)),
            DbType::Sqlite => Arc::new(SqliteDatabase::new(cfg)),
            DbType::Redis => Arc::new(RedisDatabase::new(cfg)),
            DbType::Valkey => Arc::new(ValkeyDatabase::new(cfg)),
            DbType::Firebird => Arc::new(FirebirdDatabase::new(cfg)),
        }
    }

    pub async fn create_for_restore(cfg: DatabaseConfig, restore_file: &Path) -> Arc<dyn Database> {
        match cfg.db_type {
            DbType::Postgresql => {
                let format = detect_format_from_file(restore_file);
                Arc::new(PostgresDatabase::new(cfg, format))
            }
            DbType::Mysql => Arc::new(MySQLDatabase::new(cfg)),
            DbType::Mariadb => Arc::new(MariaDBDatabase::new(cfg)),
            DbType::MongoDB => Arc::new(MongoDatabase::new(cfg)),
            DbType::Sqlite => Arc::new(SqliteDatabase::new(cfg)),
            DbType::Redis => Arc::new(RedisDatabase::new(cfg)),
            DbType::Valkey => Arc::new(ValkeyDatabase::new(cfg)),
            DbType::Firebird => Arc::new(FirebirdDatabase::new(cfg)),
        }
    }
}
