use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::storage::get_provider;
use crate::services::storage::providers::rclone::target::rclone_target;

#[test]
fn factory_resolves_the_webdav_provider_key() {
    let storage: DatabaseStorage = serde_json::from_value(serde_json::json!({
        "id": "storage-1",
        "provider": "webdav",
        "folderName": "backups",
        "config": {"url": "https://dav.example.com"},
    }))
    .unwrap();
    assert!(get_provider(&storage).is_some());
}

#[test]
fn headers_are_encoded_as_rclone_csv() {
    let storage: DatabaseStorage = serde_json::from_value(serde_json::json!({
        "id": "storage-1",
        "provider": "webdav",
        "folderName": "backups",
        "config": {
            "url": "https://dav.example.com/root",
            "headers": [
                {"key": "X-Api-Key", "value": "token"},
                {"key": "Cookie", "value": "session=a,b\"c"}
            ]
        },
    }))
    .unwrap();

    let target = rclone_target(&storage).unwrap();
    assert!(
        target
            .config_text
            .contains("headers = \"X-Api-Key\",\"token\",\"Cookie\",\"session=a,b\"\"c\"")
    );
}
