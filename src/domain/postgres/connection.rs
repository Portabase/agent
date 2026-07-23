use crate::domain::postgres::format::PostgresDumpFormat;
use crate::services::config::DatabaseConfig;
use crate::settings::CONFIG;
use anyhow::Result;
use std::path::Path;
use tokio_postgres::{Client, Config, NoTls};
use tracing::{error, info};

pub async fn connect(cfg: &DatabaseConfig) -> Result<Client> {
    info!("Connecting to postgres database {}:{}", cfg.host, cfg.port);

    let mut config = Config::new();
    config
        .host(&cfg.host)
        .port(cfg.port)
        .user(&cfg.username)
        .password(&cfg.password)
        .dbname(&cfg.database);

    let (client, connection) = config.connect(NoTls).await?;
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

pub async fn is_superuser(cfg: &DatabaseConfig) -> Result<bool> {
    let client = connect(cfg).await?;
    let is_super: bool = client
        .query_one("SELECT current_setting('is_superuser') = 'on';", &[])
        .await?
        .get(0);

    Ok(is_super)
}


pub fn select_pg_path(version: &str) -> std::path::PathBuf {
    select_pg_path_with(version, &CONFIG.pg_bin_dir)
}

pub(crate) fn select_pg_path_with(version: &str, pg_bin_dir: &str) -> std::path::PathBuf {
    let major = version.split('.').next().unwrap_or("17");

    if !pg_bin_dir.is_empty() {
        return pg_bin_dir.into();
    }

    let candidates: Vec<std::path::PathBuf> = if cfg!(target_os = "windows") {
        vec![
            // Default install path used by the official EDB Windows installer
            format!(r"C:\Program Files\PostgreSQL\{major}\bin").into(),
            format!(r"C:\Program Files (x86)\PostgreSQL\{major}\bin").into(),
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            // Homebrew on Apple Silicon
            format!("/opt/homebrew/opt/postgresql@{major}/bin").into(),
            // Homebrew on Intel
            format!("/usr/local/opt/postgresql@{major}/bin").into(),
            // Postgres.app
            format!("/Applications/Postgres.app/Contents/Versions/{major}/bin").into(),
        ]
    } else {
        vec![
            // Debian/Ubuntu packages
            format!("/usr/lib/postgresql/{major}/bin").into(),
            // Common RPM-based distro layout
            format!("/usr/pgsql-{major}/bin").into(),
        ]
    };

    if let Some(found) = candidates.into_iter().find(|p| pg_dump_exists_in(p)) {
        return found;
    }

    if let Some(dir) = find_pg_dump_in_path() {
        return dir;
    }

    format!("/usr/lib/postgresql/{}/bin", major).into()
}

pub(crate) fn pg_dump_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "pg_dump.exe"
    } else {
        "pg_dump"
    }
}

pub(crate) fn pg_dumpall_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "pg_dumpall.exe"
    } else {
        "pg_dumpall"
    }
}

pub(crate) fn psql_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "psql.exe"
    } else {
        "psql"
    }
}

pub(crate) fn pg_restore_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "pg_restore.exe"
    } else {
        "pg_restore"
    }
}

pub(crate) fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

pub(crate) fn quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

pub(crate) fn pg_dump_exists_in(dir: &std::path::Path) -> bool {
    dir.join(pg_dump_binary_name()).is_file()
}

fn find_pg_dump_in_path() -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var).find(|dir| pg_dump_exists_in(dir))
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

pub async fn terminate_all_connections(cfg: &DatabaseConfig) -> Result<()> {
    let mut admin = cfg.clone();
    admin.database = "postgres".to_string().into();

    let client = connect(&admin).await?;

    client
        .execute(
            r#"
            SELECT pg_terminate_backend(pid)
            FROM pg_stat_activity
            WHERE datname NOT IN ('postgres', 'template0', 'template1')
              AND pid <> pg_backend_pid();
            "#,
            &[],
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

pub async fn drop_all_schemas(cfg: &DatabaseConfig) -> Result<Vec<String>> {
    let client = connect(cfg).await?;
    let rows = client
        .query(
            r#"
            SELECT nspname FROM pg_namespace
            WHERE nspname NOT IN ('pg_catalog', 'information_schema', 'pg_toast')
              AND nspname NOT LIKE 'pg\_temp\_%'
              AND nspname NOT LIKE 'pg\_toast\_temp\_%'
            ORDER BY nspname
            "#,
            &[],
        )
        .await?;
    let schemas: Vec<String> = rows.iter().map(|r| r.get::<_, String>(0)).collect();
    for s in &schemas {
        client
            .batch_execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE", quote_ident(s)))
            .await?;
    }
    client
        .batch_execute("SELECT lo_unlink(oid) FROM pg_largeobject_metadata")
        .await
        .ok();
    Ok(schemas)
}

pub async fn recreate_public_schema(cfg: &DatabaseConfig, owner: &str) -> Result<()> {
    let client = connect(cfg).await?;
    client
        .batch_execute(&format!(
            "CREATE SCHEMA IF NOT EXISTS public AUTHORIZATION {}; GRANT USAGE ON SCHEMA public TO PUBLIC;",
            quote_ident(owner)
        ))
        .await?;
    Ok(())
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
