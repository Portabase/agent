use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct RcloneProviderConfig {
    pub config_text: String,
    pub remote_name: String,
    pub remote_path: String,
}
