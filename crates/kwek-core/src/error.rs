use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    /// No usable refresh token: the user has to sign in (again).
    #[error("sign-in required")]
    AuthRequired,

    /// No Google setup yet: the user has not given kwek their client file, and the build
    /// carries none.
    #[error("no Google client configured: give kwek your desktop client file (docs/setup.md)")]
    NoOAuthClient,

    /// Google answered with a non-success status. `status_code` is Google's
    /// canonical error status (`NOT_FOUND`, `ALREADY_EXISTS`, ...) when present.
    #[error("Google API {status}: {status_code:?} {message}")]
    Api {
        status: u16,
        status_code: Option<String>,
        message: String,
    },

    /// Transport failure: offline, DNS, TLS, timeout.
    #[error("network: {0}")]
    Network(String),

    /// A long-poll that ended without an answer (normal for a quiet Pub/Sub pull).
    #[error("timed out")]
    Timeout,

    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("decode: {0}")]
    Decode(String),

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn is_status(&self, code: &str) -> bool {
        matches!(self, Error::Api { status_code: Some(c), .. } if c == code)
    }

    pub fn http_status(&self) -> Option<u16> {
        match self {
            Error::Api { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn is_offline(&self) -> bool {
        matches!(self, Error::Network(_))
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Decode(e.to_string())
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        // Never include the URL: it can carry tokens in query strings.
        Error::Network(e.without_url().to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Other(e.to_string())
    }
}
