//! Installed-app OAuth: loopback redirect on 127.0.0.1, PKCE (S256), offline access.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::config::{OAuthClient, SCOPES};
use crate::error::{Error, Result};

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: u64,
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: String,
}

fn random_token() -> String {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("OS RNG unavailable");
    URL_SAFE_NO_PAD.encode(buf)
}

pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn auth_url(client: &OAuthClient, redirect: &str, state: &str, challenge: &str) -> String {
    let mut u = url::Url::parse(AUTH_URL).unwrap();
    u.query_pairs_mut()
        .append_pair("client_id", &client.id)
        .append_pair("redirect_uri", redirect)
        .append_pair("response_type", "code")
        .append_pair("scope", &SCOPES.join(" "))
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");
    u.into()
}

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>kwek</title>\
<body style=\"font:16px -apple-system,system-ui,sans-serif;display:grid;place-items:center;height:90vh;color:#18181b\">\
<div style=\"text-align:center\"><h1 style=\"font-size:20px\">Signed in to kwek</h1>\
<p>You can close this tab and go back to the app.</p></div>";

const FAIL_PAGE: &str = "<!doctype html><meta charset=utf-8><title>kwek</title>\
<body style=\"font:16px -apple-system,system-ui,sans-serif;display:grid;place-items:center;height:90vh;color:#18181b\">\
<div style=\"text-align:center\"><h1 style=\"font-size:20px\">Sign-in did not complete</h1>\
<p>Go back to kwek and try again.</p></div>";

/// Runs the browser flow. `open` receives the consent URL and must open it in the system browser.
pub async fn login(
    http: &reqwest::Client,
    client: &OAuthClient,
    open: impl FnOnce(&str) -> Result<()>,
) -> Result<TokenResponse> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let redirect = format!("http://127.0.0.1:{port}");
    let verifier = random_token();
    let state = random_token();
    open(&auth_url(client, &redirect, &state, &pkce_challenge(&verifier)))?;

    let code = tokio::time::timeout(LOGIN_TIMEOUT, accept_code(&listener, &state))
        .await
        .map_err(|_| Error::Other("sign-in timed out".into()))??;
    drop(listener);

    let resp = http
        .post(TOKEN_URL)
        .form(&[
            ("code", code.as_str()),
            ("client_id", client.id.as_str()),
            ("client_secret", client.secret.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await?;
    token_result(resp).await
}

/// Accepts connections until one carries the OAuth redirect. Stray requests (favicon,
/// prefetch) get a 404 and do not end the flow; a redirect with the wrong `state` does.
async fn accept_code(listener: &TcpListener, state: &str) -> Result<String> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = sock.read(&mut buf).await.unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        let target = req
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("/")
            .to_owned();
        let url = url::Url::parse(&format!("http://127.0.0.1{target}"))
            .map_err(|e| Error::Other(e.to_string()))?;
        let get = |k: &str| {
            url.query_pairs()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.into_owned())
        };
        let (code, err, got_state) = (get("code"), get("error"), get("state"));
        if code.is_none() && err.is_none() {
            let _ = sock
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
            continue;
        }
        let ok = code.is_some() && got_state.as_deref() == Some(state);
        let page = if ok { DONE_PAGE } else { FAIL_PAGE };
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        let _ = sock.shutdown().await;
        if let Some(e) = err {
            return Err(Error::Other(format!("sign-in refused: {e}")));
        }
        if !ok {
            return Err(Error::Other("sign-in state mismatch".into()));
        }
        return Ok(code.unwrap());
    }
}

pub async fn refresh(
    http: &reqwest::Client,
    client: &OAuthClient,
    refresh_token: &str,
) -> Result<TokenResponse> {
    let resp = http
        .post(TOKEN_URL)
        .form(&[
            ("client_id", client.id.as_str()),
            ("client_secret", client.secret.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await?;
    token_result(resp).await
}

pub async fn revoke(http: &reqwest::Client, token: &str) -> Result<()> {
    http.post(REVOKE_URL).form(&[("token", token)]).send().await?;
    Ok(())
}

async fn token_result(resp: reqwest::Response) -> Result<TokenResponse> {
    let status = resp.status();
    let body = resp.text().await?;
    if status.is_success() {
        return Ok(serde_json::from_str(&body)?);
    }
    #[derive(Deserialize)]
    struct OAuthError {
        error: String,
        #[serde(default)]
        error_description: String,
    }
    match serde_json::from_str::<OAuthError>(&body) {
        Ok(e) if e.error == "invalid_grant" => Err(Error::AuthRequired),
        Ok(e) => Err(Error::Api {
            status: status.as_u16(),
            status_code: Some(e.error),
            message: e.error_description,
        }),
        Err(_) => Err(Error::Api {
            status: status.as_u16(),
            status_code: None,
            message: "token endpoint error".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc7636_example() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn auth_url_requests_offline_pkce() {
        let c = OAuthClient {
            id: "cid".into(),
            secret: "s".into(),
        };
        let u = auth_url(&c, "http://127.0.0.1:5000", "st", "ch");
        assert!(u.contains("access_type=offline"));
        assert!(u.contains("code_challenge_method=S256"));
        assert!(u.contains("prompt=consent"));
        assert!(u.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A5000"));
    }

    #[tokio::test]
    async fn loopback_ignores_strays_and_checks_state() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move { accept_code(&listener, "good").await });
        let http = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{port}");
        assert_eq!(http.get(format!("{base}/favicon.ico")).send().await.unwrap().status(), 404);
        let r = http.get(format!("{base}/?code=abc&state=good")).send().await.unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(task.await.unwrap().unwrap(), "abc");

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move { accept_code(&listener, "good").await });
        http.get(format!("http://127.0.0.1:{port}/?code=abc&state=evil"))
            .send()
            .await
            .unwrap();
        assert!(task.await.unwrap().is_err());
    }
}
