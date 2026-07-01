use crate::domain::docker_volume::docker::client;
use crate::services::config::DatabaseConfig;
use anyhow::Result;

pub async fn run(cfg: DatabaseConfig) -> Result<bool> {
    let docker = client()?;
    match docker.inspect_volume(&cfg.volume_name).await {
        Ok(_) => Ok(true),
        Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => Ok(false),
        Err(e) => Err(e.into()),
    }
}
