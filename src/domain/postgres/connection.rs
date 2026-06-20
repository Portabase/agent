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

/// Resolves the `bin` directory of a PostgreSQL installation for the given
/// major version, in a cross-platform way.
///
/// Resolution order:
/// 1. The `PG_BIN_DIR` environment variable, if set, is used as-is. This
///    allows users/CI to override detection for non-standard installs
///    (e.g. portable PostgreSQL distributions, custom install locations).
/// 2. Platform-specific default install locations (Debian/Ubuntu packages,
///    the official Windows installer, Homebrew/Postgres.app on macOS, and
///    common RPM-based layouts on other Linux distros).
/// 3. A `PATH` lookup for `pg_dump` (`pg_dump.exe` on Windows), returning
///    its parent directory.
/// 4. The historical Debian/Ubuntu path as a last-resort fallback, so the
///    function keeps returning a `PathBuf` (never panics) even when nothing
///    was found, preserving the previous behavior for callers.
///
/// The override is sourced from `CONFIG.pg_bin_dir` (the `PG_BIN_DIR`
/// environment variable). An empty value means "unset" and falls through to
/// detection.
pub fn select_pg_path(version: &str) -> std::path::PathBuf {
    select_pg_path_with(version, &CONFIG.pg_bin_dir)
}

/// Inner resolver behind [`select_pg_path`], parameterized over the
/// `PG_BIN_DIR` override. Kept pure (no env / no `CONFIG` access) so it is
/// unit-testable without mutating process-global state.
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
