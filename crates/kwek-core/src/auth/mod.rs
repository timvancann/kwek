//! Credentials. The refresh token lives in a [`TokenStore`]; access tokens live only in memory.

pub mod flow;
pub mod store;

use std::time::{Duration, Instant};

use tokio::sync::Mutex;

pub use store::{FileStore, KeychainStore, TokenStore, default_store};

use crate::config::{OAuthClient, SharedSetup, current_client};
use crate::error::{Error, Result};

struct Access {
    token: String,
    expires: Instant,
}

pub struct TokenManager {
    http: reqwest::Client,
    /// The user's Google setup; it can arrive after start-up, from the sign-in screen.
    setup: SharedSetup,
    store: Box<dyn TokenStore>,
    access: Mutex<Option<Access>>,
}

impl TokenManager {
    pub fn new(http: reqwest::Client, setup: SharedSetup, store: Box<dyn TokenStore>) -> Self {
        Self {
            http,
            setup,
            store,
            access: Mutex::new(None),
        }
    }

    fn client(&self) -> Result<OAuthClient> {
        current_client(&self.setup)
    }

    pub fn has_refresh_token(&self) -> bool {
        matches!(self.store.load(), Ok(Some(_)))
    }

    /// Browser sign-in; stores the refresh token on success.
    pub async fn login(&self, open: impl FnOnce(&str) -> Result<()>) -> Result<()> {
        let client = self.client()?;
        let tok = flow::login(&self.http, &client, open).await?;
        let refresh = tok
            .refresh_token
            .ok_or_else(|| Error::Other("Google returned no refresh token".into()))?;
        self.store.save(&refresh)?;
        *self.access.lock().await = Some(Access {
            token: tok.access_token,
            expires: Instant::now() + Duration::from_secs(tok.expires_in),
        });
        Ok(())
    }

    /// A valid access token, refreshing when it has under a minute left.
    pub async fn access_token(&self) -> Result<String> {
        let mut guard = self.access.lock().await;
        if let Some(a) = guard.as_ref()
            && a.expires > Instant::now() + Duration::from_secs(60)
        {
            return Ok(a.token.clone());
        }
        let refresh = self.store.load()?.ok_or(Error::AuthRequired)?;
        match flow::refresh(&self.http, &self.client()?, &refresh).await {
            Ok(tok) => {
                if let Some(r) = tok.refresh_token.as_deref() {
                    self.store.save(r)?;
                }
                let token = tok.access_token.clone();
                *guard = Some(Access {
                    token: tok.access_token,
                    expires: Instant::now() + Duration::from_secs(tok.expires_in),
                });
                Ok(token)
            }
            Err(Error::AuthRequired) => {
                tracing::warn!("refresh token rejected (invalid_grant); clearing it");
                self.store.clear()?;
                *guard = None;
                Err(Error::AuthRequired)
            }
            Err(e) => Err(e),
        }
    }

    /// Drops the cached access token, e.g. after a 401.
    pub async fn invalidate(&self) {
        *self.access.lock().await = None;
    }

    /// Revokes at Google (best effort) and forgets everything locally.
    pub async fn logout(&self) -> Result<()> {
        if let Ok(Some(r)) = self.store.load()
            && let Err(e) = flow::revoke(&self.http, &r).await
        {
            tracing::warn!("token revoke failed: {e}");
        }
        self.store.clear()?;
        *self.access.lock().await = None;
        Ok(())
    }
}
