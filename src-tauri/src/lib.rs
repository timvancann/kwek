//! Tauri shell: wires kwek-core to the webview. Commands are thin calls into the engine;
//! the engine pushes changes back through [`TauriSink`] as events, after each DB commit.

mod commands;
mod notify;

use std::path::PathBuf;
use std::sync::Arc;

use kwek_core::auth::{TokenManager, default_store};
use kwek_core::chat::HttpChat;
use kwek_core::db::Db;
use kwek_core::events::EventsClient;
use kwek_core::google::Google;
use kwek_core::people::HttpPeople;
use kwek_core::sync::{Engine, Notification, Sink};
use kwek_core::views::{AuthState, MessageView, SpaceSummary, SyncStatus};
use serde_json::json;
use tauri::http::{Response, header};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};

pub struct AppState {
    pub engine: Arc<Engine>,
    pub media_dir: PathBuf,
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
        notify::show(self.app.clone(), n);
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
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
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
            let log_dir = app.path().app_log_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            std::fs::create_dir_all(&log_dir)?;
            let guard = init_logging(&log_dir);
            let media_dir = data_dir.join("media");

            let db = Db::open(&data_dir.join("kwek.db"))?;
            let http = kwek_core::http_client();
            let tokens = Arc::new(TokenManager::new(
                http.clone(),
                kwek_core::config::oauth_client().ok(),
                default_store(&data_dir),
            ));
            let g = Google::new(http.clone(), tokens.clone());
            let sink = Arc::new(TauriSink {
                app: app.handle().clone(),
            });
            let engine = Engine::new(
                db,
                Arc::new(HttpChat::new(g.clone())),
                Arc::new(HttpPeople::new(g.clone())),
                Some(EventsClient::new(g)),
                Some(tokens),
                sink,
                Some(media_dir.clone()),
            );
            tauri::async_runtime::spawn(engine.clone().run());
            app.manage(AppState {
                engine,
                media_dir,
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
            commands::set_space_prefs,
            commands::set_open,
            commands::last_own_message,
            commands::open_url,
            commands::download_attachment,
        ])
        .run(tauri::generate_context!())
        .expect("error while running kwek");
}
