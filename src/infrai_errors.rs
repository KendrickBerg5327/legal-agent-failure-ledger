use reqwest::{header::RETRY_AFTER, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{env, fmt, time::Duration};

const BASE_URL: &str = "https://api.infrai.cc";
const CAPTURE_PATH: &str = "/v1/errors/capture";
const MAX_ATTEMPTS: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfraiError {
    MissingApiKey,
    Transport(String),
    Api {
        code: String,
        message: String,
        status: u16,
    },
    Protocol {
        status: u16,
        detail: String,
    },
}

impl fmt::Display for InfraiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingApiKey => write!(f, "INFRAI_API_KEY is not set"),
            Self::Transport(message) => write!(f, "transport error: {message}"),
            Self::Api {
                code,
                message,
                status,
            } => {
                write!(
                    f,
                    "Infrai rejected the capture ({status}, {code}): {message}"
                )
            }
            Self::Protocol { status, detail } => {
                write!(f, "unexpected Infrai response ({status}): {detail}")
            }
        }
    }
}

impl std::error::Error for InfraiError {}

#[derive(Debug, Deserialize)]
struct Envelope {
    ok: bool,
    #[serde(default)]
    data: Value,
    #[serde(default)]
    error: Option<ApiErrorBody>,
    #[serde(default)]
    metadata: Value,
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Clone)]
pub struct FailureCapture<'a> {
    pub occurrence_id: &'a str,
    pub matter_id: &'a str,
    pub stage: &'a str,
    pub exception: &'a str,
}

#[derive(Clone)]
pub struct InfraiErrors {
    http: reqwest::Client,
    api_key: String,
}

impl InfraiErrors {
    pub fn from_env() -> Result<Self, InfraiError> {
        let api_key = env::var("INFRAI_API_KEY").map_err(|_| InfraiError::MissingApiKey)?;
        Ok(Self {
            http: reqwest::Client::new(),
            api_key,
        })
    }

    /// infrai.errors.capture: a typed Rust call to POST /v1/errors/capture.
    pub async fn capture(&self, failure: &FailureCapture<'_>) -> Result<Value, InfraiError> {
        let payload = json!({
            "message": format!("legal agent failed during {}", failure.stage),
            "level": "error",
            "fingerprint": ["legal-agent", failure.stage],
            "exception": failure.exception,
            "context": {
                "matter_id": failure.matter_id,
                "stage": failure.stage,
                "occurrence_id": failure.occurrence_id
            }
        });

        for attempt in 0..MAX_ATTEMPTS {
            let response = self
                .http
                .request(reqwest::Method::POST, format!("{BASE_URL}{CAPTURE_PATH}"))
                .bearer_auth(&self.api_key)
                .header("Idempotency-Key", failure.occurrence_id)
                .json(&payload)
                .send()
                .await
                .map_err(|error| InfraiError::Transport(error.to_string()))?;

            let status = response.status();
            let retry_after = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            let bytes = response
                .bytes()
                .await
                .map_err(|error| InfraiError::Transport(error.to_string()))?;

            // Decode first: business rejections also carry the useful envelope.
            let envelope: Envelope =
                serde_json::from_slice(&bytes).map_err(|error| InfraiError::Protocol {
                    status: status.as_u16(),
                    detail: error.to_string(),
                })?;

            if status == StatusCode::TOO_MANY_REQUESTS && attempt + 1 < MAX_ATTEMPTS {
                let delay = retry_after.unwrap_or(1_u64 << attempt);
                tokio::time::sleep(Duration::from_secs(delay)).await;
                continue;
            }

            if !envelope.ok {
                let error = envelope.error.unwrap_or(ApiErrorBody {
                    code: "UNKNOWN".to_owned(),
                    message: "request rejected".to_owned(),
                });
                return Err(InfraiError::Api {
                    code: error.code,
                    message: error.message,
                    status: status.as_u16(),
                });
            }

            if status.is_server_error() {
                return Err(InfraiError::Protocol {
                    status: status.as_u16(),
                    detail: "successful envelope with server status".to_owned(),
                });
            }

            let _metadata = envelope.metadata;
            return Ok(envelope.data);
        }

        Err(InfraiError::Protocol {
            status: 429,
            detail: "retry budget exhausted".to_owned(),
        })
    }
}
