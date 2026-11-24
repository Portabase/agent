use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::storage::get_provider;
use crate::services::storage::providers::rclone::target::rclone_target;

#[test]
fn factory_resolves_the_smb_provider_key() {
    let storage: DatabaseStorage = serde_json::from_value(serde_json::json!({
        "id": "storage-1",
        "provider": "smb",
        "folderName": "backups",
        "config": {
            "host": "samba.example.com",
            "share": "backups",
            "username": "portabase",
            "password": "secret"
        },
    }))
    .unwrap();

    assert!(get_provider(&storage).is_some());
}

#[test]
fn rclone_target_uses_the_share_and_remote_path() {
    let storage: DatabaseStorage = serde_json::from_value(serde_json::json!({
        "id": "storage-1",
        "provider": "smb",
        "folderName": "backups",
        "config": {
            "host": "samba.example.com",
            "port": 1445,
            "share": "backups",
            "username": "portabase",
            "password": "secret",
            "remotePath": "/daily"
        },
    }))
    .unwrap();

    let target = rclone_target(&storage).unwrap();

    assert!(target.config_text.contains("type = smb"));
    assert!(target.config_text.contains("host = samba.example.com"));
    assert!(target.config_text.contains("port = 1445"));
    assert!(target.config_text.contains("user = portabase"));
    assert!(
        target
            .config_text
            .lines()
            .any(|line| line.starts_with("pass = "))
    );
    assert_eq!(target.remote_name, "smb");
    assert_eq!(target.base_path, "backups/daily");
}
