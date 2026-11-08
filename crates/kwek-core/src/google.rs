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
/// Uploads take their own slots: a slow one must not hold up sync and text sends.
const MAX_UPLOADS: usize = 2;
const MAX_ATTEMPTS: u32 = 6;

#[derive(Clone)]
pub struct Google {
    http: reqwest::Client,
    tokens: Arc<TokenManager>,
    limiter: Arc<Semaphore>,
    uploads: Arc<Semaphore>,
}

impl Google {
    pub fn new(http: reqwest::Client, tokens: Arc<TokenManager>) -> Self {
        Self {
            http,
            tokens,
            limiter: Arc::new(Semaphore::new(MAX_IN_FLIGHT)),
            uploads: Arc::new(Semaphore::new(MAX_UPLOADS)),
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

    /// Raw bytes plus content type, for media downloads.
    pub async fn bytes(&self, url: &str) -> Result<(Vec<u8>, Option<String>)> {
        self.send(Method::GET, url, None, None).await
    }

    /// A media upload (`uploadType=multipart`): JSON metadata and the file's bytes in one
    /// multipart/related body, with the same auth, limit and retries as every other call.
    pub async fn upload<T: DeserializeOwned>(
        &self,
        url: &str,
        metadata: &Value,
        content_type: &str,
        data: &[u8],
    ) -> Result<T> {
        let (body, ct) = multipart_related(metadata, content_type, data);
        let bytes = self.send(Method::POST, url, Some(Body::Raw(&body, &ct)), None).await?.0;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// A long-poll POST (Pub/Sub pull): outside the in-flight limit, which is for Chat quota,
    /// with its own timeout. A timeout means "nothing arrived" and returns `{}`.
    pub async fn long_poll<T: DeserializeOwned>(&self, url: &str, body: &Value, wait: Duration) -> Result<T> {
        match self.send(Method::POST, url, Some(Body::Json(body)), Some(wait)).await {
            Ok((bytes, _)) if !bytes.is_empty() => Ok(serde_json::from_slice(&bytes)?),
            Ok(_) | Err(Error::Timeout) => Ok(serde_json::from_str("{}")?),
            Err(e) => Err(e),
        }
    }

    async fn json<T: DeserializeOwned>(&self, method: Method, url: &str, body: Option<&Value>) -> Result<T> {
        let bytes = self.send(method, url, body.map(Body::Json), None).await?.0;
        if bytes.is_empty() {
            return Ok(serde_json::from_str("{}")?);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// `long_poll: Some(wait)` skips the limiter, uses `wait` as the timeout and reports a
    /// timeout as [`Error::Timeout`] without retrying.
    async fn send(
        &self,
        method: Method,
        url: &str,
        body: Option<Body<'_>>,
        long_poll: Option<Duration>,
    ) -> Result<(Vec<u8>, Option<String>)> {
        let mut refreshed = false;
        let mut attempt = 0;
        loop {
            attempt += 1;
            // A permit per attempt, released before any backoff sleep, so one throttled call
            // does not hold up everything else.
            let upload = matches!(body, Some(Body::Raw(..)));
            let permit = match (long_poll, upload) {
                (Some(_), _) => None,
                (None, true) => Some(self.uploads.acquire().await.expect("limiter closed")),
                (None, false) => Some(self.limiter.acquire().await.expect("limiter closed")),
            };
            let token = self.tokens.access_token().await?;
            let mut req = self.http.request(method.clone(), url).bearer_auth(&token);
            match body {
                Some(Body::Json(b)) => req = req.json(b),
                Some(Body::Raw(bytes, content_type)) => {
                    // Time to send it even on a slow uplink: a minute plus a second per 256 KB.
                    let secs = 60 + (bytes.len() / (256 * 1024)) as u64;
                    req = req
                        .header(reqwest::header::CONTENT_TYPE, content_type)
                        .timeout(Duration::from_secs(secs))
                        .body(bytes.to_vec())
                }
                None => {}
            }
            if let Some(wait) = long_poll {
                req = req.timeout(wait);
            }
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) if long_poll.is_some() && e.is_timeout() => return Err(Error::Timeout),
                // An upload that timed out would only time out again; one that never connected
                // is worth another go.
                Err(e) if attempt < 3 && (e.is_connect() || (e.is_timeout() && !upload)) => {
                    drop(permit);
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
                let retry_after = resp
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .map(|s| Duration::from_secs(s.min(60)));
                tracing::debug!(%status, attempt, "retrying Google call");
                drop(permit);
                tokio::time::sleep(retry_after.unwrap_or_default().max(backoff(attempt))).await;
                continue;
            }
            let text = resp.text().await.unwrap_or_default();
            return Err(api_error(status, &text));
        }
    }
}

#[derive(Clone, Copy)]
enum Body<'a> {
    Json(&'a Value),
    /// Bytes and their content type.
    Raw(&'a [u8], &'a str),
}

/// A multipart/related body: the JSON metadata part, then the media part. Returns the body
/// and its content type (with the boundary).
fn multipart_related(metadata: &Value, content_type: &str, data: &[u8]) -> (Vec<u8>, String) {
    let boundary = format!("kwek-{}", uuid::Uuid::new_v4().simple());
    let mut body = Vec::with_capacity(data.len() + 512);
    body.extend_from_slice(format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n").as_bytes());
    body.extend_from_slice(metadata.to_string().as_bytes());
    body.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Type: {content_type}\r\n\r\n").as_bytes());
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (body, format!("multipart/related; boundary={boundary}"))
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
    fn multipart_related_body_has_both_parts() {
        let (body, ct) = multipart_related(&serde_json::json!({"filename": "a.png"}), "image/png", b"\x89PNG");
        let boundary = ct.strip_prefix("multipart/related; boundary=").unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with(&format!("--{boundary}\r\nContent-Type: application/json")));
        assert!(text.contains(r#"{"filename":"a.png"}"#));
        assert!(text.contains("Content-Type: image/png\r\n\r\n\u{FFFD}PNG"));
        assert!(text.ends_with(&format!("\r\n--{boundary}--\r\n")));
    }

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
