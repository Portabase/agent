use anyhow::{Context, Result, bail};
use bytes::Bytes;
use futures::{Stream, StreamExt};
use std::io::Write;
use std::path::Path;
use std::pin::Pin;
use std::process::Stdio;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tracing::info;

const BLOCKED_BACKEND_TYPES: [&str; 13] = [
    "local",
    "alias",
    "crypt",
    "chunker",
    "compress",
    "union",
    "combine",
    "hasher",
    "archive",
    "cache",
    "memory",
    "http",
    "googlephotos",
];

fn sections(config_text: &str) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();

    for line in config_text.lines() {
        let line = line.trim();

        if line.starts_with('[') && line.ends_with(']') && line.len() > 2 {
            out.push((line[1..line.len() - 1].trim().to_string(), None));
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        if key.trim().eq_ignore_ascii_case("type")
            && let Some(current) = out.last_mut()
            && current.1.is_none()
        {
            current.1 = Some(value.trim().to_ascii_lowercase());
        }
    }

    out
}


pub fn validate_config(config_text: &str, remote_name: &str) -> Result<()> {
    let sections = sections(config_text);

    if sections.is_empty() {
        bail!("rclone config contains no remote sections");
    }

    for (name, backend) in &sections {
        let Some(backend) = backend else { continue };
        if BLOCKED_BACKEND_TYPES.contains(&backend.as_str()) {
            bail!("rclone backend type '{backend}' is not allowed (remote '{name}')");
        }
    }

    if !sections.iter().any(|(name, _)| name == remote_name) {
        let available: Vec<&str> = sections.iter().map(|(name, _)| name.as_str()).collect();
        bail!(
            "remote '{remote_name}' is not defined in the rclone config (available: {})",
            available.join(", ")
        );
    }

    Ok(())
}

/// `rclone obscure <password>` — several backends (sftp, ...) require the
/// password field to be obscured rather than plain.
pub fn obscure_password(password: &str) -> Result<String> {
    let out = std::process::Command::new("rclone")
        .arg("obscure")
        .arg(password)
        .output()
        .context("failed to spawn rclone (is the binary installed in this image?)")?;

    if !out.status.success() {
        bail!(
            "rclone obscure failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Serialize ordered `(key, value)` fields into an rclone config section
/// ("json to rclone config"). Empty values are skipped; values containing line
/// breaks are rejected to prevent config injection. Order is preserved.
pub fn build_rclone_config(remote_name: &str, fields: &[(&str, String)]) -> Result<String> {
    if remote_name.contains(['\r', '\n']) {
        bail!("rclone remote name must not contain line breaks");
    }

    let mut lines = vec![format!("[{remote_name}]")];
    for (key, value) in fields {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if value.contains(['\r', '\n']) {
            bail!("rclone config value for '{key}' must not contain line breaks");
        }
        lines.push(format!("{key} = {value}"));
    }

    Ok(lines.join("\n") + "\n")
}

/// `<remote>:<remote_path>/<remote_file_path>`
pub fn remote_target(remote_name: &str, remote_path: &str, remote_file_path: &str) -> String {
    let base = remote_path.trim().trim_matches('/');

    if base.is_empty() {
        format!("{remote_name}:{remote_file_path}")
    } else {
        format!("{remote_name}:{base}/{remote_file_path}")
    }
}

pub type RcloneStream = Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>;

pub fn write_config(config_text: &str) -> Result<NamedTempFile> {
    let mut file = NamedTempFile::new().context("failed to create rclone config temp file")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o600))
            .context("failed to restrict rclone config permissions")?;
    }

    file.write_all(config_text.as_bytes())
        .context("failed to write rclone config")?;
    file.flush().context("failed to flush rclone config")?;

    Ok(file)
}

pub async fn rcat(config_path: &Path, target: &str, mut stream: RcloneStream) -> Result<()> {
    info!("rclone rcat -> {}", target);

    let mut child = Command::new("rclone")
        .arg("--config")
        .arg(config_path)
        .arg("--contimeout")
        .arg("30s")
        .arg("--timeout")
        .arg("5m")
        .arg("--retries")
        .arg("1")
        .arg("--low-level-retries")
        .arg("3")
        .arg("rcat")
        .arg(target)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn rclone (is the binary installed in this image?)")?;

    let mut stderr_pipe = child.stderr.take().context("rclone stderr unavailable")?;
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let _ = stderr_pipe.read_to_string(&mut buf).await;
        buf
    });

    let mut stdin = child.stdin.take().context("rclone stdin unavailable")?;

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(e).context("backup stream failed");
            }
        };

        if stdin.write_all(&chunk).await.is_err() {
            break;
        }
    }

    let _ = stdin.flush().await;
    drop(stdin);

    let status = child.wait().await.context("failed to wait for rclone")?;
    let stderr = stderr_task.await.unwrap_or_default();

    if !status.success() {
        bail!("rclone rcat failed ({status}): {}", stderr.trim());
    }

    Ok(())
}
