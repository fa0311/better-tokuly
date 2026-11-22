//! Key derivation from the master secret.
//!
//! One `TOKULY_SECRET` fans out into independent keys with HKDF-SHA256, so
//! every service instance derives identical keys without distributing key
//! files: the Ed25519 access-token signing key, HMAC keys for opaque tokens
//! and cursors, and so on.
use base64::Engine as _;
use hkdf::Hkdf;
use hmac::{Hmac, Mac as _};
use sha2::{Digest as _, Sha256};

const SALT: &[u8] = b"tokuly/v1";

#[derive(Clone)]
pub struct Keys {
    master: Vec<u8>,
}

impl Keys {
    pub fn new(secret: &str) -> Self {
        Self { master: secret.as_bytes().to_vec() }
    }

    /// Derives a 32-byte key for a purpose label, e.g. `"jwt/ed25519"`.
    pub fn derive(&self, label: &str) -> [u8; 32] {
        let hk = Hkdf::<Sha256>::new(Some(SALT), &self.master);
        let mut out = [0u8; 32];
        hk.expand(label.as_bytes(), &mut out).expect("32 bytes is a valid HKDF output length");
        out
    }

    /// HMAC-SHA256 of `data` under the key derived for `label`.
    pub fn mac(&self, label: &str, data: &[u8]) -> [u8; 32] {
        let key = self.derive(label);
        let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(&key).expect("HMAC accepts any key length");
        mac.update(data);
        mac.finalize().into_bytes().into()
    }

    /// Keyed hash for storing secrets we only need to verify (refresh tokens,
    /// email verification tokens, stream keys). Returned as lowercase hex.
    pub fn token_hash(&self, token: &str) -> String {
        hex::encode(self.mac("token-hash", token.as_bytes()))
    }
}

impl Keys {
    /// Encrypts a small secret for storage (XChaCha20-Poly1305). The label
    /// separates key spaces, e.g. `"totp"` or `"connection/spotify"`.
    /// Output layout: 24-byte nonce followed by the ciphertext.
    pub fn seal(&self, label: &str, plaintext: &[u8]) -> Vec<u8> {
        use chacha20poly1305::{KeyInit as _, XChaCha20Poly1305, XNonce, aead::Aead as _};
        use rand::RngExt as _;
        let cipher = XChaCha20Poly1305::new((&self.derive(&format!("seal/{label}"))).into());
        let mut nonce = [0u8; 24];
        rand::rng().fill(&mut nonce[..]);
        let mut out = nonce.to_vec();
        out.extend(cipher.encrypt(&XNonce::from(nonce), plaintext).expect("encryption cannot fail"));
        out
    }

    /// Decrypts a value produced by [`Self::seal`]; `None` if it was tampered
    /// with or sealed under another label/secret.
    pub fn open(&self, label: &str, sealed: &[u8]) -> Option<Vec<u8>> {
        use chacha20poly1305::{KeyInit as _, XChaCha20Poly1305, XNonce, aead::Aead as _};
        if sealed.len() < 24 {
            return None;
        }
        let (nonce, ciphertext) = sealed.split_at(24);
        let nonce: [u8; 24] = nonce.try_into().ok()?;
        let cipher = XChaCha20Poly1305::new((&self.derive(&format!("seal/{label}"))).into());
        cipher.decrypt(&XNonce::from(nonce), ciphertext).ok()
    }
}

/// Plain SHA-256, hex encoded.
pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn b64url(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

pub fn b64url_decode(data: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(data).ok()
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
        assert_ne!(Keys::new("another-secret-another-secret-123").derive("a"), keys.derive("a"));
    }

    #[test]
    fn seal_roundtrips_and_rejects_tampering() {
        let keys = Keys::new("0123456789abcdef0123456789abcdef");
        let sealed = keys.seal("totp", b"JBSWY3DPEHPK3PXP");
        assert_eq!(keys.open("totp", &sealed).as_deref(), Some(b"JBSWY3DPEHPK3PXP".as_slice()));
        assert!(keys.open("other", &sealed).is_none());
        let mut bad = sealed.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(keys.open("totp", &bad).is_none());
        assert_ne!(sealed, keys.seal("totp", b"JBSWY3DPEHPK3PXP"), "nonces are random");
    }

    #[test]
    fn token_hash_is_stable_hex() {
        let keys = Keys::new("0123456789abcdef0123456789abcdef");
        let h = keys.token_hash("abc");
        assert_eq!(h.len(), 64);
        assert_eq!(h, keys.token_hash("abc"));
        assert_ne!(h, keys.token_hash("abd"));
    }
}
