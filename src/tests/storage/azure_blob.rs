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
    let signed_protocol = "https,http";
    let signed_ip = String::new();
    let encryption_scope = String::new();
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
        .with_wait_for(WaitFor::message_on_stdout(
            "Azurite Blob service successfully listens on",
        ))
        .with_cmd(["azurite-blob", "--blobHost", "0.0.0.0", "--skipApiVersionCheck"])
        .start().await.unwrap();

    let host = container.get_host().await.unwrap().to_string();
    let port = container.get_host_port_ipv4(10000).await.unwrap();
    let resolved = ResolvedAzure {
        account_name: AZURITE_ACCOUNT.to_string(),
        account_key: AZURITE_KEY.to_string(),
        blob_endpoint: format!("http://{host}:{port}/{AZURITE_ACCOUNT}"),
    };
    (container, resolved)
}

#[tokio::test]
async fn spike_sas_block_roundtrip_against_azurite() {
    init_tracing_for_test();
    let (_container, resolved) = start_azurite().await;
    let container = "portabase";
    let blob = "spike/hello.txt";


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

    let block_list = BlockLookupList { latest: Some(vec![raw_id.clone()]), ..Default::default() };
    bbc.commit_block_list(block_list.try_into().unwrap(), None).await.unwrap();

    let read_url = build_sas_url(&resolved, container, blob, SasResource::Blob, "r").unwrap();
    let read_client = BlobClient::new(read_url, None, None).unwrap();
    assert!(read_client.exists().await.unwrap());
}

mod resolve {
    use crate::services::storage::providers::azure_blob::models::{
        AzureBlobProviderConfig, ensure_account_in_endpoint,
    };

    const AZURITE_KEY: &str =
        "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==";
    const CONNECTION_STRING: &str = "DefaultEndpointsProtocol=http;AccountName=devstoreaccount1;AccountKey=Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==;BlobEndpoint=http://localhost:10000/devstoreaccount1;QueueEndpoint=http://localhost:10001/devstoreaccount1;TableEndpoint=http://localhost:10002/devstoreaccount1;";


    #[test]
    fn resolve_connection_string_mode() {
        let cfg = AzureBlobProviderConfig {
            account_name: String::new(),
            account_key: String::new(),
            container_name: "portabase".into(),
            auth_mode: Some("connectionString".into()),
            connection_string: CONNECTION_STRING.into(),
            endpoint_url: None,
        };
        let r = cfg.resolve().unwrap();
        assert_eq!(r.account_name, "devstoreaccount1");
        assert_eq!(r.account_key, AZURITE_KEY);
        assert_eq!(r.blob_endpoint, "http://localhost:10000/devstoreaccount1");
    }


    #[test]
    fn resolve_account_key_mode_injects_account_path() {
        let cfg = AzureBlobProviderConfig {
            account_name: "devstoreaccount1".into(),
            account_key: AZURITE_KEY.into(),
            container_name: "portabase".into(),
            auth_mode: Some("accountKey".into()),
            connection_string: CONNECTION_STRING.into(),
            endpoint_url: Some("http://localhost:10000".into()),
        };
        let r = cfg.resolve().unwrap();
        assert_eq!(r.account_name, "devstoreaccount1");
        assert_eq!(r.account_key, AZURITE_KEY);
        assert_eq!(r.blob_endpoint, "http://localhost:10000/devstoreaccount1");
    }

    #[test]
    fn resolve_implicit_connection_string() {
        let cfg = AzureBlobProviderConfig {
            account_name: String::new(),
            account_key: String::new(),
            container_name: "portabase".into(),
            auth_mode: None,
            connection_string: CONNECTION_STRING.into(),
            endpoint_url: None,
        };
        let r = cfg.resolve().unwrap();
        assert_eq!(r.blob_endpoint, "http://localhost:10000/devstoreaccount1");
    }

    #[test]
    fn resolve_account_key_default_endpoint() {
        let cfg = AzureBlobProviderConfig {
            account_name: "myaccount".into(),
            account_key: AZURITE_KEY.into(),
            container_name: "portabase".into(),
            auth_mode: Some("accountKey".into()),
            connection_string: String::new(),
            endpoint_url: None,
        };
        let r = cfg.resolve().unwrap();
        assert_eq!(r.blob_endpoint, "https://myaccount.blob.core.windows.net");
    }

    #[test]
    fn ensure_account_keeps_host_style_endpoint() {
        let got =
            ensure_account_in_endpoint("https://myaccount.blob.core.windows.net", "myaccount");
        assert_eq!(got, "https://myaccount.blob.core.windows.net");
    }
}

#[tokio::test]
async fn upload_stream_multi_block_roundtrip() {
    init_tracing_for_test();
    use crate::services::storage::providers::azure_blob::helpers::upload_stream_to_azure;
    use futures::stream;

    let (_container, resolved) = start_azurite().await;
    let container = "portabase";
    let blob = "backups/multi.bin";

    let container_url =
        build_account_sas_container_url(&resolved, container, "b", "c", "cw").unwrap();
    azure_storage_blob::clients::BlobContainerClient::new(container_url, None, None)
        .unwrap()
        .create(None)
        .await
        .unwrap();

    let data = vec![7u8; 10 * 1024];
    let chunks: Vec<Result<Bytes, std::io::Error>> = data
        .chunks(1024)
        .map(|c| Ok(Bytes::copy_from_slice(c)))
        .collect();
    let body = Box::pin(stream::iter(chunks));

    upload_stream_to_azure(&resolved, container, blob, body, 4 * 1024)
        .await
        .unwrap();

    let read_url = build_sas_url(&resolved, container, blob, SasResource::Blob, "r").unwrap();
    let got = reqwest::get(read_url).await.unwrap().bytes().await.unwrap();
    assert_eq!(got.as_ref(), data.as_slice());
}
