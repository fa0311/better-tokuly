//! Message encryption for Web Push (RFC 8291) using the `aes128gcm` content
//! coding (RFC 8188).
//!
//! A push service only relays bytes; the notification is encrypted for the
//! browser that owns the subscription. The sender makes a one-off P-256 key
//! pair, combines an ECDH secret with the subscription's auth secret, and
//! encrypts a single AES-128-GCM record:
//!
//! ```text
//! body = salt (16) | record size (4, big endian) | key ID length (1) | sender public key (65) | ciphertext
//! ```
use aes_gcm::{Aes128Gcm, KeyInit as _, Nonce, aead::Aead as _};
use hkdf::Hkdf;
use p256::{PublicKey, SecretKey};
use rand::RngExt as _;
use sha2::Sha256;

/// Record size we declare. One record carries the whole message.
const RECORD_SIZE: u32 = 4096;
const SALT_LEN: usize = 16;
/// Uncompressed P-256 point.
pub const PUBLIC_KEY_LEN: usize = 65;
pub const AUTH_SECRET_LEN: usize = 16;
const HEADER_LEN: usize = SALT_LEN + 4 + 1 + PUBLIC_KEY_LEN;
const TAG_LEN: usize = 16;
/// Marks the end of the last record's plaintext (RFC 8188 section 2).
const LAST_RECORD_DELIMITER: u8 = 0x02;

/// Largest plaintext that fits the single record push services accept (they
/// cap the body at 4096 bytes).
pub const MAX_PLAINTEXT_LEN: usize = RECORD_SIZE as usize - HEADER_LEN - TAG_LEN - 1;

#[derive(Debug, PartialEq, Eq)]
pub enum EceError {
    /// The subscription's public key is not a P-256 point.
    InvalidPublicKey,
    /// The subscription's auth secret is not 16 bytes.
    InvalidAuthSecret,
    PlaintextTooLong,
    /// The body is not a single well-formed `aes128gcm` record.
    Malformed,
    /// Wrong key, or the body was tampered with.
    DecryptionFailed,
}

impl std::fmt::Display for EceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidPublicKey => "subscription public key is not a P-256 point",
            Self::InvalidAuthSecret => "subscription auth secret must be 16 bytes",
            Self::PlaintextTooLong => "message does not fit one push record",
            Self::Malformed => "not an aes128gcm push message",
            Self::DecryptionFailed => "push message decryption failed",
        })
    }
}

impl std::error::Error for EceError {}

/// Encrypts `plaintext` for a subscription (`ua_public`: its `p256dh` key,
/// `auth_secret`: its `auth` value) with a fresh sender key and salt.
pub fn encrypt(plaintext: &[u8], ua_public: &[u8], auth_secret: &[u8]) -> Result<Vec<u8>, EceError> {
    let mut salt = [0u8; SALT_LEN];
    rand::rng().fill(&mut salt[..]);
    encrypt_with(plaintext, ua_public, auth_secret, &random_secret_key(), salt)
}

/// A random P-256 private key. Built from raw random bytes so this does not
/// depend on which `rand_core` generation the curve crate was compiled for.
pub fn random_secret_key() -> SecretKey {
    loop {
        let mut bytes = [0u8; 32];
        rand::rng().fill(&mut bytes[..]);
        // Fails only for zero or values >= the group order (about 1 in 2^32).
        if let Ok(key) = SecretKey::from_slice(&bytes) {
            return key;
        }
    }
}

/// The uncompressed SEC1 encoding (65 bytes) of a key's public half.
pub fn public_key_bytes(secret: &SecretKey) -> Vec<u8> {
    secret.public_key().to_sec1_bytes().into_vec()
}

/// [`encrypt`] with the sender key and salt supplied (deterministic; lets the
/// RFC 8291 test vector be reproduced).
fn encrypt_with(
    plaintext: &[u8],
    ua_public: &[u8],
    auth_secret: &[u8],
    as_secret: &SecretKey,
    salt: [u8; SALT_LEN],
) -> Result<Vec<u8>, EceError> {
    if plaintext.len() > MAX_PLAINTEXT_LEN {
        return Err(EceError::PlaintextTooLong);
    }
    let ua_key = parse_public_key(ua_public)?;
    let as_public = public_key_bytes(as_secret);
    let shared = as_secret.diffie_hellman(&ua_key);
    let (key, nonce) = derive_key_and_nonce(shared.raw_secret_bytes(), auth_secret, ua_public, &as_public, &salt)?;

    let mut record = Vec::with_capacity(plaintext.len() + 1);
    record.extend_from_slice(plaintext);
    record.push(LAST_RECORD_DELIMITER);
    let ciphertext = Aes128Gcm::new((&key).into())
        .encrypt(&Nonce::from(nonce), record.as_slice())
        .expect("AES-GCM encryption of a bounded record cannot fail");

    let mut body = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    body.extend_from_slice(&salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(PUBLIC_KEY_LEN as u8);
    body.extend_from_slice(&as_public);
    body.extend_from_slice(&ciphertext);
    Ok(body)
}

/// The receiving half: what a browser does with a push message. The server
/// never needs it in production; it exists so the sending half can be checked
/// against an independent decryption (tests, diagnostics).
pub fn decrypt(body: &[u8], ua_secret: &SecretKey, auth_secret: &[u8]) -> Result<Vec<u8>, EceError> {
    if body.len() < HEADER_LEN + TAG_LEN + 1 || body[SALT_LEN + 4] as usize != PUBLIC_KEY_LEN {
        return Err(EceError::Malformed);
    }
    let (salt, rest) = body.split_at(SALT_LEN);
    let (record_size, rest) = rest.split_at(4);
    let (as_public, ciphertext) = rest[1..].split_at(PUBLIC_KEY_LEN);
    let record_size = u32::from_be_bytes(record_size.try_into().expect("4 bytes")) as usize;
    if ciphertext.len() > record_size {
        return Err(EceError::Malformed); // more than one record
    }
    let as_key = parse_public_key(as_public).map_err(|_| EceError::Malformed)?;
    let ua_public = public_key_bytes(ua_secret);
    let shared = ua_secret.diffie_hellman(&as_key);
    let (key, nonce) = derive_key_and_nonce(shared.raw_secret_bytes(), auth_secret, &ua_public, as_public, salt)?;
    let mut record = Aes128Gcm::new((&key).into())
        .decrypt(&Nonce::from(nonce), ciphertext)
        .map_err(|_| EceError::DecryptionFailed)?;
    // Strip the padding: zeros after the delimiter, then the delimiter itself.
    while record.last() == Some(&0) {
        record.pop();
    }
    match record.pop() {
        Some(LAST_RECORD_DELIMITER) => Ok(record),
        _ => Err(EceError::Malformed),
    }
}

fn parse_public_key(bytes: &[u8]) -> Result<PublicKey, EceError> {
    if bytes.len() != PUBLIC_KEY_LEN || bytes[0] != 0x04 {
        return Err(EceError::InvalidPublicKey);
    }
    PublicKey::from_sec1_bytes(bytes).map_err(|_| EceError::InvalidPublicKey)
}

/// Checks that `bytes` is a usable subscription key (`p256dh`).
pub fn validate_public_key(bytes: &[u8]) -> Result<(), EceError> {
    parse_public_key(bytes).map(|_| ())
}

/// RFC 8291 section 3.4: content-encryption key and nonce from the ECDH
/// secret, the auth secret, both public keys and the salt.
fn derive_key_and_nonce(
    ecdh_secret: &[u8],
    auth_secret: &[u8],
    ua_public: &[u8],
    as_public: &[u8],
    salt: &[u8],
) -> Result<([u8; 16], [u8; 12]), EceError> {
    if auth_secret.len() != AUTH_SECRET_LEN {
        return Err(EceError::InvalidAuthSecret);
    }
    // Mix the auth secret in, bound to both public keys.
    let mut key_info = Vec::with_capacity(14 + 2 * PUBLIC_KEY_LEN);
    key_info.extend_from_slice(b"WebPush: info\0");
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public);
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth_secret), ecdh_secret)
        .expand(&key_info, &mut ikm)
        .expect("32 bytes is a valid HKDF output length");

    // RFC 8188 section 2.2 and 2.3.
    let prk = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut key = [0u8; 16];
    prk.expand(b"Content-Encoding: aes128gcm\0", &mut key).expect("16 bytes is a valid HKDF output length");
    let mut nonce = [0u8; 12];
    prk.expand(b"Content-Encoding: nonce\0", &mut nonce).expect("12 bytes is a valid HKDF output length");
    Ok((key, nonce))
}

#[cfg(test)]
mod tests {
    use tokuly_core::keys::{b64url, b64url_decode};

    use super::*;

    // RFC 8291 section 5 and appendix A.
    const PLAINTEXT: &[u8] = b"When I grow up, I want to be a watermelon";
    const UA_PRIVATE: &str = "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94";
    const UA_PUBLIC: &str = "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    const AS_PRIVATE: &str = "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw";
    const AS_PUBLIC: &str = "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8";
    const AUTH_SECRET: &str = "BTBZMqHH6r4Tts7J_aSIgg";
    const SALT: &str = "DGv6ra1nlYgDCS1FRnbzlw";
    const BODY: &str = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";

    fn bytes(b64: &str) -> Vec<u8> {
        b64url_decode(b64).expect("test vector is base64url")
    }

    fn secret(b64: &str) -> SecretKey {
        SecretKey::from_slice(&bytes(b64)).expect("test vector is a P-256 scalar")
    }

    #[test]
    fn reproduces_the_rfc_8291_example() {
        let as_secret = secret(AS_PRIVATE);
        assert_eq!(b64url(&public_key_bytes(&as_secret)), AS_PUBLIC);
        let salt: [u8; SALT_LEN] = bytes(SALT).try_into().unwrap();
        let body = encrypt_with(PLAINTEXT, &bytes(UA_PUBLIC), &bytes(AUTH_SECRET), &as_secret, salt).unwrap();
        assert_eq!(b64url(&body), BODY);
    }

    #[test]
    fn decrypts_the_rfc_8291_example() {
        let plaintext = decrypt(&bytes(BODY), &secret(UA_PRIVATE), &bytes(AUTH_SECRET)).unwrap();
        assert_eq!(plaintext, PLAINTEXT);
    }

    #[test]
    fn roundtrips_with_fresh_keys_and_never_repeats() {
        let ua_secret = random_secret_key();
        let ua_public = public_key_bytes(&ua_secret);
        let auth = [7u8; AUTH_SECRET_LEN];
        let message = "こんにちは！通知だよ 🎉".as_bytes();
        let first = encrypt(message, &ua_public, &auth).unwrap();
        let second = encrypt(message, &ua_public, &auth).unwrap();
        assert_ne!(first, second, "salt and sender key are fresh per message");
        assert_eq!(decrypt(&first, &ua_secret, &auth).unwrap(), message);
        assert_eq!(decrypt(&second, &ua_secret, &auth).unwrap(), message);
    }

    #[test]
    fn decryption_fails_for_the_wrong_recipient_or_a_modified_body() {
        let ua_secret = random_secret_key();
        let auth = [1u8; AUTH_SECRET_LEN];
        let body = encrypt(b"secret", &public_key_bytes(&ua_secret), &auth).unwrap();
        assert_eq!(decrypt(&body, &random_secret_key(), &auth), Err(EceError::DecryptionFailed));
        assert_eq!(decrypt(&body, &ua_secret, &[2u8; AUTH_SECRET_LEN]), Err(EceError::DecryptionFailed));
        let mut tampered = body.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(decrypt(&tampered, &ua_secret, &auth), Err(EceError::DecryptionFailed));
        assert_eq!(decrypt(&body[..40], &ua_secret, &auth), Err(EceError::Malformed));
    }

    #[test]
    fn the_largest_message_fills_exactly_one_record() {
        let ua_secret = random_secret_key();
        let ua_public = public_key_bytes(&ua_secret);
        let auth = [3u8; AUTH_SECRET_LEN];
        let largest = vec![b'x'; MAX_PLAINTEXT_LEN];
        let body = encrypt(&largest, &ua_public, &auth).unwrap();
        assert_eq!(body.len(), RECORD_SIZE as usize);
        assert_eq!(decrypt(&body, &ua_secret, &auth).unwrap(), largest);
        assert_eq!(encrypt(&vec![b'x'; MAX_PLAINTEXT_LEN + 1], &ua_public, &auth), Err(EceError::PlaintextTooLong));
    }

    #[test]
    fn rejects_unusable_subscription_keys() {
        let auth = [0u8; AUTH_SECRET_LEN];
        assert_eq!(encrypt(b"x", &[4u8; PUBLIC_KEY_LEN], &auth), Err(EceError::InvalidPublicKey), "not on the curve");
        assert_eq!(encrypt(b"x", &[0u8; 10], &auth), Err(EceError::InvalidPublicKey));
        let ua_public = public_key_bytes(&random_secret_key());
        assert_eq!(encrypt(b"x", &ua_public, &[0u8; 8]), Err(EceError::InvalidAuthSecret));
        assert!(validate_public_key(&ua_public).is_ok());
    }
}
