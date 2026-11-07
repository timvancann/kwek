//! Avatar proxy with a disk cache. Attachments go through [`crate::sync::Engine::media`],
//! which needs auth; avatars are public URLs and are fetched without credentials.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::views::is_avatar_host;

pub async fn avatar(http: &reqwest::Client, cache_dir: &Path, url: &str) -> Result<(Vec<u8>, Option<String>)> {
    if !is_avatar_host(url) {
        return Err(Error::Other("avatar host not allowed".into()));
    }
    let path = cache_dir.join(format!("avatar-{:x}", Sha256::digest(url.as_bytes())));
    if let Ok(bytes) = tokio::fs::read(&path).await {
        return Ok((bytes, None));
    }
    let resp = http.get(url).send().await?;
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
        .map(str::to_owned);
    let bytes = resp.bytes().await?.to_vec();
    tokio::fs::create_dir_all(cache_dir).await?;
    tokio::fs::write(&path, &bytes).await?;
    Ok((bytes, mime))
}
