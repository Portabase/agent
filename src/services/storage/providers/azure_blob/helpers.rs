use anyhow::{Context as _, Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{Duration, Utc};
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::sign::Signer;
use url::Url;

#[derive(Debug, Clone)]
pub struct ResolvedAzure {
    pub account_name: String,
    pub account_key: String,   // base64, as Azure presents it
    pub blob_endpoint: String, // e.g. http://127.0.0.1:10000/devstoreaccount1
}

#[derive(Clone, Copy)]
pub enum SasResource {
    Blob,
    // Container-scoped Service SAS is exercised by later tasks; kept here as part of the API.
    #[allow(dead_code)]
    Container,
}

impl SasResource {
    fn code(self) -> &'static str {
        match self { SasResource::Blob => "b", SasResource::Container => "c" }
    }
}

const SAS_VERSION: &str = "2022-11-02";

fn hmac_sha256_b64(key: &[u8], data: &str) -> Result<String> {
    let pkey = PKey::hmac(key).context("hmac key")?;
    let mut signer = Signer::new(MessageDigest::sha256(), &pkey).context("signer")?;
    signer.update(data.as_bytes()).context("signer update")?;
    let sig = signer.sign_to_vec().context("sign")?;
    Ok(STANDARD.encode(sig))
}

/// Build Service SAS query pairs (raw, un-encoded) for `canonical_resource`
/// e.g. `/blob/{account}/{container}/{blob}`.
pub fn build_service_sas(
    resolved: &ResolvedAzure,
    canonical_resource: &str,
    resource: SasResource,
    permissions: &str,
) -> Result<Vec<(String, String)>> {
    let key = STANDARD
        .decode(&resolved.account_key)
        .map_err(|_| anyhow!("account key is not valid base64"))?;

    let signed_start = String::new();
    let signed_expiry = (Utc::now() + Duration::hours(1))
        .format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let signed_protocol = "https,http"; // Azurite is http
    let signed_resource = resource.code();

    // Service SAS string-to-sign for sv >= 2020-12-06 (16 fields, 15 newlines).
    let string_to_sign = format!(
        "{sp}\n{st}\n{se}\n{canon}\n{si}\n{sip}\n{spr}\n{sv}\n{sr}\n{snap}\n{enc}\n{rscc}\n{rscd}\n{rsce}\n{rscl}\n{rsct}",
        sp = permissions, st = signed_start, se = signed_expiry, canon = canonical_resource,
        si = "", sip = "", spr = signed_protocol, sv = SAS_VERSION, sr = signed_resource,
        snap = "", enc = "", rscc = "", rscd = "", rsce = "", rscl = "", rsct = "",
    );

    let sig = hmac_sha256_b64(&key, &string_to_sign)?;

    Ok(vec![
        ("sv".into(), SAS_VERSION.into()),
        ("sr".into(), signed_resource.into()),
        ("sp".into(), permissions.into()),
        ("se".into(), signed_expiry),
        ("spr".into(), signed_protocol.into()),
        ("sig".into(), sig),
    ])
}

/// Build a SAS-scoped URL for a blob (or container when `blob` is empty).
pub fn build_sas_url(
    resolved: &ResolvedAzure,
    container: &str,
    blob: &str,
    resource: SasResource,
    permissions: &str,
) -> Result<Url> {
    let canonical = if blob.is_empty() {
        format!("/blob/{}/{}", resolved.account_name, container)
    } else {
        format!("/blob/{}/{}/{}", resolved.account_name, container, blob)
    };
    let pairs = build_service_sas(resolved, &canonical, resource, permissions)?;

    let base = if blob.is_empty() {
        format!("{}/{}", resolved.blob_endpoint.trim_end_matches('/'), container)
    } else {
        format!("{}/{}/{}", resolved.blob_endpoint.trim_end_matches('/'), container, blob)
    };

    let mut url = Url::parse(&base).context("invalid blob endpoint/url")?;
    {
        let mut qp = url.query_pairs_mut();
        for (k, v) in pairs { qp.append_pair(&k, &v); }
    }
    Ok(url)
}

/// Build an Account SAS query set (raw, un-encoded).
///
/// Required because creating a *container* against Azurite cannot be authorized by a
/// container-scoped Service SAS (Azurite maps `Container_Create` to an empty required
/// permission and its check then always fails); an Account SAS is the supported path.
///
/// `services` e.g. `"b"` (blob), `resource_types` e.g. `"c"` (container) / `"co"` (container+object),
/// `permissions` e.g. `"cw"` (create+write).
pub fn build_account_sas(
    resolved: &ResolvedAzure,
    services: &str,
    resource_types: &str,
    permissions: &str,
) -> Result<Vec<(String, String)>> {
    let key = STANDARD
        .decode(&resolved.account_key)
        .map_err(|_| anyhow!("account key is not valid base64"))?;

    let signed_start = String::new();
    let signed_expiry = (Utc::now() + Duration::hours(1))
        .format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let signed_protocol = "https,http"; // Azurite is http
    let signed_ip = String::new();
    let encryption_scope = String::new();

    // Account SAS string-to-sign for sv >= 2020-12-06:
    // account \n sp \n ss \n srt \n st \n se \n sip \n spr \n sv \n ses \n  (trailing newline)
    let string_to_sign = format!(
        "{acc}\n{sp}\n{ss}\n{srt}\n{st}\n{se}\n{sip}\n{spr}\n{sv}\n{ses}\n",
        acc = resolved.account_name, sp = permissions, ss = services, srt = resource_types,
        st = signed_start, se = signed_expiry, sip = signed_ip, spr = signed_protocol,
        sv = SAS_VERSION, ses = encryption_scope,
    );

    let sig = hmac_sha256_b64(&key, &string_to_sign)?;

    Ok(vec![
        ("sv".into(), SAS_VERSION.into()),
        ("ss".into(), services.into()),
        ("srt".into(), resource_types.into()),
        ("sp".into(), permissions.into()),
        ("se".into(), signed_expiry),
        ("spr".into(), signed_protocol.into()),
        ("sig".into(), sig),
    ])
}

/// Build an Account-SAS-scoped URL for a container (used for container creation).
pub fn build_account_sas_container_url(
    resolved: &ResolvedAzure,
    container: &str,
    services: &str,
    resource_types: &str,
    permissions: &str,
) -> Result<Url> {
    let pairs = build_account_sas(resolved, services, resource_types, permissions)?;
    let base = format!("{}/{}", resolved.blob_endpoint.trim_end_matches('/'), container);
    let mut url = Url::parse(&base).context("invalid blob endpoint/url")?;
    {
        let mut qp = url.query_pairs_mut();
        for (k, v) in pairs { qp.append_pair(&k, &v); }
    }
    Ok(url)
}
