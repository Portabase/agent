use crate::services::storage::providers::google_drive::models::GoogleDriveProviderConfig;
use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use futures::Stream;
use futures::StreamExt;
use oauth2::{
    basic::BasicClient, reqwest::Client as OAuth2ReqwestClient, AuthUrl, ClientId, ClientSecret,
    RefreshToken, TokenResponse, TokenUrl,
};
use reqwest::{header, Client};
use reqwest::{Client as ReqwestClient, StatusCode};
use serde_json::{json, Value};

pub const DRIVE_FILES_URL: &str =
    "https://www.googleapis.com/drive/v3/files?supportsAllDrives=true";
pub const DRIVE_RESUMABLE_UPLOAD_URL: &str =
    "https://www.googleapis.com/upload/drive/v3/files?uploadType=resumable&supportsAllDrives=true";

pub fn append_from_pending(buf: &mut Vec<u8>, pending: &mut Bytes, remaining: &mut u64) {
    if pending.is_empty() || *remaining == 0 {
        return;
    }
    let to_take = (*remaining as usize).min(pending.len());
    buf.extend_from_slice(&pending[..to_take]);
    *pending = pending.slice(to_take..);
    *remaining -= to_take as u64;
}

pub fn collect_windows(chunks: Vec<Bytes>, total: u64, window: u64) -> Result<Vec<Vec<u8>>> {
    let mut pending = Bytes::new();
    let mut iter = chunks.into_iter();
    let mut uploaded = 0u64;
    let mut windows = Vec::new();

    while uploaded < total {
        let chunk_size = (total - uploaded).min(window);
        let mut buf = Vec::with_capacity(chunk_size as usize);
        let mut remaining = chunk_size;

        while remaining > 0 {
            if pending.is_empty() {
                match iter.next() {
                    Some(next) => pending = next,
                    None => {
                        if uploaded + buf.len() as u64 != total {
                            return Err(anyhow!("Stream ended early"));
                        }
                        break;
                    }
                }
            }
            append_from_pending(&mut buf, &mut pending, &mut remaining);
        }

        if buf.is_empty() && uploaded < total {
            return Err(anyhow!("Unexpected end of stream"));
        }
        uploaded += buf.len() as u64;
        windows.push(buf);
    }

    Ok(windows)
}

pub async fn get_google_drive_token(config: &GoogleDriveProviderConfig) -> Result<String> {
    let http_client = OAuth2ReqwestClient::new();

    let oauth_client = BasicClient::new(ClientId::new(config.client_id.clone()))
        .set_client_secret(ClientSecret::new(config.client_secret.clone()))
        .set_auth_uri(
            AuthUrl::new("https://accounts.google.com/o/oauth2/auth".to_string())
                .context("invalid auth uri")?,
        )
        .set_token_uri(
            TokenUrl::new("https://oauth2.googleapis.com/token".to_string())
                .context("invalid token uri")?,
        );

    let token_result = oauth_client
        .exchange_refresh_token(&RefreshToken::new(config.refresh_token.clone()))
        .request_async(&http_client)
        .await
        .context("failed to exchange refresh token")?;

    Ok(token_result.access_token().secret().clone())
}

pub async fn ensure_folder_path(
    config: &GoogleDriveProviderConfig,
    path_parts: &[&str],
) -> Result<String> {
    if path_parts.is_empty() {
        return Ok(config.folder_id.clone());
    }

    let token = get_google_drive_token(config).await?;
    let client = ReqwestClient::new();
    let mut parent_id = config.folder_id.clone();

    for &name in path_parts {
        let query = format!(
            "'{parent_id}' in parents and name='{name}' and mimeType='application/vnd.google-apps.folder' and trashed=false"
        );

        let res = client
            .get("https://www.googleapis.com/drive/v3/files")
            .bearer_auth(&token)
            .query(&[
                ("q", query),
                ("fields", "files(id,name)".to_string()),
                ("supportsAllDrives", "true".to_string()),
                ("includeItemsFromAllDrives", "true".to_string()),
                ("corpora", "allDrives".to_string()),
            ])
            .send()
            .await
            .context("list folders failed")?
            .json::<Value>()
            .await?;

        if let Some(files) = res["files"].as_array() {
            if let Some(folder) = files.first() {
                if let Some(id) = folder["id"].as_str() {
                    parent_id = id.to_string();
                    continue;
                }
            }
        }

        let create_payload = json!({
            "name": name,
            "mimeType": "application/vnd.google-apps.folder",
            "parents": [parent_id],
        });

        let folder = client
            .post(DRIVE_FILES_URL)
            .bearer_auth(&token)
            .json(&create_payload)
            .send()
            .await
            .context("create folder failed")?
            .json::<Value>()
            .await?;

        parent_id = folder["id"]
            .as_str()
            .ok_or_else(|| anyhow!("No id returned after folder creation"))?
            .to_string();
    }

    Ok(parent_id)
}

pub async fn find_file_by_name(
    config: &GoogleDriveProviderConfig,
    file_name: &str,
    folder_id: &str,
) -> Result<Option<String>> {
    let token = get_google_drive_token(config).await?;
    let client = ReqwestClient::new();

    let query = format!("'{folder_id}' in parents and name='{file_name}' and trashed=false");

    let res = client
        .get("https://www.googleapis.com/drive/v3/files")
        .bearer_auth(&token)
        .query(&[
            ("q", query),
            ("fields", "files(id,name)".to_string()),
            ("supportsAllDrives", "true".to_string()),
            ("includeItemsFromAllDrives", "true".to_string()),
            ("corpora", "allDrives".to_string()),
        ])
        .send()
        .await?
        .json::<Value>()
        .await?;

    if let Some(files) = res["files"].as_array() {
        if let Some(file) = files.first() {
            if let Some(id) = file["id"].as_str() {
                return Ok(Some(id.to_string()));
            }
        }
    }

    Ok(None)
}

pub async fn upload_stream_to_google_drive(
    config: &GoogleDriveProviderConfig,
    full_path: &str,
    mut content_stream: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + Unpin + 'static,
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
        .header("X-Upload-Content-Length", total_size.to_string()) // Helps a lot
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

    const CHUNK_SIZE: u64 = 8 * 1024 * 1024;

    let mut uploaded: u64 = 0;
    let mut pending = Bytes::new();

    while uploaded < total_size {
        let chunk_size = (total_size - uploaded).min(CHUNK_SIZE);

        let mut chunk_bytes = Vec::with_capacity(chunk_size as usize);
        let mut remaining = chunk_size;

        while remaining > 0 {
            if pending.is_empty() {
                match content_stream.next().await {
                    Some(Ok(bytes)) => pending = bytes,
                    Some(Err(e)) => return Err(e).context("Stream error during chunk"),
                    None => {
                        if uploaded + chunk_bytes.len() as u64 != total_size {
                            return Err(anyhow::anyhow!("Stream ended early"));
                        }
                        break;
                    }
                }
            }
            append_from_pending(&mut chunk_bytes, &mut pending, &mut remaining);
        }

        if chunk_bytes.is_empty() && uploaded < total_size {
            return Err(anyhow::anyhow!("Unexpected end of stream"));
        }

        let range_end = uploaded + chunk_bytes.len() as u64 - 1;
        let content_range = if uploaded + chunk_bytes.len() as u64 == total_size {
            format!("bytes {}-{}/{}", uploaded, range_end, total_size)
        } else {
            format!("bytes {}-{}/*", uploaded, range_end)
        };

        let mut retries = 0;
        loop {
            let res = client
                .put(&upload_url)
                .header("Content-Range", &content_range)
                .header("Content-Length", chunk_bytes.len().to_string())
                .body(chunk_bytes.clone()) // clone is cheap if small; optimize later if needed
                .send()
                .await;

            match res {
                Ok(resp)
                    if resp.status().is_success()
                        || resp.status() == StatusCode::PERMANENT_REDIRECT =>
                {
                    // 200 or 308 = good
                    uploaded += chunk_bytes.len() as u64;
                    tracing::info!("Uploaded {}/{} bytes", uploaded, total_size);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_mib_chunk_survives_eight_mib_windows() {
        let window = 8 * 1024 * 1024;
        let payload = vec![7u8; 16 * 1024 * 1024];
        let windows = collect_windows(
            vec![Bytes::from(payload.clone())],
            payload.len() as u64,
            window,
        )
        .expect("windows");

        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].len(), window as usize);
        assert_eq!(windows[1].len(), window as usize);
        let mut joined = windows[0].clone();
        joined.extend_from_slice(&windows[1]);
        assert_eq!(joined, payload);
    }

    #[test]
    fn drive_write_urls_carry_supports_all_drives() {
        assert!(DRIVE_RESUMABLE_UPLOAD_URL.contains("supportsAllDrives=true"));
        assert!(DRIVE_RESUMABLE_UPLOAD_URL.contains("uploadType=resumable"));
        assert!(DRIVE_FILES_URL.contains("supportsAllDrives=true"));
    }
}
