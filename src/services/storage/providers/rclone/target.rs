use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::storage::providers::azure_blob::models::AzureBlobProviderConfig;
use crate::services::storage::providers::google_cloud_storage::helpers::service_account_key;
use crate::services::storage::providers::google_cloud_storage::models::GoogleCloudStorageProviderConfig;
use crate::services::storage::providers::google_drive::models::GoogleDriveProviderConfig;
use crate::services::storage::providers::rclone::helpers::{build_rclone_config, validate_config};
use crate::services::storage::providers::rclone::models::RcloneProviderConfig;
use crate::services::storage::providers::s3::models::S3ProviderConfig;
use crate::services::storage::providers::sftp::helpers::build_sftp_config;
use crate::services::storage::providers::sftp::models::SftpProviderConfig;
use anyhow::{Context, Result, bail};
use tempfile::TempPath;

/// A storage channel expressed as an rclone remote. Temp files the config refers
/// to (e.g. an sftp key) live as long as this value.
pub struct RcloneTarget {
    pub config_text: String,
    pub remote_name: String,
    pub base_path: String,
    _guards: Vec<TempPath>,
}

impl RcloneTarget {
    fn new(config_text: String, remote_name: &str, base_path: String) -> Self {
        Self { config_text, remote_name: remote_name.to_string(), base_path, _guards: Vec::new() }
    }
}

pub fn rclone_target(storage: &DatabaseStorage) -> Result<RcloneTarget> {
    let config = storage.config.clone();
    match storage.provider.as_str() {
        "s3" => s3(config.try_into().context("invalid s3 storage config")?),
        "blob" => blob(config.try_into().context("invalid azure blob storage config")?),
        "google-cloud-storage" => gcs(config.try_into().context("invalid google cloud storage config")?),
        "google-drive" => drive(config.try_into().context("invalid google drive storage config")?),
        "rclone" => user_rclone(config.try_into().context("invalid rclone storage config")?),
        "sftp" => sftp(config.try_into().context("invalid sftp storage config")?),
        "local" => bail!("a local storage channel cannot be used as an rclone target"),
        other => bail!("unknown storage provider '{other}'"),
    }
}

fn s3(c: S3ProviderConfig) -> Result<RcloneTarget> {
    let scheme = if c.ssl { "https" } else { "http" };
    let endpoint = match c.port.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(port) => format!("{scheme}://{}:{port}", c.end_point_url),
        None => format!("{scheme}://{}", c.end_point_url),
    };
    let region = c.region.as_deref().map(str::trim).filter(|r| !r.is_empty()).unwrap_or("us-east-1");
    let fields = [
        ("type", "s3".to_string()),
        ("provider", "Other".to_string()),
        ("access_key_id", c.access_key),
        ("secret_access_key", c.secret_key),
        ("endpoint", endpoint),
        ("region", region.to_string()),
        ("force_path_style", "true".to_string()),
        ("no_check_bucket", "true".to_string()),
    ];
    Ok(RcloneTarget::new(build_rclone_config("s3", &fields)?, "s3", c.bucket_name))
}

fn blob(c: AzureBlobProviderConfig) -> Result<RcloneTarget> {
    let resolved = c.resolve().context("invalid azure blob storage config")?;
    if resolved.account_key.trim().is_empty() {
        bail!("azure blob needs an account key to be used as an rclone target (SAS-only connection strings are not supported)");
    }
    let fields = [
        ("type", "azureblob".to_string()),
        ("account", resolved.account_name),
        ("key", resolved.account_key),
        ("endpoint", resolved.blob_endpoint),
        // Native uploads never create the container; neither should rclone.
        ("no_check_container", "true".to_string()),
    ];
    Ok(RcloneTarget::new(build_rclone_config("blob", &fields)?, "blob", c.container_name))
}

fn gcs(c: GoogleCloudStorageProviderConfig) -> Result<RcloneTarget> {
    let mut fields = vec![
        ("type", "google cloud storage".to_string()),
        ("project_number", c.project_id.clone()),
    ];
    match c.api_endpoint.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
        // A custom endpoint means the fake-gcs emulator, which does not verify credentials.
        // rclone's `endpoint` is the JSON API base path, not the bare host.
        Some(endpoint) => {
            fields.push(("anonymous", "true".to_string()));
            fields.push(("endpoint", format!("{}/storage/v1/", endpoint.trim_end_matches('/'))));
        }
        None => fields.push((
            "service_account_credentials",
            serde_json::to_string(&service_account_key(&c))?,
        )),
    }
    // Uniform bucket-level access (the GCS default) rejects rclone's default
    // `predefinedAcl=private`; and native uploads never create the bucket.
    fields.push(("bucket_policy_only", "true".to_string()));
    fields.push(("no_check_bucket", "true".to_string()));
    Ok(RcloneTarget::new(
        build_rclone_config("google-cloud-storage", &fields)?,
        "google-cloud-storage",
        c.bucket_name,
    ))
}

fn drive(c: GoogleDriveProviderConfig) -> Result<RcloneTarget> {
    let token = serde_json::json!({
        "access_token": "",
        "expiry": "0001-01-01T00:00:00Z",
        "refresh_token": c.refresh_token,
        "token_type": "Bearer",
    });
    let fields = [
        ("type", "drive".to_string()),
        ("client_id", c.client_id),
        ("client_secret", c.client_secret),
        ("scope", "drive.file".to_string()),
        ("token", serde_json::to_string(&token)?),
        ("root_folder_id", c.folder_id),
        // Deletes (retention, restic prune) must free space, not fill the Drive trash.
        ("use_trash", "false".to_string()),
    ];
    Ok(RcloneTarget::new(build_rclone_config("google-drive", &fields)?, "google-drive", String::new()))
}

fn user_rclone(c: RcloneProviderConfig) -> Result<RcloneTarget> {
    validate_config(&c.config_text, &c.remote_name)?;
    Ok(RcloneTarget::new(c.config_text, &c.remote_name, c.remote_path))
}

fn sftp(c: SftpProviderConfig) -> Result<RcloneTarget> {
    let (config_text, key_file) = build_sftp_config(&c)?;
    let mut target = RcloneTarget::new(config_text, "sftp", c.remote_path);
    target._guards.extend(key_file.map(|f| f.into_temp_path()));
    Ok(target)
}
