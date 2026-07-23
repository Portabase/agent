use crate::domain::factory::DatabaseFactory;
use crate::domain::postgres::connection::{pg_restore_binary_name, select_pg_path, server_version};
use crate::domain::postgres::format::PostgresDumpFormat;
use crate::domain::postgres::restore::prepare_archive;
use crate::services::backup::logger::JobLogger;
use crate::services::config::{DatabaseConfig, DbType};
use crate::tests::init_tracing_for_test;
use crate::utils::compress::{compress_to_tar_gz_large, decompress_large_tar_gz};
use oauth2::url;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tracing::{error, info};
use url::Host;

async fn create_config() -> (ContainerAsync<Postgres>, DatabaseConfig) {
    let container = Postgres::default()
        .with_env_var("POSTGRES_DB", "testdb")
        .with_env_var("POSTGRES_USER", "testuser")
        .with_env_var("POSTGRES_PASSWORD", "changeme")
        .with_tag("17")
        .start()
        .await
        .unwrap();

    let host = container
        .get_host()
        .await
        .unwrap_or(Host::parse("127.0.0.1").unwrap());

    let port = container.get_host_port_ipv4(5432).await.unwrap_or(5432);

    let config = DatabaseConfig {
        name: "My test Postgres Database".to_string(),
        database: "testdb".to_string(),
        db_type: DbType::Postgresql,
        username: "testuser".to_string(),
        password: "changeme".to_string(),
        port,
        host: host.to_string(),
        generated_id: "40875631-e3d2-4dfe-a26b-2a347ecc64fd".to_string(),
        path: "".to_string(),
        max_packet_size: "".to_string(),
        volume_name: "".to_string(),
        container_name: None,
        options: std::collections::HashMap::new(),
    };

    (container, config)
}

#[tokio::test]
async fn postgres_ping_test() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;

    let db = DatabaseFactory::create_for_backup(config.clone()).await;
    let reachable = db.ping().await.unwrap_or_else(|_| false);

    assert_eq!(reachable, true);
}

#[tokio::test]
async fn is_superuser_detects_superuser_role() {
    init_tracing_for_test();

    // The testcontainer's POSTGRES_USER ("testuser") is the bootstrap superuser.
    let (_container, config) = create_config().await;

    let is_super = crate::domain::postgres::connection::is_superuser(&config)
        .await
        .unwrap();

    assert!(is_super);
}

#[tokio::test]
async fn postgres_backup_restore_test() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;

    let temp_dir = TempDir::new().unwrap();
    let backup_path = temp_dir.path();

    let db = DatabaseFactory::create_for_backup(config.clone()).await;

    let file_path = db.backup(backup_path, std::sync::Arc::new(crate::services::backup::logger::JobLogger::new())).await.unwrap();

    assert!(file_path.is_file());

    let compression = compress_to_tar_gz_large(&file_path, std::sync::Arc::new(crate::services::backup::logger::JobLogger::new())).await.unwrap();

    assert!(compression.compressed_path.is_file());

    let files = decompress_large_tar_gz(compression.compressed_path.as_path(), temp_dir.path())
        .await
        .unwrap();

    let backup_file: PathBuf;

    if files.len() == 1 {
        backup_file = files[0].clone()
    } else {
        backup_file = "".into()
    }

    let db = DatabaseFactory::create_for_restore(config.clone(), &backup_file).await;

    let reachable = db.ping().await.unwrap_or(false);

    info!("Reachable: {}", reachable);

    assert_eq!(reachable, true);

    info!("Running pg_restore: {:?}", backup_file);

    match db.restore(&backup_file, std::sync::Arc::new(crate::services::backup::logger::JobLogger::new())).await {
        Ok(_) => {
            info!("Restore succeeded for {}", config.generated_id);
            assert!(true)
        }
        Err(e) => {
            error!("Restore failed for {}: {:?}", config.generated_id, e);
            assert!(false)
        }
    }
}

#[tokio::test]
async fn postgres_password_with_slash_test() {
    init_tracing_for_test();

    let special_password = "ch/ange:me@1";

    let container = Postgres::default()
        .with_env_var("POSTGRES_DB", "testdb")
        .with_env_var("POSTGRES_USER", "testuser")
        .with_env_var("POSTGRES_PASSWORD", special_password)
        .with_tag("17")
        .start()
        .await
        .unwrap();

    let host = container
        .get_host()
        .await
        .unwrap_or(Host::parse("127.0.0.1").unwrap());

    let port = container.get_host_port_ipv4(5432).await.unwrap_or(5432);

    let config = DatabaseConfig {
        name: "My test Postgres Database with slash password".to_string(),
        database: "testdb".to_string(),
        db_type: DbType::Postgresql,
        username: "testuser".to_string(),
        password: special_password.to_string(),
        port,
        host: host.to_string(),
        generated_id: "5a1f0e3c-9b8a-4a8e-9b1b-0a1c2d3e4f5a".to_string(),
        path: "".to_string(),
        max_packet_size: "".to_string(),
        volume_name: "".to_string(),
        container_name: None,
        options: std::collections::HashMap::new(),
    };

    let db = DatabaseFactory::create_for_backup(config.clone()).await;
    let reachable = db.ping().await.unwrap_or(false);

    assert_eq!(reachable, true);
}

fn pg_dump_env(config: &DatabaseConfig) -> std::collections::HashMap<String, String> {
    let mut env = std::env::vars().collect::<std::collections::HashMap<_, _>>();
    env.insert("PGPASSWORD".to_string(), config.password.clone());
    env
}

#[tokio::test]
async fn prepare_archive_fd_locates_toc_dir() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;

    let temp_dir = TempDir::new().unwrap();

    let backup_path = crate::domain::postgres::backup::run(
        config.clone(),
        PostgresDumpFormat::Fd,
        temp_dir.path().to_path_buf(),
        pg_dump_env(&config),
        Arc::new(JobLogger::new()),
    )
    .await
    .unwrap();

    let compression = compress_to_tar_gz_large(&backup_path, Arc::new(JobLogger::new()))
        .await
        .unwrap();

    assert!(compression.compressed_path.is_file());

    let version = server_version(&config).await.unwrap();
    let pg_restore = select_pg_path(&version).join(pg_restore_binary_name());

    let logger = JobLogger::new();

    let prepared = prepare_archive(
        PostgresDumpFormat::Fd,
        &compression.compressed_path,
        &pg_restore,
        &logger,
    )
    .unwrap();

    assert!(prepared.path().join("toc.dat").exists());
    assert!(!prepared.toc().is_empty());
}

#[tokio::test]
async fn prepare_archive_fc_returns_file_path_unchanged() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;

    let temp_dir = TempDir::new().unwrap();

    let backup_path = crate::domain::postgres::backup::run(
        config.clone(),
        PostgresDumpFormat::Fc,
        temp_dir.path().to_path_buf(),
        pg_dump_env(&config),
        Arc::new(JobLogger::new()),
    )
    .await
    .unwrap();

    assert!(backup_path.is_file());

    let version = server_version(&config).await.unwrap();
    let pg_restore = select_pg_path(&version).join(pg_restore_binary_name());

    let logger = JobLogger::new();

    let prepared = prepare_archive(PostgresDumpFormat::Fc, &backup_path, &pg_restore, &logger).unwrap();

    assert_eq!(prepared.path(), backup_path.as_path());
    assert!(!prepared.toc().is_empty());
}

#[tokio::test]
async fn restore_run_unified_fc_roundtrip() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;

    let client = crate::domain::postgres::connection::connect(&config)
        .await
        .unwrap();
    client.execute("CREATE TABLE t(id int);", &[]).await.unwrap();

    let temp_dir = TempDir::new().unwrap();

    let dump_file = crate::domain::postgres::backup::run(
        config.clone(),
        PostgresDumpFormat::Fc,
        temp_dir.path().to_path_buf(),
        pg_dump_env(&config),
        Arc::new(JobLogger::new()),
    )
    .await
    .unwrap();

    assert!(dump_file.is_file());

    let format = crate::domain::postgres::connection::detect_format_from_file(&dump_file);

    let result = crate::domain::postgres::restore::run(
        config.clone(),
        format,
        dump_file,
        pg_dump_env(&config),
        Arc::new(JobLogger::new()),
    )
    .await;

    assert!(result.is_ok(), "restore::run failed: {:?}", result);
}

#[tokio::test]
async fn drop_all_schemas_removes_user_schema() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;

    let client = crate::domain::postgres::connection::connect(&config)
        .await
        .unwrap();
    client
        .batch_execute("CREATE SCHEMA IF NOT EXISTS extra_ns; CREATE TABLE IF NOT EXISTS extra_ns.t(id int);")
        .await
        .unwrap();

    let dropped = crate::domain::postgres::connection::drop_all_schemas(&config)
        .await
        .unwrap();
    assert!(dropped.iter().any(|s| s == "extra_ns"));

    let client = crate::domain::postgres::connection::connect(&config)
        .await
        .unwrap();
    let row = client
        .query_one(
            "SELECT count(*) FROM pg_namespace WHERE nspname = 'extra_ns'",
            &[],
        )
        .await
        .unwrap();
    let n: i64 = row.get(0);
    assert_eq!(n, 0);
}

mod select_pg_path_tests {
    use crate::domain::postgres::connection::{
        pg_dump_binary_name, pg_dump_exists_in, pg_dumpall_binary_name, pg_restore_binary_name,
        psql_binary_name, select_pg_path_with,
    };

    // `select_pg_path_with` takes the `PG_BIN_DIR` override as a plain
    // argument, so these tests never touch process-global env state or the
    // cached `CONFIG`. They stay deterministic regardless of whether — or at
    // which version — a real PostgreSQL install exists on the host.

    #[test]
    fn respects_pg_bin_dir_override() {
        let custom = if cfg!(target_os = "windows") {
            r"C:\custom\pg\bin"
        } else {
            "/custom/pg/bin"
        };
        let path = select_pg_path_with("16.4", custom);
        assert_eq!(path, std::path::PathBuf::from(custom));
    }

    #[test]
    fn pg_bin_dir_override_ignores_requested_version() {
        // The override is taken as-is, regardless of which version was
        // requested — this documents/locks in that behavior.
        let custom = if cfg!(target_os = "windows") {
            r"C:\custom\pg\bin"
        } else {
            "/custom/pg/bin"
        };
        let path = select_pg_path_with("not-a-version", custom);
        assert_eq!(path, std::path::PathBuf::from(custom));
    }

    #[test]
    fn empty_pg_bin_dir_falls_through_to_detection() {
        // An empty override means "unset" (matches `CONFIG.pg_bin_dir` when
        // `PG_BIN_DIR` is absent). It must not be returned as a literal empty
        // path — resolution falls through to platform defaults / PATH lookup
        // and yields a non-empty path.
        let path = select_pg_path_with("17", "");
        assert_ne!(path, std::path::PathBuf::from(""));
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

    #[test]
    fn pg_dumpall_binary_name_is_platform_specific() {
        let name = pg_dumpall_binary_name();
        if cfg!(target_os = "windows") {
            assert_eq!(name, "pg_dumpall.exe");
        } else {
            assert_eq!(name, "pg_dumpall");
        }
    }

    #[test]
    fn psql_binary_name_is_platform_specific() {
        let name = psql_binary_name();
        if cfg!(target_os = "windows") {
            assert_eq!(name, "psql.exe");
        } else {
            assert_eq!(name, "psql");
        }
    }

    #[test]
    fn pg_restore_binary_name_is_platform_correct() {
        let name = pg_restore_binary_name();
        if cfg!(target_os = "windows") {
            assert_eq!(name, "pg_restore.exe");
        } else {
            assert_eq!(name, "pg_restore");
        }
    }
}

mod quoting_tests {
    use crate::domain::postgres::connection::{quote_ident, quote_literal};

    #[test]
    fn quote_ident_escapes_double_quotes() {
        assert_eq!(quote_ident("devdb"), "\"devdb\"");
        assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_ident("drop\"; --"), "\"drop\"\"; --\"");
    }

    #[test]
    fn quote_literal_escapes_single_quotes() {
        assert_eq!(quote_literal("UTF8"), "'UTF8'");
        assert_eq!(quote_literal("O'Brien"), "'O''Brien'");
    }
}

mod clean_mode_tests {
    use crate::domain::postgres::clean_mode::RestoreCleanMode as M;
    use crate::services::config::{DatabaseConfig, DbType};

    fn cfg_with(clean_mode: Option<&str>) -> DatabaseConfig {
        let mut options = std::collections::HashMap::new();
        if let Some(v) = clean_mode {
            options.insert("clean_mode".to_string(), serde_json::json!(v));
        }
        DatabaseConfig {
            name: "t".into(),
            database: "testdb".into(),
            db_type: DbType::Postgresql,
            username: "testuser".into(),
            password: "changeme".into(),
            port: 5432,
            host: "localhost".into(),
            generated_id: "00000000-0000-0000-0000-000000000000".into(),
            path: "".into(),
            max_packet_size: "".into(),
            volume_name: "".into(),
            container_name: None,
            options,
        }
    }

    #[test]
    fn clean_mode_parsing() {
        assert_eq!(M::from_config(&cfg_with(None)), (M::Clean, None));
        assert_eq!(M::from_config(&cfg_with(Some("clean"))), (M::Clean, None));
        assert_eq!(M::from_config(&cfg_with(Some("none"))), (M::None, None));
        assert_eq!(
            M::from_config(&cfg_with(Some("drop_schemas"))),
            (M::DropSchemas, None)
        );
        assert_eq!(
            M::from_config(&cfg_with(Some("drop_database"))),
            (M::DropDatabase, None)
        );
        assert_eq!(
            M::from_config(&cfg_with(Some("bogus"))),
            (M::Clean, Some("bogus".to_string()))
        );
    }

    #[test]
    fn uses_pg_restore_clean_behavior() {
        assert!(M::Clean.uses_pg_restore_clean());
        assert!(!M::DropSchemas.uses_pg_restore_clean());
        assert!(!M::None.uses_pg_restore_clean());
        assert!(!M::DropDatabase.uses_pg_restore_clean());
    }
}

mod toc_tests {
    use crate::domain::postgres::restore::toc_creates_public_schema;

    #[test]
    fn toc_public_schema_detection() {
        let with = "123; 2615 12345 SCHEMA - public";
        let without = "200; 1259 12346 TABLE devschema users devuser";
        assert!(toc_creates_public_schema(with));
        assert!(!toc_creates_public_schema(without));
    }
}
