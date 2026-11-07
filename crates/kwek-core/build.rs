//! Bakes the OAuth desktop client into the binary.
//!
//! Source, in order: `KWEK_OAUTH_CLIENT_ID` / `KWEK_OAUTH_CLIENT_SECRET` env vars, then
//! `secrets/oauth-client.json` at the repo root (the file the console downloads). Google
//! treats a desktop client secret as non-confidential; PKCE carries the security. The file
//! stays out of git anyway. Missing credentials compile fine and fail at login time.

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let file = root.join("secrets/oauth-client.json");
    println!("cargo:rerun-if-changed={}", file.display());
    println!("cargo:rerun-if-env-changed=KWEK_OAUTH_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=KWEK_OAUTH_CLIENT_SECRET");

    let (mut id, mut secret) = (String::new(), String::new());
    if let Ok(text) = std::fs::read_to_string(&file) {
        let json: serde_json::Value =
            serde_json::from_str(&text).expect("secrets/oauth-client.json is not valid JSON");
        let client = json
            .get("installed")
            .expect("secrets/oauth-client.json has no \"installed\" key; create a Desktop app client");
        id = client["client_id"].as_str().unwrap_or_default().to_owned();
        secret = client["client_secret"].as_str().unwrap_or_default().to_owned();
    }
    if let Ok(v) = std::env::var("KWEK_OAUTH_CLIENT_ID") {
        id = v;
    }
    if let Ok(v) = std::env::var("KWEK_OAUTH_CLIENT_SECRET") {
        secret = v;
    }
    println!("cargo:rustc-env=KWEK_OAUTH_CLIENT_ID={id}");
    println!("cargo:rustc-env=KWEK_OAUTH_CLIENT_SECRET={secret}");
}
