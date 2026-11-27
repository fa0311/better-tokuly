//! The bots' API tokens, kept between starts.
//!
//! An operator may create only a few tokens an hour, so a start that made a
//! new token for every bot would lock the simulator out after a few restarts.
//! The secrets are kept in one file in the state directory instead, sealed
//! with a key derived from the operators' password (XChaCha20-Poly1305): the
//! file alone gives nobody a token. A file that does not open (another
//! password, damaged) counts as empty, and the tokens are made again.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::Context as _;
use chacha20poly1305::{KeyInit as _, XChaCha20Poly1305, XNonce, aead::Aead as _};
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "demo-tokens.sealed";

/// A token the simulator made for a bot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeptToken {
    pub id: String,
    pub secret: String,
}

/// The tokens of the last start, by bot ID.
pub struct Keep {
    /// `None`: nowhere to keep them, and every start makes new ones.
    path: Option<PathBuf>,
    key: [u8; 32],
    tokens: BTreeMap<String, KeptToken>,
}

impl Keep {
    /// The tokens kept in `dir`, sealed under `password`.
    pub fn open(dir: Option<&Path>, password: &str) -> Self {
        let key = key(password);
        let path = dir.map(|dir| dir.join(FILE_NAME));
        let tokens = match &path {
            None => {
                tracing::warn!("demo: no state directory (TOKULY_DEMO_STATE_DIR): every start makes new bot tokens");
                BTreeMap::new()
            }
            Some(path) => match std::fs::read(path) {
                Ok(sealed) => unseal(&key, &sealed).unwrap_or_else(|| {
                    tracing::warn!(path = %path.display(), "demo: the kept bot tokens do not open; making new ones");
                    BTreeMap::new()
                }),
                Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "demo: cannot read the kept bot tokens");
                    BTreeMap::new()
                }
            },
        };
        Self { path, key, tokens }
    }

    pub fn get(&self, bot_id: &str) -> Option<&KeptToken> {
        self.tokens.get(bot_id)
    }

    /// Keeps `token` for `bot_id`, on disk at once: a start that fails
    /// half-way still keeps the tokens it made.
    pub fn put(&mut self, bot_id: &str, token: KeptToken) -> anyhow::Result<()> {
        self.tokens.insert(bot_id.to_owned(), token);
        let Some(path) = &self.path else { return Ok(()) };
        save(path, &seal(&self.key, &self.tokens))
            .with_context(|| format!("keeping the bot tokens in {}", path.display()))
    }
}

/// Where the tokens are kept unless `TOKULY_DEMO_STATE_DIR` says: the user's state directory.
pub fn default_dir() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME").filter(|dir| !dir.is_empty()).map(PathBuf::from).or_else(|| {
        std::env::var_os("HOME").filter(|dir| !dir.is_empty()).map(|home| Path::new(&home).join(".local/state"))
    })?;
    Some(state.join("tokuly-demo"))
}

fn key(password: &str) -> [u8; 32] {
    let mut key = [0u8; 32];
    hkdf::Hkdf::<sha2::Sha256>::new(Some(b"tokuly-demo"), password.as_bytes())
        .expand(b"bot tokens", &mut key)
        .expect("32 bytes is a valid HKDF output length");
    key
}

/// 24-byte nonce, then the ciphertext of the tokens as JSON.
fn seal(key: &[u8; 32], tokens: &BTreeMap<String, KeptToken>) -> Vec<u8> {
    use rand::RngExt as _;
    let mut nonce = [0u8; 24];
    rand::rng().fill(&mut nonce[..]);
    let plain = serde_json::to_vec(tokens).expect("the tokens are JSON");
    let mut out = nonce.to_vec();
    out.extend(
        XChaCha20Poly1305::new(key.into()).encrypt(&XNonce::from(nonce), &plain[..]).expect("encryption cannot fail"),
    );
    out
}

fn unseal(key: &[u8; 32], sealed: &[u8]) -> Option<BTreeMap<String, KeptToken>> {
    let (nonce, ciphertext) = sealed.split_at_checked(24)?;
    let nonce: [u8; 24] = nonce.try_into().ok()?;
    let plain = XChaCha20Poly1305::new(key.into()).decrypt(&XNonce::from(nonce), ciphertext).ok()?;
    serde_json::from_slice(&plain).ok()
}

/// Writes the file whole or not at all, readable by its owner only.
fn save(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let partial = path.with_extension("partial");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&partial)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&partial, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(n: u8) -> KeptToken {
        KeptToken { id: format!("id-{n}"), secret: format!("tky_bot_{n}") }
    }

    #[test]
    fn what_is_kept_opens_with_the_same_password_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut keep = Keep::open(Some(dir.path()), "the operators' password");
        keep.put("bot-1", token(1)).unwrap();
        keep.put("bot-2", token(2)).unwrap();

        let again = Keep::open(Some(dir.path()), "the operators' password");
        assert_eq!(again.get("bot-1"), Some(&token(1)));
        assert_eq!(again.get("bot-2"), Some(&token(2)));
        // Sealed: the secrets are not in the file.
        let file = std::fs::read(dir.path().join(FILE_NAME)).unwrap();
        assert!(!file.windows(8).any(|part| part == b"tky_bot_"));
        // Another password, or a damaged file: nothing is kept, and new tokens are made.
        assert_eq!(Keep::open(Some(dir.path()), "another password, long enough").get("bot-1"), None);
        std::fs::write(dir.path().join(FILE_NAME), &file[..20]).unwrap();
        assert_eq!(Keep::open(Some(dir.path()), "the operators' password").get("bot-1"), None);
    }

    #[cfg(unix)]
    #[test]
    fn only_the_owner_reads_the_file() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("not yet there");
        Keep::open(Some(&state), "the operators' password").put("bot-1", token(1)).unwrap();
        let mode = std::fs::metadata(state.join(FILE_NAME)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn without_a_directory_nothing_is_kept() {
        let mut keep = Keep::open(None, "the operators' password");
        keep.put("bot-1", token(1)).unwrap();
        assert_eq!(keep.get("bot-1"), Some(&token(1)));
    }
}
