use crate::utils::deserializer::string_or_number_to_string;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct SftpProviderConfig {
    pub host: String,
    #[serde(default, deserialize_with = "string_or_number_to_string")]
    pub port: Option<String>,
    pub username: String,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub private_key: Option<String>,
    #[serde(default)]
    pub remote_path: String,
}
