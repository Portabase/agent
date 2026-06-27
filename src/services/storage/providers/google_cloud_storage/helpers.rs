use crate::services::storage::providers::google_cloud_storage::models::GoogleCloudStorageProviderConfig;
use anyhow::{Context, Result};
use bytes::Bytes;
use futures::Stream;
use futures::StreamExt;
use google_cloud_auth::credentials::Credentials;
use google_cloud_storage::client::Storage;
use google_cloud_storage::streaming_source::StreamingSource;
use std::pin::Pin;

/// Build service-account credentials from the inline key in config.
/// The SDK manages token exchange internally from this key.
pub fn build_credentials(cfg: &GoogleCloudStorageProviderConfig) -> Result<Credentials> {
    let key = serde_json::json!({
        "type": "service_account",
        "project_id": cfg.project_id,
        "client_email": cfg.client_email,
        "private_key": cfg.private_key,
        "private_key_id": "",
        "token_uri": "https://oauth2.googleapis.com/token",
        "universe_domain": "googleapis.com",
    });

    google_cloud_auth::credentials::service_account::Builder::new(key)
        .build()
        .context("failed to build GCS service account credentials")
}

/// Build the GCS Storage client: service-account creds + optional endpoint override.
pub async fn build_client(cfg: &GoogleCloudStorageProviderConfig) -> Result<Storage> {
    let creds = build_credentials(cfg)?;
    let mut builder = Storage::builder().with_credentials(creds);

    if let Some(ep) = cfg.api_endpoint.as_deref().filter(|s| !s.trim().is_empty()) {
        builder = builder.with_endpoint(ep.to_string());
    }

    builder.build().await.context("failed to build GCS client")
}

/// Bridges our `build_stream` byte-stream into the SDK's single-pass `StreamingSource`.
///
/// `send_buffered()` requires the source to be `Send + Sync + 'static`, but
/// `build_stream` yields a `Send`-only stream (and tightening that shared type to
/// `Sync` would affect every provider). So we pump the source through a bounded
/// channel: a `Receiver<Result<Bytes, _>>` is `Send + Sync` regardless of the
/// producing stream's `Sync`-ness, and the bound keeps memory in check via
/// backpressure (the pump task blocks when the channel is full).
pub struct StreamSource {
    rx: tokio::sync::mpsc::Receiver<Result<Bytes, std::io::Error>>,
}

impl StreamSource {
    /// Spawn a task that drains `stream` into a bounded channel and return a
    /// `StreamSource` reading from it. The stream only needs to be `Send + 'static`.
    pub fn from_stream(
        mut stream: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            while let Some(item) = stream.next().await {
                // Receiver dropped (e.g. upload aborted) -> stop pumping.
                if tx.send(item).await.is_err() {
                    break;
                }
            }
        });
        StreamSource { rx }
    }
}

impl StreamingSource for StreamSource {
    type Error = std::io::Error;

    async fn next(&mut self) -> Option<Result<Bytes, Self::Error>> {
        self.rx.recv().await
    }
}

/// Upload a single object using a pre-built client (split out so tests can inject
/// an anonymous-credential client pointed at the emulator).
pub async fn upload_with_client(
    client: &Storage,
    bucket: &str,
    object: &str,
    source: StreamSource,
) -> Result<()> {
    // `write_object` uses gRPC-style resource names: the bucket must be passed as
    // `projects/_/buckets/<name>`, not the bare bucket id (the SDK rejects the bare
    // form with "malformed bucket name"). Callers pass the bare name; format it
    // here so every call site — production and tests — goes through one correct path.
    let bucket_resource = format!("projects/_/buckets/{bucket}");
    client
        .write_object(bucket_resource, object, source)
        .send_buffered()
        .await
        .context("GCS write_object failed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway PKCS8 RSA private key generated solely for this test.
    const TEST_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDgqiznXi/jJr/Q\nlpWN5P0LyHrm/WBHyYeStKjEHefqlzbfvyevNG+uBV93rcG3rBcLreR9pfEiSeZh\npWuweIv8P83GaITat+GrKTrLRrlfeNdNCQfdx8NqVjftnBmKgm+Gfto1ouh/98ZZ\n0TSVzMyif7e0jhMBhh8DeXPETz7S3iTPkzAGTL1uRpUscueHjT0hODci4ONyuxkE\n70rnuhfktjXMXFhqHdxfV0Us39V/9aoXb4cRbPCqmW5mZ7SWMkxzMgEh5LZLdBOD\nyu9aXDGsTOImk2ZzmMHFWLfF+1yrrKrWcUZrw2vXtAnFc+j6ssjVeH2eiFpVm0oh\nAap7So2nAgMBAAECggEALDxVqxi4hRlUG1YLDG1SBcfrqx+onXno39ICiNr6lw4/\nF78jqTPB6ZnVOlNUGT4hK4OJwdOyrvWuDvvrQEv8BCbr9W0O+6HJJVJw6SV7yniY\nq+pjSh/TMlTXnklmHgegvfKsNHNnJAs9WuH+YKB6imRrX3m59ErcQGrhiH2x+QK1\n+jzru4Ac6qntCTwa4SifMmc7D6JgF7DAZL2U4O0yGe/2fpNuTL7F47rleEF327Hy\nUVyT20hzpVAh0PBI11Hi+vgaChx5MNuPjPEM5pWNzsMQz8llh66uk3CKT3/h93hk\nLPVyLFRBDN51TCJe0ngk84RyBAHywiwvfVUwUrI5gQKBgQDylA3agH7kBK1G0czs\ngJTa89FyFEzU3rls6Qzw3WlOE6P0EDCDCd3/JgFunwnAT1TuXJJC38cDRASlj4gU\nYNx5asLc5ifPB0YYlzafZLqryOH7IBmI0kIMdq9L0+BCam8JiobpZ3ICLvRl8fe5\n0Hg9rEX5tbuE86j1h4mdIFK7yQKBgQDtGGNpCSPcsLlfHV0NBAtDU9cv5EDl6EPa\nV+q7M2Fkk5wCR6O4zfn6scftWIfWFFdy0PJuGKUnsRmkeTFTjxx79pjxFa1IxHts\nO8OSTscM3YcpT0fENxPs+PnGF60+7geYnrrtFzxbMbm5hJzR5+iAdGX8hTXEAFxQ\n60VPstfV7wKBgQCkRJZNDQ7gojok5xX6YehrjQicVBrjXB/9HKRix8zzzmEMeZog\nYqIukjIOEyyrSg2djJqPJrLCB2GOK/BevGkQ37ctl74FeEuDg4K91ZyDj/lX8ZjZ\nCmknv4ddthD7aM/giipqDF8sE1f1YTH8ZqvGN877FpHxqn8UJcCO4sCj4QKBgQDd\n4QvvGOmhtxTTKTSSYK11pXlkzTPatAEDzXDTHaNQLz85dveFk+UTsdoKiOYd9s1b\nmqS1WYT9XyRDIlOCAhTDAaRhQUr4JT/nqwo72lM2+/1oMFRWEMEp7Fo7Ap9TnAgp\n0KnYBP2rzh4jujHT0jZoOAXVSohlU30REQu9KP4JqwKBgCCbN2rDmjmuJV8N7nB7\nX7wJrO6kr0jAnINsh4Jo+ej87HzqBz0RJ12AsaZozYWwXjocn4WebAGVmIuX+Hop\n1fBwoa/cjv0HjgYqdHftpk/wjagmNJgafXRIYFCSq0IXdcLbCWv96pnPEs57U6Kf\njUsAlB9U5GSz55FWCVshit/e\n-----END PRIVATE KEY-----\n";

    fn test_cfg() -> GoogleCloudStorageProviderConfig {
        GoogleCloudStorageProviderConfig {
            project_id: "test-proj".into(),
            bucket_name: "test-bucket".into(),
            client_email: "svc@test-proj.iam.gserviceaccount.com".into(),
            private_key: TEST_PRIVATE_KEY.into(),
            api_endpoint: None,
        }
    }

    #[tokio::test]
    async fn build_credentials_accepts_valid_service_account_key() {
        let cfg = test_cfg();
        let creds = build_credentials(&cfg);
        assert!(creds.is_ok(), "expected creds to build, got {:?}", creds.err());
    }
}
