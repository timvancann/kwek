//! Updates on Android, for release builds: an app can't replace itself there, so kwek asks
//! the same `latest.json` as the desktop updater (`plugins.updater.endpoints` in
//! tauri.conf.json) for a newer version at start and twice a day, and offers its APK to
//! download. Opening the downloaded file installs it over this one, keeping kwek's data,
//! as long as it is signed with the same key (docs/releasing.md).

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

/// Set by the release workflow (`KWEK_UPDATES=1`) when it builds.
pub const ENABLED: bool = option_env!("KWEK_UPDATES").is_some();

/// The APK's entry in latest.json's `platforms`.
const PLATFORM: &str = "android-aarch64";
const EVERY: Duration = Duration::from_secs(12 * 60 * 60);
/// Long enough after start for the window to listen.
const FIRST_AFTER: Duration = Duration::from_secs(20);
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// Told to the window as `update:checked`, like the desktop's checks.
#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Checked {
    Download { version: String, url: String },
}

/// Checks at start and then every [`EVERY`], for as long as kwek runs.
pub async fn run<R: Runtime>(app: AppHandle<R>) {
    let Some(endpoint) = endpoint(&app) else {
        tracing::warn!("no updater endpoint in tauri.conf.json");
        return;
    };
    let client = match reqwest::Client::builder().timeout(CHECK_TIMEOUT).build() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("update check: {e}");
            return;
        }
    };
    let current = app.package_info().version.to_string();
    tokio::time::sleep(FIRST_AFTER).await;
    loop {
        match fetch(&client, &endpoint).await.and_then(|m| newer(&m, &current)) {
            Ok(Some((version, url))) => {
                tracing::info!("update available: {version}");
                let _ = app.emit("update:checked", Checked::Download { version, url });
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("update check failed: {e}"),
        }
        tokio::time::sleep(EVERY).await;
    }
}

fn endpoint<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let updater = app.config().plugins.0.get("updater")?;
    Some(updater.get("endpoints")?.get(0)?.as_str()?.to_owned())
}

async fn fetch(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let res = client.get(url).send().await.map_err(|e| e.without_url().to_string())?;
    let res = res.error_for_status().map_err(|e| e.without_url().to_string())?;
    res.text().await.map_err(|e| e.without_url().to_string())
}

/// The version and APK link in `manifest` (latest.json) when it is newer than `current`.
/// A release without an APK offers nothing.
fn newer(manifest: &str, current: &str) -> Result<Option<(String, String)>, String> {
    let json: serde_json::Value = serde_json::from_str(manifest).map_err(|e| format!("latest.json: {e}"))?;
    let version = json["version"].as_str().ok_or("latest.json has no version")?;
    let (Some(latest), Some(ours)) = (parse(version), parse(current)) else {
        return Err(format!("can't compare versions {version} and {current}"));
    };
    if latest <= ours {
        return Ok(None);
    }
    let Some(url) = json["platforms"][PLATFORM]["url"].as_str() else {
        return Ok(None);
    };
    Ok(Some((version.trim_start_matches('v').to_owned(), url.to_owned())))
}

/// "0.10.2" (or "v0.10.2") as numbers, which compare the way versions do.
fn parse(v: &str) -> Option<Vec<u64>> {
    v.trim_start_matches('v').split('.').map(|p| p.parse().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str, android: bool) -> String {
        let mut platforms = serde_json::json!({
            "darwin-aarch64": {"signature": "s", "url": "https://example.com/kwek.app.tar.gz"},
        });
        if android {
            platforms[PLATFORM] = serde_json::json!({"signature": "s", "url": "https://example.com/kwek-android.apk"});
        }
        serde_json::json!({"version": version, "platforms": platforms}).to_string()
    }

    #[test]
    fn offers_a_newer_apk_only() {
        let apk = Some(("0.10.0".to_owned(), "https://example.com/kwek-android.apk".to_owned()));
        let cases = [
            ("0.10.0", true, "0.9.3", apk.clone()),
            ("v0.10.0", true, "0.9.3", apk.clone()),
            ("0.10.0", true, "0.10.0", None),
            ("0.9.3", true, "0.10.0", None),
            // A release from before Android builds: nothing to download.
            ("0.10.0", false, "0.9.3", None),
        ];
        for (latest, android, current, want) in cases {
            assert_eq!(
                newer(&manifest(latest, android), current).unwrap(),
                want,
                "{latest} over {current}"
            );
        }
    }

    #[test]
    fn unreadable_manifests_are_errors() {
        assert!(newer("<html>", "0.4.0").is_err());
        assert!(newer(r#"{"platforms":{}}"#, "0.4.0").is_err());
        assert!(newer(&manifest("next", true), "0.4.0").is_err());
    }
}
