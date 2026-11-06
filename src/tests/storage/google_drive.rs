use crate::services::storage::providers::google_drive::helpers::{
    DRIVE_FILES_URL, DRIVE_RESUMABLE_UPLOAD_URL, upload_chunks,
};
use crate::tests::services::local_storage_tests::MASTER_KEY_B64;
use crate::utils::file::decrypt_file_stream_gcm;
use crate::utils::stream::build_stream;

use bytes::Bytes;
use futures::stream;
use reqwest::Client;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const WINDOW: usize = 8 * 1024 * 1024;

async fn drive_session() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/upload"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

async fn received_puts(server: &MockServer) -> Vec<(String, Vec<u8>)> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .map(|request| {
            let content_range = request.headers["content-range"]
                .to_str()
                .unwrap()
                .to_string();
            (content_range, request.body)
        })
        .collect()
}

#[tokio::test]
async fn sixteen_mib_chunk_is_sent_as_two_eight_mib_windows() {
    let server = drive_session().await;
    let payload: Vec<u8> = (0..2 * WINDOW).map(|index| (index % 251) as u8).collect();
    let content_stream = stream::iter(vec![Ok(Bytes::from(payload.clone()))]);

    upload_chunks(
        &Client::new(),
        &format!("{}/upload", server.uri()),
        content_stream,
        payload.len() as u64,
    )
    .await
    .unwrap();

    let puts = received_puts(&server).await;
    assert_eq!(puts.len(), 2);
    assert_eq!(puts[0].0, "bytes 0-8388607/*");
    assert_eq!(puts[1].0, "bytes 8388608-16777215/16777216");
    assert_eq!(puts[0].1.len(), WINDOW);
    assert_eq!(puts[1].1.len(), WINDOW);
    assert_eq!(
        [puts[0].1.as_slice(), puts[1].1.as_slice()].concat(),
        payload
    );
}

#[tokio::test]
async fn encrypted_dump_uploaded_to_drive_decrypts_back() {
    let server = drive_session().await;
    let work = TempDir::new().unwrap();
    let dump_path = work.path().join("dump.sql");
    let plaintext: Vec<u8> = (0..WINDOW + 1024 * 1024)
        .map(|index| (index % 253) as u8)
        .collect();
    std::fs::write(&dump_path, &plaintext).unwrap();
    let upload = build_stream(&dump_path, true, &MASTER_KEY_B64.to_string())
        .await
        .unwrap();

    upload_chunks(
        &Client::new(),
        &format!("{}/upload", server.uri()),
        upload.stream,
        plaintext.len() as u64,
    )
    .await
    .unwrap();

    let puts = received_puts(&server).await;
    let uploaded: Vec<u8> = puts.iter().flat_map(|(_, body)| body.clone()).collect();
    let (last_range, _) = puts.last().unwrap();
    assert!(last_range.ends_with(&format!("/{}", uploaded.len())));
    let encrypted_path = work.path().join("dump.sql.enc");
    let decrypted_path = work.path().join("dump.restored.sql");
    std::fs::write(&encrypted_path, &uploaded).unwrap();
    decrypt_file_stream_gcm(
        encrypted_path,
        decrypted_path.clone(),
        MASTER_KEY_B64.to_string(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(decrypted_path).unwrap(), plaintext);
}

#[tokio::test]
async fn stream_shorter_than_the_dump_never_finalizes_the_upload() {
    let server = drive_session().await;
    let content_stream = stream::iter(vec![Ok(Bytes::from(vec![1u8; 1024]))]);

    let error = upload_chunks(
        &Client::new(),
        &format!("{}/upload", server.uri()),
        content_stream,
        2048,
    )
    .await
    .unwrap_err();

    assert_eq!(error.to_string(), "Stream ended early");
    assert!(received_puts(&server).await.is_empty());
}

#[tokio::test]
async fn empty_dump_finalizes_a_zero_byte_file() {
    let server = drive_session().await;

    upload_chunks(
        &Client::new(),
        &format!("{}/upload", server.uri()),
        stream::iter(Vec::<Result<Bytes, std::io::Error>>::new()),
        0,
    )
    .await
    .unwrap();

    let puts = received_puts(&server).await;
    assert_eq!(puts.len(), 1);
    assert_eq!(puts[0].0, "bytes */0");
    assert!(puts[0].1.is_empty());
}

#[test]
fn drive_write_urls_carry_supports_all_drives() {
    assert!(DRIVE_RESUMABLE_UPLOAD_URL.contains("supportsAllDrives=true"));
    assert!(DRIVE_RESUMABLE_UPLOAD_URL.contains("uploadType=resumable"));
    assert!(DRIVE_FILES_URL.contains("supportsAllDrives=true"));
}
