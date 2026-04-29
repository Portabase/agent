use crate::services::config::DatabaseConfig;
use anyhow::Result;
use mongodb::Client;

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
    if cfg.username.is_empty() || cfg.password.is_empty() {
        Ok(format!(
            "mongodb://{}:{}/{}",
            cfg.host, cfg.port, cfg.database
        ))
    } else {
        Ok(format!(
            "mongodb://{}:{}@{}:{}/{}?authSource=admin",
            cfg.username, cfg.password, cfg.host, cfg.port, cfg.database
        ))
    }
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
