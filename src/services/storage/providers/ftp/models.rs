use crate::utils::deserializer::string_or_number_to_string;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum FtpsMode {
    #[default]
    None,
    Explicit,
    Implicit,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FtpProviderConfig {
    pub host: String,
    #[serde(default, deserialize_with = "string_or_number_to_string")]
    pub port: Option<String>,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub remote_path: String,
    #[serde(default)]
    pub ftps_mode: FtpsMode,
}
