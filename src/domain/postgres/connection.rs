use crate::domain::postgres::format::PostgresDumpFormat;
use crate::services::config::DatabaseConfig;
use anyhow::Result;
use std::path::Path;
use tokio_postgres::{Client, NoTls};
use tracing::{error, info};

pub async fn connect(cfg: &DatabaseConfig) -> Result<Client> {
    info!("Connecting to postgres database {}:{}", cfg.host, cfg.port);
    let dsn = format!(
        "host={} port={} user={} password={} dbname={}",
        cfg.host, cfg.port, cfg.username, cfg.password, cfg.database
    );

    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            error!("Postgres connection error: {}", e);
        }
    });
    Ok(client)
}

pub async fn server_version(cfg: &DatabaseConfig) -> Result<String> {
    let client = connect(cfg).await?;
    let version: String = client.query_one("SHOW server_version;", &[]).await?.get(0);

    Ok(version)
}

pub fn select_pg_path(version: &str) -> std::path::PathBuf {
    let major = version.split('.').next().unwrap_or("17");
    format!("/usr/lib/postgresql/{}/bin", major).into()
}

pub async fn terminate_connections(cfg: &DatabaseConfig) -> Result<()> {
    let mut admin = cfg.clone();
    admin.database = "postgres".to_string().into();

    let client = connect(&admin).await?;

    client
        .execute(
            r#"
            SELECT pg_terminate_backend(pid)
            FROM pg_stat_activity
            WHERE datname = $1
              AND pid <> pg_backend_pid();
            "#,
            &[&cfg.database],
        )
        .await?;

    Ok(())
}

pub fn detect_format_from_file(restore_file: &Path) -> PostgresDumpFormat {
    match restore_file.extension().and_then(|e| e.to_str()) {
        Some("dump") => PostgresDumpFormat::Fc,
        Some("gz") => PostgresDumpFormat::Fd,
        // Some("tar.gz") => PostgresDumpFormat::Fd,
        _ => PostgresDumpFormat::Fc,
    }
}

pub async fn detect_format_from_size(cfg: &DatabaseConfig) -> PostgresDumpFormat {
    info!(
        "Detecting database format {:?} - {:?}",
        cfg.name, cfg.generated_id
    );
    let client = match connect(cfg).await {
        Ok(c) => c,
        Err(_) => return PostgresDumpFormat::Fc,
    };

    let row = match client
        .query_one("SELECT pg_database_size(current_database());", &[])
        .await
    {
        Ok(r) => r,
        Err(_) => return PostgresDumpFormat::Fc,
    };

    let size_bytes: i64 = row.get(0);
    info!("Size of database is {} bytes", size_bytes);

    // > 1 Go
    if size_bytes > 1_000_000_000 {
        info!("Using -Fd format");
        PostgresDumpFormat::Fd
    } else {
        info!("Using -Fc format");
        PostgresDumpFormat::Fc
    }
}
