use crate::services::storage::providers::google_drive::models::GoogleDriveProviderConfig;
use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use futures::Stream;
use futures::StreamExt;
use reqwest::{Client as ReqwestClient, StatusCode};
use reqwest::{Client, header};
use serde_json::{Value, json};

pub const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";
pub const DRIVE_RESUMABLE_UPLOAD_URL: &str =
    "https://www.googleapis.com/upload/drive/v3/files?uploadType=resumable&supportsAllDrives=true";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

pub async fn get_google_drive_token(config: &GoogleDriveProviderConfig) -> Result<String> {
    exchange_refresh_token(&ReqwestClient::new(), GOOGLE_TOKEN_URL, config).await
}

/// OAuth 2.0 refresh-token grant (RFC 6749 §6), client credentials in the body.
pub(crate) async fn exchange_refresh_token(
    client: &Client,
    token_url: &str,
    config: &GoogleDriveProviderConfig,
) -> Result<String> {
    let response = client
        .post(token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", config.refresh_token.as_str()),
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
        ])
        .send()
        .await
        .context("failed to exchange refresh token")?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read token response")?;
    if !status.is_success() {
        return Err(anyhow!(
            "failed to exchange refresh token: {status}: {body}"
        ));
    }

    let token: Value = serde_json::from_str(&body).context("invalid token response")?;
    token["access_token"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("token response has no access_token"))
}

/// A file or folder returned by a Drive `files.list` call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriveItem {
    pub id: String,
    pub created_time: String,
}

/// Drive allows several folders with the same name under one parent. Every
/// caller must resolve a name to the same folder, so pick the oldest one
/// (ties broken by id) instead of whatever the list happens to return first.
pub fn pick_oldest(items: &[DriveItem]) -> Option<&DriveItem> {
    items.iter().min_by(|a, b| {
        a.created_time
            .cmp(&b.created_time)
            .then_with(|| a.id.cmp(&b.id))
    })
}

/// Parses a `files.list` response. A non-2xx status or a body without a
/// `files` array is an error: treating it as "nothing found" makes callers
/// create a duplicate folder or upload over an existing file.
pub fn parse_files_list(status: u16, body: &str) -> Result<Vec<DriveItem>> {
    if !(200..300).contains(&status) {
        return Err(anyhow!("Drive files.list failed: HTTP {status}: {body}"));
    }
    let value: Value = serde_json::from_str(body).context("Drive files.list: invalid JSON")?;
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Drive files.list: response has no files array"))?;
    Ok(files
        .iter()
        .filter_map(|file| {
            Some(DriveItem {
                id: file.get("id")?.as_str()?.to_string(),
                created_time: file
                    .get("createdTime")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        })
        .collect())
}

/// Escapes a value for a single-quoted string in a Drive search query.
pub fn escape_query_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

pub(crate) async fn list_files(
    client: &Client,
    api_base: &str,
    token: &str,
    query: &str,
) -> Result<Vec<DriveItem>> {
    let res = client
        .get(format!("{api_base}/files"))
        .bearer_auth(token)
        .query(&[
            ("q", query),
            ("fields", "files(id,createdTime)"),
            ("orderBy", "createdTime"),
            ("pageSize", "100"),
            ("supportsAllDrives", "true"),
            ("includeItemsFromAllDrives", "true"),
            ("corpora", "allDrives"),
        ])
        .send()
        .await
        .context("Drive files.list request failed")?;
    let status = res.status().as_u16();
    let body = res.text().await.unwrap_or_default();
    parse_files_list(status, &body)
}

async fn list_folders_named(
    client: &Client,
    api_base: &str,
    token: &str,
    parent_id: &str,
    name: &str,
) -> Result<Vec<DriveItem>> {
    let query = format!(
        "'{}' in parents and name='{}' and mimeType='application/vnd.google-apps.folder' and trashed=false",
        escape_query_value(parent_id),
        escape_query_value(name)
    );
    list_files(client, api_base, token, &query).await
}

async fn create_folder(
    client: &Client,
    api_base: &str,
    token: &str,
    parent_id: &str,
    name: &str,
) -> Result<String> {
    let res = client
        .post(format!("{api_base}/files?supportsAllDrives=true"))
        .bearer_auth(token)
        .json(&json!({
            "name": name,
            "mimeType": "application/vnd.google-apps.folder",
            "parents": [parent_id],
        }))
        .send()
        .await
        .context("Drive folder create request failed")?;
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("Drive folder create failed: HTTP {status}: {body}"));
    }
    let folder: Value = serde_json::from_str(&body).context("Drive folder create: invalid JSON")?;
    folder["id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("No id returned after folder creation"))
}

/// Resolves `path_parts` under `root_id`, creating missing folders. After a
/// create the name is listed again: when a concurrent caller created the same
/// folder, everybody converges on the oldest one.
pub(crate) async fn ensure_folder_path_at(
    client: &Client,
    api_base: &str,
    token: &str,
    root_id: &str,
    path_parts: &[&str],
) -> Result<String> {
    let mut parent_id = root_id.to_string();

    for &name in path_parts {
        let existing = list_folders_named(client, api_base, token, &parent_id, name).await?;
        if let Some(oldest) = pick_oldest(&existing) {
            parent_id = oldest.id.clone();
            continue;
        }

        let created_id = create_folder(client, api_base, token, &parent_id, name).await?;
        let listed = list_folders_named(client, api_base, token, &parent_id, name).await?;
        parent_id = pick_oldest(&listed)
            .map(|oldest| oldest.id.clone())
            .unwrap_or(created_id);
    }

    Ok(parent_id)
}

pub async fn ensure_folder_path(
    config: &GoogleDriveProviderConfig,
    path_parts: &[&str],
) -> Result<String> {
    if path_parts.is_empty() {
        return Ok(config.folder_id.clone());
    }

    let token = get_google_drive_token(config).await?;
    ensure_folder_path_at(
        &ReqwestClient::new(),
        DRIVE_API_BASE,
        &token,
        &config.folder_id,
        path_parts,
    )
    .await
}

pub async fn find_file_by_name(
    config: &GoogleDriveProviderConfig,
    file_name: &str,
    folder_id: &str,
) -> Result<Option<String>> {
    let token = get_google_drive_token(config).await?;
    let query = format!(
        "'{}' in parents and name='{}' and trashed=false",
        escape_query_value(folder_id),
        escape_query_value(file_name)
    );
    let files = list_files(&ReqwestClient::new(), DRIVE_API_BASE, &token, &query).await?;
    Ok(pick_oldest(&files).map(|file| file.id.clone()))
}

pub async fn upload_stream_to_google_drive(
    config: &GoogleDriveProviderConfig,
    full_path: &str,
    content_stream: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + Unpin + 'static,
    total_size: u64,
    mime_type: Option<&str>,
) -> Result<()> {
    let path_parts: Vec<&str> = full_path.split('/').filter(|s| !s.is_empty()).collect();
    if path_parts.is_empty() {
        return Err(anyhow::anyhow!("Invalid path: empty"));
    }

    let file_name = *path_parts.last().unwrap();
    let folder_path = &path_parts[..path_parts.len() - 1];

    let folder_id = ensure_folder_path(config, folder_path).await?;

    if find_file_by_name(config, file_name, &folder_id)
        .await?
        .is_some()
    {
        return Err(anyhow::anyhow!("File already exists: {}", full_path));
    }

    let token = get_google_drive_token(config).await?;
    let client = Client::new();

    let mime = mime_type.unwrap_or("application/octet-stream");

    let metadata = json!({
        "name": file_name,
        "parents": [folder_id],
        "mimeType": mime,
    });

    let session_res = client
        .post(DRIVE_RESUMABLE_UPLOAD_URL)
        .bearer_auth(&token)
        .header("X-Upload-Content-Type", mime)
        .json(&metadata)
        .send()
        .await
        .context("Failed to initiate resumable upload")?;

    if session_res.status() != StatusCode::OK {
        let text = session_res.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Initiate failed: {}", text));
    }

    let upload_url = session_res
        .headers()
        .get(header::LOCATION)
        .ok_or_else(|| anyhow::anyhow!("No Location header"))?
        .to_str()?
        .to_string();

    upload_chunks(&client, &upload_url, content_stream, total_size).await
}

pub async fn upload_chunks(
    client: &Client,
    upload_url: &str,
    mut content_stream: impl Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
    source_size: u64,
) -> Result<()> {
    const CHUNK_SIZE: usize = 8 * 1024 * 1024;

    let mut uploaded: u64 = 0;
    let mut pending = Bytes::new();
    let mut stream_ended = false;

    while !stream_ended {
        let mut chunk_bytes = Vec::with_capacity(CHUNK_SIZE);

        loop {
            if pending.is_empty() {
                match content_stream.next().await {
                    Some(Ok(bytes)) => pending = bytes,
                    Some(Err(e)) => return Err(e).context("Stream error during chunk"),
                    None => {
                        stream_ended = true;
                        break;
                    }
                }
            } else if chunk_bytes.len() < CHUNK_SIZE {
                let to_take = (CHUNK_SIZE - chunk_bytes.len()).min(pending.len());
                chunk_bytes.extend_from_slice(&pending.split_to(to_take));
            } else {
                break;
            }
        }

        let chunk_end = uploaded + chunk_bytes.len() as u64;
        if stream_ended && chunk_end < source_size {
            return Err(anyhow::anyhow!("Stream ended early"));
        }

        let content_range = match (chunk_bytes.is_empty(), stream_ended) {
            (true, _) => format!("bytes */{chunk_end}"),
            (false, true) => format!("bytes {uploaded}-{}/{chunk_end}", chunk_end - 1),
            (false, false) => format!("bytes {uploaded}-{}/*", chunk_end - 1),
        };
        let chunk_bytes = Bytes::from(chunk_bytes);

        let mut retries = 0;
        loop {
            let res = client
                .put(upload_url)
                .header("Content-Range", &content_range)
                .header("Content-Length", chunk_bytes.len().to_string())
                .body(chunk_bytes.clone())
                .send()
                .await;

            match res {
                Ok(resp)
                    if resp.status().is_success()
                        || resp.status() == StatusCode::PERMANENT_REDIRECT =>
                {
                    // 200 or 308 = good
                    uploaded = chunk_end;
                    tracing::info!("Uploaded {} bytes", uploaded);
                    break;
                }
                Ok(resp) if resp.status() == StatusCode::TOO_MANY_REQUESTS => {
                    // Backoff on 429
                    tokio::time::sleep(std::time::Duration::from_secs(5 * (1 << retries))).await;
                }
                Ok(resp) => {
                    let text = resp.text().await.unwrap_or_default();
                    return Err(anyhow::anyhow!("Chunk upload failed: {}", text));
                }
                Err(e) if e.is_timeout() || e.is_connect() => {
                    if retries > 5 {
                        return Err(e).context("Too many retries");
                    }
                    retries += 1;
                    tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(retries))).await;
                }
                Err(e) => return Err(e).context("Chunk request failed"),
            }
        }
    }

    Ok(())
}
