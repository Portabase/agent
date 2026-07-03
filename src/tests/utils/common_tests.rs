use crate::utils::common::{BackupMethod, choose_restore_path, vec_to_option_json};
use serde_json::json;
use std::path::{Path, PathBuf};

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

    assert_eq!(
        result,
        Some(json!([
            { "id": 1 },
            { "id": 2 }
        ]))
    );
}

#[test]
fn choose_restore_path_single_file_returns_that_file() {
    let dir = Path::new("/tmp/extract");
    let archive = Path::new("/tmp/backup.tar.gz");
    let files = vec![PathBuf::from("/tmp/extract/dump.sql")];
    // Single extracted file: restore from that file directly.
    assert_eq!(
        choose_restore_path(&files, dir, archive),
        PathBuf::from("/tmp/extract/dump.sql")
    );
}

#[test]
fn choose_restore_path_multi_file_non_docker_volume_returns_archive_path() {
    let dir = Path::new("/tmp/extract");
    let archive = Path::new("/tmp/backup.tar.gz");
    let files = vec![
        PathBuf::from("/tmp/extract/toc.dat"),
        PathBuf::from("/tmp/extract/3141.dat.gz"),
    ];
    let chosen = choose_restore_path(&files, dir, archive);
    assert_eq!(chosen, PathBuf::from("/tmp/backup.tar.gz"));
    assert_eq!(chosen.extension().and_then(|e| e.to_str()), Some("gz"));
}
