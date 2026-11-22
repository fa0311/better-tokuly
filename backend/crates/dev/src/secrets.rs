//! The secrets this domain hands out, and how to recognise them.
//!
//! Every secret has the shape `tkl_<kind>_<36 random base62><6 checksum>`:
//! a fixed prefix so that secret scanners (and people) recognise a leaked
//! Tokuly credential and what it is, about 214 bits of randomness, and a
//! CRC32 of everything before it, so a scanner can tell a real secret from
//! a lookalike — and the server can discard a mistyped one — without
//! asking the database.
//!
//! Only a keyed hash of a secret is stored (`Keys::token_hash`).
use tokuly_core::{api_tokens::API_TOKEN_PREFIX, ids::random_base62};

const RANDOM_CHARS: usize = 36;
const CHECKSUM_CHARS: usize = 6;

/// What a secret is for. The first three are bearer tokens for the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    /// Personal access token.
    Personal,
    /// Bot token.
    Bot,
    /// OAuth access token.
    Access,
    /// OAuth refresh token.
    Refresh,
    /// OAuth authorization code.
    Code,
    /// Device code of the device flow.
    Device,
    /// OAuth client secret.
    ClientSecret,
    /// Webhook signing secret.
    Webhook,
}

impl SecretKind {
    const ALL: [Self; 8] = [
        Self::Personal,
        Self::Bot,
        Self::Access,
        Self::Refresh,
        Self::Code,
        Self::Device,
        Self::ClientSecret,
        Self::Webhook,
    ];

    fn tag(self) -> &'static str {
        match self {
            Self::Personal => "pat",
            Self::Bot => "bot",
            Self::Access => "oat",
            Self::Refresh => "ort",
            Self::Code => "oac",
            Self::Device => "odc",
            Self::ClientSecret => "cs",
            Self::Webhook => "whs",
        }
    }

    pub fn prefix(self) -> String {
        format!("{API_TOKEN_PREFIX}{}_", self.tag())
    }
}

/// CRC-32 (IEEE 802.3), the checksum GitHub-style tokens carry.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

fn checksum(body: &str) -> String {
    let mut value = crc32(body.as_bytes());
    let mut out = [b'0'; CHECKSUM_CHARS];
    for slot in out.iter_mut().rev() {
        *slot = BASE62[(value % 62) as usize];
        value /= 62;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A new secret of the given kind.
pub fn generate(kind: SecretKind) -> String {
    let body = format!("{}{}", kind.prefix(), random_base62(RANDOM_CHARS));
    let checksum = checksum(&body);
    body + &checksum
}

/// What kind of secret `secret` is, if it is well formed (prefix, length,
/// alphabet and checksum). Says nothing about whether it was ever issued.
pub fn kind_of(secret: &str) -> Option<SecretKind> {
    let kind = SecretKind::ALL.into_iter().find(|kind| secret.starts_with(&kind.prefix()))?;
    let rest = &secret[kind.prefix().len()..];
    if rest.len() != RANDOM_CHARS + CHECKSUM_CHARS || !rest.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let (body, sum) = secret.split_at(secret.len() - CHECKSUM_CHARS);
    (checksum(body) == sum).then_some(kind)
}

/// Whether `secret` is a well-formed secret of exactly this kind.
pub fn is(secret: &str, kind: SecretKind) -> bool {
    kind_of(secret) == Some(kind)
}

/// What is shown of a secret after its creation: `tkl_pat_…x7Qd`.
pub fn hint(secret: &str) -> String {
    match kind_of(secret) {
        Some(kind) => format!("{}…{}", kind.prefix(), &secret[secret.len() - 4..]),
        None => String::new(),
    }
}

/// A code a person types on the device page: eight letters without vowels
/// (no accidental words) or look-alikes, shown as `BCDF-GHJK` (RFC 8628 §6.1).
pub fn generate_user_code() -> String {
    use rand::RngExt as _;
    const ALPHABET: &[u8; 20] = b"BCDFGHJKLMNPQRSTVWXZ";
    let mut rng = rand::rng();
    let code: String = (0..8).map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char).collect();
    format!("{}-{}", &code[..4], &code[4..])
}

/// A typed user code without the dash, spaces or case.
pub fn normalize_user_code(input: &str) -> String {
    input.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn generated_secrets_are_recognised_by_kind() {
        for kind in SecretKind::ALL {
            let secret = generate(kind);
            assert!(secret.starts_with("tkl_"), "{secret}");
            assert_eq!(secret.len(), kind.prefix().len() + 42);
            assert_eq!(kind_of(&secret), Some(kind));
            assert!(is(&secret, kind));
        }
        assert_ne!(generate(SecretKind::Personal), generate(SecretKind::Personal));
        assert!(generate(SecretKind::Personal).starts_with("tkl_pat_"));
        assert!(generate(SecretKind::Bot).starts_with("tkl_bot_"));
        assert!(!is(&generate(SecretKind::Refresh), SecretKind::Access));
    }

    #[test]
    fn a_damaged_secret_fails_its_checksum() {
        let secret = generate(SecretKind::Personal);
        let mut typo = secret.clone().into_bytes();
        let i = "tkl_pat_".len() + 3;
        typo[i] = if typo[i] == b'a' { b'b' } else { b'a' };
        assert_eq!(kind_of(std::str::from_utf8(&typo).unwrap()), None);
        assert_eq!(kind_of(&secret[..secret.len() - 1]), None);
        assert_eq!(kind_of(&format!("{secret}0")), None);
        assert_eq!(kind_of(&secret.replace("tkl_pat_", "tkl_bot_")), None, "the prefix is part of the checksum");
        for junk in [
            "",
            "tkl_",
            "tkl_pat_",
            "tkl_xyz_abc",
            "eyJhbGciOiJFZERTQSJ9.e30.sig",
            "tkl_pat_日本語日本語日本語日本語日本語日本語日本語",
        ] {
            assert_eq!(kind_of(junk), None, "{junk:?}");
        }
    }

    #[test]
    fn hints_show_the_kind_and_the_tail_only() {
        let secret = generate(SecretKind::Personal);
        let hint = hint(&secret);
        assert_eq!(hint, format!("tkl_pat_…{}", &secret[secret.len() - 4..]));
        assert_eq!(super::hint("nonsense"), "");
    }

    #[test]
    fn user_codes_are_typed_forgivingly() {
        let code = generate_user_code();
        assert_eq!(code.len(), 9);
        assert_eq!(&code[4..5], "-");
        assert!(code.replace('-', "").bytes().all(|b| b"BCDFGHJKLMNPQRSTVWXZ".contains(&b)));
        assert_eq!(normalize_user_code(&code.to_lowercase().replace('-', " ")), code.replace('-', ""));
    }
}
