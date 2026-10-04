use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose};
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Repository password: hex(HKDF-SHA256(master key, salt = "",
/// info = "portabase/restic/v1/<generated_id>", 32 bytes)). The dashboard derives
/// the same value (`src/lib/restic/repo.ts`), so it is never stored nor sent.
pub fn derive_password(master_key_b64: &str, generated_id: &str) -> Result<String> {
    let master_key = general_purpose::STANDARD
        .decode(master_key_b64.trim())
        .context("master key is not valid base64")?;
    let info = format!("portabase/restic/v1/{generated_id}");
    Ok(hex::encode(hkdf_sha256_32(&master_key, info.as_bytes())))
}

/// User of the dashboard's local-storage servers (`/storage/restic`, `/storage/sync`).
pub const LOCAL_STORAGE_USER: &str = "portabase";

/// Their password: hex(HKDF-SHA256(master key, salt = "", info = "portabase/local-storage/v1",
/// 32 bytes)). The dashboard derives the same value (`src/lib/local-storage/credential.ts`).
pub fn local_storage_password(master_key_b64: &str) -> Result<String> {
    let master_key = general_purpose::STANDARD
        .decode(master_key_b64.trim())
        .context("master key is not valid base64")?;
    Ok(hex::encode(hkdf_sha256_32(&master_key, b"portabase/local-storage/v1")))
}

/// RFC 5869 with an empty salt and a single expand block (L = HashLen = 32).
pub fn hkdf_sha256_32(ikm: &[u8], info: &[u8]) -> [u8; 32] {
    // An empty HMAC key is zero-padded: the RFC's default salt of HashLen zeros.
    let mut extract = HmacSha256::new_from_slice(&[]).expect("HMAC takes any key length");
    extract.update(ikm);
    let prk = extract.finalize().into_bytes();

    let mut expand = HmacSha256::new_from_slice(&prk).expect("HMAC takes any key length");
    expand.update(info);
    expand.update(&[1]);
    expand.finalize().into_bytes().into()
}
