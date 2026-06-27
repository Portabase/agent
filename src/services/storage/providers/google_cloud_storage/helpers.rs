use crate::services::storage::providers::google_cloud_storage::models::GoogleCloudStorageProviderConfig;
use anyhow::{Context, Result};
use bytes::Bytes;
use futures::Stream;
use futures::StreamExt;
use google_cloud_auth::credentials::Credentials;
use google_cloud_storage::client::Storage;
use google_cloud_storage::streaming_source::{SizeHint, StreamingSource};
use std::pin::Pin;

pub fn build_credentials(cfg: &GoogleCloudStorageProviderConfig) -> Result<Credentials> {
    // Service-account JSON stores the PEM with `\n` escape sequences. When the key is
    // carried through config as a JSON string those can arrive as literal two-char `\n`
    // sequences rather than real newlines, so the PEM parser finds no `-----BEGIN-----`
    // line ("no items found"). Normalize them back to real newlines. A PEM that already
    // has real newlines contains no literal `\n` pairs, so this is a no-op for it.
    let private_key = cfg.private_key.replace("\\n", "\n");

    let key = serde_json::json!({
        "type": "service_account",
        "project_id": cfg.project_id,
        "client_email": cfg.client_email,
        "private_key": private_key,
        "private_key_id": "",
        "token_uri": "https://oauth2.googleapis.com/token",
        "universe_domain": "googleapis.com",
    });

    google_cloud_auth::credentials::service_account::Builder::new(key)
        .build()
        .context("failed to build GCS service account credentials")
}

pub async fn build_client(cfg: &GoogleCloudStorageProviderConfig) -> Result<Storage> {
    let endpoint = cfg.api_endpoint.as_deref().filter(|s| !s.trim().is_empty());

    // A custom endpoint means a local emulator (fake-gcs-server), which does not verify
    // credentials. Use anonymous creds so a dummy/empty `private_key` in the emulator
    // config doesn't trip the service-account PEM parser. Real GCS still uses the
    // service-account key built from config.
    let builder = if let Some(ep) = endpoint {
        let creds = google_cloud_auth::credentials::anonymous::Builder::new().build();
        Storage::builder()
            .with_credentials(creds)
            .with_endpoint(ep.to_string())
    } else {
        Storage::builder().with_credentials(build_credentials(cfg)?)
    };

    builder.build().await.context("failed to build GCS client")
}

/// Bridges `build_stream`'s `Send`-only byte stream into the SDK's `StreamingSource`
/// (which `send_buffered` requires to be `Send + Sync + 'static`) via a bounded mpsc
/// channel. Also reports an exact `size_hint`: the SDK picks single-shot vs resumable
/// upload from `size_hint().upper()` — an unknown bound forces resumable unconditionally
/// (see `upload_with_client`).
pub struct StreamSource {
    rx: tokio::sync::mpsc::Receiver<Result<Bytes, std::io::Error>>,
    total_size: u64,
}

impl StreamSource {
    pub fn from_stream(
        mut stream: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
        total_size: u64,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            while let Some(item) = stream.next().await {
                if tx.send(item).await.is_err() {
                    break;
                }
            }
        });
        StreamSource { rx, total_size }
    }
}

impl StreamingSource for StreamSource {
    type Error = std::io::Error;
    async fn next(&mut self) -> Option<Result<Bytes, Self::Error>> {
        self.rx.recv().await
    }

    // Report the exact size so the SDK can choose single-shot uploads. The default
    // impl returns an unknown bound, which forces the resumable path unconditionally.
    async fn size_hint(&self) -> Result<SizeHint, Self::Error> {
        Ok(SizeHint::with_exact(self.total_size))
    }
}

pub async fn upload_with_client(
    client: &Storage,
    bucket: &str,
    object: &str,
    source: StreamSource,
    force_single_shot: bool,
) -> Result<()> {
    // `write_object` uses gRPC-style resource names: the bucket must be passed as
    // `projects/_/buckets/<name>`, not the bare bucket id.
    let bucket_resource = format!("projects/_/buckets/{bucket}");

    let mut builder = client.write_object(bucket_resource, object, source);

    // Resumable uploads follow a server-generated `Location` URL. When pointed at a
    // custom `apiEndpoint` on a non-443 port, the SDK's transport drops the port from
    // the `Host` header (google-cloud-gax-internal `host.rs`), so emulators that build
    // the `Location` from `Host` hand back a portless URL the SDK then hangs on. A
    // single-shot upload issues one request to the configured endpoint (no `Location`
    // to follow), sidestepping the bug. We force it only for custom endpoints; against
    // real GCS we keep resumable (bounded memory + resume on large backups).
    if force_single_shot {
        builder = builder.with_resumable_upload_threshold(usize::MAX);
    }

    builder
        .send_buffered()
        .await
        .context("GCS write_object failed")?;
    Ok(())
}
