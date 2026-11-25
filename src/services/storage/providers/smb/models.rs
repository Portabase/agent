use crate::utils::deserializer::string_or_number_to_string;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct SmbProviderConfig {
    pub host: String,
    #[serde(default, deserialize_with = "string_or_number_to_string")]
    pub port: Option<String>,
    pub share: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub remote_path: String,
}
