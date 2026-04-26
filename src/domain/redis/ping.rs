use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use tokio::process::Command;
use tokio::time::{Duration, timeout};
use tracing::{debug, error, info};

pub async fn run(cfg: DatabaseConfig) -> Result<bool> {
    let mut cmd = Command::new("redis-cli");
    cmd.arg("-h")
        .arg(&cfg.host)
        .arg("-p")
        .arg(cfg.port.to_string());

    if !cfg.username.is_empty() {
        cmd.arg("--user").arg(&cfg.username);
    }

    if !cfg.password.is_empty() {
        cmd.arg("-a").arg(&cfg.password);
    }

    cmd.arg("PING");

    debug!("Command Ping Redis: {:?}", cmd);

    let result = timeout(Duration::from_secs(10), cmd.output()).await;

    match result {
        Ok(output) => {
            let output = output.context("Failed to execute redis-cli")?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            

            if stderr.contains("NOAUTH") {
                error!("Redis authentication failed (NOAUTH required)");
                return Ok(false);
            }

            if !output.status.success() {
                error!("Redis command failed with status: {:?}", output.status);
                return Ok(false);
            }

            Ok(stdout.contains("PONG"))
        }
        Err(_) => {
            info!("Timeout connecting to Redis at {}:{}", cfg.host, cfg.port);
            Ok(false)
        }
    }
}
