use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleCloudStorageProviderConfig {
    pub project_id: String,
    pub bucket_name: String,
    pub client_email: String,
    pub private_key: String,
    #[serde(default)]
    pub api_endpoint: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_camel_case_with_optional_endpoint() {
        let json = serde_json::json!({
            "projectId": "my-proj",
            "bucketName": "my-bucket",
            "clientEmail": "svc@my-proj.iam.gserviceaccount.com",
            "privateKey": "-----BEGIN PRIVATE KEY-----\nMII...\n-----END PRIVATE KEY-----\n"
        });
        let cfg: GoogleCloudStorageProviderConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.project_id, "my-proj");
        assert_eq!(cfg.bucket_name, "my-bucket");
        assert_eq!(cfg.client_email, "svc@my-proj.iam.gserviceaccount.com");
        assert!(cfg.private_key.contains("BEGIN PRIVATE KEY"));
        assert!(cfg.api_endpoint.is_none());
    }

    #[test]
    fn deserializes_with_endpoint() {
        let json = serde_json::json!({
            "projectId": "p", "bucketName": "b",
            "clientEmail": "e", "privateKey": "k",
            "apiEndpoint": "http://localhost:4443"
        });
        let cfg: GoogleCloudStorageProviderConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.api_endpoint.as_deref(), Some("http://localhost:4443"));
    }
}
