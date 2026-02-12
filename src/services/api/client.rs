#![allow(dead_code)]

use reqwest::{Client, Method};
use serde::de::DeserializeOwned;
use std::time::Duration;
use crate::services::api::ApiError;

#[derive(Clone, Debug)]
pub struct ApiClient {
    base_url: String,
    http: Client,
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
        }
    }

    pub async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
    ) -> Result<Option<T>, ApiError> {
        let url = format!("{}{}", self.base_url, path);

        let res = self.http.request(method, &url).send().await?;
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

        let res = self
            .http
            .request(method, &url)
            .json(body)
            .send()
            .await?;

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
