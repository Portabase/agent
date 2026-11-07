use super::connection;
use super::format::MssqlFormat;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

pub async fn run(cfg: DatabaseConfig, restore_file: PathBuf, logger: Arc<JobLogger>) -> Result<()> {
    logger.log("debug", format!("Starting MSSQL restore for database {}", cfg.name));

    let Some(format) = MssqlFormat::from_path(&restore_file) else {
        let message = format!(
            "Unsupported MSSQL backup file {}: expected .bacpac or .dacpac",
            restore_file.display()
        );
        logger.log("error", message.clone());
        anyhow::bail!(message);
    };

    let target_exists = connection::check_restore_target(&cfg).await?;

    for leftover in connection::leftover_side_databases(&cfg).await? {
        logger.log("warn", format!("Removing side database {leftover} left by an interrupted restore"));
        drop_or_warn(&cfg, &leftover, &logger).await;
    }

    let restore_into = if target_exists {
        let side_database = connection::side_database_name(&cfg);
        logger.log("info", format!("Restoring into side database {side_database}; {} stays online and is replaced only if this succeeds", cfg.database));
        side_database
    } else {
        cfg.database.clone()
    };

    if let Err(error) = run_sqlpackage(cfg.clone(), restore_into.clone(), restore_file, format, Arc::clone(&logger)).await {
        drop_or_warn(&cfg, &restore_into, &logger).await;
        if target_exists {
            logger.log("info", format!("{} was left untouched", cfg.database));
        }
        return Err(error);
    }

    if target_exists {
        replace_target(&cfg, &restore_into, &logger).await?;
    }

    logger.log("info", format!("MSSQL restore completed for {}", cfg.name));
    Ok(())
}

async fn drop_or_warn(cfg: &DatabaseConfig, database: &str, logger: &JobLogger) {
    if let Err(error) = connection::drop_database(cfg, database).await {
        logger.log("warn", format!("Could not remove {database}: {error:#}"));
    }
}

async fn replace_target(cfg: &DatabaseConfig, side_database: &str, logger: &JobLogger) -> Result<()> {
    let start = Instant::now();
    if let Err(error) = connection::drop_database(cfg, &cfg.database).await {
        drop_or_warn(cfg, side_database, logger).await;
        return Err(error);
    }
    logger.log_command(format!("DROP DATABASE IF EXISTS [{}]", cfg.database), None, Some(0), Some(start.elapsed().as_millis() as f64));

    let start = Instant::now();
    connection::rename_database(cfg, side_database, &cfg.database)
        .await
        .with_context(|| format!("{} was dropped and the restored data is in {side_database}: rename it by hand", cfg.database))?;
    logger.log_command(format!("ALTER DATABASE [{side_database}] MODIFY NAME = [{}]", cfg.database), None, Some(0), Some(start.elapsed().as_millis() as f64));
    Ok(())
}

async fn run_sqlpackage(
    cfg: DatabaseConfig,
    restore_into: String,
    restore_file: PathBuf,
    format: MssqlFormat,
    logger: Arc<JobLogger>,
) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        let connection_string = format!(
            "Server=tcp:{},{};Database={};User Id={};Password={};TrustServerCertificate=True;Encrypt=False",
            cfg.host, cfg.port, restore_into, cfg.username, cfg.password
        );

        logger.log("info", format!(
            "MSSQL restore: {} → {}:{}/{}",
            restore_file.display(),
            cfg.host,
            cfg.port,
            restore_into
        ));

        let start = Instant::now();
        let mut command = Command::new("sqlpackage");
        match format {
            MssqlFormat::Dacpac => command.args(["/a:Publish", "/p:AllowIncompatiblePlatform=True"]),
            MssqlFormat::Bacpac => command.arg("/a:Import"),
        };
        let output = command
            .arg(format!("/tcs:{}", connection_string))
            .arg(format!("/sf:{}", restore_file.display()))
            .output()
            .with_context(|| format!("Failed to run sqlpackage restore for {}", cfg.name))?;

        let duration_ms = start.elapsed().as_millis() as f64;
        let exit_code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let combined = format!("{}{}", stdout, stderr);

        if !output.status.success() {
            logger.log_command("sqlpackage", if combined.is_empty() { None } else { Some(combined.clone()) }, Some(exit_code), Some(duration_ms));
            logger.log("error", format!(
                "MSSQL restore failed for {} — stderr: {} stdout: {}",
                cfg.name, stderr, stdout
            ));
            anyhow::bail!("MSSQL restore failed for {}: {}", cfg.name, stderr);
        }

        logger.log_command("sqlpackage", if combined.is_empty() { None } else { Some(combined) }, Some(0), Some(duration_ms));
        Ok(())
    })
    .await?
}
