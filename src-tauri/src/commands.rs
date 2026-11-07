//! IPC commands: request/response against local state. Errors cross as plain sentences the
//! UI can show in a toast.

use kwek_core::Error;
use kwek_core::sync::Engine;
use kwek_core::views::{AuthState, MessageView, SearchResult, SpaceSummary, SyncStatus, UserRef};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::AppState;

type Res<T> = Result<T, String>;

fn msg(e: Error) -> String {
    // The real response always goes to the log; the toast gets a sentence.
    tracing::warn!("command failed: {e}");
    match e {
        Error::AuthRequired => "Sign in to Google again to continue.".into(),
        Error::NoOAuthClient => "This build has no Google client configured.".into(),
        Error::Network(_) | Error::Timeout => "No connection. Try again when you are back online.".into(),
        Error::Api { message, .. } if message.contains("Chat app not found") => {
            "Google Chat refuses this until the kwek Chat app is configured (docs/setup.md, step 3).".into()
        }
        Error::Api { status: 403, .. } => "Google refused this action (no permission).".into(),
        Error::Api { status: 429, .. } => "Google is rate limiting kwek. Try again in a moment.".into(),
        Error::Api { message, .. } if !message.is_empty() => format!("Google Chat: {message}"),
        Error::Api { status: 404, .. } => "Google Chat could not find that message or conversation.".into(),
        other => other.to_string(),
    }
}

/// Synchronous commands run on the main thread in Tauri 2; reads that touch SQLite or the
/// Keychain go to the blocking pool instead, so a long sync transaction never stalls the UI.
async fn blocking<T: Send + 'static>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&Engine) -> kwek_core::Result<T> + Send + 'static,
) -> Res<T> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || f(&engine))
        .await
        .map_err(|e| e.to_string())?
        .map_err(msg)
}

#[tauri::command]
pub async fn auth_status(state: State<'_, AppState>) -> Res<AuthState> {
    blocking(&state, |e| Ok(e.auth_state())).await
}

#[tauri::command]
pub async fn login(app: AppHandle, state: State<'_, AppState>) -> Res<()> {
    state
        .engine
        .login(|url| {
            app.opener()
                .open_url(url, None::<&str>)
                .map_err(|e| Error::Other(format!("could not open the browser: {e}")))
        })
        .await
        .map_err(msg)
}

#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> Res<()> {
    state.engine.logout().await.map_err(msg)
}

#[tauri::command]
pub fn sync_status(state: State<'_, AppState>) -> SyncStatus {
    state.engine.sync_status()
}

#[tauri::command]
pub fn refresh(state: State<'_, AppState>) {
    state.engine.poke();
}

#[tauri::command]
pub async fn list_spaces(state: State<'_, AppState>) -> Res<Vec<SpaceSummary>> {
    blocking(&state, |e| e.list_spaces()).await
}

#[tauri::command]
pub async fn get_messages(
    state: State<'_, AppState>,
    space: String,
    before: Option<i64>,
    limit: u32,
) -> Res<Vec<MessageView>> {
    blocking(&state, move |e| e.get_messages(&space, before, limit)).await
}

#[tauri::command]
pub async fn load_older(state: State<'_, AppState>, space: String) -> Res<u32> {
    state.engine.load_older(&space).await.map_err(msg)
}

/// Returns the cached thread now and refreshes it from the API in the background; fresh
/// replies arrive as `messages:upserted`.
#[tauri::command]
pub async fn get_thread(state: State<'_, AppState>, thread: String) -> Res<Vec<MessageView>> {
    let t = thread.clone();
    let cached = blocking(&state, move |e| e.get_thread_cached(&t)).await?;
    let engine = state.engine.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = engine.fetch_thread(&thread).await {
            tracing::warn!("thread fetch failed: {e}");
        }
    });
    Ok(cached)
}

#[tauri::command]
pub async fn send_message(
    state: State<'_, AppState>,
    space: String,
    thread: Option<String>,
    text: String,
) -> Res<MessageView> {
    let text = text.trim_end().to_owned();
    if text.trim().is_empty() {
        return Err("Nothing to send.".into());
    }
    state
        .engine
        .send_message(&space, thread.as_deref(), &text)
        .await
        .map_err(msg)
}

#[tauri::command]
pub async fn retry_send(state: State<'_, AppState>, name: String) -> Res<()> {
    state.engine.retry_send(&name).await.map_err(msg)
}

#[tauri::command]
pub async fn discard_failed(state: State<'_, AppState>, name: String) -> Res<()> {
    state.engine.discard_failed(&name).await.map_err(msg)
}

#[tauri::command]
pub async fn edit_message(state: State<'_, AppState>, name: String, text: String) -> Res<()> {
    if text.trim().is_empty() {
        return Err("A message cannot be empty. Delete it instead.".into());
    }
    state.engine.edit_message(&name, text.trim_end()).await.map_err(msg)
}

#[tauri::command]
pub async fn delete_message(state: State<'_, AppState>, name: String) -> Res<()> {
    state.engine.delete_message(&name).await.map_err(msg)
}

#[tauri::command]
pub async fn toggle_reaction(state: State<'_, AppState>, message: String, emoji: String) -> Res<()> {
    state.engine.toggle_reaction(&message, &emoji).await.map_err(msg)
}

#[tauri::command]
pub async fn mark_read(state: State<'_, AppState>, space: String, up_to: i64) -> Res<()> {
    state.engine.mark_read(&space, up_to).await.map_err(msg)
}

#[tauri::command]
pub async fn mark_thread_read(state: State<'_, AppState>, thread: String, up_to: i64) -> Res<()> {
    state.engine.mark_thread_read(&thread, up_to).await.map_err(msg)
}

#[tauri::command]
pub async fn search(state: State<'_, AppState>, query: String, limit: u32, offset: u32) -> Res<SearchResult> {
    blocking(&state, move |e| e.search(&query, limit, offset)).await
}

#[tauri::command]
pub async fn list_members(state: State<'_, AppState>, space: String) -> Res<Vec<UserRef>> {
    blocking(&state, move |e| e.list_members(&space)).await
}

#[tauri::command]
pub async fn set_space_prefs(
    state: State<'_, AppState>,
    space: String,
    muted: Option<bool>,
    starred: Option<bool>,
) -> Res<()> {
    blocking(&state, move |e| e.set_space_prefs(&space, muted, starred)).await
}

#[tauri::command]
pub fn set_open(state: State<'_, AppState>, space: Option<String>, thread: Option<String>) {
    state.engine.set_open(space, thread);
}

#[tauri::command]
pub async fn last_own_message(
    state: State<'_, AppState>,
    space: String,
    thread: Option<String>,
) -> Res<Option<MessageView>> {
    blocking(&state, move |e| e.last_own_message(&space, thread.as_deref())).await
}

/// Opens a link in the system browser. Only web and mail links; never files or apps.
#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> Res<()> {
    let ok = url::Url::parse(&url)
        .map(|u| matches!(u.scheme(), "https" | "http" | "mailto"))
        .unwrap_or(false);
    if !ok {
        return Err("kwek only opens web and mail links.".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// Saves an attachment to ~/Downloads and opens it.
#[tauri::command]
pub async fn download_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
    resource: String,
    name: String,
) -> Res<String> {
    let (bytes, _) = state.engine.media(&state.media_dir, &resource).await.map_err(msg)?;
    let dir = dirs_downloads().ok_or("No Downloads folder found.")?;
    let safe: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c == ':' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let safe = if safe.trim_matches('.').is_empty() {
        "attachment".to_owned()
    } else {
        safe
    };
    let mut path = dir.join(&safe);
    let mut n = 1;
    while path.exists() {
        let (stem, ext) = match safe.rsplit_once('.') {
            Some((s, e)) => (s.to_owned(), format!(".{e}")),
            None => (safe.clone(), String::new()),
        };
        path = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    tokio::fs::write(&path, bytes).await.map_err(|e| e.to_string())?;
    // Mark it as downloaded, like a browser does, so Gatekeeper checks it before it can run.
    // Without the flag, a malicious `.command` or `.app` attachment would open unchecked.
    if let Err(e) = quarantine(&path) {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(format!(
            "Could not mark the download as quarantined, so it was not saved: {e}"
        ));
    }
    let p = path.to_string_lossy().to_string();
    let _ = app.opener().reveal_item_in_dir(&path);
    Ok(p)
}

/// Sets `com.apple.quarantine` the way browsers do: flags 0081 (downloaded, not yet
/// approved), hex timestamp, agent name.
fn quarantine(path: &std::path::Path) -> std::io::Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let value = format!("0081;{now:x};kwek;");
    let status = std::process::Command::new("/usr/bin/xattr")
        .args(["-w", "com.apple.quarantine", &value])
        .arg(path)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("xattr failed"))
    }
}

fn dirs_downloads() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Downloads"))
}
