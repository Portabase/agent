use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use tracing::{debug, error, info};

pub async fn run(cfg: DatabaseConfig, restore_file: PathBuf) -> Result<()> {
    let handle = tokio::task::spawn_blocking(move || -> Result<()> {
        debug!("Starting restore for database {}", cfg.name);

        let mut sql_content = String::new();
        let mut file = File::open(&restore_file)
            .with_context(|| format!("Failed to open restore file {}", restore_file.display()))?;
        file.read_to_string(&mut sql_content)
            .with_context(|| format!("Failed to read restore file {}", restore_file.display()))?;

        let drop_create_cmd = format!(
            "DROP DATABASE IF EXISTS {0}; CREATE DATABASE {0};",
            cfg.database
        );

        let drop_status = Command::new("mysql")
            .arg("--host")
            .arg(&cfg.host)
            .arg("--port")
            .arg(cfg.port.to_string())
            .arg("--user")
            .arg(&cfg.username)
            .arg("-e")
            .arg(&drop_create_cmd)
            .env("MYSQL_PWD", &cfg.password)
            .status()
            .with_context(|| format!("Failed to drop/recreate database {}", cfg.name))?;

        if !drop_status.success() {
            error!("Drop/create database failed for {}", cfg.name);
            anyhow::bail!("Failed to drop/recreate database {}", cfg.name);
        }
        info!("Database {} dropped and recreated", cfg.name);

        let mut child = Command::new("mysql")
            .arg("--host")
            .arg(&cfg.host)
            .arg("--port")
            .arg(cfg.port.to_string())
            .arg("--user")
            .arg(&cfg.username)
            .arg(&cfg.database)
            .env("MYSQL_PWD", &cfg.password)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("Failed to start mysql restore for {}", cfg.name))?;

        let mut stdin = child.stdin.take().context("Failed to open child stdin")?;
        stdin
            .write_all(sql_content.as_bytes())
            .context("Failed to write SQL content to mysql stdin")?;
        stdin.flush()?;
        drop(stdin);

        let output = child
            .wait_with_output()
            .with_context(|| format!("Failed to complete mysql restore for {}", cfg.name))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("MySQL restore failed for {}: {}", cfg.name, stderr);
            anyhow::bail!("MySQL restore failed for {}", cfg.name);
        }

        info!("Restore finished successfully for database {}", cfg.name);
        Ok(())
    });

    handle.await??;

    Ok(())
}
