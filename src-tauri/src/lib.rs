//! Tauri shell: wires kwek-core to the webview. Commands are thin calls into the engine;
//! the engine pushes changes back through [`TauriSink`] as events, after each DB commit.

mod commands;
mod notify;

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use kwek_core::auth::{TokenManager, default_store};
use kwek_core::chat::HttpChat;
use kwek_core::config::{GoogleSetup, SharedSetup};
use kwek_core::db::Db;
use kwek_core::events::EventsClient;
use kwek_core::google::Google;
use kwek_core::people::HttpPeople;
use kwek_core::search::embed::{E5, Embedder};
use kwek_core::sync::{Engine, Notification, Sink};
use kwek_core::views::{AuthState, MessageView, SpaceSummary, SyncStatus};
use serde_json::json;
use tauri::http::{Response, header};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};

pub struct AppState {
    pub engine: Arc<Engine>,
    pub media_dir: PathBuf,
    pub data_dir: PathBuf,
    pub setup: SharedSetup,
    _log_guard: tracing_appender::non_blocking::WorkerGuard,
}

struct TauriSink {
    app: AppHandle,
}

impl Sink for TauriSink {
    fn spaces_changed(&self, spaces: Vec<SpaceSummary>) {
        let _ = self.app.emit("spaces:changed", spaces);
    }
    fn messages_upserted(&self, space: &str, messages: Vec<MessageView>) {
        let _ = self
            .app
            .emit("messages:upserted", json!({ "space": space, "messages": messages }));
    }
    fn messages_deleted(&self, space: &str, names: Vec<String>) {
        let _ = self
            .app
            .emit("messages:deleted", json!({ "space": space, "names": names }));
    }
    fn status(&self, status: SyncStatus) {
        let _ = self.app.emit("sync:status", status);
    }
    fn auth(&self, state: AuthState) {
        let _ = self.app.emit("auth:changed", state);
    }
    fn notify(&self, n: Notification) {
        // In front: a quiet pop-up inside kwek. Otherwise a system notification, with sound.
        let focused = self
            .app
            .get_webview_window("main")
            .and_then(|w| w.is_focused().ok())
            .unwrap_or(false);
        let payload =
            json!({ "title": n.title, "body": n.body, "space": n.space, "message": n.message, "thread": n.thread });
        if !focused || self.app.emit("notify:inapp", payload).is_err() {
            notify::show(self.app.clone(), n);
        }
    }
    fn badge(&self, count: u32) {
        if let Some(w) = self.app.get_webview_window("main") {
            let _ = w.set_badge_count(if count > 0 { Some(count as i64) } else { None });
        }
    }
}

fn init_logging(dir: &std::path::Path) -> tracing_appender::non_blocking::WorkerGuard {
    let file = tracing_appender::rolling::daily(dir, "kwek.log");
    let (writer, guard) = tracing_appender::non_blocking(file);
    // Message bodies and tokens are never logged at info; debug is opt-in via RUST_LOG.
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "kwek=info,kwek_core=info,kwek_app=info".into());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_writer(writer)
        .with_ansi(false)
        .try_init();
    guard
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // Debug builds run next to the installed app (own data folder, see setup), so only
    // release builds are single-instance.
    #[cfg(not(debug_assertions))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
    }));
    builder
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol("media", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().trim_start_matches('/').to_owned();
            tauri::async_runtime::spawn(async move {
                let state = app.state::<AppState>();
                let decode = |s: &str| percent_encoding::percent_decode_str(s).decode_utf8_lossy().to_string();
                let result = match path.strip_prefix("avatar/") {
                    Some(url) => kwek_core::media::avatar(&state.media_dir, &decode(url)).await,
                    None => state.engine.media(&state.media_dir, &decode(&path)).await,
                };
                let resp = match result {
                    Ok((bytes, mime)) => Response::builder()
                        .header(
                            header::CONTENT_TYPE,
                            mime.unwrap_or_else(|| "application/octet-stream".into()),
                        )
                        .header(header::CACHE_CONTROL, "max-age=31536000, immutable")
                        .body(bytes),
                    Err(e) => {
                        tracing::debug!("media request failed: {e}");
                        Response::builder().status(404).body(Vec::new())
                    }
                };
                responder.respond(resp.unwrap_or_else(|_| Response::new(Vec::new())));
            });
        })
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            // Debug builds keep their own database and tokens, apart from the installed app.
            #[cfg(debug_assertions)]
            let data_dir = data_dir.join("dev");
            let log_dir = app.path().app_log_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            std::fs::create_dir_all(&log_dir)?;
            let guard = init_logging(&log_dir);
            let media_dir = data_dir.join("media");

            let db = Db::open(&data_dir.join("kwek.db"))?;
            let http = kwek_core::http_client();
            // The user's Google setup (given on the sign-in screen), else the build's own.
            let setup: SharedSetup = Arc::new(RwLock::new(GoogleSetup::load(&data_dir)));
            let tokens = Arc::new(TokenManager::new(http.clone(), setup.clone(), default_store(&data_dir)));
            let g = Google::new(http.clone(), tokens.clone());
            let sink = Arc::new(TauriSink {
                app: app.handle().clone(),
            });
            // The search model ships in the bundle's resources (`just model` fetches it).
            let model_dir = app.path().resource_dir()?.join("models/multilingual-e5-small");
            let embedder: Arc<dyn Embedder> = Arc::new(E5::new(model_dir));
            if !embedder.available() {
                tracing::warn!("search model files missing; search by meaning is off");
            }
            let engine = Engine::new(
                db,
                Arc::new(HttpChat::new(g.clone())),
                Arc::new(HttpPeople::new(g.clone())),
                Some(EventsClient::new(g, setup.clone())),
                Some(tokens),
                sink,
                Some(media_dir.clone()),
                Some(embedder),
            );
            tauri::async_runtime::spawn(engine.clone().run());
            tauri::async_runtime::spawn(engine.clone().backfill());
            tauri::async_runtime::spawn(engine.clone().index());
            app.manage(AppState {
                engine,
                media_dir,
                data_dir,
                setup,
                _log_guard: guard,
            });
            tracing::info!("kwek started");
            #[cfg(debug_assertions)]
            if std::env::var_os("KWEK_DEVTOOLS").is_some()
                && let Some(w) = app.get_webview_window("main")
            {
                w.open_devtools();
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::Focused(focused) = event
                && let Some(state) = window.app_handle().try_state::<AppState>()
            {
                state.engine.set_window_focused(*focused);
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::auth_status,
            commands::google_setup,
            commands::set_google_setup,
            commands::login,
            commands::logout,
            commands::sync_status,
            commands::refresh,
            commands::list_spaces,
            commands::get_messages,
            commands::load_older,
            commands::get_thread,
            commands::send_message,
            commands::retry_send,
            commands::discard_failed,
            commands::edit_message,
            commands::delete_message,
            commands::toggle_reaction,
            commands::mark_read,
            commands::mark_thread_read,
            commands::search,
            commands::list_members,
            commands::list_senders,
            commands::set_space_prefs,
            commands::set_open,
            commands::last_own_message,
            commands::open_url,
            commands::download_attachment,
        ])
        .run(tauri::generate_context!())
        .expect("error while running kwek");
}
