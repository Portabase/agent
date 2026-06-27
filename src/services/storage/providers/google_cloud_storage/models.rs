use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct GoogleCloudStorageProviderConfig {
    pub project_id: String,
    pub bucket_name: String,
    pub client_email: String,
    pub private_key: String,
    #[serde(default)]
    pub api_endpoint: Option<String>,
}
