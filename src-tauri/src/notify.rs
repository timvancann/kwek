//! System notifications with click-to-open, on macOS and Windows. tauri-plugin-notification
//! cannot report clicks on desktop, so this talks to the system directly.
//!
//! On macOS a notification that waits for its click parks a thread until it is clicked or
//! dismissed, so at most a few wait at once; beyond that, notifications still show but
//! clicking only brings kwek to the front. On Windows a toast reports its click through a
//! callback, so every one can wait.

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(any(target_os = "macos", windows))]
use serde_json::json;
use tauri::AppHandle;
#[cfg(any(target_os = "macos", windows))]
use tauri::{Emitter, Manager};

use kwek_core::sync::Notification;

#[cfg(target_os = "macos")]
const MAX_WAITING: usize = 6;
#[cfg(target_os = "macos")]
static WAITING: AtomicUsize = AtomicUsize::new(0);

/// Not built yet elsewhere (Android would use tauri-plugin-notification): only the in-app
/// pop-up shows.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn show(_app: AppHandle, n: Notification) {
    tracing::debug!("system notification skipped on this platform: {}", n.title);
}

/// A call or meeting has a Join button, and clicking anywhere joins too.
#[cfg(any(target_os = "macos", windows))]
fn join_url(n: &Notification) -> Option<String> {
    n.url.clone().filter(|u| u.starts_with("https://"))
}

#[cfg(any(target_os = "macos", windows))]
fn open_meeting(app: &AppHandle, url: &str) {
    use tauri_plugin_opener::OpenerExt;
    if let Err(e) = app.opener().open_url(url, None::<&str>) {
        tracing::warn!("could not open the meeting: {e}");
    }
}

/// Brings kwek to the front at the message the notification was about.
#[cfg(any(target_os = "macos", windows))]
fn open_message(app: &AppHandle, n: &Notification) {
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

#[cfg(windows)]
pub fn show(app: AppHandle, n: Notification) {
    use tauri_winrt_notification::{Sound, Toast};
    // Windows shows a toast only for an app id that a Start menu shortcut carries. The
    // installer gives kwek's shortcut its identifier; a debug build has no shortcut, so its
    // toasts borrow PowerShell's.
    let app_id = if cfg!(debug_assertions) {
        Toast::POWERSHELL_APP_ID.to_owned()
    } else {
        app.config().identifier.clone()
    };
    // Off the async runtime: showing a toast is a blocking COM call.
    std::thread::spawn(move || {
        let join = join_url(&n);
        let mut toast = Toast::new(&app_id)
            .title(&n.title)
            .text1(&n.body)
            .sound(Some(Sound::Default));
        if join.is_some() {
            toast = toast.add_button("Join", "join");
        }
        let toast = toast.on_activated(move |_| {
            match &join {
                Some(url) => open_meeting(&app, url),
                None => open_message(&app, &n),
            }
            Ok(())
        });
        if let Err(e) = toast.show() {
            tracing::warn!("notification failed: {e}");
        }
    });
}

#[cfg(target_os = "macos")]
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
        // Only when kwek can wait for the click; otherwise the Join button would lead nowhere.
        let join = join_url(&n);
        if join.is_some() && wait {
            opts.main_button(mac_notification_sys::MainButton::SingleAction("Join"));
        }
        let result = mac_notification_sys::send_notification(&n.title, None, &n.body, Some(&opts));
        if wait {
            WAITING.fetch_sub(1, Ordering::SeqCst);
        }
        match result {
            Ok(mac_notification_sys::NotificationResponse::Click)
            | Ok(mac_notification_sys::NotificationResponse::ActionButton(_)) => match &join {
                Some(url) => open_meeting(&app, url),
                None => open_message(&app, &n),
            },
            Ok(_) => {}
            Err(e) => tracing::warn!("notification failed: {e}"),
        }
    });
}
