use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::storage::providers::sftp::helpers::build_sftp_config;
use crate::services::storage::providers::sftp::models::SftpProviderConfig;
use crate::services::storage::get_provider;

fn storage(config: serde_json::Value) -> DatabaseStorage {
    serde_json::from_value(serde_json::json!({
        "id": "storage-1",
        "provider": "sftp",
        "folderName": "backups",
        "config": config,
    }))
    .unwrap()
}

#[test]
fn config_deserializes_from_dashboard_camel_case() {
    let s = storage(serde_json::json!({
        "host": "backup.example.com",
        "port": 2222,
        "username": "deploy",
        "privateKey": "-----BEGIN KEY-----",
        "remotePath": "/srv/backups",
    }));
    let config: SftpProviderConfig = s.config.try_into().unwrap();
    assert_eq!(config.host, "backup.example.com");
    assert_eq!(config.port.as_deref(), Some("2222"));
    assert_eq!(config.username, "deploy");
    assert_eq!(config.remote_path, "/srv/backups");
    assert_eq!(config.private_key.as_deref(), Some("-----BEGIN KEY-----"));
}

#[test]
fn build_config_emits_key_file_for_key_auth() {
    let config = SftpProviderConfig {
        host: "h".into(),
        port: Some("2222".into()),
        username: "u".into(),
        password: None,
        private_key: Some("PEMDATA".into()),
        remote_path: String::new(),
    };
    let (text, key) = build_sftp_config(&config).unwrap();
    let key = key.expect("key auth must produce a key file");
    assert!(text.contains("[sftp]"));
    assert!(text.contains("type = sftp"));
    assert!(text.contains("host = h"));
    assert!(text.contains("port = 2222"));
    assert!(text.contains("user = u"));
    assert!(text.contains(&format!("key_file = {}", key.path().display())));
    assert_eq!(std::fs::read_to_string(key.path()).unwrap(), "PEMDATA");
    assert!(!text.contains("pass ="));
}

#[test]
fn build_config_omits_port_when_absent() {
    let config = SftpProviderConfig {
        host: "h".into(),
        port: None,
        username: "u".into(),
        password: None,
        private_key: Some("K".into()),
        remote_path: String::new(),
    };
    let (text, _key) = build_sftp_config(&config).unwrap();
    assert!(!text.contains("port ="));
}

#[test]
fn factory_resolves_the_sftp_provider_key() {
    let s = storage(serde_json::json!({
        "host": "h", "username": "u", "password": "pw",
    }));
    assert!(get_provider(&s).is_some());
}
