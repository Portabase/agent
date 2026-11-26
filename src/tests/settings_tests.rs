use crate::settings::{parse_max_concurrent_backups, parse_restore_path};
use std::env::VarError;

#[test]
fn unset_means_unlimited() {
    assert_eq!(
        parse_max_concurrent_backups(Err(VarError::NotPresent)),
        None
    );
}

#[test]
fn blank_means_unlimited() {
    assert_eq!(parse_max_concurrent_backups(Ok("".to_string())), None);
    assert_eq!(parse_max_concurrent_backups(Ok("   ".to_string())), None);
}

#[test]
fn valid_value_is_parsed() {
    assert_eq!(parse_max_concurrent_backups(Ok("2".to_string())), Some(2));
}

#[test]
fn surrounding_whitespace_is_trimmed() {
    assert_eq!(parse_max_concurrent_backups(Ok(" 2 ".to_string())), Some(2));
}

#[test]
#[should_panic(expected = "must be valid Unicode")]
fn non_unicode_value_panics() {
    parse_max_concurrent_backups(Err(VarError::NotUnicode("\u{fffd}".into())));
}

#[test]
#[should_panic(expected = "must be at least 1")]
fn zero_panics() {
    parse_max_concurrent_backups(Ok("0".to_string()));
}

#[test]
#[should_panic(expected = "must be a valid positive integer")]
fn malformed_value_panics() {
    parse_max_concurrent_backups(Ok("not-a-number".to_string()));
}

#[test]
#[should_panic(expected = "must not exceed")]
fn value_exceeding_semaphore_max_permits_panics() {
    let over_limit = (tokio::sync::Semaphore::MAX_PERMITS as u128 + 1).to_string();
    parse_max_concurrent_backups(Ok(over_limit));
}

#[test]
fn restore_path_empty_is_none() {
    assert_eq!(parse_restore_path(None), None);
    assert_eq!(parse_restore_path(Some("".to_string())), None);
    assert_eq!(parse_restore_path(Some("  ".to_string())), None);
    assert_eq!(parse_restore_path(Some("/restore".to_string())), Some("/restore".to_string()));
}
