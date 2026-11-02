use super::logger::JobLogger;
use super::result::JobStatus;
use crate::domain::docker_volume::docker;
pub use crate::services::config::Phase;
use crate::services::config::{HookCommand, Hooks};
use anyhow::{Context, Result, anyhow, bail};
use bollard::exec::StartExecResults;
use bollard::models::ExecConfig;
use futures_util::StreamExt;
use std::process::Stdio;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct HookEnv {
    pub db_name: String,
    pub generated_id: String,
    pub backup_id: String,
}

impl HookEnv {
    fn vars(&self, phase: Phase, status: Option<JobStatus>) -> Vec<(String, String)> {
        let mut vars = vec![
            ("PORTABASE_DB_NAME".to_string(), self.db_name.clone()),
            (
                "PORTABASE_GENERATED_ID".to_string(),
                self.generated_id.clone(),
            ),
            ("PORTABASE_BACKUP_ID".to_string(), self.backup_id.clone()),
            ("PORTABASE_PHASE".to_string(), phase.as_str().to_string()),
        ];
        if let Some(status) = status {
            vars.push(("PORTABASE_STATUS".to_string(), status.as_str().to_string()));
        }
        vars
    }
}

pub struct JobHooks {
    hooks: Hooks,
    env: HookEnv,
}

impl JobHooks {
    pub fn new(hooks: Option<Hooks>, env: HookEnv) -> Self {
        Self {
            hooks: hooks.unwrap_or_default(),
            env,
        }
    }

    pub async fn pre(&self, logger: &JobLogger) -> Result<()> {
        run_phase(&self.hooks, Phase::Pre, &self.env, None, logger).await
    }

    pub async fn post(&self, status: JobStatus, logger: &JobLogger) {
        if let Err(e) = run_phase(&self.hooks, Phase::Post, &self.env, Some(status), logger).await {
            logger.log("error", format!("Post-backup hook failed: {e:#}"));
        }
    }

    pub async fn finish(&self, status: JobStatus, logger: &JobLogger) {
        let phase = match status {
            JobStatus::Success => Phase::OnSuccess,
            JobStatus::Failed => Phase::OnFailure,
        };
        if let Err(e) = run_phase(&self.hooks, phase, &self.env, Some(status), logger).await {
            logger.log("error", format!("{} hook failed: {e:#}", phase.as_str()));
        }
    }
}

const INHERITED_ENV: [&str; 4] = ["PATH", "HOME", "LANG", "TZ"];

struct HookOutput {
    exit_code: Option<i64>,
    output: String,
}

pub async fn run_phase(
    hooks: &Hooks,
    phase: Phase,
    env: &HookEnv,
    status: Option<JobStatus>,
    logger: &JobLogger,
) -> Result<()> {
    let Some(hook) = phase.command(hooks) else {
        return Ok(());
    };

    let timeout = Duration::from_secs(hooks.timeout_secs());
    let vars = env.vars(phase, status);
    logger.log("info", format!("Running {} hook", phase.as_str()));

    let label = describe(phase, hook);
    let start = Instant::now();
    let outcome = tokio::time::timeout(timeout, run_one(hook, &vars)).await;
    let duration_ms = start.elapsed().as_millis() as f64;

    let out = match outcome {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            logger.log_command(&label, Some(format!("{e:#}")), Some(-1), Some(duration_ms));
            bail!("{label} failed: {e:#}");
        }
        Err(_) => {
            // ponytail: docker has no exec-kill API, so a timed-out docker exec keeps running in its container.
            logger.log_command(&label, None, Some(-1), Some(duration_ms));
            bail!("{label} timed out after {}s", timeout.as_secs());
        }
    };

    let exit_code = out.exit_code.map(|c| c as i32);
    let output = (!out.output.trim().is_empty()).then_some(out.output);
    logger.log_command(&label, output, exit_code, Some(duration_ms));

    match out.exit_code {
        Some(0) => Ok(()),
        Some(code) => bail!("{label} exited with code {code}"),
        None => bail!("{label} ended without an exit code"),
    }
}

fn describe(phase: Phase, hook: &HookCommand) -> String {
    let argv = hook.command.join(" ");
    match &hook.container {
        Some(container) => format!("[hook:{}] docker exec {container}: {argv}", phase.as_str()),
        None => format!("[hook:{}] {argv}", phase.as_str()),
    }
}

async fn run_one(hook: &HookCommand, vars: &[(String, String)]) -> Result<HookOutput> {
    match &hook.container {
        Some(container) => run_docker_exec(container, hook, vars).await,
        None => run_local(hook, vars).await,
    }
}

async fn run_local(hook: &HookCommand, vars: &[(String, String)]) -> Result<HookOutput> {
    let (program, args) = hook
        .command
        .split_first()
        .ok_or_else(|| anyhow!("empty command"))?;

    let passthrough = INHERITED_ENV
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)));

    let out = tokio::process::Command::new(program)
        .args(args)
        .env_clear()
        .envs(passthrough)
        .envs(vars.iter().cloned())
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .with_context(|| format!("failed to start {program}"))?;

    let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
    output.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(HookOutput {
        exit_code: out.status.code().map(i64::from),
        output,
    })
}

async fn run_docker_exec(
    container: &str,
    hook: &HookCommand,
    vars: &[(String, String)],
) -> Result<HookOutput> {
    let docker = docker::client()?;
    let exec = docker
        .create_exec(
            container,
            ExecConfig {
                cmd: Some(hook.command.clone()),
                user: hook.user.clone(),
                env: Some(vars.iter().map(|(k, v)| format!("{k}={v}")).collect()),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await
        .with_context(|| format!("failed to create exec in container {container}"))?;

    let mut output = String::new();
    if let StartExecResults::Attached {
        output: mut stream, ..
    } = docker
        .start_exec(&exec.id, None)
        .await
        .with_context(|| format!("failed to start exec in container {container}"))?
    {
        while let Some(chunk) = stream.next().await {
            output.push_str(&chunk.context("failed to read exec output")?.to_string());
        }
    }

    let inspect = docker
        .inspect_exec(&exec.id)
        .await
        .context("failed to inspect exec")?;
    Ok(HookOutput {
        exit_code: inspect.exit_code,
        output,
    })
}
