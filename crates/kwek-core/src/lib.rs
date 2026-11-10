//! kwek core: everything that is not UI. Owns all I/O, credentials and state; the Tauri
//! shell is a thin adapter over this crate.

pub mod auth;
pub mod calendar;
pub mod chat;
pub mod config;
pub mod db;
pub mod error;
pub mod events;
pub mod google;
pub mod media;
pub mod people;
pub mod presence;
pub mod render;
pub mod search;
pub mod sync;
pub mod ts;
pub mod views;

pub use error::{Error, Result};
pub use reqwest;

/// Default data directory, used by the spike. The app uses Tauri's app data dir, which
/// resolves to the same place for the bundle id `nl.timvancann.kwek`.
pub fn default_data_dir() -> std::path::PathBuf {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    home.join("Library/Application Support/nl.timvancann.kwek")
}

/// One HTTP client for the whole process.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("kwek/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        // Pub/Sub pull long-polls; give it room.
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .expect("http client")
}
