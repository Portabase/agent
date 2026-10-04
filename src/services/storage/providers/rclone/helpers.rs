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

/// Entries must not contain spaces: the type is compared with its spaces removed (rclone resolves a
/// backend by name, registered prefix, or name without spaces: `google photos` = `googlephotos`).
const BLOCKED_BACKEND_TYPES: [&str; 14] = [
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
    "gphotos",
];

fn valid_remote_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '@' | '-'))
}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn forbidden_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "ssh" | "env_auth" | "use_msi" | "use_az" | "use_kerberos" | "kerberos_ccache" | "key_use_agent" | "set_env" | "unix_socket"
    ) || key.ends_with("_command")
        || key.ends_with("_file")
        || key.ends_with("_path")
}

/// Strict check for user-pasted rclone configs (never for configs Portabase builds itself).
/// Exactly one `[remote]` of plain `key = value` lines: rclone and a naive parser must read it
/// identically, and nothing may run a local command or read host credentials.
pub fn validate_config(config_text: &str, remote_name: &str) -> Result<()> {
    let mut names = Vec::new();
    let mut pairs = Vec::new();
    let mut before_header = None;

    for (i, line) in config_text.split('\n').enumerate() {
        let (n, line) = (i + 1, line.trim());
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            names.push(line[1..line.len() - 1].trim());
        } else if let Some((key, value)) = line.split_once('=').filter(|(k, _)| !k.is_empty()) {
            if names.is_empty() {
                before_header.get_or_insert(n);
            }
            pairs.push((key.trim(), value.trim()));
        } else {
            bail!("rclone config line {n} is not a [section] header or a 'key = value' pair");
        }
    }

    if let Some(n) = before_header {
        bail!("rclone config line {n} appears before any [section]");
    }
    if names.len() != 1 {
        bail!("rclone config must contain exactly one [section] (found {})", names.len());
    }
    let name = names[0];
    if !valid_remote_name(name) {
        bail!("invalid rclone remote name '{name}'");
    }
    for (key, _) in &pairs {
        if !valid_key(key) {
            bail!("invalid rclone config key '{key}' (remote '{name}')");
        }
    }
    // rclone (goconfig) unquotes `value` and `"""value"""` and drops what follows the closing quote.
    for (key, value) in &pairs {
        if value.starts_with('`') || value.starts_with("\"\"\"") {
            bail!("rclone config value for key '{key}' must not be quoted (remote '{name}')");
        }
    }
    let mut seen = std::collections::HashSet::new();
    for (key, _) in &pairs {
        if !seen.insert(key.to_ascii_lowercase()) {
            bail!("duplicate rclone config key '{key}' (remote '{name}')");
        }
    }
    for (key, _) in &pairs {
        if forbidden_key(key) {
            bail!("rclone config key '{key}' is not allowed (remote '{name}')");
        }
    }
    let Some((_, backend)) = pairs.iter().find(|(k, _)| k.eq_ignore_ascii_case("type")) else {
        bail!("rclone remote '{name}' has no type");
    };
    let backend = backend.to_ascii_lowercase();
    if backend.is_empty() || !backend.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == ' ') {
        bail!("invalid rclone backend type '{backend}' (remote '{name}')");
    }
    let compact = backend.replace(' ', "");
    if BLOCKED_BACKEND_TYPES.contains(&compact.as_str()) {
        bail!("rclone backend type '{backend}' is not allowed (remote '{name}')");
    }
    // The other OCI providers read the host's OCI config or instance identity. Exact match on
    // purpose: rclone 1.75.1 compares the key and the value case-sensitively, and falls back to
    // host credentials for `PROVIDER = no_auth` (key ignored) and `provider = NO_AUTH` (unknown value).
    if compact == "oracleobjectstorage" {
        if !pairs.contains(&("provider", "no_auth")) {
            bail!("rclone oracleobjectstorage remotes must use provider = no_auth (remote '{name}')");
        }
    }
    if remote_name != name {
        bail!("remote '{remote_name}' is not defined in the rclone config (available: {name})");
    }

    Ok(())
}

/// `rclone obscure -` reads the password from stdin, so it never shows up in argv.
pub fn obscure_password(password: &str) -> Result<String> {
    let mut child = std::process::Command::new("rclone")
        .arg("obscure")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn rclone (is the binary installed in this image?)")?;

    // Dropping the handle closes stdin, which is what ends rclone's read.
    child
        .stdin
        .take()
        .context("rclone stdin unavailable")?
        .write_all(password.as_bytes())
        .context("failed to write the password to rclone obscure")?;

    let out = child.wait_with_output().context("failed to wait for rclone obscure")?;

    if !out.status.success() {
        bail!(
            "rclone obscure failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

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
