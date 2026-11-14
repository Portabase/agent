use crate::services::storage::providers::google_drive::helpers::{
    DRIVE_RESUMABLE_UPLOAD_URL, DriveItem, ensure_folder_path_at, escape_query_value,
    exchange_refresh_token, list_files, parse_files_list, pick_oldest, upload_chunks,
};
use crate::services::storage::providers::google_drive::models::GoogleDriveProviderConfig;
use crate::tests::services::local_storage_tests::MASTER_KEY_B64;
use crate::utils::file::decrypt_file_stream_gcm;
use crate::utils::stream::build_stream;

use bytes::Bytes;
use futures::stream;
use reqwest::Client;
use tempfile::TempDir;
use wiremock::matchers::{body_string_contains, method, path, query_param_contains};
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
}

fn drive_config() -> GoogleDriveProviderConfig {
    GoogleDriveProviderConfig {
        client_id: "cid".to_string(),
        client_secret: "secret".to_string(),
        refresh_token: "rt-1".to_string(),
        folder_id: "root".to_string(),
    }
}

#[tokio::test]
async fn refresh_token_is_exchanged_for_an_access_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=rt-1"))
        .and(body_string_contains("client_id=cid"))
        .and(body_string_contains("client_secret=secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "at-1",
            "expires_in": 3599,
            "token_type": "Bearer"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let token = exchange_refresh_token(
        &Client::new(),
        &format!("{}/token", server.uri()),
        &drive_config(),
    )
    .await
    .unwrap();

    assert_eq!(token, "at-1");
}

#[tokio::test]
async fn rejected_refresh_token_surfaces_the_oauth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "invalid_grant",
            "error_description": "Token has been expired or revoked."
        })))
        .mount(&server)
        .await;

    let error = exchange_refresh_token(
        &Client::new(),
        &format!("{}/token", server.uri()),
        &drive_config(),
    )
    .await
    .unwrap_err();

    assert!(format!("{error:#}").contains("invalid_grant"), "{error:#}");
}

fn item(id: &str, created_time: &str) -> DriveItem {
    DriveItem {
        id: id.to_string(),
        created_time: created_time.to_string(),
    }
}

#[test]
fn oldest_of_same_named_folders_wins() {
    let items = [
        item("new", "2026-11-08T03:00:22.325Z"),
        item("old", "2026-11-06T12:33:28.307Z"),
    ];
    assert_eq!(pick_oldest(&items).unwrap().id, "old");
    assert!(pick_oldest(&[]).is_none());
}

#[test]
fn oldest_tie_breaks_on_id() {
    let items = [
        item("z", "2026-11-06T12:33:28.307Z"),
        item("a", "2026-11-06T12:33:28.307Z"),
    ];
    assert_eq!(pick_oldest(&items).unwrap().id, "a");
}

#[test]
fn failed_list_is_an_error_not_an_empty_folder() {
    let body = r#"{"error":{"code":403,"errors":[{"reason":"rateLimitExceeded"}]}}"#;
    let error = parse_files_list(403, body).unwrap_err().to_string();
    assert!(error.contains("HTTP 403"), "{error}");
}

#[test]
fn list_without_files_array_is_an_error() {
    let error = parse_files_list(200, r#"{"kind":"drive#fileList"}"#)
        .unwrap_err()
        .to_string();
    assert!(error.contains("no files array"), "{error}");
}

#[test]
fn list_reads_ids_and_created_times() {
    let body = r#"{"files":[{"id":"a","createdTime":"2026-11-06T12:33:28.307Z"},{"id":"b"}]}"#;
    assert_eq!(
        parse_files_list(200, body).unwrap(),
        vec![item("a", "2026-11-06T12:33:28.307Z"), item("b", "")]
    );
}

#[test]
fn query_values_are_escaped() {
    assert_eq!(escape_query_value("o'neil\\db"), "o\\'neil\\\\db");
}

#[tokio::test]
async fn list_error_surfaces_from_list_files() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/files"))
        .respond_with(ResponseTemplate::new(500).set_body_string("backendError"))
        .mount(&server)
        .await;

    let error = list_files(&Client::new(), &server.uri(), "t", "q")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("HTTP 500"), "{error}");
}

#[tokio::test]
async fn failed_list_never_creates_a_folder() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/files"))
        .respond_with(ResponseTemplate::new(401).set_body_string("invalid credentials"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"id":"dup"}"#))
        .expect(0)
        .mount(&server)
        .await;

    assert!(
        ensure_folder_path_at(&Client::new(), &server.uri(), "t", "root", &["backups"])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn existing_folders_resolve_to_the_oldest() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/files"))
        .and(query_param_contains("q", "name='backups'"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"files":[{"id":"new","createdTime":"2026-11-08T03:00:00Z"},{"id":"old","createdTime":"2026-11-06T12:00:00Z"}]}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let id = ensure_folder_path_at(&Client::new(), &server.uri(), "t", "root", &["backups"])
        .await
        .unwrap();
    assert_eq!(id, "old");
}

#[tokio::test]
async fn concurrent_create_converges_on_the_oldest_folder() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/files"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"files":[]}"#))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/files"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"files":[{"id":"theirs","createdTime":"2026-11-08T03:00:00.100Z"},{"id":"mine","createdTime":"2026-11-08T03:00:00.200Z"}]}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/files"))
        .and(query_param_contains("supportsAllDrives", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"id":"mine"}"#))
        .expect(1)
        .mount(&server)
        .await;

    let id = ensure_folder_path_at(&Client::new(), &server.uri(), "t", "root", &["backups"])
        .await
        .unwrap();
    assert_eq!(id, "theirs");
}
