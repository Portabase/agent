use crate::domain::docker_volume::docker::{
    client, create_helper, remove_helper, resolve_helper_image, start_container, stop_container,
};
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use bollard::query_parameters::DownloadFromContainerOptions;
use futures_util::StreamExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

pub async fn run(cfg: DatabaseConfig, backup_dir: PathBuf, logger: Arc<JobLogger>) -> Result<PathBuf> {
    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        futures::executor::block_on(async move {
            logger.log("info", format!("Starting docker-volume backup for {}", cfg.name));

            let docker = client()?;
            let image = resolve_helper_image(&docker).await?;
            logger.log("debug", format!("Helper image: {image}"));

            if let Some(name) = &cfg.container_name {
                logger.log("info", format!("Stopping container {name} for consistent backup"));
                stop_container(&docker, name).await?;
            }

            let result = async {
                let helper = create_helper(&docker, &image, &cfg.volume_name, &cfg.generated_id, true, None).await?;

                let file_path = backup_dir.join(format!("{}.tar", cfg.generated_id));
                let start = Instant::now();

                let dl_opts = DownloadFromContainerOptions { path: "/vol".to_string() };
                let mut stream = docker.download_from_container(&helper.id, Some(dl_opts));

                let mut out = File::create(&file_path)
                    .await
                    .with_context(|| format!("Failed to create backup file {}", file_path.display()))?;
                let mut bytes_written: u64 = 0;
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.context("Error streaming volume archive from Docker")?;
                    bytes_written += chunk.len() as u64;
                    out.write_all(&chunk).await?;
                }
                out.flush().await?;

                let duration_ms = start.elapsed().as_millis() as f64;
                logger.log_command("docker download_from_container", None, Some(0), Some(duration_ms));
                logger.log("info", format!("Volume backup wrote {bytes_written} bytes to {}", file_path.display()));

                remove_helper(&docker, &helper.id).await;
                anyhow::Ok(file_path)
            }
            .await;

            if let Some(name) = &cfg.container_name {
                if let Err(e) = start_container(&docker, name).await {
                    logger.log("error", format!("Failed to restart container {name}: {e}"));
                }
            }

            result
        })
    })
    .await?
}
