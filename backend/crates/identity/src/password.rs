//! Password hashing (Argon2id), run off the async executor.
use argon2::{
    Argon2,
    password_hash::{PasswordHasher as _, PasswordVerifier as _, phc::PasswordHash},
};
use tokuly_core::prelude::*;

/// Hashes a password into a PHC string.
pub async fn hash(password: &str) -> AppResult<String> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|h| h.to_string())
            .map_err(|e| AppError::internal(anyhow::anyhow!("password hash: {e}")))
    })
    .await
    .map_err(|e| AppError::internal(anyhow::anyhow!("password hash task: {e}")))?
}

/// Verifies a password. `stored` of `None` (unknown user, or an account
/// without a password) still performs a hash so timing does not reveal which.
pub async fn verify(password: &str, stored: Option<&str>) -> bool {
    // PHC string of a throwaway password, used only to burn comparable time.
    const DUMMY: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$Z2KmMbTmM0LZj2pP3YBBDX0LPsgqtTNT0EXW4bXDq1g";
    let password = password.to_owned();
    let (hash, real) = match stored {
        Some(h) => (h.to_owned(), true),
        None => (DUMMY.to_owned(), false),
    };
    tokio::task::spawn_blocking(move || {
        let ok = PasswordHash::new(&hash)
            .map(|parsed| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &parsed)
                    .is_ok()
            })
            .unwrap_or(false);
        ok && real
    })
    .await
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hash_then_verify() {
        let h = hash("correct horse battery").await.unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify("correct horse battery", Some(&h)).await);
        assert!(!verify("wrong", Some(&h)).await);
        assert!(!verify("anything", None).await);
        assert!(!verify("anything", Some("not-a-phc-string")).await);
    }
}
