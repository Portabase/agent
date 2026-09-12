use crate::services::storage::providers::sftp::models::SftpProviderConfig;
use anyhow::{Context, Result, bail};
use std::io::Write;
use std::process::Command;
use tempfile::NamedTempFile;

pub fn obscure_password(password: &str) -> Result<String> {
    let out = Command::new("rclone")
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

fn write_key(private_key: &str) -> Result<NamedTempFile> {
    let mut file = NamedTempFile::new().context("failed to create sftp key temp file")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o600))
            .context("failed to restrict sftp key permissions")?;
    }

    file.write_all(private_key.as_bytes())
        .context("failed to write sftp key")?;
    file.flush().context("failed to flush sftp key")?;
    Ok(file)
}

pub fn build_sftp_config(
    config: &SftpProviderConfig,
) -> Result<(String, Option<NamedTempFile>)> {
    if config.host.trim().is_empty() {
        bail!("sftp host is required");
    }
    if config.username.trim().is_empty() {
        bail!("sftp username is required");
    }

    let has_password = config.password.as_deref().is_some_and(|p| !p.trim().is_empty());
    let has_key = config.private_key.as_deref().is_some_and(|k| !k.trim().is_empty());
    if !has_password && !has_key {
        bail!("sftp requires a password or a private key");
    }

    let mut lines = vec![
        "[sftp]".to_string(),
        "type = sftp".to_string(),
        format!("host = {}", config.host.trim()),
    ];

    if let Some(port) = config.port.as_deref() {
        let port = port.trim();
        if !port.is_empty() {
            lines.push(format!("port = {port}"));
        }
    }

    lines.push(format!("user = {}", config.username.trim()));

    let mut key_file: Option<NamedTempFile> = None;
    if has_key {
        let file = write_key(config.private_key.as_deref().unwrap())?;
        lines.push(format!("key_file = {}", file.path().display()));
        key_file = Some(file);
    }

    if has_password {
        let obscured = obscure_password(config.password.as_deref().unwrap())?;
        lines.push(format!("pass = {obscured}"));
    }

    Ok((lines.join("\n") + "\n", key_file))
}
