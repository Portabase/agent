use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::storage::get_provider;
use crate::services::storage::providers::ftp::helpers::build_ftp_config;
use crate::services::storage::providers::ftp::models::{FtpProviderConfig, FtpsMode};

fn storage(config: serde_json::Value) -> DatabaseStorage {
    serde_json::from_value(serde_json::json!({"id":"storage-1","provider":"ftp","folderName":"backups","config":config})).unwrap()
}

#[test]
fn config_deserializes_dashboard_camel_case_and_explicit_ftps() {
    let config: FtpProviderConfig = storage(serde_json::json!({"host":"ftp.example.com","port":21,"username":"backup","password":"secret","remotePath":"/srv/backups","ftpsMode":"explicit"})).config.try_into().unwrap();
    assert_eq!(config.port.as_deref(), Some("21"));
    assert_eq!(config.ftps_mode, FtpsMode::Explicit);
    let text = build_ftp_config(&config).unwrap();
    assert!(text.contains("type = ftp"));
    assert!(text.contains("explicit_tls = true"));
    assert!(!text.contains("\ntls = true"));
}

#[test]
fn implicit_ftps_emits_only_tls() {
    let config = FtpProviderConfig {
        host: "h".into(),
        port: Some("990".into()),
        username: "u".into(),
        password: "p".into(),
        remote_path: String::new(),
        ftps_mode: FtpsMode::Implicit,
    };
    let text = build_ftp_config(&config).unwrap();
    assert!(text.contains("tls = true"));
    assert!(!text.contains("explicit_tls"));
}

#[test]
fn factory_resolves_ftp_and_builder_rejects_blank_credentials() {
    assert!(
        get_provider(&storage(
            serde_json::json!({"host":"h","username":"u","password":"p"})
        ))
        .is_some()
    );
    let config = FtpProviderConfig {
        host: " ".into(),
        port: None,
        username: "u".into(),
        password: "p".into(),
        remote_path: String::new(),
        ftps_mode: FtpsMode::None,
    };
    assert!(
        build_ftp_config(&config)
            .unwrap_err()
            .to_string()
            .contains("host")
    );
}
