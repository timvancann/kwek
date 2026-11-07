//! macOS notifications with click-to-open. tauri-plugin-notification cannot report clicks on
//! desktop, so this talks to the system directly. Each notification waits for its response
//! on a plain thread; clicking it focuses the window and asks the UI to open the message.

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use kwek_core::sync::Notification;

pub fn show(app: AppHandle, n: Notification) {
    std::thread::spawn(move || {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            // Inside the .app bundle notifications belong to kwek; in `tauri dev` there is no
            // bundle, and macOS attributes them to the default app instead.
            if std::env::current_exe().is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS")) {
                let _ = mac_notification_sys::set_application("nl.timvancann.kwek");
            }
        });
        let mut opts = mac_notification_sys::Notification::new();
        opts.wait_for_click(true);
        match mac_notification_sys::send_notification(&n.title, None, &n.body, Some(&opts)) {
            Ok(mac_notification_sys::NotificationResponse::Click)
            | Ok(mac_notification_sys::NotificationResponse::ActionButton(_)) => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                let _ = app.emit(
                    "nav:open",
                    json!({ "space": n.space, "message": n.message, "thread": n.thread }),
                );
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("notification failed: {e}"),
        }
    });
}
