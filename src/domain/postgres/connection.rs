use crate::domain::postgres::format::PostgresDumpFormat;
use crate::services::config::DatabaseConfig;
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
pub fn select_pg_path(version: &str) -> std::path::PathBuf {
    let major = version.split('.').next().unwrap_or("17");

    if let Ok(dir) = std::env::var("PG_BIN_DIR") {
        return dir.into();
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

fn pg_dump_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "pg_dump.exe"
    } else {
        "pg_dump"
    }
}

fn pg_dump_exists_in(dir: &std::path::Path) -> bool {
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

#[cfg(test)]
mod select_pg_path_tests {
    use super::*;
    use std::sync::Mutex;

    // `std::env::set_var`/`remove_var` are process-global and, as of the
    // 2024 edition, marked `unsafe` because mutating them concurrently
    // from multiple threads is undefined behavior. Rust runs tests in
    // parallel by default, so without serializing access here, these
    // tests could race against each other over `PG_BIN_DIR`.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    // Tests below intentionally avoid asserting on whether a *real*
    // PostgreSQL install is or isn't found on the machine running the
    // tests (CI runners and developer machines may or may not have one,
    // at any version) — that would make the tests environment-dependent
    // and flaky. Instead, `PG_BIN_DIR` is always set to a deterministic
    // value so behavior doesn't depend on the local system.

    #[test]
    fn respects_pg_bin_dir_override() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let custom = if cfg!(target_os = "windows") {
            r"C:\custom\pg\bin"
        } else {
            "/custom/pg/bin"
        };
        // SAFETY: serialized via ENV_LOCK.
        unsafe {
            std::env::set_var("PG_BIN_DIR", custom);
        }
        let path = select_pg_path("16.4");
        // SAFETY: serialized via ENV_LOCK.
        unsafe {
            std::env::remove_var("PG_BIN_DIR");
        }
        assert_eq!(path, std::path::PathBuf::from(custom));
    }

    #[test]
    fn pg_bin_dir_override_ignores_requested_version() {
        // The override is taken as-is, regardless of which version was
        // requested — this documents/locks in that behavior.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let custom = if cfg!(target_os = "windows") {
            r"C:\custom\pg\bin"
        } else {
            "/custom/pg/bin"
        };
        // SAFETY: serialized via ENV_LOCK.
        unsafe {
            std::env::set_var("PG_BIN_DIR", custom);
        }
        let path = select_pg_path("not-a-version");
        // SAFETY: serialized via ENV_LOCK.
        unsafe {
            std::env::remove_var("PG_BIN_DIR");
        }
        assert_eq!(path, std::path::PathBuf::from(custom));
    }

    #[test]
    fn pg_dump_binary_name_is_platform_specific() {
        let name = pg_dump_binary_name();
        if cfg!(target_os = "windows") {
            assert_eq!(name, "pg_dump.exe");
        } else {
            assert_eq!(name, "pg_dump");
        }
    }

    #[test]
    fn pg_dump_exists_in_is_false_for_nonexistent_dir() {
        let dir = std::path::Path::new("this/path/almost-certainly/does-not-exist-12345");
        assert!(!pg_dump_exists_in(dir));
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
