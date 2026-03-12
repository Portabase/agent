use serde_json::json;
use crate::utils::common::{vec_to_option_json, BackupMethod};

#[test]
fn backup_method_to_string_automatic() {
    let method = BackupMethod::Automatic;
    assert_eq!(method.to_string(), "automatic");
}

#[test]
fn backup_method_to_string_manual() {
    let method = BackupMethod::Manual;
    assert_eq!(method.to_string(), "manual");
}

#[test]
fn vec_to_option_json_returns_none_when_empty() {
    let v: Vec<i32> = vec![];
    let result = vec_to_option_json(v);

    assert!(result.is_none());
}

#[test]
fn vec_to_option_json_serializes_vector() {
    let v = vec![1, 2, 3];
    let result = vec_to_option_json(v);

    assert_eq!(result, Some(json!([1, 2, 3])));
}

#[test]
fn vec_to_option_json_serializes_struct_vector() {
    #[derive(serde::Serialize)]
    struct Item {
        id: u32,
    }

    let v = vec![Item { id: 1 }, Item { id: 2 }];
    let result = vec_to_option_json(v);

    assert_eq!(result, Some(json!([
            { "id": 1 },
            { "id": 2 }
        ])));
}