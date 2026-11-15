use crate::domain::docker_volume::docker::{
    client, create_helper, idle_command, is_podman, remove_helper, resolve_helper_image,
    start_container, stop_container,
};
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use bollard::Docker;
use bollard::container::LogOutput;
use bollard::exec::StartExecResults;
use bollard::models::ExecConfig;
use bollard::query_parameters::DownloadFromContainerOptions;
use futures::StreamExt;
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
            let podman = is_podman(&docker).await?;
            logger.log("debug", format!("Helper image: {image}"));

            if let Some(name) = &cfg.container_name {
                logger.log("info", format!("Stopping container {name} for consistent backup"));
                stop_container(&docker, name).await?;
            }

            let result = async {
                let file_path = backup_dir.join(format!("{}.tar", cfg.generated_id));
                let mut out = File::create(&file_path)
                    .await
                    .with_context(|| format!("Failed to create backup file {}", file_path.display()))?;
                let helper = create_helper(
                    &docker,
                    &image,
                    &cfg.volume_name,
                    &cfg.generated_id,
                    true,
                    podman.then(idle_command),
                )
                .await?;

                let start = Instant::now();
                let written = if podman {
                    archive_with_tar(&docker, &helper.id, &mut out, &logger).await
                } else {
                    download_archive(&docker, &helper.id, &mut out).await
                };
                remove_helper(&docker, &helper.id).await;
                let bytes_written = written?;
                out.flush().await?;

                let duration_ms = start.elapsed().as_millis() as f64;
                let command = if podman { "podman exec tar" } else { "docker download_from_container" };
                logger.log_command(command, None, Some(0), Some(duration_ms));
                logger.log("info", format!("Volume backup wrote {bytes_written} bytes to {}", file_path.display()));

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

async fn download_archive(docker: &Docker, helper_id: &str, out: &mut File) -> Result<u64> {
    let dl_opts = DownloadFromContainerOptions { path: "/vol".to_string() };
    let mut stream = docker.download_from_container(helper_id, Some(dl_opts));
    let mut bytes_written: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Error streaming volume archive from Docker")?;
        bytes_written += chunk.len() as u64;
        out.write_all(&chunk).await?;
    }
    Ok(bytes_written)
}

async fn archive_with_tar(docker: &Docker, helper_id: &str, out: &mut File, logger: &JobLogger) -> Result<u64> {
    start_container(docker, helper_id).await?;
    let exec = docker
        .create_exec(
            helper_id,
            ExecConfig {
                cmd: Some(vec![
                    "tar".into(),
                    "-C".into(),
                    "/".into(),
                    "-cf".into(),
                    "-".into(),
                    "vol".into(),
                ]),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await
        .context("Failed to create tar exec")?;

    let mut bytes_written: u64 = 0;
    let mut stderr = Vec::new();
    if let StartExecResults::Attached { mut output, .. } =
        docker.start_exec(&exec.id, None).await.context("Failed to run tar exec")?
    {
        while let Some(frame) = output.next().await {
            match frame.context("Error streaming volume archive from tar")? {
                LogOutput::StdOut { message } => {
                    bytes_written += message.len() as u64;
                    out.write_all(&message).await?;
                }
                LogOutput::StdErr { message } => stderr.extend_from_slice(&message),
                _ => {}
            }
        }
    }

    let exit_code = docker
        .inspect_exec(&exec.id)
        .await
        .context("Failed to inspect tar exec")?
        .exit_code;
    let stderr = String::from_utf8_lossy(&stderr);
    if !tar_succeeded(exit_code, &stderr) {
        anyhow::bail!("tar in helper container exited with {exit_code:?}: {}", stderr.trim());
    }
    if !stderr.trim().is_empty() {
        logger.log("warn", stderr.trim().to_string());
    }
    Ok(bytes_written)
}

pub fn tar_succeeded(exit_code: Option<i64>, stderr: &str) -> bool {
    match exit_code {
        Some(0) => true,
        Some(1) => {
            let mut lines = stderr.lines().filter(|line| !line.trim().is_empty()).peekable();
            lines.peek().is_some() && lines.all(|line| line.ends_with("file changed as we read it"))
        }
        _ => false,
    }
}
