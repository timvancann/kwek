use crate::error::{Error, Result};

/// GCP project that owns the Pub/Sub topic and the OAuth client (see docs/setup.md).
pub const GCP_PROJECT: &str = "your-project";
pub const PUBSUB_TOPIC: &str = "projects/your-project/topics/chat-events";

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

#[derive(Clone, Debug)]
pub struct OAuthClient {
    pub id: String,
    pub secret: String,
}

pub fn oauth_client() -> Result<OAuthClient> {
    let id = env!("KWEK_OAUTH_CLIENT_ID");
    let secret = env!("KWEK_OAUTH_CLIENT_SECRET");
    if id.is_empty() {
        return Err(Error::NoOAuthClient);
    }
    Ok(OAuthClient {
        id: id.to_owned(),
        secret: secret.to_owned(),
    })
}
