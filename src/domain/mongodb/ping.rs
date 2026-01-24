#![allow(dead_code)]

use crate::services::config::DatabaseConfig;
use anyhow::Result;
use mongodb::bson::doc;
use tracing::{error};
use crate::domain::mongodb::connection::connect;

pub async fn run(cfg: DatabaseConfig) -> Result<bool> {
    let client = connect(cfg.clone()).await?;
    let db_name = if cfg.username.is_empty() { &cfg.database } else { "admin" };
    match client.database(db_name).run_command(doc! {"ping": 1}).await {
        Ok(_) => Ok(true),
        Err(e) => {
            error!("--- MongoDB Connection Error Details ---");
            error!("Target Host: {}:{}", cfg.host, cfg.port);
            error!("Error Kind: {:?}", e.kind);
            error!("Full Error: {}", e);
            error!("Check you database network connectivity");
            error!("----------------------------------------");
            Err(anyhow::anyhow!("Ping failed for {}: {}", cfg.name, e))
        }
    }
}
