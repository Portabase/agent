use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use tiberius::{AuthMethod, Client, Config};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

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

pub async fn drop_database(cfg: &DatabaseConfig) -> Result<()> {
    let mut client = build_client(cfg, Some("master"))
        .await
        .with_context(|| format!("Failed to connect to master for {}", cfg.name))?;
    client
        .execute(DROP_DATABASE_SQL, &[&cfg.database.as_str()])
        .await
        .with_context(|| format!("Failed to drop MSSQL database {}", cfg.database))?;
    Ok(())
}
