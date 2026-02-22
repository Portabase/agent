use crate::services::config::DatabaseConfig;

pub async fn run(_cfg: DatabaseConfig) -> anyhow::Result<bool> {
    Ok(true)
}
