use serde::{Deserialize, Serialize};
use crate::utils::deserializer::string_or_number_to_string;

#[derive(Debug, Deserialize, Serialize)]
pub struct S3ProviderConfig {
    pub access_key: String,
    pub secret_key: String,
    pub bucket_name: String,
    pub end_point_url: String,
    pub ssl: bool,
    pub region: Option<String>,
    #[serde(default, deserialize_with = "string_or_number_to_string")]
    pub port: Option<String>,
}

