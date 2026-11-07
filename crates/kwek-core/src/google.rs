//! The one door to Google: auth header, a shared in-flight limit, and backoff on 429/5xx.

use std::sync::Arc;
use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::auth::TokenManager;
use crate::error::{Error, Result};

const MAX_IN_FLIGHT: usize = 4;
const MAX_ATTEMPTS: u32 = 6;

#[derive(Clone)]
pub struct Google {
    http: reqwest::Client,
    tokens: Arc<TokenManager>,
    limiter: Arc<Semaphore>,
}

impl Google {
    pub fn new(http: reqwest::Client, tokens: Arc<TokenManager>) -> Self {
        Self {
            http,
            tokens,
            limiter: Arc::new(Semaphore::new(MAX_IN_FLIGHT)),
        }
    }

    pub fn tokens(&self) -> &Arc<TokenManager> {
        &self.tokens
    }

    pub async fn get<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        self.json(Method::GET, url, None).await
    }

    pub async fn post<T: DeserializeOwned>(&self, url: &str, body: &Value) -> Result<T> {
        self.json(Method::POST, url, Some(body)).await
    }

    pub async fn patch<T: DeserializeOwned>(&self, url: &str, body: &Value) -> Result<T> {
        self.json(Method::PATCH, url, Some(body)).await
    }

    pub async fn put<T: DeserializeOwned>(&self, url: &str, body: &Value) -> Result<T> {
        self.json(Method::PUT, url, Some(body)).await
    }

    pub async fn delete(&self, url: &str) -> Result<()> {
        self.json::<Value>(Method::DELETE, url, None).await.map(|_| ())
    }

    pub async fn json<T: DeserializeOwned>(&self, method: Method, url: &str, body: Option<&Value>) -> Result<T> {
        let bytes = self.send(method, url, body).await?.0;
        if bytes.is_empty() {
            return Ok(serde_json::from_str("{}")?);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Raw bytes plus content type, for media downloads.
    pub async fn bytes(&self, url: &str) -> Result<(Vec<u8>, Option<String>)> {
        self.send(Method::GET, url, None).await
    }

    async fn send(&self, method: Method, url: &str, body: Option<&Value>) -> Result<(Vec<u8>, Option<String>)> {
        let _permit = self.limiter.acquire().await.expect("limiter closed");
        let mut refreshed = false;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let token = self.tokens.access_token().await?;
            let mut req = self.http.request(method.clone(), url).bearer_auth(&token);
            if let Some(b) = body {
                req = req.json(b);
            }
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) if attempt < 3 && (e.is_timeout() || e.is_connect()) => {
                    tokio::time::sleep(backoff(attempt)).await;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            let status = resp.status();
            if status.is_success() {
                let ct = resp
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                return Ok((resp.bytes().await?.to_vec(), ct));
            }
            if status == StatusCode::UNAUTHORIZED && !refreshed {
                refreshed = true;
                self.tokens.invalidate().await;
                continue;
            }
            let retryable = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            if retryable && attempt < MAX_ATTEMPTS {
                tracing::debug!(%status, attempt, "retrying Google call");
                tokio::time::sleep(backoff(attempt)).await;
                continue;
            }
            let text = resp.text().await.unwrap_or_default();
            return Err(api_error(status, &text));
        }
    }
}

/// Exponential backoff with full jitter: random in [0, min(30 s, 500 ms * 2^attempt)].
fn backoff(attempt: u32) -> Duration {
    let cap = (500u64 << attempt.min(6)).min(30_000);
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).ok();
    Duration::from_millis(u64::from_le_bytes(b) % (cap + 1))
}

pub fn api_error(status: StatusCode, body: &str) -> Error {
    #[derive(Deserialize)]
    struct Envelope {
        error: Inner,
    }
    #[derive(Deserialize)]
    struct Inner {
        #[serde(default)]
        message: String,
        #[serde(default)]
        status: Option<String>,
    }
    match serde_json::from_str::<Envelope>(body) {
        Ok(e) => Error::Api {
            status: status.as_u16(),
            status_code: e.error.status,
            message: e.error.message,
        },
        Err(_) => Error::Api {
            status: status.as_u16(),
            status_code: None,
            message: body.chars().take(200).collect(),
        },
    }
}

/// Percent-encodes a query parameter value.
pub fn q(v: &str) -> String {
    percent_encoding::utf8_percent_encode(v, percent_encoding::NON_ALPHANUMERIC).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_google_error_envelope() {
        let e = api_error(
            StatusCode::CONFLICT,
            r#"{"error":{"code":409,"message":"exists","status":"ALREADY_EXISTS"}}"#,
        );
        assert!(e.is_status("ALREADY_EXISTS"));
        assert_eq!(e.http_status(), Some(409));
    }

    #[test]
    fn backoff_is_capped() {
        for a in 1..20 {
            assert!(backoff(a) <= Duration::from_secs(30));
        }
    }
}
