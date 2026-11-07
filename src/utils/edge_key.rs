use anyhow::Context;
use base64::{Engine as _, engine::general_purpose};
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use serde_json::Value;
use sha2::Sha256;
use thiserror::Error;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct EdgeKey {
    #[serde(rename = "serverUrl")]
    pub server_url: String,
    #[serde(rename = "agentId")]
    pub agent_id: String,
    #[serde(rename = "masterKeyB64")]
    pub master_key_b64: String,
}

#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum EdgeKeyError {
    #[error("Base64 decoding error: {0}")]
    Base64Error(#[from] base64::DecodeError),
    #[error("JSON parsing error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("EDGE_KEY INVALID")]
    InvalidKey,
}

pub fn decode_edge_key(edge_key: &str) -> Result<EdgeKey, EdgeKeyError> {
    let mut key = edge_key.to_string();
    let padding_needed = (4 - key.len() % 4) % 4;
    key.push_str(&"=".repeat(padding_needed));

    let decoded_bytes = general_purpose::URL_SAFE.decode(&key)?;
    let decoded_str = String::from_utf8_lossy(&decoded_bytes);

    let parsed: Value = serde_json::from_str(&decoded_str)?;

    if parsed.get("serverUrl").is_some()
        && parsed.get("agentId").is_some()
        && parsed.get("masterKeyB64").is_some()
    {
        let edge_key: EdgeKey = serde_json::from_value(parsed)?;
        Ok(edge_key)
    } else {
        Err(EdgeKeyError::InvalidKey)
    }
}

pub fn derive_key(master_key_b64: &str, info: &[u8]) -> anyhow::Result<[u8; 32]> {
    let master_key = general_purpose::STANDARD
        .decode(master_key_b64.trim())
        .context("master key is not valid base64")?;
    Ok(hkdf_sha256_32(&master_key, info))
}

pub fn hkdf_sha256_32(ikm: &[u8], info: &[u8]) -> [u8; 32] {
    let mut extract = Hmac::<Sha256>::new_from_slice(&[]).expect("HMAC takes any key length");
    extract.update(ikm);
    let prk = extract.finalize().into_bytes();

    let mut expand = Hmac::<Sha256>::new_from_slice(&prk).expect("HMAC takes any key length");
    expand.update(info);
    expand.update(&[1]);
    expand.finalize().into_bytes().into()
}
