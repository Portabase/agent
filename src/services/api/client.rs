#![allow(dead_code)]

use crate::services::api::ApiError;
use base64::{Engine as _, engine::general_purpose};
use hmac::{Hmac, KeyInit, Mac};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Client, Method, RequestBuilder};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::error;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug)]
pub struct ApiClient {
    base_url: String,
    http: Client,
    agent_key: Option<Vec<u8>>,
}

pub fn derive_agent_key(master_key: &[u8], agent_id: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(master_key).expect("HMAC accepts any key length");
    mac.update(format!("portabase-agent-auth:v1:{agent_id}").as_bytes());
    mac.finalize().into_bytes().to_vec()
}

pub fn sign_request(agent_key: &[u8], method: &str, path: &str, timestamp: &str, body: &[u8]) -> String {
    let body_hash = hex::encode(Sha256::digest(body));
    let mut mac = HmacSha256::new_from_slice(agent_key).expect("HMAC accepts any key length");
    mac.update(format!("{method}\n{path}\n{timestamp}\n{body_hash}").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

impl ApiClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build http client");

        Self {
            base_url: base_url.into(),
            http,
            agent_key: None,
        }
    }

    pub fn with_agent_auth(mut self, agent_id: &str, master_key_b64: &str) -> Self {
        let master_key = match general_purpose::STANDARD.decode(master_key_b64) {
            Ok(key) => key,
            Err(_) => {
                error!("Invalid EDGE_KEY: master key is not valid base64");
                panic!("Cannot initialize AgentContext due to invalid EDGE_KEY");
            }
        };
        self.agent_key = Some(derive_agent_key(&master_key, agent_id));
        self
    }

    fn sign(&self, request: RequestBuilder, method: &Method, path: &str, body: &[u8]) -> RequestBuilder {
        let Some(agent_key) = &self.agent_key else {
            return request;
        };
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before 1970")
            .as_secs()
            .to_string();
        let signature = sign_request(agent_key, method.as_str(), &format!("/api{path}"), &timestamp, body);
        request
            .header("X-Agent-Timestamp", timestamp)
            .header("X-Agent-Signature", signature)
    }

    pub async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
    ) -> Result<Option<T>, ApiError> {
        let url = format!("{}{}", self.base_url, path);

        let request = self.sign(self.http.request(method.clone(), &url), &method, path, b"");
        let res = request.send().await?;
        let status = res.status();
        let body = res.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(ApiError::HttpResponse { status, body });
        }

        if body.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(serde_json::from_str::<T>(&body)?))
        }
    }
    pub async fn request_with_body<T, B>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> Result<Option<T>, ApiError>
    where
        T: DeserializeOwned,
        B: serde::Serialize,
    {
        let url = format!("{}{}", self.base_url, path);

        // Serialize once: the signature covers exactly the bytes that are sent.
        let bytes = serde_json::to_vec(body)?;
        let request = self
            .http
            .request(method.clone(), &url)
            .header(CONTENT_TYPE, "application/json");
        let res = self.sign(request, &method, path, &bytes).body(bytes).send().await?;

        let status = res.status();
        let body_text = res.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(ApiError::HttpResponse {
                status,
                body: body_text,
            });
        }

        if body_text.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(serde_json::from_str::<T>(&body_text)?))
        }
    }
}
