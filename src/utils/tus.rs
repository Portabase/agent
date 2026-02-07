use anyhow::Result;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};

const PATCH_CHUNK_SIZE: usize = 1 * 1024 * 1024;

pub async fn upload_to_tus_stream_with_headers<S>(
    encrypted_stream: S,
    tus_endpoint: &str,
    extra_headers: HeaderMap,
) -> Result<()>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
{
    let client = reqwest::Client::new();

    let mut headers = HeaderMap::new();
    headers.insert("Tus-Resumable", HeaderValue::from_static("1.0.0"));
    headers.insert("Upload-Defer-Length", HeaderValue::from_static("1"));

    let resp = client
        .post(tus_endpoint)
        .headers(headers.clone())
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("Failed to create upload: {}", resp.status());
    }

    let upload_url = resp
        .headers()
        .get("Location")
        .ok_or_else(|| anyhow::anyhow!("Missing Location header"))?
        .to_str()?
        .to_string();

    let mut stream = Box::pin(
        encrypted_stream.map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))),
    );

    let mut offset: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        for sub_chunk in chunk.chunks(PATCH_CHUNK_SIZE) {
            let mut patch_headers = extra_headers.clone();
            patch_headers.insert("Tus-Resumable", HeaderValue::from_static("1.0.0"));
            patch_headers.insert("Upload-Offset", HeaderValue::from_str(&offset.to_string())?);
            patch_headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/offset+octet-stream"),
            );

            let patch_resp = client
                .patch(&upload_url)
                .headers(patch_headers)
                .body(sub_chunk.to_vec())
                .send()
                .await?;

            if !patch_resp.status().is_success() {
                anyhow::bail!(
                    "Chunk upload failed at offset {}: {}",
                    offset,
                    patch_resp.status()
                );
            }
            offset += sub_chunk.len() as u64;
        }
    }

    let mut finalize_headers = extra_headers.clone();
    finalize_headers.insert("Tus-Resumable", HeaderValue::from_static("1.0.0"));
    finalize_headers.insert("Upload-Offset", HeaderValue::from_str(&offset.to_string())?);
    finalize_headers.insert("Upload-Length", HeaderValue::from_str(&offset.to_string())?);
    finalize_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/offset+octet-stream"),
    );

    let finalize_resp = client
        .patch(&upload_url)
        .headers(finalize_headers)
        .send()
        .await?;

    if !finalize_resp.status().is_success() {
        anyhow::bail!("Failed to finalize upload");
    }

    Ok(())
}
