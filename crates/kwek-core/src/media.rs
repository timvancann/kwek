//! Avatar proxy with a disk cache. Attachments go through [`crate::sync::Engine::media`],
//! which needs auth; avatars are public URLs and are fetched without credentials. Also the
//! byte ranges a `<video>` asks for.

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

/// At most this much per answer to a `Range` request: WebKit asks for the rest of a video and
/// comes back for what it didn't get, so a big file never sits in memory whole.
pub const MAX_RANGE: u64 = 4 * 1024 * 1024;

/// The bytes a `Range` header asks for in a file of `total` bytes, first and last inclusive:
/// the first range only, and at most [`MAX_RANGE`] of it. `None` when it can't be served (416).
pub fn byte_range(header: &str, total: u64) -> Option<(u64, u64)> {
    let spec = header.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    let last = total.checked_sub(1)?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => match n.parse::<u64>().ok()? {
            0 => return None,
            n => (total.saturating_sub(n), last),
        },
        (s, "") => (s.parse().ok()?, last),
        (s, e) => {
            let (s, e): (u64, u64) = (s.parse().ok()?, e.parse().ok()?);
            if e < s {
                return None;
            }
            (s, e.min(last))
        }
    };
    if start > last {
        return None;
    }
    Some((start, end.min(start + MAX_RANGE - 1)))
}

/// The bytes from `start` to `end` (inclusive) of a file.
pub async fn read_range(path: &Path, start: u64, end: u64) -> Result<Vec<u8>> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let mut f = tokio::fs::File::open(path).await?;
    f.seek(std::io::SeekFrom::Start(start)).await?;
    let mut buf = vec![0; (end - start + 1) as usize];
    f.read_exact(&mut buf).await?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_as_webkit_asks_for_them() {
        assert_eq!(byte_range("bytes=0-1", 1000), Some((0, 1)), "the probe before playing");
        assert_eq!(byte_range("bytes=100-", 1000), Some((100, 999)));
        assert_eq!(byte_range("bytes=-10", 1000), Some((990, 999)), "the last ten");
        assert_eq!(byte_range("bytes=-5000", 1000), Some((0, 999)), "more than there is");
        assert_eq!(
            byte_range("bytes=900-5000", 1000),
            Some((900, 999)),
            "end past the file"
        );
        assert_eq!(byte_range("bytes=0-1, 5-6", 1000), Some((0, 1)), "first range only");
        let big = 100 * MAX_RANGE;
        assert_eq!(byte_range("bytes=0-", big), Some((0, MAX_RANGE - 1)), "capped");
    }

    #[test]
    fn ranges_that_cannot_be_served() {
        assert_eq!(byte_range("bytes=1000-", 1000), None, "starts past the end");
        assert_eq!(byte_range("bytes=5-2", 1000), None);
        assert_eq!(byte_range("bytes=-0", 1000), None);
        assert_eq!(byte_range("bytes=0-1", 0), None, "empty file");
        assert_eq!(byte_range("items=0-1", 1000), None);
        assert_eq!(byte_range("bytes=a-b", 1000), None);
    }

    #[tokio::test]
    async fn reads_just_the_range() {
        let path = std::env::temp_dir().join(format!("kwek-range-{}", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, b"0123456789").await.unwrap();
        assert_eq!(read_range(&path, 2, 4).await.unwrap(), b"234");
        let _ = tokio::fs::remove_file(&path).await;
    }

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
