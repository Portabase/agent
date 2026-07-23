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

pub async fn server_version_major(cfg: &DatabaseConfig) -> Result<u32> {
    let v = server_version(cfg).await?;
    Ok(v.split(['.', ' '])
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(17))
}

pub async fn is_superuser(cfg: &DatabaseConfig) -> Result<bool> {
    let client = connect(cfg).await?;
    let is_super: bool = client
        .query_one("SELECT current_setting('is_superuser') = 'on';", &[])
        .await?
        .get(0);

    Ok(is_super)
}

pub async fn can_drop_database(cfg: &DatabaseConfig) -> Result<bool> {
    let client = connect(cfg).await?;
    let row = client
        .query_one(
            "SELECT r.rolsuper OR (r.rolcreatedb AND pg_catalog.pg_has_role(current_user, d.datdba, 'USAGE')) \
             FROM pg_roles r, pg_database d \
             WHERE r.rolname = current_user AND d.datname = current_database()",
            &[],
        )
        .await?;
    Ok(row.get(0))
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

pub async fn drop_and_recreate_database(cfg: &DatabaseConfig) -> Result<()> {
    let mut admin_cfg = cfg.clone();
    admin_cfg.database = "postgres".to_string();
    let admin = connect(&admin_cfg).await?;

    let row = admin
        .query_opt(
            r#"
            SELECT pg_encoding_to_char(encoding), datcollate, datctype,
                   pg_get_userbyid(datdba), datistemplate
            FROM pg_database WHERE datname = $1
            "#,
            &[&cfg.database],
        )
        .await?;

    let (encoding, collate, ctype, owner) = match &row {
        Some(r) => (
            r.get::<_, String>(0),
            r.get::<_, String>(1),
            r.get::<_, String>(2),
            r.get::<_, String>(3),
        ),
        None => ("UTF8".into(), "C".into(), "C".into(), cfg.username.clone()),
    };

    if let Some(r) = &row {
        if r.get::<_, bool>(4) {
            anyhow::bail!("Refusing to drop template database {}", cfg.database);
        }
    }

    let db = quote_ident(&cfg.database);

    if let Err(e) = admin
        .batch_execute(&format!("ALTER DATABASE {db} WITH ALLOW_CONNECTIONS false"))
        .await
    {
        tracing::warn!("ALLOW_CONNECTIONS false failed for {}: {e}", cfg.database);
    }

    let major = server_version_major(&admin_cfg).await?;
    let drop_stmt = if major >= 13 {
        format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)")
    } else {
        format!("DROP DATABASE IF EXISTS {db}")
    };

    let mut last_err = None;
    let mut dropped = false;
    for _ in 0..3 {
        let _ = terminate_connections(cfg).await;
        match admin.batch_execute(&drop_stmt).await {
            Ok(()) => {
                dropped = true;
                break;
            }
            Err(e) => {
                last_err = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }

    if !dropped {
        let _ = admin
            .batch_execute(&format!("ALTER DATABASE {db} WITH ALLOW_CONNECTIONS true"))
            .await;
        return Err(last_err
            .map(anyhow::Error::from)
            .unwrap_or_else(|| anyhow::anyhow!("DROP DATABASE {} failed", cfg.database)));
    }

    admin
        .batch_execute(&format!(
            "CREATE DATABASE {db} OWNER {} TEMPLATE template0 ENCODING {} LC_COLLATE {} LC_CTYPE {}",
            quote_ident(&owner),
            quote_literal(&encoding),
            quote_literal(&collate),
            quote_literal(&ctype),
        ))
        .await?;

    Ok(())
}

pub fn sniff_format(restore_file: &Path) -> Result<PostgresDumpFormat> {
    use std::io::Read;
    let mut f = std::fs::File::open(restore_file)?;
    let mut magic = [0u8; 5];
    let n = f.read(&mut magic)?;
    let head = &magic[..n];
    if head.starts_with(b"PGDMP") {
        Ok(PostgresDumpFormat::Fc)
    } else if head.starts_with(&[0x1f, 0x8b]) {
        Ok(PostgresDumpFormat::Fd)
    } else {
        anyhow::bail!("Unrecognized dump format for {:?}", restore_file)
    }
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
