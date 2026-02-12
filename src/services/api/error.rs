use reqwest::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum ApiError {
    #[error("http client error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("api error: status={status}, body={body}")]
    HttpResponse {
        status: StatusCode,
        body: String,
    },

    #[error("api returned unexpected response")]
    UnexpectedResponse,
}
