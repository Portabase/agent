use std::process::Stdio;
use tracing::{error, info};
use tokio::io::AsyncWriteExt;
use crate::services::config::DatabaseConfig;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

pub async fn run(cfg: DatabaseConfig) -> anyhow::Result<bool> {
    let db_path = format!("{}/{}:{}", cfg.host, cfg.port, cfg.database);

    info!("Running Ping database from {}", db_path);

    let mut child = Command::new("isql-fb")
        .arg("-q")
        .arg("-user")
        .arg(&cfg.username)
        .arg("-password")
        .arg(&cfg.password)
        .arg(&db_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let query = b"SELECT 1 FROM RDB$DATABASE;\nQUIT;\n";

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(query).await?;
    }

    let output = match timeout(Duration::from_secs(5), child.wait_with_output()).await {
        Ok(res) => res?,
        Err(_) => return Ok(false),
    };

    if !output.status.success() {
        error!("Error output for firebird: {:?}", output);
        return Ok(false);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if stderr.to_lowercase().contains("error") {
        error!("Error output for firebird: {:?}", output);
        return Ok(false);
    }

    if stdout.contains("1") {
        return Ok(true);
    }

    info!("stdout {}", stdout);
    error!("stderr {}", stderr);

    Ok(false)
}
