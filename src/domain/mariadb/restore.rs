use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

pub async fn run(cfg: DatabaseConfig, restore_file: PathBuf, logger: Arc<JobLogger>) -> Result<()> {
    let handle = tokio::task::spawn_blocking(move || -> Result<()> {
        logger.log("info", format!("Starting restore for database {}", cfg.name));

        let mut sql_content = String::new();
        let mut file = File::open(&restore_file)
            .with_context(|| format!("Failed to open restore file {}", restore_file.display()))?;
        file.read_to_string(&mut sql_content)
            .with_context(|| format!("Failed to read restore file {}", restore_file.display()))?;

        let drop_create_cmd = format!(
            "DROP DATABASE IF EXISTS `{0}`; CREATE DATABASE `{0}`;",
            cfg.database
        );

        let drop_start = Instant::now();
        let drop_output = Command::new("mariadb")
            .arg("--host")
            .arg(&cfg.host)
            .arg("--port")
            .arg(cfg.port.to_string())
            .arg("--user")
            .arg(&cfg.username)
            .arg("-e")
            .arg(&drop_create_cmd)
            .env("MYSQL_PWD", &cfg.password)
            .output()
            .with_context(|| format!("Failed to drop/recreate database {}", cfg.name))?;

        let drop_duration_ms = drop_start.elapsed().as_millis() as f64;
        let drop_exit_code = drop_output.status.code().unwrap_or(-1);
        let drop_stderr = String::from_utf8_lossy(&drop_output.stderr).to_string();

        if !drop_output.status.success() {
            logger.log_command("mariadb", Some(drop_stderr.clone()), Some(drop_exit_code), Some(drop_duration_ms));
            logger.log("error", format!("Drop/create database failed for {}: {}", cfg.name, drop_stderr));
            anyhow::bail!("Failed to drop/recreate database {}", cfg.name);
        }
        logger.log_command("mariadb", if drop_stderr.is_empty() { None } else { Some(drop_stderr) }, Some(0), Some(drop_duration_ms));
        logger.log("info", format!("Database {} dropped and recreated", cfg.name));

        let start = Instant::now();

        let mut child = Command::new("mariadb")
            .arg("--host")
            .arg(&cfg.host)
            .arg("--port")
            .arg(cfg.port.to_string())
            .arg("--user")
            .arg(&cfg.username)
            .arg("--database")
            .arg(&cfg.database)
            .env("MYSQL_PWD", &cfg.password)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("Failed to start MariaDB restore for {}", cfg.name))?;

        let mut stdin = child.stdin.take().context("Failed to open child stdin")?;
        stdin
            .write_all(sql_content.as_bytes())
            .context("Failed to write SQL content to MariaDB stdin")?;
        stdin.flush()?;
        drop(stdin);

        let output = child
            .wait_with_output()
            .with_context(|| format!("Failed to complete MariaDB restore for {}", cfg.name))?;

        let duration_ms = start.elapsed().as_millis() as f64;
        let exit_code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            logger.log_command("mariadb", Some(stderr.clone()), Some(exit_code), Some(duration_ms));
            logger.log("error", format!("MariaDB restore failed for {}: {}", cfg.name, stderr));
            anyhow::bail!("MariaDB restore failed for {}", cfg.name);
        }

        logger.log_command("mariadb", if stderr.is_empty() { None } else { Some(stderr) }, Some(0), Some(duration_ms));
        logger.log("info", format!("Restore finished successfully for database {}", cfg.name));
        Ok(())
    });

    handle.await??;

    Ok(())
}
