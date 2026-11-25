use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use std::io::{Read, Write};

pub fn encode(raw: &[u8]) -> Result<String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(raw)?;
    Ok(STANDARD.encode(encoder.finish()?))
}

pub fn decode(encoded: &str) -> Result<Vec<u8>> {
    let compressed = STANDARD
        .decode(encoded)
        .context("manifest is not valid base64")?;
    let mut raw = Vec::new();
    GzDecoder::new(compressed.as_slice())
        .read_to_end(&mut raw)
        .context("manifest is not valid gzip")?;
    Ok(raw)
}

pub fn system_identifier(raw: &[u8]) -> Option<u64> {
    serde_json::from_slice::<serde_json::Value>(raw)
        .ok()?
        .get("System-Identifier")?
        .as_u64()
}
