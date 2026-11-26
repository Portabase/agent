use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::storage::get_provider;
use crate::services::storage::providers::ftp::helpers::build_ftp_config;
use crate::services::storage::providers::ftp::models::{FtpProviderConfig, FtpsMode};

fn storage(config: serde_json::Value) -> DatabaseStorage {
    serde_json::from_value(serde_json::json!({"id":"storage-1","provider":"ftp","folderName":"backups","config":config})).unwrap()
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
