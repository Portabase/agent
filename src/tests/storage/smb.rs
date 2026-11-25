use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::storage::get_provider;

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
