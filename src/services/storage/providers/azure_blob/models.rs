use crate::services::storage::providers::azure_blob::helpers::ResolvedAzure;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Deserialize, Serialize)]
pub struct AzureBlobProviderConfig {
    #[serde(default)]
    pub account_name: String,
    #[serde(default)]
    pub account_key: String,
    pub container_name: String,
    #[serde(default)]
    pub auth_mode: Option<String>,
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


pub(crate) fn ensure_account_in_endpoint(endpoint: &str, account: &str) -> String {
    let trimmed = endpoint.trim_end_matches('/');
    if account.is_empty() {
        return trimmed.to_string();
    }
    if let Ok(url) = Url::parse(trimmed) {
        let host = url.host_str().unwrap_or("");
        if host.contains(account) {
            return trimmed.to_string();
        }
        let path = url.path().trim_matches('/');
        if path == account || path.starts_with(&format!("{account}/")) {
            return trimmed.to_string();
        }
    }
    format!("{trimmed}/{account}")
}

impl AzureBlobProviderConfig {
    pub fn resolve(&self) -> Result<ResolvedAzure> {
        let mode = self.auth_mode.as_deref().unwrap_or("").trim();
        let has_connection_string = !self.connection_string.trim().is_empty();

        if mode == "connectionString" || (mode.is_empty() && has_connection_string) {
            if !has_connection_string {
                return Err(anyhow!(
                    "authMode is connectionString but connectionString is empty"
                ));
            }
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

        if self.account_name.trim().is_empty() {
            return Err(anyhow!("accountName required for accountKey auth"));
        }
        if self.account_key.trim().is_empty() {
            return Err(anyhow!("accountKey required for accountKey auth"));
        }

        let blob_endpoint = match self
            .endpoint_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(endpoint) => ensure_account_in_endpoint(endpoint, &self.account_name),
            None => format!("https://{}.blob.core.windows.net", self.account_name),
        };

        Ok(ResolvedAzure {
            account_name: self.account_name.clone(),
            account_key: self.account_key.clone(),
            blob_endpoint,
        })
    }
}
