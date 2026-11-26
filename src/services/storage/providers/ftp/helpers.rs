use crate::services::storage::providers::ftp::models::{FtpProviderConfig, FtpsMode};
use crate::services::storage::providers::rclone::helpers::{build_rclone_config, obscure_password};
use anyhow::{Result, bail};

fn validate_port(port: &str) -> Result<()> {
    let port = port
        .parse::<u16>()
        .map_err(|_| anyhow::anyhow!("ftp port must be between 1 and 65535"))?;
    if port == 0 {
        bail!("ftp port must be between 1 and 65535");
    }
    Ok(())
}

pub fn build_ftp_config(config: &FtpProviderConfig) -> Result<String> {
    if config.host.trim().is_empty() {
        bail!("ftp host is required");
    }
    if config.username.trim().is_empty() {
        bail!("ftp username is required");
    }
    if config.password.trim().is_empty() {
        bail!("ftp password is required");
    }
    let port = config.port.as_deref().unwrap_or("").trim();
    if !port.is_empty() {
        validate_port(port)?;
    }
    let fields = [
        ("type", "ftp".to_string()),
        ("host", config.host.trim().to_string()),
        ("port", port.to_string()),
        ("user", config.username.trim().to_string()),
        ("pass", obscure_password(&config.password)?),
        (
            "explicit_tls",
            if config.ftps_mode == FtpsMode::Explicit {
                "true".into()
            } else {
                String::new()
            },
        ),
        (
            "tls",
            if config.ftps_mode == FtpsMode::Implicit {
                "true".into()
            } else {
                String::new()
            },
        ),
    ];
    build_rclone_config("ftp", &fields)
}
