use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ResultRestoreResponse {
    pub message: String,
    pub status: bool,
}
