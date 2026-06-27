use crate::services::storage::providers::azure_blob::helpers::ResolvedAzure;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct AzureBlobProviderConfig {
    pub account_name: String,
    pub account_key: String,
    pub container_name: String,
    #[serde(default)]
    pub connection_string: String,
    #[serde(default)]
    pub endpoint_url: Option<String>,
}

fn parse_connection_string(cs: &str) -> std::collections::HashMap<String, String> {
    cs.split(';')
        .filter(|s| !s.trim().is_empty())
        .filter_map(|pair| {
            let mut it = pair.splitn(2, '=');
            let k = it.next()?.trim().to_string();
            let v = it.next()?.trim().to_string();
            Some((k, v))
        })
        .collect()
}

impl AzureBlobProviderConfig {
    pub fn resolve(&self) -> Result<ResolvedAzure> {
        if !self.connection_string.trim().is_empty() {
            let map = parse_connection_string(&self.connection_string);
            let account_name = map
                .get("AccountName")
                .cloned()
                .unwrap_or_else(|| self.account_name.clone());
            let account_key = map
                .get("AccountKey")
                .cloned()
                .unwrap_or_else(|| self.account_key.clone());
            let blob_endpoint = map
                .get("BlobEndpoint")
                .cloned()
                .ok_or_else(|| anyhow!("connection string missing BlobEndpoint"))?;
            return Ok(ResolvedAzure {
                account_name,
                account_key,
                blob_endpoint,
            });
        }

        let blob_endpoint = self
            .endpoint_url
            .clone()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow!("endpointUrl required when connectionString is empty"))?;

        Ok(ResolvedAzure {
            account_name: self.account_name.clone(),
            account_key: self.account_key.clone(),
            blob_endpoint,
        })
    }
}
