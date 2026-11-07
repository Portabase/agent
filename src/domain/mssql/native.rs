use super::connection::build_client;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use crate::utils::compress::tar_gz_file_to;
use anyhow::{Context, Result, anyhow};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tiberius::ToSql;
use tiberius::error::Error as TiberiusError;
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufReader};

const COMPRESSION_UNSUPPORTED: u32 = 1844;
const BACKUP_SQL: &str = "BACKUP DATABASE @P1 TO DISK = @P2 WITH COPY_ONLY, FORMAT, CHECKSUM";
const BACKUP_COMPRESSED_SQL: &str =
    "BACKUP DATABASE @P1 TO DISK = @P2 WITH COPY_ONLY, FORMAT, CHECKSUM, COMPRESSION";
const VERIFY_SQL: &str = "RESTORE VERIFYONLY FROM DISK = @P1 WITH CHECKSUM";
const FILE_LIST_SQL: &str = "RESTORE FILELISTONLY FROM DISK = @P1";
const TARGET_FILES_SQL: &str = "SELECT name, physical_name FROM sys.master_files WHERE database_id = DB_ID(@P1);
SELECT CAST(SERVERPROPERTY('InstanceDefaultDataPath') AS nvarchar(260)), CAST(SERVERPROPERTY('InstanceDefaultLogPath') AS nvarchar(260));";
const TAKE_OVER_SQL: &str = "DECLARE @q nvarchar(258) = QUOTENAME(@P1);
IF DB_ID(@P1) <= 4
    THROW 50000, N'Refusing to restore over a system database; nothing was changed', 1;
IF DB_ID(@P1) IS NOT NULL
    EXEC(N'ALTER DATABASE ' + @q + N' SET SINGLE_USER WITH ROLLBACK IMMEDIATE');";
const RELEASE_SQL: &str = "DECLARE @q nvarchar(258) = QUOTENAME(@P1);
IF DB_ID(@P1) > 4 AND DATABASEPROPERTYEX(@P1, 'Status') = 'ONLINE'
    EXEC(N'ALTER DATABASE ' + @q + N' SET MULTI_USER');";

pub(crate) fn join_server_path(directory: &str, file_name: &str) -> String {
    let separator = if directory.contains('\\') { '\\' } else { '/' };
    format!("{}{}{}", directory.trim_end_matches(['/', '\\']), separator, file_name)
}

pub(crate) struct BackupFile<'a> {
    pub logical_name: &'a str,
    pub physical_name: &'a str,
    pub kind: &'a str,
}

pub(crate) fn target_path(
    database: &str,
    file: &BackupFile,
    existing_files: &HashMap<String, String>,
    data_directory: &str,
    log_directory: &str,
) -> String {
    if let Some(existing_path) = existing_files.get(file.logical_name) {
        return existing_path.clone();
    }
    let directory = if file.kind == "L" { log_directory } else { data_directory };
    let original_name = file.physical_name.rsplit(['/', '\\']).next().unwrap_or_default();
    let extension = original_name.rfind('.').map_or("", |index| &original_name[index..]);
    join_server_path(directory, &format!("{database}_{}{extension}", file.logical_name))
}

fn shared_file(cfg: &DatabaseConfig) -> Result<(String, PathBuf)> {
    let option = |name: &str| {
        cfg.options
            .get(name)
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
    };
    let (Some(server_path), Some(agent_path)) = (option("bak_server_path"), option("bak_agent_path")) else {
        anyhow::bail!("MSSQL .bak needs the bak_server_path and bak_agent_path options for {}", cfg.name);
    };
    let file_name = format!("{}.bak", cfg.generated_id);
    Ok((join_server_path(server_path, &file_name), Path::new(agent_path).join(file_name)))
}

async fn remove_shared_file(agent_file: &Path, logger: &JobLogger) {
    match tokio::fs::remove_file(agent_file).await {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => logger.log(
            "warn",
            format!("Could not remove {}, the next backup overwrites it: {error}", agent_file.display()),
        ),
        _ => {}
    }
}

pub async fn backup(cfg: &DatabaseConfig, backup_dir: &Path, logger: Arc<JobLogger>) -> Result<PathBuf> {
    let (server_file, agent_file) = shared_file(cfg)?;
    let outcome = backup_to_archive(cfg, backup_dir, &server_file, &agent_file, Arc::clone(&logger)).await;
    remove_shared_file(&agent_file, &logger).await;
    outcome
}

async fn backup_to_archive(
    cfg: &DatabaseConfig,
    backup_dir: &Path,
    server_file: &str,
    agent_file: &Path,
    logger: Arc<JobLogger>,
) -> Result<PathBuf> {
    logger.log("info", format!("Starting MSSQL bak backup: {}:{}/{} → {}", cfg.host, cfg.port, cfg.database, server_file));
    let mut client = build_client(cfg, Some("master"))
        .await
        .map_err(|error| anyhow!("Failed to connect to master for {}: {error}", cfg.name))?;

    let params: [&dyn ToSql; 2] = [&cfg.database.as_str(), &server_file];
    let start = Instant::now();
    match client.execute(BACKUP_COMPRESSED_SQL, &params).await {
        Err(TiberiusError::Server(error)) if error.code() == COMPRESSION_UNSUPPORTED => {
            logger.log("info", format!("Backup compression is not available on this server: {}", error.message()));
            client.execute(BACKUP_SQL, &params).await
        }
        outcome => outcome,
    }
    .map_err(|error| anyhow!("MSSQL backup failed for {}: {error}", cfg.name))?;
    logger.log_command(format!("BACKUP DATABASE [{}] TO DISK = {}", cfg.database, server_file), None, Some(0), Some(start.elapsed().as_millis() as f64));

    if cfg.options.get("bak_verify").and_then(|value| value.as_bool()).unwrap_or(false) {
        let start = Instant::now();
        client
            .execute(VERIFY_SQL, &[&server_file])
            .await
            .map_err(|error| anyhow!("MSSQL backup verification failed for {}: {error}", cfg.name))?;
        logger.log_command("RESTORE VERIFYONLY", None, Some(0), Some(start.elapsed().as_millis() as f64));
    }

    if let Err(error) = tokio::fs::metadata(agent_file).await {
        anyhow::bail!(
            "SQL Server wrote {server_file} but the agent cannot read {}: bak_server_path and bak_agent_path must be the same folder ({error})",
            agent_file.display()
        );
    }

    let archive = backup_dir.join(format!("{}.tar.gz", cfg.generated_id));
    tar_gz_file_to(agent_file, &archive, logger).await?;
    Ok(archive)
}

pub async fn restore(cfg: &DatabaseConfig, restore_file: &Path, logger: Arc<JobLogger>) -> Result<()> {
    let (server_file, agent_file) = shared_file(cfg)?;
    let outcome = async {
        copy_to_shared(restore_file, &agent_file)
            .await
            .with_context(|| format!("Failed to copy the backup to {}", agent_file.display()))?;
        restore_from_shared(cfg, &server_file, &logger).await
    }
    .await;
    remove_shared_file(&agent_file, &logger).await;
    outcome
}

async fn copy_to_shared(restore_file: &Path, agent_file: &Path) -> Result<()> {
    let mut reader = BufReader::with_capacity(8 * 1024 * 1024, File::open(restore_file).await?);
    let mut writer = File::create(agent_file).await?;
    tokio::io::copy_buf(&mut reader, &mut writer).await?;
    writer.flush().await?;
    Ok(())
}

async fn restore_from_shared(cfg: &DatabaseConfig, server_file: &str, logger: &JobLogger) -> Result<()> {
    logger.log("info", format!("Starting MSSQL bak restore: {} → {}:{}/{}", server_file, cfg.host, cfg.port, cfg.database));
    let mut client = build_client(cfg, Some("master"))
        .await
        .with_context(|| format!("Failed to connect to master for {}", cfg.name))?;

    let backup_rows = client
        .query(FILE_LIST_SQL, &[&server_file])
        .await
        .with_context(|| format!("SQL Server cannot read {server_file}; nothing was changed"))?
        .into_first_result()
        .await
        .with_context(|| format!("SQL Server cannot read {server_file}; nothing was changed"))?;

    let mut target_results = client
        .query(TARGET_FILES_SQL, &[&cfg.database.as_str()])
        .await?
        .into_results()
        .await?;
    let default_paths = target_results
        .pop()
        .and_then(|rows| rows.into_iter().next())
        .context("SQL Server did not report its default paths")?;
    let existing_files: HashMap<String, String> = target_results
        .pop()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| Some((row.get::<&str, _>(0)?.to_string(), row.get::<&str, _>(1)?.to_string())))
        .collect();
    let data_directory: &str = default_paths.get(0).context("SQL Server did not report its default data path")?;
    let log_directory: &str = default_paths.get(1).context("SQL Server did not report its default log path")?;

    let relocations: Vec<(String, String)> = backup_rows
        .iter()
        .filter_map(|row| {
            let file = BackupFile {
                logical_name: row.get("LogicalName")?,
                physical_name: row.get("PhysicalName")?,
                kind: row.get("Type")?,
            };
            let path = target_path(&cfg.database, &file, &existing_files, data_directory, log_directory);
            Some((file.logical_name.to_string(), path))
        })
        .collect();

    let moves = (0..relocations.len())
        .map(|index| format!("MOVE @P{} TO @P{}", 3 + 2 * index, 4 + 2 * index))
        .collect::<Vec<_>>()
        .join(", ");
    let restore_sql = format!("{TAKE_OVER_SQL}\nRESTORE DATABASE @P1 FROM DISK = @P2 WITH REPLACE, {moves};");

    let database = cfg.database.as_str();
    let mut params: Vec<&dyn ToSql> = vec![&database, &server_file];
    for (logical_name, path) in &relocations {
        params.push(logical_name);
        params.push(path);
    }

    let start = Instant::now();
    if let Err(restore_error) = client.execute(restore_sql, &params).await {
        if let Err(release_error) = client.execute(RELEASE_SQL, &[&database]).await {
            logger.log("warn", format!("Could not set {database} back to MULTI_USER: {release_error}"));
        }
        anyhow::bail!("MSSQL restore failed for {}: {restore_error}", cfg.name);
    }
    logger.log_command(format!("RESTORE DATABASE [{database}] WITH REPLACE"), None, Some(0), Some(start.elapsed().as_millis() as f64));
    logger.log("info", format!("MSSQL restore completed for {}", cfg.name));
    Ok(())
}
