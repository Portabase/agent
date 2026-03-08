use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct RestoreResult {
    #[serde(rename = "generatedId")]
    pub generated_id: String,
    pub status: String,
}