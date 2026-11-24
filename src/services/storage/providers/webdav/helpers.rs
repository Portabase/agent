use crate::services::storage::providers::rclone::helpers::{build_rclone_config, obscure_password};
use crate::services::storage::providers::webdav::models::{WebDavHeader, WebDavProviderConfig};
use anyhow::{Result, bail};

fn csv_field(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn serialize_headers(headers: &[WebDavHeader]) -> Result<String> {
    let mut values = Vec::with_capacity(headers.len() * 2);
    for header in headers {
        let key = header.key.trim();
        if !valid_header_name(key) {
            bail!("invalid WebDAV header name '{key}'");
        }
        if header.value.contains(['\r', '\n']) {
            bail!("WebDAV header values must not contain line breaks");
        }
        values.push(csv_field(key));
        values.push(csv_field(&header.value));
    }
    Ok(values.join(","))
}

pub fn build_webdav_config(config: &WebDavProviderConfig) -> Result<String> {
    let parsed = url::Url::parse(config.url.trim())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("WebDAV URL must use http or https");
    }

    let has_basic = config
        .username
        .as_deref()
        .is_some_and(|v| !v.trim().is_empty())
        || config.password.as_deref().is_some_and(|v| !v.is_empty());
    let has_bearer = config
        .bearer_token
        .as_deref()
        .is_some_and(|v| !v.trim().is_empty());
    if has_basic && has_bearer {
        bail!("WebDAV basic authentication and bearer token are mutually exclusive");
    }

    let pass = match config.password.as_deref().filter(|v| !v.is_empty()) {
        Some(password) => obscure_password(password)?,
        None => String::new(),
    };
    let headers = serialize_headers(&config.headers)?;
    build_rclone_config(
        "webdav",
        &[
            ("type", "webdav".into()),
            ("url", config.url.trim().into()),
            ("vendor", "other".into()),
            ("user", config.username.clone().unwrap_or_default()),
            ("pass", pass),
            (
                "bearer_token",
                config.bearer_token.clone().unwrap_or_default(),
            ),
            ("headers", headers),
        ],
    )
}
