use super::json::{self, Line};
use super::password::derive_password;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::logger::JobLogger;
use crate::services::storage::providers::local::{LOCAL_STORAGE_USER, local_storage_password};
use crate::services::storage::providers::rclone::helpers::{max_connections, remote_target, write_config};
use crate::services::storage::providers::rclone::target::{RcloneTarget, rclone_target};
use crate::settings::CONFIG;
use crate::utils::edge_key::EdgeKey;
use crate::utils::file::folder_or_default;
use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::Stdio;
use tempfile::NamedTempFile;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

pub struct ResticRepo {
    repository: String,
    password: String,
    cache_dir: Option<PathBuf>,
    config: Option<NamedTempFile>,
    rest_auth: Option<(String, String)>,
    connections: Option<u32>,
    _target: Option<RcloneTarget>,
}

pub struct ResticRun {
    pub code: i32,
    pub stdout: String,
    pub summary: Option<Value>,
    pub errors: Vec<String>,
    pub stderr_tail: String,
}

impl ResticRun {
    pub fn error(&self, op: &str) -> anyhow::Error {
        match self.code {
            10 => anyhow!("restic {op}: repository does not exist"),
            11 => anyhow!("restic {op}: repository is locked by another operation (waited 5 minutes)"),
            12 => anyhow!("restic {op}: wrong repository password (did the master key change?)"),
            code => anyhow!("restic {op} failed (exit {code}): {}", self.stderr_tail.trim()),
        }
    }
}

pub fn is_snapshot_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn usable_cache_dir(cache_dir: PathBuf) -> Option<PathBuf> {
    match std::fs::create_dir_all(&cache_dir) {
        Ok(()) => Some(cache_dir),
        Err(e) => {
            tracing::warn!("restic cache dir {} unusable ({e}); running without cache", cache_dir.display());
            None
        }
    }
}

impl ResticRepo {
    pub fn open(storage: &DatabaseStorage, generated_id: &str, edge_key: &EdgeKey) -> Result<Self> {
        let password = derive_password(&edge_key.master_key_b64, generated_id)?;
        let cache_dir = PathBuf::from(&CONFIG.data_path).join("cache/restic");
        if storage.provider == "local" {
            return Ok(Self::rest(
                &format!("{}/storage/restic", edge_key.server_url.trim_end_matches('/')),
                generated_id,
                &password,
                &local_storage_password(&edge_key.master_key_b64)?,
                cache_dir,
            ));
        }
        let target = rclone_target(storage)?;
        let mut repo = Self::new(
            &target.config_text,
            &target.remote_name,
            &target.base_path,
            folder_or_default(storage.folder_name.as_deref()),
            generated_id,
            &password,
            cache_dir,
        )?;
        repo._target = Some(target);
        Ok(repo)
    }

    pub fn new(
        config_text: &str,
        remote_name: &str,
        base_path: &str,
        folder: &str,
        generated_id: &str,
        password: &str,
        cache_dir: PathBuf,
    ) -> Result<Self> {
        let path = remote_target(remote_name, base_path, &format!("{folder}/restic/{generated_id}"));
        Ok(Self {
            repository: format!("rclone:{path}"),
            password: password.to_string(),
            cache_dir: usable_cache_dir(cache_dir),
            config: Some(write_config(config_text)?),
            rest_auth: None,
            connections: max_connections(config_text),
            _target: None,
        })
    }

    pub fn rest(base_url: &str, generated_id: &str, password: &str, server_password: &str, cache_dir: PathBuf) -> Self {
        Self {
            repository: format!("rest:{}/{generated_id}/", base_url.trim_end_matches('/')),
            password: password.to_string(),
            cache_dir: usable_cache_dir(cache_dir),
            config: None,
            rest_auth: Some((LOCAL_STORAGE_USER.to_string(), server_password.to_string())),
            connections: None,
            _target: None,
        }
    }

    pub fn is_append_only(&self) -> bool {
        self.rest_auth.is_some()
    }

    pub fn repository(&self) -> &str {
        &self.repository
    }

    pub async fn run<I, S>(&self, args: I, logger: &JobLogger) -> Result<ResticRun>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut args: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
        if let Some(n) = self.connections {
            args.extend(["-o".into(), format!("rclone.connections={n}").into()]);
        }
        let shown: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        logger.log("debug", format!("restic {}", shown.join(" ")));

        let mut cmd = Command::new("restic");
        cmd.args(&args).args(["--retry-lock", "5m"]);
        match &self.cache_dir {
            Some(dir) => cmd.env("RESTIC_CACHE_DIR", dir),
            None => cmd.arg("--no-cache"),
        };
        if let Some(config) = &self.config {
            cmd.env("RCLONE_CONFIG", config.path());
        }
        if let Some((user, pass)) = &self.rest_auth {
            cmd.env("RESTIC_REST_USERNAME", user).env("RESTIC_REST_PASSWORD", pass);
        }
        let mut child = cmd
            .env("RESTIC_REPOSITORY", &self.repository)
            .env("RESTIC_PASSWORD", &self.password)
            .env("RESTIC_PROGRESS_FPS", "0.2")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => anyhow!("restic binary not found"),
                _ => anyhow!(e).context("failed to start restic"),
            })?;

        let stderr = child.stderr.take().context("restic stderr unavailable")?;
        let stderr_task = tokio::spawn(async move {
            let mut errors = Vec::new();
            let mut tail = Vec::new();
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match json::parse(&line) {
                    Line::Error { message, item } => errors.push(match item {
                        Some(item) => format!("{item}: {message}"),
                        None => message,
                    }),
                    Line::ExitError { message, .. } => tail.push(message),
                    Line::Text(text) if !text.trim().is_empty() => tail.push(text),
                    _ => {}
                }
            }
            (errors, tail)
        });

        let mut run = ResticRun { code: -1, stdout: String::new(), summary: None, errors: Vec::new(), stderr_tail: String::new() };
        let stdout = child.stdout.take().context("restic stdout unavailable")?;
        let mut lines = BufReader::new(stdout).lines();
        let mut next_pct = 10u32;
        while let Some(line) = lines.next_line().await.context("failed to read restic output")? {
            match json::parse(&line) {
                Line::Status { percent_done } => {
                    let pct = (percent_done * 100.0) as u32;
                    if pct >= next_pct {
                        logger.log("info", format!("restic: {pct}% done"));
                        next_pct = pct / 10 * 10 + 10;
                    }
                }
                Line::Summary(value) => run.summary = Some(value),
                _ => {
                    run.stdout.push_str(&line);
                    run.stdout.push('\n');
                }
            }
        }

        let status = child.wait().await.context("failed to wait for restic")?;
        let (errors, tail) = stderr_task.await.unwrap_or_default();
        run.code = status.code().unwrap_or(-1);
        run.errors = errors;
        run.stderr_tail = tail[tail.len().saturating_sub(10)..].join("\n");
        Ok(run)
    }

    pub async fn ensure_initialized(&self, logger: &JobLogger) -> Result<()> {
        let cat = self.run(["cat", "config"], logger).await?;
        match cat.code {
            0 => Ok(()),
            10 => {
                logger.log("info", format!("Initializing restic repository {}", self.repository));
                let init = self.run(["init"], logger).await?;
                if init.code == 0 { Ok(()) } else { Err(init.error("init")) }
            }
            _ => Err(cat.error("cat config")),
        }
    }
}
