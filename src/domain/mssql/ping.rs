use crate::services::config::DatabaseConfig;
use anyhow::Result;
use tracing::{error, info};

pub async fn run(cfg: DatabaseConfig) -> Result<bool> {
    info!("Running ping for MSSQL database {}", cfg.name);

    match super::connection::build_client(&cfg).await {
        Ok(mut client) => match client.simple_query("SELECT 1").await {
            Ok(_) => {
                info!("MSSQL ping succeeded for {}", cfg.name);
                Ok(true)
            }
            Err(e) => {
                error!("MSSQL ping query failed for {}: {:?}", cfg.name, e);
                Ok(false)
            }
        },
        Err(e) => {
            error!("MSSQL connection failed for {}: {:?}", cfg.name, e);
            Ok(false)
        }
    }
}
