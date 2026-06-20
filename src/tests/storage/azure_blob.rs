use crate::services::storage::providers::azure_blob::helpers::{
    ResolvedAzure, SasResource, build_account_sas_container_url, build_sas_url,
};
use crate::tests::init_tracing_for_test;

use azure_core::http::RequestContent;
use azure_storage_blob::clients::BlobClient;
use azure_storage_blob::models::BlockLookupList;
use bytes::Bytes;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};

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
