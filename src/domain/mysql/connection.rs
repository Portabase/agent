use crate::services::config::DatabaseConfig;
use std::process::Command;
use anyhow::Result;

pub async fn server_version(cfg: &DatabaseConfig) -> Result<String> {

    let output = Command::new("mysql")
        .arg("--host").arg(&cfg.host)
        .arg("--port").arg(cfg.port.to_string())
        .arg("--user").arg(&cfg.username)
        .arg("-e").arg("SELECT VERSION();")
        .env("MYSQL_PWD", &cfg.password)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Version query failed: {}", stderr);
    }

    let version = String::from_utf8_lossy(&output.stdout)
        .lines()
        .nth(1) // skip column header
        .unwrap_or_default()
        .trim()
        .to_string();

    Ok(version)
}

