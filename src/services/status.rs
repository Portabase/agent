#![allow(dead_code)]

use crate::core::context::Context;
use crate::domain::factory::DatabaseFactory;
use crate::services::api::endpoints::status::DatabasePayload;
use crate::services::api::models::agent::status::DatabaseStatus;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::api::models::agent::status::PingResult;
use crate::services::config::{build_config, files_method, DatabaseConfig, DbType, InputDatabaseConfig};
use crate::settings::CONFIG;
use crate::utils::file::decrypt_json_gcm;
use futures_util::future::try_join_all;
use reqwest::Client;
use std::collections::HashSet;
use std::error::Error;
use std::sync::Arc;
use tracing::info;

pub fn resolve_dashboard_config(
    status: &mut DatabaseStatus,
    master_key_b64: &str,
) -> Result<(), String> {
    if status.config_encrypted != Some(true) {
        return Ok(());
    }
    let ciphertext = status
        .config_ciphertext
        .as_deref()
        .ok_or("config_encrypted set but config_ciphertext missing")?;

    let plaintext = decrypt_json_gcm(ciphertext, master_key_b64)
        .map_err(|e| format!("Failed to decrypt config: {e}"))?;
    let input: InputDatabaseConfig = serde_json::from_slice(&plaintext)
        .map_err(|e| format!("Failed to parse decrypted config: {e}"))?;
    status.resolved_config = Some(build_config(input)?);
    Ok(())
}

/// A storage channel sent encrypted as a one-element JSON array.
fn decrypt_storage(ciphertext: &str, master_key_b64: &str, what: &str) -> Result<Option<DatabaseStorage>, String> {
    let plaintext = decrypt_json_gcm(ciphertext, master_key_b64)
        .map_err(|e| format!("Failed to decrypt {what} storage: {e}"))?;
    let storages: Vec<DatabaseStorage> = serde_json::from_slice(&plaintext)
        .map_err(|e| format!("Failed to parse {what} storage: {e}"))?;
    Ok(storages.into_iter().next())
}

/// Decrypts the storage channel of a snapshot restore (`restore.storageCiphertext`).
pub fn resolve_restore_storage(status: &mut DatabaseStatus, master_key_b64: &str) -> Result<(), String> {
    if let Some(ciphertext) = status.data.restore.storage_ciphertext.clone() {
        status.data.restore.storage = decrypt_storage(&ciphertext, master_key_b64, "restore")?;
    }
    Ok(())
}

/// `method` reported in the ping for a files source declared in databases.json;
/// `None` for dashboard-managed sources (the dashboard owns their method) and other dbms.
pub fn payload_method(db: &DatabaseConfig, local_ids: &HashSet<String>) -> Option<&'static str> {
    (matches!(db.db_type, DbType::Files) && local_ids.contains(&db.generated_id)).then(|| files_method(db))
}

pub struct StatusService {
    ctx: Arc<Context>,
    client: Client,
}

impl StatusService {
    pub fn new(ctx: Arc<Context>) -> Self {
        StatusService {
            ctx,
            client: Client::new(),
        }
    }

    pub async fn ping(
        &self,
        databases: &[DatabaseConfig],
        local_ids: &HashSet<String>,
    ) -> Result<PingResult, Box<dyn Error>> {
        let edge_key = &self.ctx.edge_key;

        let databases_payload: Vec<DatabasePayload> =
            try_join_all(databases.into_iter().map(|db| async move {
                let db_engine = DatabaseFactory::create_for_backup(db.clone()).await;

                let reachable = db_engine.ping().await?;
                info!("Ping {} => {:?}", db.name, reachable);

                Ok::<DatabasePayload, anyhow::Error>(DatabasePayload {
                    name: &db.name,
                    dbms: &db.db_type.as_str(),
                    generated_id: &db.generated_id,
                    ping_status: reachable,
                    method: payload_method(db, local_ids),
                })
            }))
            .await?;

        let version_str = CONFIG.app_version.as_str();
        let mut result = self
            .ctx
            .api
            .agent_status(&edge_key.agent_id, &version_str, databases_payload)
            .await?
            .unwrap();

        for db in result.databases.iter_mut() {
            if db.storages_encrypted == Some(true) {
                let ciphertext = db
                    .storages_ciphertext
                    .as_deref()
                    .ok_or("storages_encrypted set but storages_ciphertext missing")?;

                let plaintext = decrypt_json_gcm(ciphertext, &edge_key.master_key_b64)
                    .map_err(|e| format!("Failed to decrypt storages: {e}"))?;

                db.storages = serde_json::from_slice::<Vec<DatabaseStorage>>(&plaintext)
                    .map_err(|e| format!("Failed to parse decrypted storages: {e}"))?;
            }

            if let Err(e) = resolve_dashboard_config(db, &edge_key.master_key_b64) {
                tracing::warn!("Skipping dashboard config for {}: {e}", db.generated_id);
            }
            if let Err(e) = resolve_restore_storage(db, &edge_key.master_key_b64) {
                tracing::warn!("Snapshot restore storage unreadable for {}: {e}", db.generated_id);
            }
        }
        Ok(result)
    }
}
