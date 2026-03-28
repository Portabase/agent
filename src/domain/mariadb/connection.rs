use std::path::PathBuf;
use crate::services::config::DatabaseConfig;
use anyhow::Result;
use std::process::Command;

pub async fn server_version(cfg: &DatabaseConfig) -> Result<String> {
    let output = Command::new("mariadb")
        .arg("--host")
        .arg(&cfg.host)
        .arg("--port")
        .arg(cfg.port.to_string())
        .arg("--user")
        .arg(&cfg.username)
        .arg("-e")
        .arg("SELECT VERSION();")
        .env("MYSQL_PWD", &cfg.password)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Version query failed: {}", stderr);
    }

    let version = String::from_utf8_lossy(&output.stdout)
        .lines()
        .nth(1)
        .unwrap_or_default()
        .trim()
        .to_string();

    Ok(version)
}


pub fn select_mariadb_path(version: &str) -> PathBuf {
    let mut parts = version.split('.');
    let major = parts.next().and_then(|v| v.parse::<u32>().ok()).unwrap_or(10);
    let minor = parts.next().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);

    if major < 10 || (major == 10 && minor <= 6) {
        "/usr/local/mariadb-10.6/bin".into()
    } else {
        "/usr/local/mariadb-12.1/bin".into()
    }
}