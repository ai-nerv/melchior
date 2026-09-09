//! Signing in to a subscription rather than exporting a key. Tokens are stored in
//! `$XDG_DATA_HOME/magi/credentials.json` at mode `0600`, one entry per provider, and never in
//! the journal, which is meant to be readable and shareable.

mod flow;

pub use flow::{Pkce, authorize_url, exchange, listen_for_code};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// What a provider gave back, and when it stops working.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    /// Sent as a bearer token.
    pub access: String,
    /// Exchanged for a new access token; absent for the providers that issue none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<String>,
    /// Unix seconds after which `access` is no longer accepted.
    pub expires_at: u64,
}

/// How long before the stated expiry a token is treated as already expired, so that one does not
/// expire while the request carrying it is in flight.
const EARLY: u64 = 60;

impl Tokens {
    /// Whether this should be renewed before it is used.
    #[must_use]
    pub fn is_stale(&self, now: u64) -> bool {
        self.expires_at <= now.saturating_add(EARLY)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    /// Keyed by provider id, which is what a config and a model name already use.
    #[serde(default)]
    pub providers: BTreeMap<String, Tokens>,
}

impl Store {
    /// Read the store, or an empty one if there is none. A corrupt file is an error rather than
    /// silently replaced, since overwriting it signs the user out of everything.
    pub fn load() -> Result<Self, Error> {
        Self::load_from(&path())
    }

    /// Read a store from a named file.
    pub fn load_from(path: &std::path::Path) -> Result<Self, Error> {
        match std::fs::read_to_string(path) {
            Ok(source) => serde_json::from_str(&source).map_err(|e| Error::Corrupt {
                path: path.to_owned(),
                detail: e.to_string(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// Write the store back, readable only by its owner.
    pub fn save(&self) -> Result<(), Error> {
        self.save_to(&path())
    }

    /// Write a store to a named file, readable only by its owner.
    pub fn save_to(&self, path: &std::path::Path) -> Result<(), Error> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        Ok(())
    }

    #[must_use]
    pub fn get(&self, provider: &str) -> Option<&Tokens> {
        self.providers.get(provider)
    }

    pub fn put(&mut self, provider: &str, tokens: Tokens) {
        self.providers.insert(provider.to_owned(), tokens);
    }

    /// Forget one provider. Returns whether there was anything to forget.
    pub fn forget(&mut self, provider: &str) -> bool {
        self.providers.remove(provider).is_some()
    }
}

/// Where credentials live, beside the sessions rather than in a config directory people copy.
#[must_use]
pub fn path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("magi").join("credentials.json")
}

/// Seconds since the epoch.
#[must_use]
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Anything that can go wrong holding a credential.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("credentials: {0}")]
    Io(#[from] std::io::Error),

    /// The store is not valid JSON; reported rather than repaired.
    #[error("{path} is not readable as credentials ({detail}); move it aside to start again")]
    Corrupt {
        path: PathBuf,
        detail: String,
    },

    #[error("credentials: {0}")]
    Encode(#[from] serde_json::Error),

    #[error("{0}")]
    Refused(String),

    #[error("not signed in to {0}; run `magi auth login {0}`")]
    NotSignedIn(String),

    /// The provider issued no refresh token, and the access token has expired.
    #[error("the session for {0} expired; run `magi auth login {0}` again")]
    Expired(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(expires_at: u64) -> Tokens {
        Tokens {
            access: "at".into(),
            refresh: Some("rt".into()),
            expires_at,
        }
    }

    #[test]
    fn a_token_is_stale_before_it_actually_expires() {
        assert!(tokens(1000).is_stale(1000 - EARLY));
        assert!(!tokens(1000).is_stale(1000 - EARLY - 1));
    }

    #[test]
    fn an_expired_token_is_stale() {
        assert!(tokens(500).is_stale(1000));
    }

    #[test]
    fn credentials_live_beside_the_sessions_not_the_config() {
        let path = path();
        assert!(
            path.ends_with("magi/credentials.json"),
            "{}",
            path.display()
        );
        assert!(
            !path.to_string_lossy().contains(".config"),
            "{}",
            path.display()
        );
    }

    #[test]
    fn a_store_round_trips() {
        let mut store = Store::default();
        store.put("anthropic", tokens(1000));
        let json = serde_json::to_string(&store).expect("encode");
        let back: Store = serde_json::from_str(&json).expect("decode");
        assert_eq!(back.get("anthropic"), Some(&tokens(1000)));
    }

    #[test]
    fn forgetting_says_whether_there_was_anything_to_forget() {
        let mut store = Store::default();
        store.put("anthropic", tokens(1000));
        assert!(store.forget("anthropic"));
        assert!(!store.forget("anthropic"));
    }

    #[test]
    fn a_provider_that_issues_no_refresh_token_is_still_storable() {
        let json = r#"{"providers":{"p":{"access":"a","expires_at":1}}}"#;
        let store: Store = serde_json::from_str(json).expect("decode");
        assert_eq!(store.get("p").expect("tokens").refresh, None);
    }
}
