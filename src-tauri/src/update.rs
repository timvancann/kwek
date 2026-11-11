//! Updates for release builds: kwek asks GitHub's latest release for a newer version at
//! start, twice a day, and when Check for Updates is chosen in the kwek menu. A newer one
//! is offered in the window; on yes it downloads, checks the signature against the key
//! built into kwek (`plugins.updater` in tauri.conf.json), replaces the app and restarts.
//!
//! Builds from source (`just run`, `just install`) leave all of this out: they carry their
//! own Google client and signing identity, and a release would replace both.

use std::time::Duration;

use serde::Serialize;
use tauri::menu::{Menu, MenuItem, MenuItemKind, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex;

/// Set by the release workflow (`KWEK_UPDATES=1`) when it builds.
pub const ENABLED: bool = option_env!("KWEK_UPDATES").is_some();

const MENU_ID: &str = "check-for-updates";
const EVERY: Duration = Duration::from_secs(12 * 60 * 60);
/// Long enough after start for the window to listen.
const FIRST_AFTER: Duration = Duration::from_secs(20);
/// A stalled check must not stop the next ones.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// For the whole download (about 150 MB with the model), so it is generous: a stalled one
/// ends with an error toast instead of spinning forever.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// The update found by the last check, until it is installed.
#[derive(Default)]
pub struct Updates {
    found: Mutex<Option<Update>>,
}

/// What a check tells the window, as `update:checked`.
#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Checked {
    Available {
        version: String,
    },
    /// Only when asked for: a check on its own says nothing when there is nothing new.
    Latest {
        version: String,
    },
    Failed,
}

pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R, tauri_plugin_updater::Config> {
    tauri_plugin_updater::Builder::new().build()
}

/// The standard menu bar with Check for Updates… under About in the kwek menu.
pub fn install_menu(app: &AppHandle) -> tauri::Result<()> {
    let menu = Menu::default(app)?;
    if let Some(MenuItemKind::Submenu(kwek)) = menu.items()?.first() {
        let item = MenuItem::with_id(app, MENU_ID, "Check for Updates…", true, None::<&str>)?;
        kwek.insert(&item, 1)?;
        kwek.insert(&PredefinedMenuItem::separator(app)?, 1)?;
    }
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        if event.id() == MENU_ID {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { check(&app, true).await });
        }
    });
    Ok(())
}

/// Checks at start and then every [`EVERY`], for as long as kwek runs.
pub async fn run(app: AppHandle) {
    tokio::time::sleep(FIRST_AFTER).await;
    loop {
        check(&app, false).await;
        tokio::time::sleep(EVERY).await;
    }
}

async fn check(app: &AppHandle, asked: bool) {
    let found = match app.updater_builder().timeout(CHECK_TIMEOUT).build() {
        Ok(updater) => updater.check().await,
        Err(e) => Err(e),
    };
    let checked = match found {
        Ok(Some(update)) => {
            tracing::info!("update available: {}", update.version);
            let version = update.version.clone();
            *app.state::<Updates>().found.lock().await = Some(update);
            Checked::Available { version }
        }
        Ok(None) => {
            // A release taken back: its toast must not install it any more.
            *app.state::<Updates>().found.lock().await = None;
            if !asked {
                return;
            }
            Checked::Latest {
                version: app.package_info().version.to_string(),
            }
        }
        Err(e) => {
            tracing::warn!("update check failed: {e}");
            if !asked {
                return;
            }
            Checked::Failed
        }
    };
    let _ = app.emit("update:checked", checked);
}

/// Downloads and installs the update the last check found, then restarts kwek.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let Some(mut update) = app.state::<Updates>().found.lock().await.clone() else {
        return Err("There's no update to install.".into());
    };
    update.timeout = Some(DOWNLOAD_TIMEOUT);
    tracing::info!("installing kwek {}", update.version);
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| {
        tracing::warn!("update failed: {e}");
        "The update didn't install. Try again later, or download it from GitHub.".to_string()
    })?;
    app.restart()
}
