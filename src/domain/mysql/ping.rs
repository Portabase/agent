use crate::services::config::DatabaseConfig;
use std::collections::HashMap;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

pub async fn run(cfg: DatabaseConfig, env: HashMap<String, String>) -> anyhow::Result<bool> {
    let mut cmd = Command::new("mysqladmin");
    cmd.arg("--host")
        .arg(cfg.host)
        .arg("--port")
        .arg(cfg.port.to_string())
        .arg("--user")
        .arg(cfg.username)
        .arg("ping")
        .envs(env);

    let result = timeout(Duration::from_secs(10), cmd.output()).await;

    match result {
        Ok(output) => {
            let output = output?;
            Ok(output.status.success())
        }
        Err(_) => Ok(false),
    }
}
