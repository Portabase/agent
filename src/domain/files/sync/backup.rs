use super::rclone::{Mode, replica_target, sync_dir};
use super::stats::SyncStats;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use crate::services::storage::providers::local::{LOCAL_STORAGE_USER, local_storage_password};
use crate::services::storage::providers::rclone::helpers::{build_rclone_config, obscure_password};
use crate::services::storage::providers::rclone::target::rclone_target;
use crate::utils::edge_key::EdgeKey;
use crate::utils::file::folder_or_default;
use anyhow::Result;

pub fn replica_path(storage: &DatabaseStorage, generated_id: &str) -> String {
    format!("{}/sync/{generated_id}/current", folder_or_default(storage.folder_name.as_deref()))
}

const LOCAL_REMOTE: &str = "pblocal";

pub fn local_sync_config(edge_key: &EdgeKey) -> Result<String> {
    build_rclone_config(
        LOCAL_REMOTE,
        &[
            ("type", "webdav".into()),
            ("url", format!("{}/storage/sync", edge_key.server_url.trim_end_matches('/'))),
            ("vendor", "rclone".into()),
            ("user", LOCAL_STORAGE_USER.into()),
            ("pass", obscure_password(&local_storage_password(&edge_key.master_key_b64)?)?),
        ],
    )
}

pub async fn replicate(edge_key: &EdgeKey, cfg: &DatabaseConfig, storage: &DatabaseStorage, mode: Mode, logger: &JobLogger) -> Result<SyncStats> {
    if storage.provider == "local" {
        let dest = format!("{LOCAL_REMOTE}:{}/current", cfg.generated_id);
        return sync_dir(&local_sync_config(edge_key)?, &dest, cfg, mode, logger).await;
    }
    let target = rclone_target(storage)?;
    let dest = replica_target(
        &target.remote_name,
        &target.base_path,
        folder_or_default(storage.folder_name.as_deref()),
        &cfg.generated_id,
    );
    sync_dir(&target.config_text, &dest, cfg, mode, logger).await
}
