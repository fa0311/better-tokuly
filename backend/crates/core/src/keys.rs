//! Key derivation from the master secret.
//!
//! One `TOKULY_SECRET` fans out into independent keys with HKDF-SHA256, so
//! every service instance derives identical keys without distributing key
//! files: the Ed25519 access-token signing key, HMAC keys for opaque tokens
//! and cursors, and so on.
//!
//! While the secret is being replaced, `TOKULY_SECRET_PREVIOUS` holds the
//! old one: whatever is signed, hashed or sealed now uses the new secret,
//! and what was made under the old one still verifies and opens
//! (`docs/configuration.md`, "TOKULY_SECRET を入れ替える").
use base64::Engine as _;
use hkdf::Hkdf;
use hmac::{Hmac, Mac as _};
use sha2::{Digest as _, Sha256};

const SALT: &[u8] = b"tokuly/v1";

#[derive(Clone)]
pub struct Keys {
    /// The secret everything new is made with.
    master: Vec<u8>,
    /// The secret being replaced, if any: only checked against.
    previous: Option<Vec<u8>>,
}

impl Keys {
    pub fn new(secret: &str) -> Self {
        Self {
            master: secret.as_bytes().to_vec(),
            previous: None,
        }
    }

    /// Also accepts what was made under `previous` (empty: nothing).
    pub fn with_previous(mut self, previous: &str) -> Self {
        self.previous = (!previous.is_empty()).then(|| previous.as_bytes().to_vec());
        self
    }

    /// Whether a previous secret is set.
    pub fn has_previous(&self) -> bool {
        self.previous.is_some()
    }

    /// The secrets, the current one first.
    fn masters(&self) -> impl Iterator<Item = &[u8]> {
        std::iter::once(self.master.as_slice()).chain(self.previous.as_deref())
    }

    /// Derives a 32-byte key for a purpose label, e.g. `"jwt/ed25519"`.
    pub fn derive(&self, label: &str) -> [u8; 32] {
        derive(&self.master, label)
    }

    /// The key for `label` under every secret, the current one first: what a
    /// signature made with [`Self::derive`] is checked against.
    pub fn derive_all(&self, label: &str) -> Vec<[u8; 32]> {
        self.masters().map(|master| derive(master, label)).collect()
    }

    /// HMAC-SHA256 of `data` under the key derived for `label`.
    ///
    /// Used as a signature, check it with [`Self::verify_mac`]. Used as a
    /// stable pseudonym (blur offsets, anonymous names, viewer counting),
    /// it changes when the secret does.
    pub fn mac(&self, label: &str, data: &[u8]) -> [u8; 32] {
        mac(&self.derive(label), data)
    }

    /// [`Self::mac`] under every secret, the current one first.
    pub fn macs(&self, label: &str, data: &[u8]) -> Vec<[u8; 32]> {
        self.derive_all(label).iter().map(|key| mac(key, data)).collect()
    }

    /// Whether `tag` is [`Self::mac`] of `data` (or its first `tag.len()`
    /// bytes, at least 16) under the current or the previous secret.
    pub fn verify_mac(&self, label: &str, data: &[u8], tag: &[u8]) -> bool {
        (16..=32).contains(&tag.len()) && self.macs(label, data).iter().any(|mac| secure_eq(&mac[..tag.len()], tag))
    }

    /// Keyed hash for storing secrets we only need to verify (refresh tokens,
    /// email verification tokens, stream keys). Returned as lowercase hex.
    /// Look a presented secret up with [`Self::token_hashes`].
    pub fn token_hash(&self, token: &str) -> String {
        hex::encode(self.mac("token-hash", token.as_bytes()))
    }

    /// [`Self::token_hash`] under every secret, the current one first: what
    /// a stored hash of a presented secret may be (`token_hash = ANY($1)`).
    /// A row found under the previous one is best rewritten with the first.
    pub fn token_hashes(&self, token: &str) -> Vec<String> {
        self.macs("token-hash", token.as_bytes()).iter().map(hex::encode).collect()
    }
}

fn derive(master: &[u8], label: &str) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(SALT), master);
    let mut out = [0u8; 32];
    hk.expand(label.as_bytes(), &mut out)
        .expect("32 bytes is a valid HKDF output length");
    out
}

fn mac(key: &[u8; 32], data: &[u8]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(key)
        .expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

impl Keys {
    /// Encrypts a small secret for storage (XChaCha20-Poly1305). The label
    /// separates key spaces, e.g. `"totp"` or `"connection/spotify"`.
    /// Output layout: 24-byte nonce followed by the ciphertext. A column of
    /// sealed values is listed in its crate's `SEALED` (`tokuly rekey`).
    pub fn seal(&self, label: &str, plaintext: &[u8]) -> Vec<u8> {
        use chacha20poly1305::{KeyInit as _, XChaCha20Poly1305, XNonce, aead::Aead as _};
        use rand::RngExt as _;
        let cipher = XChaCha20Poly1305::new((&self.derive(&format!("seal/{label}"))).into());
        let mut nonce = [0u8; 24];
        rand::rng().fill(&mut nonce[..]);
        let mut out = nonce.to_vec();
        out.extend(
            cipher
                .encrypt(&XNonce::from(nonce), plaintext)
                .expect("encryption cannot fail"),
        );
        out
    }

    /// Decrypts a value produced by [`Self::seal`] under the current or the
    /// previous secret; `None` if it was tampered with or sealed under
    /// another label/secret.
    pub fn open(&self, label: &str, sealed: &[u8]) -> Option<Vec<u8>> {
        self.derive_all(&format!("seal/{label}")).iter().find_map(|key| open(key, sealed))
    }

    /// For a value sealed under the previous secret: its plaintext, to be
    /// sealed again with [`Self::seal`] (`tokuly rekey`). `None` when it is
    /// sealed under the current secret already, or opens with neither.
    pub fn open_previous(&self, label: &str, sealed: &[u8]) -> Option<Vec<u8>> {
        let previous = self.previous.as_deref()?;
        let label = format!("seal/{label}");
        if open(&derive(&self.master, &label), sealed).is_some() {
            return None;
        }
        open(&derive(previous, &label), sealed)
    }
}

fn open(key: &[u8; 32], sealed: &[u8]) -> Option<Vec<u8>> {
    use chacha20poly1305::{KeyInit as _, XChaCha20Poly1305, XNonce, aead::Aead as _};
    if sealed.len() < 24 {
        return None;
    }
    let (nonce, ciphertext) = sealed.split_at(24);
    let nonce: [u8; 24] = nonce.try_into().ok()?;
    XChaCha20Poly1305::new(key.into()).decrypt(&XNonce::from(nonce), ciphertext).ok()
}

/// Plain SHA-256, hex encoded.
pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn b64url(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

pub fn b64url_decode(data: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(data)
        .ok()
}

/// A random URL-safe secret with `bytes` bytes of entropy.
pub fn random_token(bytes: usize) -> String {
    use rand::RngExt as _;
    let mut buf = vec![0u8; bytes];
    rand::rng().fill(&mut buf[..]);
    b64url(&buf)
}

/// Constant-time equality for secrets.
pub fn secure_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq as _;
    a.len() == b.len() && a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivation_is_deterministic_and_label_separated() {
        let keys = Keys::new("0123456789abcdef0123456789abcdef");
        assert_eq!(keys.derive("a"), keys.derive("a"));
        assert_ne!(keys.derive("a"), keys.derive("b"));
        assert_ne!(
            Keys::new("another-secret-another-secret-123").derive("a"),
            keys.derive("a")
        );
    }

    #[test]
    fn seal_roundtrips_and_rejects_tampering() {
        let keys = Keys::new("0123456789abcdef0123456789abcdef");
        let sealed = keys.seal("totp", b"JBSWY3DPEHPK3PXP");
        assert_eq!(
            keys.open("totp", &sealed).as_deref(),
            Some(b"JBSWY3DPEHPK3PXP".as_slice())
        );
        assert!(keys.open("other", &sealed).is_none());
        let mut bad = sealed.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(keys.open("totp", &bad).is_none());
        assert_ne!(
            sealed,
            keys.seal("totp", b"JBSWY3DPEHPK3PXP"),
            "nonces are random"
        );
    }

    #[test]
    fn token_hash_is_stable_hex() {
        let keys = Keys::new("0123456789abcdef0123456789abcdef");
        let h = keys.token_hash("abc");
        assert_eq!(h.len(), 64);
        assert_eq!(h, keys.token_hash("abc"));
        assert_ne!(h, keys.token_hash("abd"));
    }

    const OLD: &str = "0123456789abcdef0123456789abcdef";
    const NEW: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn what_the_previous_secret_made_still_checks_out_after_a_rotation() {
        let before = Keys::new(OLD);
        let sealed = before.seal("totp", b"seed");
        let tag = before.mac("media/link-image", b"url");
        let hash = before.token_hash("refresh");

        let rotated = Keys::new(NEW).with_previous(OLD);
        assert_eq!(rotated.open("totp", &sealed).as_deref(), Some(b"seed".as_slice()));
        assert!(rotated.verify_mac("media/link-image", b"url", &tag[..18]));
        assert!(rotated.token_hashes("refresh").contains(&hash));
        assert_eq!(rotated.token_hashes("refresh")[0], Keys::new(NEW).token_hash("refresh"), "the current one first");
        assert_eq!(rotated.open_previous("totp", &sealed).as_deref(), Some(b"seed".as_slice()));

        let resealed = rotated.seal("totp", b"seed");
        assert_eq!(rotated.open_previous("totp", &resealed), None, "already under the new secret");
        assert_eq!(Keys::new(NEW).open("totp", &resealed).as_deref(), Some(b"seed".as_slice()));
    }

    #[test]
    fn once_the_previous_secret_is_removed_nothing_it_made_checks_out() {
        let before = Keys::new(OLD);
        let sealed = before.seal("totp", b"seed");
        let tag = before.mac("media/link-image", b"url");
        let after = Keys::new(NEW).with_previous("");
        assert!(!after.has_previous());
        assert_eq!(after.open("totp", &sealed), None);
        assert!(!after.verify_mac("media/link-image", b"url", &tag));
        assert!(!after.token_hashes("refresh").contains(&before.token_hash("refresh")));
        assert_eq!(after.open_previous("totp", &sealed), None);
    }

    #[test]
    fn a_short_or_wrong_tag_does_not_verify() {
        let keys = Keys::new(OLD);
        let tag = keys.mac("a", b"data");
        assert!(keys.verify_mac("a", b"data", &tag));
        assert!(!keys.verify_mac("a", b"data", &tag[..8]), "too short to mean anything");
        assert!(!keys.verify_mac("b", b"data", &tag));
        assert!(!keys.verify_mac("a", b"date", &tag));
    }
}
