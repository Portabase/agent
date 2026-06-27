use crate::services::storage::providers::google_cloud_storage::helpers::{
    StreamSource, upload_with_client,
};
use crate::tests::init_tracing_for_test;

use bytes::Bytes;
use futures::stream;
use google_cloud_storage::client::Storage;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};

const BUCKET: &str = "portabase";

async fn start_fake_gcs() -> (testcontainers::ContainerAsync<GenericImage>, String) {
    // Natural random host port (no port-80 pin). The provider forces a single-shot
    // upload for custom endpoints, which issues one request to this endpoint and never
    // follows a server-built `Location` — so it works on any port, unlike the resumable
    // path that the SDK's Host-header port-drop bug breaks on non-443 ports.
    let container = GenericImage::new("fsouza/fake-gcs-server", "latest")
        .with_exposed_port(4443.tcp())
        .with_wait_for(WaitFor::message_on_stderr("server started at"))
        .with_cmd(["-scheme", "http", "-backend", "memory", "-port", "4443"])
        .start()
        .await
        .unwrap();

    let host = container.get_host().await.unwrap().to_string();
    let port = container.get_host_port_ipv4(4443).await.unwrap();
    let endpoint = format!("http://{host}:{port}");
    (container, endpoint)
}

async fn anon_client(endpoint: &str) -> Storage {
    let creds = google_cloud_auth::credentials::anonymous::Builder::new().build();
    Storage::builder()
        .with_credentials(creds)
        .with_endpoint(endpoint.to_string())
        .build()
        .await
        .unwrap()
}

async fn create_bucket(endpoint: &str) {
    let url = format!("{endpoint}/storage/v1/b?project=test-project");
    let res = reqwest::Client::new()
        .post(&url)
        .json(&serde_json::json!({ "name": BUCKET }))
        .send()
        .await
        .unwrap();
    assert!(
        res.status().is_success(),
        "bucket create failed: {}",
        res.status()
    );
}

#[tokio::test]
async fn upload_stream_roundtrip_against_fake_gcs() {
    init_tracing_for_test();
    let (_container, endpoint) = start_fake_gcs().await;
    create_bucket(&endpoint).await;

    let object = "backups/multi.bin";

    // 10 KiB fed as 1 KiB chunks -> multi-chunk streaming path.
    let data = vec![7u8; 10 * 1024];
    let chunks: Vec<Result<Bytes, std::io::Error>> = data
        .chunks(1024)
        .map(|c| Ok(Bytes::copy_from_slice(c)))
        .collect();
    let source = StreamSource::from_stream(Box::pin(stream::iter(chunks)), data.len() as u64);

    let client = anon_client(&endpoint).await;

    // force_single_shot = true (custom endpoint). Guard with a timeout so a regression
    // into the resumable path (which would hang forever on this non-443 port) fails the
    // test instead of stalling it.
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        upload_with_client(&client, BUCKET, object, source, true),
    )
    .await
    .expect("upload hung (regressed to resumable path on a non-443 endpoint?)")
    .unwrap();

    let read_url = format!(
        "{endpoint}/storage/v1/b/{BUCKET}/o/{}?alt=media",
        object.replace('/', "%2F")
    );
    let got = reqwest::get(&read_url)
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(got.as_ref(), data.as_slice());
}
