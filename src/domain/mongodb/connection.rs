use crate::services::config::DatabaseConfig;
use anyhow::Result;
use mongodb::Client;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

const USERINFO_ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

pub async fn connect(cfg: DatabaseConfig) -> Result<Client> {
    let uri = get_mongo_uri(cfg)?;
    let mut options = mongodb::options::ClientOptions::parse(&uri).await?;
    options.server_selection_timeout = Some(std::time::Duration::from_secs(3));
    options.connect_timeout = Some(std::time::Duration::from_secs(3));
    let client = Client::with_options(options)?;
    Ok(client)
}

pub fn select_mongo_path() -> std::path::PathBuf {
    "/usr/local/mongodb/bin".to_string().into()
}

pub fn get_mongo_uri(cfg: DatabaseConfig) -> Result<String> {
    Ok(build_mongo_uri(&cfg, true))
}

pub fn build_mongo_uri(cfg: &DatabaseConfig, include_db: bool) -> String {
    let is_multi_host = cfg.host.contains(',');
    let is_srv = cfg.port == 0 && !is_multi_host;
    let scheme = if is_srv { "mongodb+srv" } else { "mongodb" };
    let has_auth = !cfg.username.is_empty() && !cfg.password.is_empty();

    let credentials = if has_auth {
        format!(
            "{}:{}@",
            utf8_percent_encode(&cfg.username, USERINFO_ENCODE),
            utf8_percent_encode(&cfg.password, USERINFO_ENCODE)
        )
    } else {
        String::new()
    };

    let authority = if is_srv || is_multi_host {
        cfg.host.clone()
    } else {
        format!("{}:{}", cfg.host, cfg.port)
    };

    let path = if include_db {
        format!("/{}", cfg.database)
    } else {
        "/".to_string()
    };

    let mut params: Vec<String> = Vec::new();

    match cfg.options.get("auth_source").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => params.push(format!(
            "authSource={}",
            utf8_percent_encode(s, USERINFO_ENCODE)
        )),
        _ if has_auth => params.push("authSource=admin".to_string()),
        _ => {}
    }

    if let Some(rs) = cfg.options.get("replica_set").and_then(|v| v.as_str()) {
        if !rs.is_empty() {
            params.push(format!(
                "replicaSet={}",
                utf8_percent_encode(rs, USERINFO_ENCODE)
            ));
        }
    }

    if cfg.options.get("tls").and_then(|v| v.as_bool()) == Some(true) {
        params.push("tls=true".to_string());
    }

    let query = if params.is_empty() {
        String::new()
    } else {
        format!("?{}", params.join("&"))
    };

    format!("{}://{}{}{}{}", scheme, credentials, authority, path, query)
}

pub fn extract_db_name(dry_output: &str) -> Option<String> {
    let mut dbs = std::collections::HashSet::new();
    for line in dry_output.lines() {
        if let Some(pos) = line.find("archive prelude ") {
            let rest = &line[pos + "archive prelude ".len()..];
            if let Some(dot) = rest.find('.') {
                let db = &rest[..dot];
                dbs.insert(db.to_string());
            }
        }
    }
    dbs.into_iter().next()
}
