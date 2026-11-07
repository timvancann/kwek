//! Where the refresh token lives. Access tokens are never persisted.

use std::path::PathBuf;

use crate::config::KEYCHAIN_SERVICE;
use crate::error::{Error, Result};

pub trait TokenStore: Send + Sync {
    fn load(&self) -> Result<Option<String>>;
    fn save(&self, refresh_token: &str) -> Result<()>;
    fn clear(&self) -> Result<()>;
}

/// macOS Keychain. Used by release builds.
pub struct KeychainStore {
    account: String,
}

impl KeychainStore {
    pub fn new() -> Self {
        Self {
            account: "refresh_token".into(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry> {
        keyring::Entry::new(KEYCHAIN_SERVICE, &self.account).map_err(|e| Error::Other(e.to_string()))
    }
}

impl Default for KeychainStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenStore for KeychainStore {
    fn load(&self) -> Result<Option<String>> {
        match self.entry()?.get_password() {
            Ok(t) => Ok(Some(t)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::Other(format!("keychain: {e}"))),
        }
    }

    fn save(&self, refresh_token: &str) -> Result<()> {
        self.entry()?
            .set_password(refresh_token)
            .map_err(|e| Error::Other(format!("keychain: {e}")))
    }

    fn clear(&self) -> Result<()> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::Other(format!("keychain: {e}"))),
        }
    }
}

/// A 0600 file. Debug builds only: the Keychain binds an item to the binary that created it,
/// so every rebuild would prompt.
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl TokenStore for FileStore {
    fn load(&self) -> Result<Option<String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(t) if !t.trim().is_empty() => Ok(Some(t.trim().to_owned())),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn save(&self, refresh_token: &str) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&self.path)?;
        f.write_all(refresh_token.as_bytes())?;
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// Keychain in release builds; in debug builds a file when `KWEK_TOKEN_STORE=file`.
pub fn default_store(data_dir: &std::path::Path) -> Box<dyn TokenStore> {
    if cfg!(debug_assertions) && std::env::var("KWEK_TOKEN_STORE").as_deref() == Ok("file") {
        Box::new(FileStore::new(data_dir.join("dev-refresh-token")))
    } else {
        Box::new(KeychainStore::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn file_store_round_trip_with_0600() {
        let dir = std::env::temp_dir().join(format!("kwek-test-{}", uuid::Uuid::new_v4()));
        let store = FileStore::new(dir.join("tok"));
        assert_eq!(store.load().unwrap(), None);
        store.save("abc").unwrap();
        assert_eq!(store.load().unwrap().as_deref(), Some("abc"));
        let mode = std::fs::metadata(dir.join("tok")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        store.clear().unwrap();
        assert_eq!(store.load().unwrap(), None);
        store.clear().unwrap();
        std::fs::remove_dir_all(dir).ok();
    }
}
