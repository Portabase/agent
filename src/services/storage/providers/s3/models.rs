use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct S3ProviderConfig {
    pub access_key: String,
    pub secret_key: String,
    pub bucket_name: String,
    pub end_point_url: String,
    pub ssl: bool,
    pub region: Option<String>,
}


