//! Optionally bakes a Google Cloud setup into the binary: the OAuth desktop client, the
//! project it lives in, and the Pub/Sub topic for live events (see docs/setup.md). Release
//! builds carry none and ask the user for their client file on first start; a baked one
//! skips that step, which suits development.
//!
//! Source, in order: `KWEK_OAUTH_CLIENT_ID` / `KWEK_OAUTH_CLIENT_SECRET` / `KWEK_GCP_PROJECT`
//! env vars, then `secrets/oauth-client.json` at the repo root (the file the console
//! downloads; it names its project). The topic is `KWEK_PUBSUB_TOPIC`, default `chat-events`.
//! Google treats a desktop client secret as non-confidential; PKCE carries the security. The
//! file stays out of git anyway.

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let file = root.join("secrets/oauth-client.json");
    println!("cargo:rerun-if-changed={}", file.display());
    println!("cargo:rerun-if-env-changed=KWEK_OAUTH_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=KWEK_OAUTH_CLIENT_SECRET");
    println!("cargo:rerun-if-env-changed=KWEK_GCP_PROJECT");
    println!("cargo:rerun-if-env-changed=KWEK_PUBSUB_TOPIC");
    println!("cargo:rerun-if-env-changed=KWEK_PRESENCE_TOPIC");

    let (mut id, mut secret, mut project) = (String::new(), String::new(), String::new());
    if let Ok(text) = std::fs::read_to_string(&file) {
        let json: serde_json::Value = serde_json::from_str(&text).expect("secrets/oauth-client.json is not valid JSON");
        let client = json
            .get("installed")
            .expect("secrets/oauth-client.json has no \"installed\" key; create a Desktop app client");
        id = client["client_id"].as_str().unwrap_or_default().to_owned();
        secret = client["client_secret"].as_str().unwrap_or_default().to_owned();
        project = client["project_id"].as_str().unwrap_or_default().to_owned();
    }
    if let Ok(v) = std::env::var("KWEK_OAUTH_CLIENT_ID") {
        id = v;
    }
    if let Ok(v) = std::env::var("KWEK_OAUTH_CLIENT_SECRET") {
        secret = v;
    }
    if let Ok(v) = std::env::var("KWEK_GCP_PROJECT") {
        project = v;
    }
    let topic = std::env::var("KWEK_PUBSUB_TOPIC").unwrap_or_else(|_| "chat-events".into());
    println!("cargo:rustc-env=KWEK_OAUTH_CLIENT_ID={id}");
    println!("cargo:rustc-env=KWEK_OAUTH_CLIENT_SECRET={secret}");
    println!("cargo:rustc-env=KWEK_GCP_PROJECT={project}");
    println!("cargo:rustc-env=KWEK_PUBSUB_TOPIC={topic}");
}
