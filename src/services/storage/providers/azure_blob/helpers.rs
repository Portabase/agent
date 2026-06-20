use anyhow::{Context as _, Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{Duration, Utc};
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::sign::Signer;
use url::Url;
use azure_core::http::RequestContent;
use azure_storage_blob::clients::{BlobClient, BlockBlobClient};
use azure_storage_blob::models::BlockLookupList;
use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt};
use std::pin::Pin;
use tracing::info;

#[derive(Debug, Clone)]
pub struct ResolvedAzure {
    pub account_name: String,
    pub account_key: String,
    pub blob_endpoint: String,
}

#[derive(Clone, Copy)]
pub enum SasResource {
    Blob,
    #[allow(dead_code)]
    Container,
}

impl SasResource {
    fn code(self) -> &'static str {
        match self { SasResource::Blob => "b", SasResource::Container => "c" }
    }
}

pub(crate) const SAS_VERSION: &str = "2022-11-02";

pub(crate) fn hmac_sha256_b64(key: &[u8], data: &str) -> Result<String> {
    let pkey = PKey::hmac(key).context("hmac key")?;
    let mut signer = Signer::new(MessageDigest::sha256(), &pkey).context("signer")?;
    signer.update(data.as_bytes()).context("signer update")?;
    let sig = signer.sign_to_vec().context("sign")?;
    Ok(STANDARD.encode(sig))
}

/// Build Service SAS query pairs (raw, un-encoded) for `canonical_resource`
/// e.g. `/blob/{account}/{container}/{blob}`.
pub fn build_service_sas(
    resolved: &ResolvedAzure,
    canonical_resource: &str,
    resource: SasResource,
    permissions: &str,
) -> Result<Vec<(String, String)>> {
    let key = STANDARD
        .decode(&resolved.account_key)
        .map_err(|_| anyhow!("account key is not valid base64"))?;

    let signed_start = String::new();
    let signed_expiry = (Utc::now() + Duration::hours(1))
        .format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let signed_protocol = "https,http"; // Azurite is http
    let signed_resource = resource.code();

    let string_to_sign = format!(
        "{sp}\n{st}\n{se}\n{canon}\n{si}\n{sip}\n{spr}\n{sv}\n{sr}\n{snap}\n{enc}\n{rscc}\n{rscd}\n{rsce}\n{rscl}\n{rsct}",
        sp = permissions, st = signed_start, se = signed_expiry, canon = canonical_resource,
        si = "", sip = "", spr = signed_protocol, sv = SAS_VERSION, sr = signed_resource,
        snap = "", enc = "", rscc = "", rscd = "", rsce = "", rscl = "", rsct = "",
    );

    let sig = hmac_sha256_b64(&key, &string_to_sign)?;

    Ok(vec![
        ("sv".into(), SAS_VERSION.into()),
        ("sr".into(), signed_resource.into()),
        ("sp".into(), permissions.into()),
        ("se".into(), signed_expiry),
        ("spr".into(), signed_protocol.into()),
        ("sig".into(), sig),
    ])
}

/// Build a SAS-scoped URL for a blob (or container when `blob` is empty).
pub fn build_sas_url(
    resolved: &ResolvedAzure,
    container: &str,
    blob: &str,
    resource: SasResource,
    permissions: &str,
) -> Result<Url> {
    let canonical = if blob.is_empty() {
        format!("/blob/{}/{}", resolved.account_name, container)
    } else {
        format!("/blob/{}/{}/{}", resolved.account_name, container, blob)
    };
    let pairs = build_service_sas(resolved, &canonical, resource, permissions)?;

    let base = if blob.is_empty() {
        format!("{}/{}", resolved.blob_endpoint.trim_end_matches('/'), container)
    } else {
        format!("{}/{}/{}", resolved.blob_endpoint.trim_end_matches('/'), container, blob)
    };

    let mut url = Url::parse(&base).context("invalid blob endpoint/url")?;
    {
        let mut qp = url.query_pairs_mut();
        for (k, v) in pairs { qp.append_pair(&k, &v); }
    }
    Ok(url)
}

/// Default block size for the provider path (mirrors the S3 provider's PART_SIZE).
pub const BLOCK_SIZE: usize = 100 * 1024 * 1024;

type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>;

/// Stage one block under a zero-padded sequential id; records the RAW id bytes.
async fn stage_block(
    bbc: &BlockBlobClient,
    index: u32,
    block: Bytes,
    block_ids: &mut Vec<Vec<u8>>,
) -> Result<()> {
    let raw_id = format!("{index:032}").into_bytes();
    let len = block.len() as u64;
    bbc.stage_block(&raw_id, len, RequestContent::from(block.to_vec()), None)
        .await
        .map_err(|e| anyhow!("stage_block {index} failed: {e}"))?;
    block_ids.push(raw_id);
    info!("staged azure block {index} ({len} bytes)");
    Ok(())
}

/// Stream `body` to `{container}/{blob}` using Azure block upload. Never buffers the
/// full payload: at most one `block_size` block plus one inbound chunk is resident
/// (mirrors the S3 provider's per-part guarantee).
///
/// Assumes the container already exists — S3-faithful, no container creation. Uncommitted
/// blocks are garbage-collected by Azure if `commit_block_list` is never reached, so no
/// explicit abort is needed on the error path (unlike S3 multipart).
pub async fn upload_stream_to_azure(
    resolved: &ResolvedAzure,
    container: &str,
    blob: &str,
    mut body: ByteStream,
    block_size: usize,
) -> Result<()> {
    let url = build_sas_url(resolved, container, blob, SasResource::Blob, "cw")?;
    let blob_client = BlobClient::new(url, None, None).context("blob client")?;
    let bbc = blob_client.block_blob_client();

    let mut buffer = BytesMut::with_capacity(block_size);
    let mut block_ids: Vec<Vec<u8>> = Vec::new();
    let mut index: u32 = 0;

    while let Some(item) = body.next().await {
        let bytes = item.context("stream error during upload")?;
        buffer.extend_from_slice(&bytes);

        while buffer.len() >= block_size {
            let block = buffer.split_to(block_size).freeze();
            stage_block(&bbc, index, block, &mut block_ids).await?;
            index += 1;
        }
    }

    if !buffer.is_empty() {
        let block = buffer.split().freeze();
        stage_block(&bbc, index, block, &mut block_ids).await?;
    }

    if block_ids.is_empty() {
        stage_block(&bbc, 0, Bytes::new(), &mut block_ids).await?;
    }

    let block_list = BlockLookupList {
        latest: Some(block_ids),
        ..Default::default()
    };
    bbc.commit_block_list(block_list.try_into()?, None)
        .await
        .map_err(|e| anyhow!("commit_block_list failed: {e}"))?;

    Ok(())
}
