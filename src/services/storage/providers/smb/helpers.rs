use crate::services::storage::providers::rclone::helpers::{build_rclone_config, obscure_password};
use crate::services::storage::providers::smb::models::SmbProviderConfig;
use anyhow::{Result, bail};

fn validate_port(port: &str) -> Result<()> {
    let port = port
        .parse::<u16>()
        .map_err(|_| anyhow::anyhow!("smb port must be between 1 and 65535"))?;
    if port == 0 {
        bail!("smb port must be between 1 and 65535");
    }
    Ok(())
}

pub fn build_smb_config(config: &SmbProviderConfig) -> Result<String> {
    if config.host.trim().is_empty() {
        bail!("smb host is required");
    }
    if config.share.trim().trim_matches('/').is_empty() {
        bail!("smb share is required");
    }
    if config.username.trim().is_empty() {
        bail!("smb username is required");
    }
    if config.password.trim().is_empty() {
        bail!("smb password is required");
    }

    let port = config.port.as_deref().unwrap_or("").trim();
    if !port.is_empty() {
        validate_port(port)?;
    }

    build_rclone_config(
        "smb",
        &[
            ("type", "smb".to_string()),
            ("host", config.host.trim().to_string()),
            ("port", port.to_string()),
            ("user", config.username.trim().to_string()),
            ("pass", obscure_password(&config.password)?),
        ],
    )
}
