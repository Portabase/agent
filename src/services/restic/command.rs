use super::json::{self, Line};
use super::password::{derive_password, local_storage_password, LOCAL_STORAGE_USER};
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::logger::JobLogger;
use crate::services::storage::providers::rclone::helpers::{remote_target, write_config};
use crate::services::storage::providers::rclone::target::{RcloneTarget, rclone_target};
use crate::settings::CONFIG;
use crate::utils::edge_key::EdgeKey;
use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::Stdio;
use tempfile::NamedTempFile;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// One source's repository on one storage channel:
/// `rclone:<remote>:<base>/<folder>/restic/<generated_id>`, or
/// `rest:<server_url>/storage/restic/<generated_id>/` on the dashboard's local storage.
pub struct ResticRepo {
    repository: String,
    password: String,
    /// `None` when the cache directory cannot be created: restic then runs `--no-cache`.
    cache_dir: Option<PathBuf>,
    /// rclone config of `rclone:` repositories; `None` on the dashboard's REST server.
    config: Option<NamedTempFile>,
    /// HTTP basic auth of `rest:` repositories (the dashboard's local storage).
    rest_auth: Option<(String, String)>,
    /// Keeps temp files referenced by the config (sftp key) alive.
    _target: Option<RcloneTarget>,
}

/// What a finished restic command left behind.
pub struct ResticRun {
    pub code: i32,
    /// stdout lines that are neither `status` nor `summary` (e.g. `snapshots --json`).
    pub stdout: String,
    /// The last `summary` line, if any.
    pub summary: Option<Value>,
    /// `error` lines as "item: message" (unreadable files and the like).
    pub errors: Vec<String>,
    /// Last plain-text / `exit_error` stderr lines, for failure messages.
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

/// A full snapshot id. Ids from the dashboard reach restic as positional arguments:
/// anything else could be parsed as a flag (e.g. `--password-command=<shell>`).
pub fn is_snapshot_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

/// restic fails hard (not warns) on an unusable cache dir, so probe it up front.
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
        let folder = storage
            .folder_name
            .as_deref()
            .map(|f| f.trim().trim_matches('/'))
            .filter(|f| !f.is_empty())
            .unwrap_or("backups");
        let mut repo = Self::new(
            &target.config_text,
            &target.remote_name,
            &target.base_path,
            folder,
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
            _target: None,
        })
    }

    /// The dashboard's local storage (`rclone serve restic --append-only`):
    /// `rest:<base_url>/<generated_id>/`, basic auth only in the child env.
    pub fn rest(base_url: &str, generated_id: &str, password: &str, server_password: &str, cache_dir: PathBuf) -> Self {
        Self {
            repository: format!("rest:{}/{generated_id}/", base_url.trim_end_matches('/')),
            password: password.to_string(),
            cache_dir: usable_cache_dir(cache_dir),
            config: None,
            rest_auth: Some((LOCAL_STORAGE_USER.to_string(), server_password.to_string())),
            _target: None,
        }
    }

    /// The dashboard's server refuses deletions: the agent cannot forget there.
    pub fn is_append_only(&self) -> bool {
        self.rest_auth.is_some()
    }

    pub fn repository(&self) -> &str {
        &self.repository
    }

    /// `restic <args> --retry-lock 5m` (+ `--no-cache` without a cache dir); secrets only in the child env. `status`
    /// lines are logged every 10 %.
    pub async fn run<I, S>(&self, args: I, logger: &JobLogger) -> Result<ResticRun>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
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
            // No TTY: without this restic prints no status lines at all.
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

    /// `restic cat config`; exit 10 (no repository) → `restic init`. Callers hold
    /// the source's FileLock, so two inits never race.
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
