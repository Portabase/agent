use crate::services::storage::providers::rclone::helpers::{build_rclone_config, obscure_password};
use crate::services::storage::providers::sftp::models::SftpProviderConfig;
use anyhow::{Context, Result, bail};
use std::io::Write;
use tempfile::NamedTempFile;

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

    let mut key_file: Option<NamedTempFile> = None;
    let mut key_file_path = String::new();
    if has_key {
        let file = write_key(config.private_key.as_deref().unwrap())?;
        key_file_path = file.path().display().to_string();
        key_file = Some(file);
    }

    let pass = if has_password {
        obscure_password(config.password.as_deref().unwrap())?
    } else {
        String::new()
    };

    let port = config
        .port
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();

    let fields: &[(&str, String)] = &[
        ("type", "sftp".to_string()),
        ("host", config.host.trim().to_string()),
        ("port", port),
        ("user", config.username.trim().to_string()),
        ("key_file", key_file_path),
        ("pass", pass),
    ];

    let config_text = build_rclone_config("sftp", fields)?;
    Ok((config_text, key_file))
}
