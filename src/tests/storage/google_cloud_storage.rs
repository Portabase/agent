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

/// Start fake-gcs-server (the standard GCS emulator) over plain HTTP and return
/// (container, endpoint_base_url).
///
/// We pin the host-side port to 80 (the implicit default port for `http://`)
/// rather than letting Docker assign a random one. This works around a real
/// defect in `google-cloud-gax-internal` (the transport crate underpinning
/// the `google-cloud-storage` SDK): `crate::host::header()` computes the
/// `Host:` header it sends on every request from `Uri::authority().host()`,
/// which **always discards the port**, for *any* custom (non-`googleapis.com`)
/// endpoint - see
/// `google-cloud-gax-internal-0.7.14/src/host.rs::origin_and_header`. So with
/// `with_endpoint("http://localhost:<random-port>")` the SDK actually sends
/// `Host: localhost` (no port) on the wire. fake-gcs-server builds the
/// resumable-upload `Location` header by reflecting that inbound `Host`
/// header verbatim (ignoring `-external-url` when a `Host` header is
/// present), so the session URL it hands back is `http://localhost/...`
/// with no port. The SDK then follows that URL for every chunk PUT and
/// connects to the implicit default port 80, where nothing is listening,
/// and retries forever. Confirmed by hand with curl: sending
/// `Host: localhost` (no port) to the emulator reproduces a portless
/// `Location` header byte-for-byte.
///
/// Pinning the container's published port to 80 makes the *coincidentally*
/// correct: "no port" on the wire now legitimately means port 80, which is
/// where the emulator actually is.
async fn start_fake_gcs() -> (testcontainers::ContainerAsync<GenericImage>, String) {
    let container = GenericImage::new("fsouza/fake-gcs-server", "latest")
        .with_exposed_port(4443.tcp())
        .with_wait_for(WaitFor::message_on_stderr("server started at"))
        .with_mapped_port(80, 4443.tcp())
        .with_cmd(["-scheme", "http", "-backend", "memory", "-port", "4443"])
        .start()
        .await
        .unwrap();

    let host = container.get_host().await.unwrap().to_string();
    let port = container.get_host_port_ipv4(4443).await.unwrap();
    let endpoint = format!("http://{host}:{port}");
    (container, endpoint)
}

/// fake-gcs-server ignores auth, so anonymous credentials exercise the real
/// streaming upload path without contacting Google.
async fn anon_client(endpoint: &str) -> Storage {
    let creds = google_cloud_auth::credentials::anonymous::Builder::new().build();
    Storage::builder()
        .with_credentials(creds)
        .with_endpoint(endpoint.to_string())
        .build()
        .await
        .unwrap()
}

/// Create the target bucket via the emulator's JSON API.
async fn create_bucket(endpoint: &str) {
    let url = format!("{endpoint}/storage/v1/b?project=test-project");
    let res = reqwest::Client::new()
        .post(&url)
        .json(&serde_json::json!({ "name": BUCKET }))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success(), "bucket create failed: {}", res.status());
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
    let source = StreamSource::from_stream(Box::pin(stream::iter(chunks)));

    let client = anon_client(&endpoint).await;
    // `write_object` (gRPC-style resource semantics, even when routed over the JSON
    // transport to the emulator) requires the bucket parameter in
    // `projects/_/buckets/<name>` form, not the bare bucket name. The JSON REST calls
    // below (bucket creation, media readback) use the bare name as the emulator's
    // plain JSON API expects.
    let bucket_resource = format!("projects/_/buckets/{BUCKET}");
    upload_with_client(&client, &bucket_resource, object, source).await.unwrap();

    // Read the object back via the emulator's media download endpoint and assert
    // it reassembles to the exact source bytes.
    let read_url =
        format!("{endpoint}/storage/v1/b/{BUCKET}/o/{}?alt=media", object.replace('/', "%2F"));
    let got = reqwest::get(&read_url).await.unwrap().bytes().await.unwrap();
    assert_eq!(got.as_ref(), data.as_slice());
}
