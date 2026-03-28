use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use tokio::process::Command;
use tokio::time::{Duration, timeout};
use tracing::{debug, info, error};

pub async fn run(cfg: DatabaseConfig) -> Result<bool> {
    let mut cmd = Command::new("valkey-cli");
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

    debug!("Command Ping Valkey: {:?}", cmd);

    let result = timeout(Duration::from_secs(10), cmd.output()).await;

    match result {
        Ok(output) => {
            let output = output.context("Failed to execute valkey-cli")?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            if !stdout.is_empty() {
                error!("Valkey stderr: {}", stderr);
            }

            if stderr.contains("NOAUTH") {
                error!("Valkey authentication failed (NOAUTH required)");
                return Ok(false);
            }

            if !output.status.success() {
                error!("Valkey command failed with status: {:?}", output.status);
                return Ok(false);
            }

            Ok(stdout.contains("PONG"))
        }
        Err(_) => {
            info!("Timeout connecting to Valkey at {}:{}", cfg.host, cfg.port);
            Ok(false)
        }
    }
}
