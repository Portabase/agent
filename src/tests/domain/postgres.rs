use crate::domain::factory::DatabaseFactory;
use crate::services::config::{DatabaseConfig, DbType};
use crate::tests::init_tracing_for_test;
use crate::utils::compress::{compress_to_tar_gz_large, decompress_large_tar_gz};
use oauth2::url;
use std::path::PathBuf;
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
        include_globals: false,
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
async fn backup_with_include_globals_produces_a_manifest_bundle() {
    init_tracing_for_test();

    let (_container, mut config) = create_config().await;
    config.include_globals = true;

    let temp_dir = TempDir::new().unwrap();
    let backup_path = temp_dir.path();

    let db = DatabaseFactory::create_for_backup(config.clone()).await;
    let file_path = db
        .backup(backup_path, std::sync::Arc::new(crate::services::backup::logger::JobLogger::new()))
        .await
        .unwrap();

    assert!(file_path.to_string_lossy().ends_with(".tar.gz"));

    let extract_dir = TempDir::new().unwrap();
    let files = decompress_large_tar_gz(&file_path, extract_dir.path()).await.unwrap();
    assert!(files.len() >= 3, "expected dump + globals.sql + manifest.json, got {:?}", files);

    let manifest = crate::domain::postgres::bundle::BundleManifest::read(extract_dir.path())
        .unwrap()
        .expect("manifest.json must be present when include_globals is true");
    assert_eq!(manifest.has_globals, true);
    assert_eq!(manifest.format, "fc");

    assert!(extract_dir.path().join("globals.sql").is_file());
    assert!(extract_dir.path().join(&manifest.dump_path).exists());
}

#[tokio::test]
async fn backup_without_include_globals_is_unaffected() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;
    assert_eq!(config.include_globals, false);

    let temp_dir = TempDir::new().unwrap();
    let backup_path = temp_dir.path();

    let db = DatabaseFactory::create_for_backup(config.clone()).await;
    let file_path = db
        .backup(backup_path, std::sync::Arc::new(crate::services::backup::logger::JobLogger::new()))
        .await
        .unwrap();

    // Same artifact shape as before this feature existed: a bare `.dump`
    // file, not a bundle.
    assert!(file_path.to_string_lossy().ends_with(".dump"));
}

#[tokio::test]
async fn globals_dump_then_apply_round_trips() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;
    let temp_dir = TempDir::new().unwrap();

    let version = crate::domain::postgres::connection::server_version(&config)
        .await
        .unwrap();

    let mut env = std::env::vars().collect::<std::collections::HashMap<_, _>>();
    env.insert("PGPASSWORD".to_string(), config.password.clone());

    let logger = std::sync::Arc::new(crate::services::backup::logger::JobLogger::new());

    let globals_path = crate::domain::postgres::globals::dump(
        &config,
        &version,
        temp_dir.path(),
        &env,
        &logger,
    )
    .unwrap();

    assert!(globals_path.is_file());
    let contents = std::fs::read_to_string(&globals_path).unwrap();
    assert!(contents.contains("ROLE"), "expected role statements in globals.sql, got: {contents}");

    // Re-applying an already-applied globals.sql must not error out the
    // caller: roles/tablespaces already existing on the cluster are expected
    // and must be swallowed as warnings, not failures.
    crate::domain::postgres::globals::apply(&config, &version, &globals_path, &env, &logger);
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
        include_globals: false,
    };

    let db = DatabaseFactory::create_for_backup(config.clone()).await;
    let reachable = db.ping().await.unwrap_or(false);

    assert_eq!(reachable, true);
}

mod select_pg_path_tests {
    use crate::domain::postgres::connection::{
        pg_dump_binary_name, pg_dump_exists_in, pg_dumpall_binary_name, psql_binary_name,
        select_pg_path_with,
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
}
