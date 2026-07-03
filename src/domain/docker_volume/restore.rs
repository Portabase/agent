use crate::domain::docker_volume::docker::{
    client, create_helper, remove_helper, resolve_helper_image, start_container, stop_container,
};
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use bollard::exec::StartExecResults;
use bollard::models::ExecConfig;
use bollard::query_parameters::UploadToContainerOptions;
use futures_util::StreamExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

pub async fn run(cfg: DatabaseConfig, archive: PathBuf, logger: Arc<JobLogger>) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        futures::executor::block_on(async move {
            logger.log("info", format!("Starting docker-volume restore for {}", cfg.name));

            let docker = client()?;
            let image = resolve_helper_image(&docker).await?;

            logger.log("debug", format!("Restore archive: {}", archive.display()));

            if let Some(name) = &cfg.container_name {
                logger.log("info", format!("Stopping container {name} for restore"));
                stop_container(&docker, name).await?;
            }

            let result = async {
                let helper = create_helper(
                    &docker,
                    &image,
                    &cfg.volume_name,
                    &cfg.generated_id,
                    false,
                    Some(vec![
                        "sh".into(),
                        "-c".into(),
                        "trap 'exit 0' TERM; sleep 2147483647 & wait".into(),
                    ]),
                )
                .await?;
                start_container(&docker, &helper.id).await?;


                let exec = docker
                    .create_exec(
                        &helper.id,
                        ExecConfig {
                            cmd: Some(vec![
                                "sh".to_string(),
                                "-c".to_string(),
                                "rm -rf /vol/* /vol/.[!.]* 2>/dev/null || true".to_string(),
                            ]),
                            attach_stdout: Some(true),
                            attach_stderr: Some(true),
                            ..Default::default()
                        },
                    )
                    .await
                    .context("Failed to create wipe exec")?;

                if let StartExecResults::Attached { mut output, .. } =
                    docker.start_exec(&exec.id, None).await.context("Failed to run wipe exec")?
                {
                    while output.next().await.is_some() {}
                }

                let start = Instant::now();

                let file = tokio::fs::File::open(&archive)
                    .await
                    .with_context(|| format!("Failed to open {}", archive.display()))?;
                let stream = tokio_util::io::ReaderStream::new(file);

                let up_opts = UploadToContainerOptions { path: "/".to_string(), ..Default::default() };
                docker
                    .upload_to_container(&helper.id, Some(up_opts), bollard::body_try_stream(stream))
                    .await
                    .context("Failed to upload volume archive")?;

                let duration_ms = start.elapsed().as_millis() as f64;
                logger.log_command("docker upload_to_container", None, Some(0), Some(duration_ms));

                remove_helper(&docker, &helper.id).await;
                logger.log("info", format!("Volume restore completed for {}", cfg.name));
                anyhow::Ok(())
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
