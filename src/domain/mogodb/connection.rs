use crate::services::config::DatabaseConfig;
use anyhow::Result;
use mongodb::Client;

pub async fn connect(cfg: DatabaseConfig) -> Result<Client> {
    let uri = get_mongo_uri(cfg);
    let mut options = mongodb::options::ClientOptions::parse(&uri).await?;
    options.server_selection_timeout = Some(std::time::Duration::from_secs(3));
    options.connect_timeout = Some(std::time::Duration::from_secs(3));
    let client = Client::with_options(options)?;
    Ok(client)
}

pub fn select_mongo_path() -> std::path::PathBuf {
    "/usr/local/mongodb/bin".to_string().into()
}

pub fn get_mongo_uri(cfg: DatabaseConfig) -> String {
    if cfg.username.is_empty() {
        format!("mongodb://{}:{}/{}", cfg.host, cfg.port, cfg.database)

    } else {
        format!(
            "mongodb://{}:{}@{}:{}/{}?authSource=admin",
            cfg.username, cfg.password, cfg.host, cfg.port, cfg.database
        )
    }
}
