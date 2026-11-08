//! The Google side of kwek: which OAuth client signs in, and which Cloud project's Pub/Sub
//! topic carries live events. Each user brings their own (docs/setup.md): the client file
//! the console downloads is handed to kwek once and kept in its data folder. A build can
//! also carry one baked in (build.rs), which development builds use.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/chat.spaces.readonly",
    "https://www.googleapis.com/auth/chat.messages",
    "https://www.googleapis.com/auth/chat.messages.reactions",
    "https://www.googleapis.com/auth/chat.memberships.readonly",
    "https://www.googleapis.com/auth/chat.users.readstate",
    "https://www.googleapis.com/auth/pubsub",
    "https://www.googleapis.com/auth/directory.readonly",
    // people/me: who the signed-in user is (users/{id}, name, email).
    "https://www.googleapis.com/auth/userinfo.profile",
    "https://www.googleapis.com/auth/userinfo.email",
];

pub const KEYCHAIN_SERVICE: &str = "nl.timvancann.kwek";

/// The topic docs/setup.md has users create (`chat-events`), unless the build names another
/// with `KWEK_PUBSUB_TOPIC`.
pub const DEFAULT_TOPIC: &str = env!("KWEK_PUBSUB_TOPIC");

const SETUP_FILE: &str = "google-client.json";

#[derive(Clone, Debug, PartialEq)]
pub struct OAuthClient {
    pub id: String,
    pub secret: String,
}

/// Everything kwek needs from the user's Google Cloud setup.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoogleSetup {
    pub client_id: String,
    pub client_secret: String,
    pub project: String,
    pub topic: String,
}

/// The setup in use, shared by sign-in and live events; empty until the user provides one.
pub type SharedSetup = Arc<RwLock<Option<GoogleSetup>>>;

impl GoogleSetup {
    /// From the JSON the Cloud console downloads for a Desktop app client. Errors are
    /// sentences for the user: they pasted or picked the wrong file.
    pub fn from_client_json(text: &str) -> Result<Self> {
        let bad = |s: &str| Error::Other(s.to_owned());
        let json: serde_json::Value = serde_json::from_str(text.trim())
            .map_err(|_| bad("That isn't the client file: it should be the JSON Google Cloud downloads."))?;
        let Some(client) = json.get("installed") else {
            return Err(if json.get("web").is_some() {
                bad(
                    "That's a Web application client. Create a Desktop app client instead (setup guide, step 2, Clients).",
                )
            } else {
                bad("That isn't a Desktop app client file. Create one in the setup guide, step 2, Clients.")
            });
        };
        let field = |k: &str| client.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_owned();
        let setup = GoogleSetup {
            client_id: field("client_id"),
            client_secret: field("client_secret"),
            project: field("project_id"),
            topic: DEFAULT_TOPIC.to_owned(),
        };
        if !setup.complete() {
            return Err(bad(
                "The client file is missing its client ID, secret or project. Download it again from the console.",
            ));
        }
        Ok(setup)
    }

    fn complete(&self) -> bool {
        [&self.client_id, &self.client_secret, &self.project, &self.topic]
            .iter()
            .all(|s| !s.trim().is_empty())
    }

    /// The one baked in at build time, if any.
    pub fn built_in() -> Option<Self> {
        let s = GoogleSetup {
            client_id: env!("KWEK_OAUTH_CLIENT_ID").to_owned(),
            client_secret: env!("KWEK_OAUTH_CLIENT_SECRET").to_owned(),
            project: env!("KWEK_GCP_PROJECT").to_owned(),
            topic: env!("KWEK_PUBSUB_TOPIC").to_owned(),
        };
        (!s.client_id.is_empty() && !s.project.is_empty()).then_some(s)
    }

    /// The user's own setup from `dir`; with none saved, the built-in one. A saved file that
    /// can't be used gives None, not the built-in client: the stored sign-in belongs to the
    /// client the user gave, and the setup step is where they can fix it.
    pub fn load(dir: &Path) -> Option<Self> {
        match std::fs::read_to_string(path(dir)) {
            Ok(text) => match serde_json::from_str::<Self>(&text) {
                Ok(s) if s.complete() => Some(s),
                Ok(_) => {
                    tracing::warn!("{SETUP_FILE} has empty fields; asking for the client file again");
                    None
                }
                Err(e) => {
                    tracing::warn!("{SETUP_FILE} is unreadable ({e}); asking for the client file again");
                    None
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::built_in(),
            Err(e) => {
                tracing::warn!("cannot read {SETUP_FILE}: {e}");
                None
            }
        }
    }

    /// Kept beside the database, readable by the user only. The client secret of a desktop
    /// client is not confidential (Google's words; PKCE protects sign-in), but there is no
    /// reason to share it either. Written to a private temp file, flushed, then renamed, so
    /// a crash never leaves a half-written or briefly readable file.
    pub fn save(&self, dir: &Path) -> Result<()> {
        let p = path(dir);
        let tmp = p.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self).map_err(|e| Error::Other(e.to_string()))?;
        let io = |e: std::io::Error| Error::Other(format!("could not save the Google setup: {e}"));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(io)?;
        f.write_all(text.as_bytes()).map_err(io)?;
        f.sync_all().map_err(io)?;
        std::fs::rename(&tmp, &p).map_err(io)
    }

    pub fn client(&self) -> OAuthClient {
        OAuthClient {
            id: self.client_id.clone(),
            secret: self.client_secret.clone(),
        }
    }

    pub fn topic_path(&self) -> String {
        format!("projects/{}/topics/{}", self.project, self.topic)
    }

    /// This device's pull subscription on the topic.
    pub fn subscription(&self, device_id: &str) -> String {
        format!("projects/{}/subscriptions/chat-events-{device_id}", self.project)
    }
}

fn path(dir: &Path) -> PathBuf {
    dir.join(SETUP_FILE)
}

/// The client in use, or the error that sends the user to the setup step.
pub fn current_client(setup: &SharedSetup) -> Result<OAuthClient> {
    current(setup)
        .as_ref()
        .map(GoogleSetup::client)
        .ok_or(Error::NoOAuthClient)
}

/// The setup in use. The lock only guards plain data, so a poisoned one is still good.
pub fn current(setup: &SharedSetup) -> Option<GoogleSetup> {
    setup.read().unwrap_or_else(PoisonError::into_inner).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOP: &str = r#"{"installed":{"client_id":"123-abc.apps.googleusercontent.com","project_id":"acme-chat","auth_uri":"https://accounts.google.com/o/oauth2/auth","token_uri":"https://oauth2.googleapis.com/token","client_secret":"s3cret","redirect_uris":["http://localhost"]}}"#;

    #[test]
    fn reads_the_console_file() {
        let s = GoogleSetup::from_client_json(&format!("\n{DESKTOP}\n")).unwrap();
        assert_eq!(s.client_id, "123-abc.apps.googleusercontent.com");
        assert_eq!(s.client_secret, "s3cret");
        assert_eq!(s.project, "acme-chat");
        assert_eq!(s.topic_path(), "projects/acme-chat/topics/chat-events");
        assert_eq!(s.subscription("d1"), "projects/acme-chat/subscriptions/chat-events-d1");
    }

    #[test]
    fn explains_the_wrong_file() {
        let err = |t: &str| GoogleSetup::from_client_json(t).unwrap_err().to_string();
        assert!(err("not json").contains("isn't the client file"));
        assert!(err(r#"{"web":{"client_id":"x"}}"#).contains("Web application client"));
        assert!(err(r#"{"type":"service_account"}"#).contains("Desktop app client"));
        assert!(err(r#"{"installed":{"client_id":"x","client_secret":"y"}}"#).contains("missing"));
    }

    #[test]
    fn saved_setup_wins_and_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("kwek-setup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            GoogleSetup::load(&dir),
            GoogleSetup::built_in(),
            "nothing saved: the built-in one"
        );
        let s = GoogleSetup::from_client_json(DESKTOP).unwrap();
        s.save(&dir).unwrap();
        assert_eq!(GoogleSetup::load(&dir), Some(s.clone()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path(&dir)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Saving again replaces it.
        let mut other = s.clone();
        other.project = "other-project".into();
        other.save(&dir).unwrap();
        assert_eq!(GoogleSetup::load(&dir), Some(other));
        // A broken or incomplete file asks again rather than borrowing the built-in client.
        std::fs::write(path(&dir), "garbage").unwrap();
        assert_eq!(GoogleSetup::load(&dir), None);
        std::fs::write(
            path(&dir),
            r#"{"client_id":"","client_secret":"s","project":"p","topic":"t"}"#,
        )
        .unwrap();
        assert_eq!(GoogleSetup::load(&dir), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_setup_means_no_client() {
        let shared: SharedSetup = Arc::new(RwLock::new(None));
        assert!(matches!(current_client(&shared), Err(Error::NoOAuthClient)));
        *shared.write().unwrap() = Some(GoogleSetup::from_client_json(DESKTOP).unwrap());
        assert_eq!(
            current_client(&shared).unwrap().id,
            "123-abc.apps.googleusercontent.com"
        );
    }
}
