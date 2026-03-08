use super::service::BackupService;
use crate::utils::common::BackupMethod;
use anyhow::{Result, anyhow};
use crate::services::api::models::agent::backup::BackupResponse;

impl BackupService {

    pub async fn create_backup_record(
        &self,
        generated_id: &str,
        method: &BackupMethod,
    ) -> Result<BackupResponse> {

        let response = self
            .ctx
            .api
            .backup_create(
                method.to_string(),
                self.ctx.edge_key.agent_id.clone(),
                generated_id,
            )
            .await?;

        response.ok_or_else(|| anyhow!("backup_create returned empty response"))
    }
}