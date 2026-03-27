use super::connection::connect;
use crate::services::config::DatabaseConfig;

pub async fn run(cfg: DatabaseConfig) -> anyhow::Result<bool> {
    Ok(connect(&cfg).await.is_ok())
}
