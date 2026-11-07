use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use tiberius::{AuthMethod, Client, Config};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

const MAX_SIDE_DATABASE_NAME: usize = 123;
const RUN_ID_LENGTH: usize = 8;
const CHECK_RESTORE_TARGET_SQL: &str = "IF DB_ID(@P1) <= 4
    THROW 50000, N'Refusing to restore over a system database; nothing was changed', 1;
IF DB_ID(@P1) IS NOT NULL AND ISNULL(HAS_PERMS_BY_NAME(NULL, 'DATABASE', 'CREATE DATABASE'), 0) = 0
    THROW 50000, N'Restore needs CREATE DATABASE permission (dbcreator) to create the restored database; nothing was changed', 1;
IF DB_ID(@P1) IS NOT NULL
    AND CAST(SERVERPROPERTY('EngineEdition') AS int) NOT IN (5, 8)
    AND ISNULL(HAS_PERMS_BY_NAME(@P1, 'DATABASE', 'CONTROL'), 0) = 0
    AND ISNULL(HAS_PERMS_BY_NAME(NULL, NULL, 'ALTER ANY DATABASE'), 0) = 0
    AND ISNULL(IS_SRVROLEMEMBER('dbcreator'), 0) = 0
    THROW 50000, N'Restore needs CONTROL on the target database, or ALTER ANY DATABASE, to replace it; nothing was changed', 1;
SELECT CASE WHEN DB_ID(@P1) IS NULL THEN 0 ELSE 1 END;";
const LEFTOVER_SIDE_DATABASES_SQL: &str = "SELECT name FROM sys.databases WHERE CHARINDEX(@P1, name) > 0";
const RENAME_DATABASE_SQL: &str = "DECLARE @from nvarchar(258) = QUOTENAME(@P1), @to nvarchar(258) = QUOTENAME(@P2);
DECLARE @single bit = 0;
IF CAST(SERVERPROPERTY('EngineEdition') AS int) NOT IN (5, 8)
BEGIN
    EXEC(N'ALTER DATABASE ' + @from + N' SET SINGLE_USER WITH ROLLBACK IMMEDIATE');
    SET @single = 1;
END
BEGIN TRY
    EXEC(N'ALTER DATABASE ' + @from + N' MODIFY NAME = ' + @to);
END TRY
BEGIN CATCH
    IF @single = 1
        EXEC(N'ALTER DATABASE ' + @from + N' SET MULTI_USER');
    THROW;
END CATCH
IF @single = 1
    EXEC(N'ALTER DATABASE ' + @to + N' SET MULTI_USER');";
const DROP_DATABASE_SQL: &str = "DECLARE @q nvarchar(258) = QUOTENAME(@P1);
IF DB_ID(@P1) IS NOT NULL
BEGIN
    IF DB_ID(@P1) <= 4
        THROW 50000, N'Refusing to restore over a system database; nothing was dropped', 1;
    IF ISNULL(HAS_PERMS_BY_NAME(NULL, 'DATABASE', 'CREATE DATABASE'), 0) = 0
        THROW 50000, N'Restore needs CREATE DATABASE permission (dbcreator) to recreate the target database; nothing was dropped', 1;
    DECLARE @single bit = 0;
    IF CAST(SERVERPROPERTY('EngineEdition') AS int) NOT IN (5, 8)
    BEGIN
        EXEC(N'ALTER DATABASE ' + @q + N' SET SINGLE_USER WITH ROLLBACK IMMEDIATE');
        SET @single = 1;
    END
    BEGIN TRY
        EXEC(N'DROP DATABASE ' + @q);
    END TRY
    BEGIN CATCH
        IF @single = 1 AND DB_ID(@P1) IS NOT NULL
            EXEC(N'ALTER DATABASE ' + @q + N' SET MULTI_USER');
        THROW;
    END CATCH
END";

pub async fn build_client(cfg: &DatabaseConfig, database: Option<&str>) -> Result<Client<Compat<TcpStream>>> {
    let mut config = Config::new();
    config.host(&cfg.host);
    config.port(cfg.port);
    if let Some(database) = database {
        config.database(database);
    }
    config.authentication(AuthMethod::sql_server(&cfg.username, &cfg.password));
    config.trust_cert();

    let tcp = TcpStream::connect(config.get_addr()).await?;
    tcp.set_nodelay(true)?;
    let client = Client::connect(config, tcp.compat_write()).await?;
    Ok(client)
}

async fn master_client(cfg: &DatabaseConfig) -> Result<Client<Compat<TcpStream>>> {
    build_client(cfg, Some("master"))
        .await
        .with_context(|| format!("Failed to connect to master for {}", cfg.name))
}

fn side_database_marker(cfg: &DatabaseConfig) -> String {
    format!("_restore_{}_", cfg.generated_id)
}

pub fn side_database_name(cfg: &DatabaseConfig) -> String {
    let marker = side_database_marker(cfg);
    let prefix_length = MAX_SIDE_DATABASE_NAME.saturating_sub(marker.len() + RUN_ID_LENGTH);
    let prefix: String = cfg.database.chars().take(prefix_length).collect();
    let run_id = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}{marker}{}", &run_id[..RUN_ID_LENGTH])
}

pub async fn check_restore_target(cfg: &DatabaseConfig) -> Result<bool> {
    let context = || format!("Cannot restore MSSQL database {}", cfg.database);
    let row = master_client(cfg)
        .await?
        .query(CHECK_RESTORE_TARGET_SQL, &[&cfg.database.as_str()])
        .await
        .with_context(context)?
        .into_row()
        .await
        .with_context(context)?;
    Ok(row.and_then(|row| row.get::<i32, _>(0)) == Some(1))
}

pub async fn leftover_side_databases(cfg: &DatabaseConfig) -> Result<Vec<String>> {
    let marker = side_database_marker(cfg);
    let rows = master_client(cfg)
        .await?
        .query(LEFTOVER_SIDE_DATABASES_SQL, &[&marker.as_str()])
        .await?
        .into_first_result()
        .await?;
    Ok(rows
        .iter()
        .filter_map(|row| row.get::<&str, _>(0).map(str::to_string))
        .collect())
}

pub async fn drop_database(cfg: &DatabaseConfig, database: &str) -> Result<()> {
    master_client(cfg)
        .await?
        .execute(DROP_DATABASE_SQL, &[&database])
        .await
        .with_context(|| format!("Failed to drop MSSQL database {database}"))?;
    Ok(())
}

pub async fn rename_database(cfg: &DatabaseConfig, from: &str, to: &str) -> Result<()> {
    master_client(cfg)
        .await?
        .execute(RENAME_DATABASE_SQL, &[&from, &to])
        .await
        .with_context(|| format!("Failed to rename MSSQL database {from} to {to}"))?;
    Ok(())
}
