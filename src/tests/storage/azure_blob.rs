use crate::services::storage::providers::azure_blob::helpers::{
    ResolvedAzure, SAS_VERSION, SasResource, build_sas_url, hmac_sha256_b64,
};
use crate::tests::init_tracing_for_test;

use anyhow::{Context as _, anyhow};
use azure_core::http::RequestContent;
use azure_storage_blob::clients::BlobClient;
use azure_storage_blob::models::BlockLookupList;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use chrono::{Duration, Utc};
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};
use url::Url;

/// Build an Account SAS query set (test-only). Azurite cannot authorize container-create with
/// a container-scoped Service SAS, so tests create the target container with an Account SAS.
/// Reuses the production HMAC primitive (`hmac_sha256_b64`) to avoid duplicating signing logic.
fn build_account_sas(
    resolved: &ResolvedAzure,
    services: &str,
    resource_types: &str,
    permissions: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    let key = STANDARD
        .decode(&resolved.account_key)
        .map_err(|_| anyhow!("account key is not valid base64"))?;

    let signed_start = String::new();
    let signed_expiry = (Utc::now() + Duration::hours(1))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let signed_protocol = "https,http"; // Azurite is http
    let signed_ip = String::new();
    let encryption_scope = String::new();

    // Account SAS string-to-sign for sv >= 2020-12-06:
    // account \n sp \n ss \n srt \n st \n se \n sip \n spr \n sv \n ses \n  (trailing newline)
    let string_to_sign = format!(
        "{acc}\n{sp}\n{ss}\n{srt}\n{st}\n{se}\n{sip}\n{spr}\n{sv}\n{ses}\n",
        acc = resolved.account_name, sp = permissions, ss = services, srt = resource_types,
        st = signed_start, se = signed_expiry, sip = signed_ip, spr = signed_protocol,
        sv = SAS_VERSION, ses = encryption_scope,
    );

    let sig = hmac_sha256_b64(&key, &string_to_sign)?;

    Ok(vec![
        ("sv".into(), SAS_VERSION.into()),
        ("ss".into(), services.into()),
        ("srt".into(), resource_types.into()),
        ("sp".into(), permissions.into()),
        ("se".into(), signed_expiry),
        ("spr".into(), signed_protocol.into()),
        ("sig".into(), sig),
    ])
}

/// Build an Account-SAS-scoped URL for a container (test-only container creation).
fn build_account_sas_container_url(
    resolved: &ResolvedAzure,
    container: &str,
    services: &str,
    resource_types: &str,
    permissions: &str,
) -> anyhow::Result<Url> {
    let pairs = build_account_sas(resolved, services, resource_types, permissions)?;
    let base = format!("{}/{}", resolved.blob_endpoint.trim_end_matches('/'), container);
    let mut url = Url::parse(&base).context("invalid blob endpoint/url")?;
    {
        let mut qp = url.query_pairs_mut();
        for (k, v) in pairs {
            qp.append_pair(&k, &v);
        }
    }
    Ok(url)
}

const AZURITE_ACCOUNT: &str = "devstoreaccount1";
const AZURITE_KEY: &str =
    "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==";

async fn start_azurite() -> (testcontainers::ContainerAsync<GenericImage>, ResolvedAzure) {
    let container = GenericImage::new("mcr.microsoft.com/azure-storage/azurite", "latest")
        .with_exposed_port(10000.tcp())
        // The current `latest` image logs (on stdout):
        //   "Azurite Blob service successfully listens on http://0.0.0.0:10000"
        // Older builds phrased it "...is successfully listening"; this substring matches
        // the wording the pulled image actually emits.
        .with_wait_for(WaitFor::message_on_stdout(
            "Azurite Blob service successfully listens on",
        ))
        // The GA SDK sends a very recent `x-ms-version`; Azurite 3.35 rejects unknown
        // versions unless we tell it to skip that check.
        .with_cmd(["azurite-blob", "--blobHost", "0.0.0.0", "--skipApiVersionCheck"])
        .start().await.unwrap();
    let port = container.get_host_port_ipv4(10000).await.unwrap();
    let resolved = ResolvedAzure {
        account_name: AZURITE_ACCOUNT.to_string(),
        account_key: AZURITE_KEY.to_string(),
        blob_endpoint: format!("http://127.0.0.1:{port}/{AZURITE_ACCOUNT}"),
    };
    (container, resolved)
}

#[tokio::test]
async fn spike_sas_block_roundtrip_against_azurite() {
    init_tracing_for_test();
    let (_container, resolved) = start_azurite().await;
    let container = "portabase";
    let blob = "spike/hello.txt";

    // Container creation must use an Account SAS (service=blob, resource-type=container,
    // perms=create+write). Azurite cannot authorize container-create with a Service SAS.
    let container_url =
        build_account_sas_container_url(&resolved, container, "b", "c", "cw").unwrap();
    let container_client =
        azure_storage_blob::clients::BlobContainerClient::new(container_url, None, None).unwrap();
    container_client.create(None).await.unwrap();

    let blob_url = build_sas_url(&resolved, container, blob, SasResource::Blob, "cw").unwrap();
    let blob_client = BlobClient::new(blob_url.clone(), None, None).unwrap();
    let bbc = blob_client.block_blob_client();

    let payload = Bytes::from_static(b"hello azurite");
    let raw_id = format!("{:032}", 0u32).into_bytes();
    bbc.stage_block(&raw_id, payload.len() as u64, RequestContent::from(payload.to_vec()), None)
        .await.unwrap();

    // `BlockLookupList.latest` is `Option<Vec<Vec<u8>>>` and base64-encodes each entry
    // internally during XML serialization, exactly as `stage_block` base64-encodes the
    // `blockid` query. So `latest` must hold the SAME RAW id bytes passed to `stage_block`.
    let block_list = BlockLookupList { latest: Some(vec![raw_id.clone()]), ..Default::default() };
    bbc.commit_block_list(block_list.try_into().unwrap(), None).await.unwrap();

    let read_url = build_sas_url(&resolved, container, blob, SasResource::Blob, "r").unwrap();
    let read_client = BlobClient::new(read_url, None, None).unwrap();
    assert!(read_client.exists().await.unwrap());
}

#[tokio::test]
async fn upload_stream_multi_block_roundtrip() {
    init_tracing_for_test();
    use crate::services::storage::providers::azure_blob::helpers::upload_stream_to_azure;
    use futures::stream;

    let (_container, resolved) = start_azurite().await;
    let container = "portabase";
    let blob = "backups/multi.bin";

    // Container setup (provider itself never creates it): Account SAS create.
    let container_url =
        build_account_sas_container_url(&resolved, container, "b", "c", "cw").unwrap();
    azure_storage_blob::clients::BlobContainerClient::new(container_url, None, None)
        .unwrap()
        .create(None)
        .await
        .unwrap();

    // 10 KiB fed as 1 KiB chunks, forced into 4 KiB blocks => 3 blocks (multi-block path).
    let data = vec![7u8; 10 * 1024];
    let chunks: Vec<Result<Bytes, std::io::Error>> = data
        .chunks(1024)
        .map(|c| Ok(Bytes::copy_from_slice(c)))
        .collect();
    let body = Box::pin(stream::iter(chunks));

    upload_stream_to_azure(&resolved, container, blob, body, 4 * 1024)
        .await
        .unwrap();

    // Verify the committed blob reassembles to the exact source bytes via a read-SAS GET.
    let read_url = build_sas_url(&resolved, container, blob, SasResource::Blob, "r").unwrap();
    let got = reqwest::get(read_url).await.unwrap().bytes().await.unwrap();
    assert_eq!(got.as_ref(), data.as_slice());
}
