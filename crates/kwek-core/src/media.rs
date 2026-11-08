//! Avatar proxy with a disk cache. Attachments go through [`crate::sync::Engine::media`],
//! which needs auth; avatars are public URLs and are fetched without credentials.

use std::path::Path;
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::views::is_avatar_host;

const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;

/// No redirects: a redirect could leave the allowed avatar host.
fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("avatar client")
    })
}

const MAX_FAVICON_BYTES: usize = 256 * 1024;

/// A plain host name: what a favicon is looked up by.
fn is_host(host: &str) -> bool {
    (3..=253).contains(&host.len())
        && host.contains('.')
        && !host.starts_with(['.', '-'])
        && host
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
}

/// Google's favicon service redirects to gstatic; nowhere else.
fn favicon_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let ok = attempt.url().scheme() == "https"
                    && attempt
                        .url()
                        .host_str()
                        .is_some_and(|h| h == "www.google.com" || h.ends_with(".gstatic.com") || h == "gstatic.com")
                    && attempt.previous().len() < 5;
                if ok { attempt.follow() } else { attempt.stop() }
            }))
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("favicon client")
    })
}

/// A site's icon, for links in messages: from Google's favicon service (Google already sees
/// every message, so no new party learns which sites are linked), cached on disk.
pub async fn favicon(cache_dir: &Path, host: &str) -> Result<(Vec<u8>, Option<String>)> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if !is_host(&host) {
        return Err(Error::Other("not a host name".into()));
    }
    let path = cache_dir.join(format!("favicon-{host}"));
    let mime_path = cache_dir.join(format!("favicon-{host}.type"));
    if let Ok(bytes) = tokio::fs::read(&path).await {
        return Ok((bytes, tokio::fs::read_to_string(&mime_path).await.ok()));
    }
    let url = format!("https://www.google.com/s2/favicons?domain={host}&sz=64");
    let resp = favicon_client().get(&url).send().await?;
    // Unknown sites get a 404 here (and the webview shows a plain link mark).
    if resp.content_length().is_some_and(|n| n as usize > MAX_FAVICON_BYTES) {
        return Err(Error::Other("favicon too large".into()));
    }
    if !resp.status().is_success() {
        return Err(Error::Api {
            status: resp.status().as_u16(),
            status_code: None,
            message: "favicon fetch failed".into(),
        });
    }
    let mime = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .filter(|m| m.starts_with("image/"))
        .ok_or_else(|| Error::Other("favicon is not an image".into()))?;
    let bytes = resp.bytes().await?.to_vec();
    if bytes.len() > MAX_FAVICON_BYTES {
        return Err(Error::Other("favicon too large".into()));
    }
    tokio::fs::create_dir_all(cache_dir).await?;
    // Type first, then the icon via a temp file and rename: a reader never sees half an icon.
    tokio::fs::write(&mime_path, &mime).await?;
    let tmp = cache_dir.join(format!("favicon-{host}.{}.tmp", uuid::Uuid::new_v4().simple()));
    tokio::fs::write(&tmp, &bytes).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok((bytes, Some(mime)))
}

pub async fn avatar(cache_dir: &Path, url: &str) -> Result<(Vec<u8>, Option<String>)> {
    if !is_avatar_host(url) {
        return Err(Error::Other("avatar host not allowed".into()));
    }
    let key = format!("avatar-{:x}", Sha256::digest(url.as_bytes()));
    let path = cache_dir.join(&key);
    let mime_path = cache_dir.join(format!("{key}.type"));
    if let Ok(bytes) = tokio::fs::read(&path).await {
        return Ok((bytes, tokio::fs::read_to_string(&mime_path).await.ok()));
    }
    let resp = client().get(url).send().await?;
    if !resp.status().is_success() {
        return Err(Error::Api {
            status: resp.status().as_u16(),
            status_code: None,
            message: "avatar fetch failed".into(),
        });
    }
    let mime = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .filter(|m| m.starts_with("image/"))
        .ok_or_else(|| Error::Other("avatar is not an image".into()))?;
    let bytes = resp.bytes().await?.to_vec();
    if bytes.len() > MAX_AVATAR_BYTES {
        return Err(Error::Other("avatar too large".into()));
    }
    tokio::fs::create_dir_all(cache_dir).await?;
    tokio::fs::write(&path, &bytes).await?;
    tokio::fs::write(&mime_path, &mime).await?;
    Ok((bytes, Some(mime)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favicons_only_for_plain_host_names() {
        assert!(is_host("github.com"));
        assert!(is_host("docs.google.com"));
        assert!(!is_host("localhost"), "no dot");
        assert!(!is_host("../etc/passwd"));
        assert!(!is_host("evil.com/x?y"));
        assert!(!is_host("-bad.com"));
        assert!(!is_host("UPPER.com"), "lowercased by the caller first");
    }
}
