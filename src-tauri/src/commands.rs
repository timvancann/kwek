//! IPC commands: request/response against local state. Errors cross as plain sentences the
//! UI can show in a toast.

use std::sync::PoisonError;

use kwek_core::Error;
use kwek_core::calendar::busy_line;
use kwek_core::config::GoogleSetup;
use kwek_core::presence::{Choice, NewCustom};
use kwek_core::sync::{Engine, Outgoing};
use kwek_core::ts;
use kwek_core::views::{
    AuthState, CalendarView, GoogleSetupView, ListedMessage, MessageView, OwnPresenceView, PresenceView, ProfileView,
    ScheduledView, SearchResult, SpaceSummary, SyncStatus, UserRef,
};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::AppState;

type Res<T> = Result<T, String>;

fn msg(e: Error) -> String {
    // The real response always goes to the log; the toast gets a sentence.
    tracing::warn!("command failed: {e}");
    match e {
        Error::AuthRequired => "Sign in to Google again to continue.".into(),
        Error::NoOAuthClient => "Give kwek your Google client file first.".into(),
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

fn setup_view(s: &GoogleSetup) -> GoogleSetupView {
    GoogleSetupView {
        project: s.project.clone(),
        built_in: GoogleSetup::built_in().as_ref() == Some(s),
    }
}

#[tauri::command]
pub fn google_setup(state: State<'_, AppState>) -> Option<GoogleSetupView> {
    state
        .setup
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(setup_view)
}

/// `json` is the client file the Cloud console downloads. While signed out, or while signed
/// in without any setup (the session is stuck until it gets one); not to switch the project
/// under a working session, which belongs to the client it signed in with.
#[tauri::command]
pub async fn set_google_setup(state: State<'_, AppState>, json: String) -> Res<GoogleSetupView> {
    let (shared, dir) = (state.setup.clone(), state.data_dir.clone());
    blocking(&state, move |e| {
        let missing = shared.read().unwrap_or_else(PoisonError::into_inner).is_none();
        if !missing && !matches!(e.auth_state(), AuthState::SignedOut) {
            return Err(Error::Other("Sign out first to switch Google projects.".into()));
        }
        let setup = GoogleSetup::from_client_json(&json)?;
        setup.save(&dir)?;
        let view = setup_view(&setup);
        tracing::info!(project = %setup.project, "Google setup saved");
        *shared.write().unwrap_or_else(PoisonError::into_inner) = Some(setup);
        // A session waiting for a client goes on now.
        e.poke();
        Ok(view)
    })
    .await
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
        .map_err(msg)?;
    state.agenda.poke();
    state.presence.poke();
    Ok(())
}

#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> Res<()> {
    // While the sign-in still works: the device's presence subscription goes too.
    state.presence.leave().await;
    state.engine.logout().await.map_err(msg)?;
    state.agenda.poke();
    Ok(())
}

#[tauri::command]
pub fn calendar(state: State<'_, AppState>) -> CalendarView {
    state.agenda.view()
}

/// Signs in again to grant what the sign-in lacks (calendar, status, free/busy); the chat
/// session carries on meanwhile.
#[tauri::command]
pub async fn connect_calendar(app: AppHandle, state: State<'_, AppState>) -> Res<()> {
    state
        .engine
        .reauthorize(|url| {
            app.opener()
                .open_url(url, None::<&str>)
                .map_err(|e| Error::Other(format!("could not open the browser: {e}")))
        })
        .await
        .map_err(msg)?;
    state.agenda.poke();
    state.presence.poke();
    Ok(())
}

/// Everything a profile card shows. `end_of_day_us` is local midnight, from the webview,
/// which knows the time zone. Parts that can't be had are left out, never an error.
#[tauri::command]
pub async fn profile(state: State<'_, AppState>, user: String, end_of_day_us: i64) -> Res<ProfileView> {
    let u = user.clone();
    let (person, dm_space, shared_spaces) = blocking(&state, move |e| e.profile_basics(&u)).await?;
    let now = ts::now();
    let busy = async {
        let Some(email) = &person.email else {
            return (None, "unavailable");
        };
        match state.agenda.busy(email, now, end_of_day_us.max(now + ts::MINUTE)).await {
            Ok(blocks) => (Some(busy_line(&blocks, now, end_of_day_us)), "ok"),
            Err(e) if e.http_status() == Some(403) => (None, "needsPermission"),
            Err(e) => {
                tracing::debug!("no free/busy for the card: {e}");
                (None, "unavailable")
            }
        }
    };
    let (role, (busy, calendar)) = tokio::join!(state.engine.role(&user), busy);
    let role = role.unwrap_or_else(|e| {
        tracing::debug!("no role for the card: {e}");
        Default::default()
    });
    let next_meeting = person.email.as_deref().and_then(|m| state.agenda.next_with(m, now));
    Ok(ProfileView {
        user: person,
        title: role.title,
        department: role.department,
        busy,
        calendar: calendar.into(),
        next_meeting,
        shared_spaces,
        dm_space,
    })
}

/// Colleagues' presence as last heard.
#[tauri::command]
pub fn presence(state: State<'_, AppState>) -> Vec<PresenceView> {
    state.presence.all()
}

#[tauri::command]
pub fn own_presence(state: State<'_, AppState>) -> OwnPresenceView {
    state.presence.own()
}

/// Your status from the picker: the custom status (None clears it) and availability.
#[tauri::command]
pub async fn set_status(state: State<'_, AppState>, custom: Option<NewCustom>, choice: Choice) -> Res<OwnPresenceView> {
    if let Some(c) = &custom
        && (c.text.trim().is_empty() || c.text.chars().count() > 64)
    {
        return Err("A status needs some text, up to 64 characters.".into());
    }
    state.presence.set_status(custom, choice).await.map_err(msg)?;
    Ok(state.presence.own())
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

/// Well under Google Chat's 200 MB: a file is read into memory to upload, and a big one on a
/// slow connection would take minutes.
const MAX_ATTACHMENT: usize = 50 * 1024 * 1024;

/// Keeps a file the user added to the composer in the outbox folder until it is sent.
/// The body is the raw file; `x-name` (percent-encoded) and `x-type` describe it. Returns an
/// id for `send_message`: the webview never names a path to upload.
#[tauri::command]
pub async fn stage_attachment(state: State<'_, AppState>, request: tauri::ipc::Request<'_>) -> Res<String> {
    let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
        return Err("Expected the file's bytes.".into());
    };
    let header = |k: &str| {
        request
            .headers()
            .get(k)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned()
    };
    let name = percent_encoding::percent_decode_str(&header("x-name"))
        .decode_utf8_lossy()
        .trim()
        .to_owned();
    // The webview leaves the type empty for extensions it doesn't know.
    let content_type = match header("x-type") {
        t if t.is_empty() => "application/octet-stream".to_owned(),
        t => t,
    };
    if data.len() > MAX_ATTACHMENT {
        return Err("That file is over 50 MB, too big to send from kwek.".into());
    }
    let name = if name.is_empty() {
        if content_type.starts_with("video/") {
            "video"
        } else if content_type.starts_with("image/") {
            "image"
        } else {
            "file"
        }
        .to_owned()
    } else {
        name
    };
    let id = uuid::Uuid::new_v4().simple().to_string();
    let dir = state.data_dir.join("outbox");
    let path = dir.join(&id);
    let data = data.clone();
    let p = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&p, data)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("Could not keep the file for sending: {e}"))?;
    state.staged.lock().unwrap().insert(
        id.clone(),
        Outgoing {
            path,
            name,
            content_type,
        },
    );
    Ok(id)
}

/// The user took a file out of the composer.
#[tauri::command]
pub fn unstage_attachment(state: State<'_, AppState>, id: String) {
    if let Some(f) = state.staged.lock().unwrap().remove(&id) {
        let _ = std::fs::remove_file(f.path);
    }
}

#[tauri::command]
pub async fn send_message(
    state: State<'_, AppState>,
    space: String,
    thread: Option<String>,
    text: String,
    attachments: Option<Vec<String>>,
) -> Res<MessageView> {
    let text = text.trim_end().to_owned();
    let ids = attachments.unwrap_or_default();
    if text.trim().is_empty() && ids.is_empty() {
        return Err("Nothing to send.".into());
    }
    let files = take_staged(&state, &ids)?;
    match state
        .engine
        .send_message(&space, thread.as_deref(), &text, files.clone())
        .await
    {
        Ok(v) => Ok(v),
        Err(e) => {
            // Nothing was queued, and the composer has already let go of the files: the
            // staged copies have no one left to use them.
            for f in files {
                let _ = std::fs::remove_file(f.path);
            }
            Err(msg(e))
        }
    }
}

/// Takes staged ids out of the staging map, all or nothing.
fn take_staged(state: &State<'_, AppState>, ids: &[String]) -> Res<Vec<Outgoing>> {
    let mut staged = state.staged.lock().unwrap();
    if ids.iter().any(|id| !staged.contains_key(id)) {
        return Err("An attachment is no longer there. Add it again.".into());
    }
    Ok(ids.iter().filter_map(|id| staged.remove(id)).collect())
}

/// Keeps a message to send at `at_us`; kwek sends it then, or as soon as it runs again.
#[tauri::command]
pub async fn schedule_message(
    state: State<'_, AppState>,
    space: String,
    thread: Option<String>,
    text: String,
    attachments: Option<Vec<String>>,
    at_us: i64,
) -> Res<()> {
    let text = text.trim_end().to_owned();
    let ids = attachments.unwrap_or_default();
    if text.trim().is_empty() && ids.is_empty() {
        return Err("Nothing to send.".into());
    }
    if at_us <= ts::now() {
        return Err("Pick a time that's still to come.".into());
    }
    let files = take_staged(&state, &ids)?;
    let result = state
        .engine
        .schedule(&space, thread.as_deref(), &text, files.clone(), at_us)
        .await;
    if let Err(e) = result {
        for f in files {
            let _ = std::fs::remove_file(f.path);
        }
        return Err(msg(e));
    }
    Ok(())
}

#[tauri::command]
pub async fn scheduled_messages(state: State<'_, AppState>) -> Res<Vec<ScheduledView>> {
    blocking(&state, |e| e.scheduled()).await
}

#[tauri::command]
pub async fn cancel_scheduled(state: State<'_, AppState>, id: i64) -> Res<()> {
    state.engine.cancel_scheduled(id).await.map_err(msg)
}

#[tauri::command]
pub async fn send_scheduled_now(state: State<'_, AppState>, id: i64) -> Res<()> {
    state.engine.send_scheduled_now(id).await.map_err(msg)
}

#[tauri::command]
pub async fn set_saved(state: State<'_, AppState>, message: String, saved: bool) -> Res<()> {
    state.engine.set_saved(&message, saved).await.map_err(msg)
}

/// `at_us: None` clears the reminder.
#[tauri::command]
pub async fn set_reminder(state: State<'_, AppState>, message: String, at_us: Option<i64>) -> Res<()> {
    if at_us.is_some_and(|a| a <= ts::now()) {
        return Err("Pick a time that's still to come.".into());
    }
    state.engine.set_reminder(&message, at_us).await.map_err(msg)
}

/// `kind`: `activity`, `saved` or `reminders`.
#[tauri::command]
pub async fn listed_messages(state: State<'_, AppState>, kind: String) -> Res<Vec<ListedMessage>> {
    blocking(&state, move |e| match kind.as_str() {
        "activity" => e.activity(),
        "saved" => e.saved(),
        "reminders" => e.reminders(),
        other => Err(Error::Other(format!("no such list: {other}"))),
    })
    .await
}

/// The composer's text rendered as it will look once sent.
#[tauri::command]
pub async fn preview_message(state: State<'_, AppState>, text: String) -> Res<Vec<kwek_core::render::Block>> {
    blocking(&state, move |e| e.preview(&text)).await
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
pub async fn list_senders(state: State<'_, AppState>) -> Res<Vec<UserRef>> {
    blocking(&state, move |e| e.list_senders()).await
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
    hidden: Option<bool>,
) -> Res<()> {
    blocking(&state, move |e| e.set_space_prefs(&space, muted, starred, hidden)).await
}

/// The user's own order for the Spaces section, top first.
#[tauri::command]
pub async fn set_space_order(state: State<'_, AppState>, names: Vec<String>) -> Res<()> {
    blocking(&state, move |e| e.set_space_order(&names)).await
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

/// Installs the update the last check found and restarts kwek; only returns on failure.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Res<()> {
    #[cfg(desktop)]
    {
        if !crate::update::ENABLED {
            return Err("Builds from source don't update themselves.".into());
        }
        crate::update::install(&app).await
    }
    #[cfg(mobile)]
    {
        let _ = app;
        Err("kwek doesn't update itself on this device.".into())
    }
}

/// Saves an attachment to ~/Downloads and opens it.
#[tauri::command]
pub async fn download_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
    resource: String,
    name: String,
) -> Res<String> {
    let (cached, _) = state
        .engine
        .media_file(&state.media_dir, &resource)
        .await
        .map_err(msg)?;
    let dir = app.path().download_dir().map_err(|_| "No Downloads folder found.")?;
    let safe = safe_file_name(&name);
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
    tokio::fs::copy(&cached, &path).await.map_err(|e| e.to_string())?;
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

/// A sender's file name made safe to save on any system. Windows drops trailing dots and
/// spaces and refuses some characters and device names; a name it rewrites would get its
/// Mark of the Web on a different path than the file, so those never reach the disk. The
/// same rule on every system, so a file saves under the same name everywhere.
fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if "/\\:<>\"|?*".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim_end_matches(['.', ' ']).trim_start();
    if cleaned.trim_matches('.').is_empty() {
        return "attachment".to_owned();
    }
    let stem = cleaned.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || (stem.chars().count() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.chars().last().is_some_and(|c| c.is_ascii_digit() || "¹²³".contains(c)));
    if reserved { format!("_{cleaned}") } else { cleaned.to_owned() }
}

/// Sets `com.apple.quarantine` the way browsers do: flags 0081 (downloaded, not yet
/// approved), hex timestamp, agent name.
#[cfg(target_os = "macos")]
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

/// The Mark of the Web, as browsers write it: zone 3 (internet), so SmartScreen and Office's
/// Protected View check the file before it runs or opens.
#[cfg(windows)]
fn quarantine(path: &std::path::Path) -> std::io::Result<()> {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    std::fs::write(stream, "[ZoneTransfer]\r\nZoneId=3\r\n")
}

/// Nothing comparable on Android or Linux.
#[cfg(not(any(target_os = "macos", windows)))]
fn quarantine(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::safe_file_name;

    #[test]
    fn file_names_windows_would_rewrite_are_cleaned() {
        assert_eq!(safe_file_name("evil.exe."), "evil.exe");
        assert_eq!(safe_file_name("evil.exe . ."), "evil.exe");
        assert_eq!(safe_file_name("a<b>c|d?e*f\"g.txt"), "a_b_c_d_e_f_g.txt");
        assert_eq!(safe_file_name("../../x"), ".._.._x");
        assert_eq!(safe_file_name("CON.txt"), "_CON.txt");
        assert_eq!(safe_file_name("com1"), "_com1");
        assert_eq!(safe_file_name("LPT².log"), "_LPT².log");
        assert_eq!(safe_file_name("COMET.txt"), "COMET.txt");
        assert_eq!(safe_file_name("..."), "attachment");
        assert_eq!(safe_file_name("report 2026.pdf"), "report 2026.pdf");
    }
}
