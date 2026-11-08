//! macOS notifications with click-to-open. tauri-plugin-notification cannot report clicks on
//! desktop, so this talks to the system directly. A notification that waits for its click
//! parks a thread until it is clicked or dismissed, so at most a few wait at once; beyond
//! that, notifications still show but clicking only brings kwek to the front.

use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use kwek_core::sync::Notification;

const MAX_WAITING: usize = 6;
static WAITING: AtomicUsize = AtomicUsize::new(0);

pub fn show(app: AppHandle, n: Notification) {
    let wait = WAITING.fetch_add(1, Ordering::SeqCst) < MAX_WAITING;
    if !wait {
        WAITING.fetch_sub(1, Ordering::SeqCst);
    }
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
        // The user's alert sound; macOS's per-app notification settings can turn it off.
        opts.wait_for_click(wait).default_sound();
        let result = mac_notification_sys::send_notification(&n.title, None, &n.body, Some(&opts));
        if wait {
            WAITING.fetch_sub(1, Ordering::SeqCst);
        }
        match result {
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
