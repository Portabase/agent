use super::models::DatabaseConfig;
use std::collections::HashMap;

pub const FILES_METHODS: [&str; 4] = ["archive", "snapshot", "sync", "copy"];

pub fn files_method(cfg: &DatabaseConfig) -> &'static str {
    match cfg.options.get("method").and_then(|v| v.as_str()) {
        Some("snapshot") => "snapshot",
        Some("sync") => "sync",
        Some("copy") => "copy",
        _ => "archive",
    }
}

pub fn list_option_args(cfg: &DatabaseConfig, flags: &[(&'static str, &str)]) -> (Vec<String>, Vec<&'static str>) {
    let mut args = Vec::new();
    let mut invalid = Vec::new();
    for &(key, flag) in flags {
        match cfg.options.get(key).map(|v| v.as_array()) {
            None => {}
            Some(Some(items)) => args.extend(
                items
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(|p| format!("{flag}={p}")),
            ),
            Some(None) => invalid.push(key),
        }
    }
    (args, invalid)
}

pub fn validate_files_options(name: &str, options: Option<&HashMap<String, serde_json::Value>>) -> Result<(), String> {
    if let Some(method) = options.and_then(|options| options.get("method")) {
        if !method.as_str().is_some_and(|m| FILES_METHODS.contains(&m)) {
            return Err(format!(
                "Unknown method {method} for files source '{}' (expected archive, snapshot, sync or copy)",
                name
            ));
        }
    }
    if let Some(limit) = options.and_then(|options| options.get("max_delete")) {
        if !limit.is_u64() {
            return Err(format!(
                "Invalid max_delete {limit} for files source '{}' (expected a whole number >= 0)",
                name
            ));
        }
    }
    if let Some(size) = options.and_then(|options| options.get("max_delete_size")) {
        if !size.as_str().is_some_and(is_size) {
            return Err(format!(
                "Invalid max_delete_size {size} for files source '{}' (expected a size such as 500M or 10G)",
                name
            ));
        }
    }
    Ok(())
}

fn is_size(value: &str) -> bool {
    let number = value.strip_suffix(|suffix: char| "bkmgtp".contains(suffix.to_ascii_lowercase())).unwrap_or(value);
    let (whole, fraction) = number.split_once('.').unwrap_or((number, "0"));
    let all_digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    all_digits(whole) && all_digits(fraction)
}
