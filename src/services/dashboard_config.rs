#![allow(dead_code)]

use crate::services::api::models::agent::status::PingResult;
use crate::services::config::{DatabaseConfig, DatabasesConfig};
use std::collections::HashSet;
use std::path::Path;

pub fn merge(local: &[DatabaseConfig], dashboard: &[DatabaseConfig]) -> DatabasesConfig {
    let mut databases: Vec<DatabaseConfig> = local.to_vec();
    for d in dashboard {
        if let Some(slot) = databases
            .iter_mut()
            .find(|c| c.generated_id == d.generated_id)
        {
            *slot = d.clone();
        } else {
            databases.push(d.clone());
        }
    }
    DatabasesConfig { databases }
}

/// `generated_id`s of local (databases.json) sources that no dashboard config replaces.
pub fn local_only_ids(local: &[DatabaseConfig], dashboard: &[DatabaseConfig]) -> HashSet<String> {
    local
        .iter()
        .filter(|c| !dashboard.iter().any(|d| d.generated_id == c.generated_id))
        .map(|c| c.generated_id.clone())
        .collect()
}

pub fn collect_configs(ping: &PingResult) -> Vec<DatabaseConfig> {
    ping.databases
        .iter()
        .filter_map(|db| db.resolved_config.clone())
        .collect()
}

pub fn load_cache(path: &Path) -> Vec<DatabaseConfig> {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    match serde_json::from_str::<DatabasesConfig>(&contents) {
        Ok(cfg) => cfg.databases,
        Err(e) => {
            tracing::warn!("Dashboard cache at {:?} is corrupt ({e}); ignoring", path);
            Vec::new()
        }
    }
}

pub fn persist_cache(path: &Path, databases: &[DatabaseConfig]) -> std::io::Result<()> {
    let wrapper = DatabasesConfig {
        databases: databases.to_vec(),
    };
    let json = serde_json::to_string_pretty(&wrapper)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
