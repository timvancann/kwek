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
