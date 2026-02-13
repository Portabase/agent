use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeriodicTask {
    pub task: String,
    pub cron: String,
    pub args: Vec<String>,
    pub enabled: bool,
    pub metadata: Option<Value>,
}
