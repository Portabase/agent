use anyhow::{Context, Result};
use bytes::Bytes;
use futures::{Stream, StreamExt};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use tracing::{error, info};

const PATCH_CHUNK_SIZE: usize = 1 * 1024 * 1024;

pub async fn upload_to_tus_stream_with_headers<S>(
    encrypted_stream: S,
    tus_endpoint: &str,
    extra_headers: HeaderMap,
    total_size: u64,
) -> Result<()>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
{
    let client = reqwest::Client::new();

    info!("File size: {}", total_size);
    info!("Endpoint URL: {}", tus_endpoint);

    let mut create_headers = HeaderMap::new();
    create_headers.insert("Tus-Resumable", HeaderValue::from_static("1.0.0"));
    create_headers.insert("Upload-Defer-Length", HeaderValue::from_static("1"));

    let resp = client
        .post(tus_endpoint)
        .headers(create_headers.clone())
        .send()
        .await
        .context("Failed to send POST to create TUS upload")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = resp
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read body>".into());

        error!(
            "TUS creation failed | status={} | headers={:?} | body={}",
            status, headers, body
        );

        anyhow::bail!(
            "Failed to create upload.\nStatus: {}\nHeaders: {:?}\nBody: {}",
            status,
            headers,
            body
        );
    }

    let upload_url = resp
        .headers()
        .get("Location")
        .context("TUS creation response missing Location header")?
        .to_str()
        .context("Invalid Location header value")?
        .to_string();

    let mut stream = Box::pin(encrypted_stream);
    let mut offset: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Stream produced IO error")?;

        for sub_chunk in chunk.chunks(PATCH_CHUNK_SIZE) {
            let mut patch_headers = extra_headers.clone();
            patch_headers.insert("Tus-Resumable", HeaderValue::from_static("1.0.0"));
            patch_headers.insert(
                "Upload-Offset",
                HeaderValue::from_str(&offset.to_string())
                    .context("Invalid offset header value")?,
            );
            patch_headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/offset+octet-stream"),
            );

            let patch_resp = client
                .patch(&upload_url)
                .headers(patch_headers)
                .body(sub_chunk.to_vec())
                .send()
                .await
                .with_context(|| format!("PATCH request failed at offset {}", offset))?;

            if !patch_resp.status().is_success() {
                let status = patch_resp.status();
                let headers = patch_resp.headers().clone();
                let body = patch_resp
                    .text()
                    .await
                    .unwrap_or_else(|_| "<failed to read body>".into());

                error!(
                    "TUS PATCH failure | offset={} | status={} | body={}",
                    offset, status, body
                );

                anyhow::bail!(
                    "Chunk upload failed.\n\
                     URL: {}\n\
                     Offset: {}\n\
                     Status: {}\n\
                     Headers: {:?}\n\
                     Body: {}",
                    upload_url,
                    offset,
                    status,
                    headers,
                    body
                );
            }

            if let Some(server_offset) = patch_resp.headers().get("Upload-Offset") {
                let server_offset = server_offset
                    .to_str()
                    .context("Invalid Upload-Offset header")?
                    .parse::<u64>()
                    .context("Failed to parse Upload-Offset header")?;

                let expected = offset + sub_chunk.len() as u64;

                if server_offset != expected {
                    anyhow::bail!(
                        "Offset mismatch detected.\nLocal expected: {}\nServer returned: {}",
                        expected,
                        server_offset
                    );
                }
            }

            offset += sub_chunk.len() as u64;
        }
    }

    let mut finalize_headers = extra_headers.clone();
    finalize_headers.insert("Tus-Resumable", HeaderValue::from_static("1.0.0"));
    finalize_headers.insert(
        "Upload-Offset",
        HeaderValue::from_str(&offset.to_string()).context("Invalid finalize offset header")?,
    );
    finalize_headers.insert(
        "Upload-Length",
        HeaderValue::from_str(&offset.to_string()).context("Invalid finalize length header")?,
    );
    finalize_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/offset+octet-stream"),
    );

    let finalize_resp = client
        .patch(&upload_url)
        .headers(finalize_headers)
        .send()
        .await
        .context("Finalize PATCH request failed")?;

    if !finalize_resp.status().is_success() {
        let status = finalize_resp.status();
        let body = finalize_resp
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read body>".into());

        error!(
            "TUS finalize failure | offset={} | status={} | body={}",
            offset, status, body
        );

        anyhow::bail!(
            "Finalize upload failed.\n\
             URL: {}\n\
             Final offset: {}\n\
             Status: {}\n\
             Body: {}",
            upload_url,
            offset,
            status,
            body
        );
    }

    info!("Upload completed successfully. Final size: {}", offset);

    Ok(())
}
