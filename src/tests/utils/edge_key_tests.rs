use base64::{engine::general_purpose, Engine as _};
use serde_json::json;
use crate::utils::edge_key::{decode_edge_key, EdgeKeyError};

#[test]
fn decode_valid_edge_key() {
    let edge_key_b64 = "eyJzZXJ2ZXJVcmwiOiJodHRwOi8vbG9jYWxob3N0Ojg4ODciLCJhZ2VudElkIjoiNjI1MDQzY2YtN2MwMC00M2M4LWJjYzktZDM1MTk5ODk2ZGNkIiwibWFzdGVyS2V5QjY0IjoiQlhWM1hvbEM2NTZTVjdkTmdjV1BHUWxrKytycExJNmxHRGk3Q1BCNWllbz0ifQ==";
    let decoded = decode_edge_key(edge_key_b64).unwrap();

    assert_eq!(decoded.server_url, "http://localhost:8887");
    assert_eq!(decoded.agent_id, "625043cf-7c00-43c8-bcc9-d35199896dcd");
    assert_eq!(decoded.master_key_b64, "BXV3XolC656SV7dNgcWPGQlk++rpLI6lGDi7CPB5ieo=");
}

#[test]
fn decode_edge_key_missing_field() {
    let incomplete_json = json!({
        "serverUrl": "http://localhost:8887",
        "agentId": "123"
        // masterKeyB64 is missing
    })
        .to_string();

    let b64 = general_purpose::URL_SAFE.encode(incomplete_json);
    let result = decode_edge_key(&b64);

    match result {
        Err(EdgeKeyError::InvalidKey) => {}
        _ => panic!("Expected InvalidKey error"),
    }
}

#[test]
fn decode_edge_key_invalid_base64() {
    let invalid_b64 = "!!!notbase64!!!";
    let result = decode_edge_key(invalid_b64);

    match result {
        Err(EdgeKeyError::Base64Error(_)) => {}
        _ => panic!("Expected Base64Error"),
    }
}

#[test]
fn decode_edge_key_invalid_json() {
    let invalid_json_b64 = general_purpose::URL_SAFE.encode("not a json string");
    let result = decode_edge_key(&invalid_json_b64);

    match result {
        Err(EdgeKeyError::JsonError(_)) => {}
        _ => panic!("Expected JsonError"),
    }
}
