//! Passkeys (WebAuthn): registration and discoverable sign-in.
//!
//! Ceremony state lives in Valkey for five minutes under a random challenge
//! ID that the client echoes back when finishing.
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokuly_core::{keys::random_token, prelude::*};
use tokuly_proto::identity::Passkey as PasskeyMsg;
use webauthn_rs::prelude::{
    DiscoverableAuthentication, Passkey, PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
};

use crate::{Identity, users::UserRow};

const CEREMONY_TTL: Duration = Duration::from_secs(300);

#[derive(Serialize, Deserialize)]
struct RegistrationState {
    user_id: Uuid,
    name: String,
    state: PasskeyRegistration,
}

fn webauthn_err(e: impl std::fmt::Display) -> AppError {
    AppError::invalid("PASSKEY_FAILED", format!("passkey verification failed: {e}"))
}

impl Identity {
    /// Starts registering a passkey; returns `(challenge_id, creation options JSON)`.
    pub(crate) async fn begin_passkey_registration(&self, user: &UserRow, name: &str) -> AppResult<(String, String)> {
        let existing: Vec<serde_json::Value> = sqlx::query_scalar("SELECT credential FROM passkeys WHERE user_id = $1")
            .bind(user.id)
            .fetch_all(&self.core.db)
            .await?;
        ensure!(existing.len() < 20, precondition, "TOO_MANY_PASSKEYS", "remove a passkey before adding another");
        let exclude = existing
            .into_iter()
            .filter_map(|v| serde_json::from_value::<Passkey>(v).ok())
            .map(|p| p.cred_id().clone())
            .collect::<Vec<_>>();
        let (options, state) = self
            .webauthn
            .start_passkey_registration(user.id, &user.handle, &user.display_name, Some(exclude))
            .map_err(webauthn_err)?;
        let challenge_id = random_token(18);
        let name = name.trim();
        let name = if name.is_empty() { "Passkey".to_owned() } else { name.chars().take(60).collect() };
        self.core
            .cache
            .set_json(
                &format!("id:pk:reg:{challenge_id}"),
                &RegistrationState { user_id: user.id, name, state },
                Some(CEREMONY_TTL),
            )
            .await?;
        let json = serde_json::to_string(&options.public_key)
            .map_err(|e| AppError::internal(anyhow::anyhow!("options encode: {e}")))?;
        Ok((challenge_id, json))
    }

    pub(crate) async fn finish_passkey_registration(
        &self,
        user_id: Uuid,
        challenge_id: &str,
        credential_json: &str,
    ) -> AppResult<PasskeyMsg> {
        let state: RegistrationState = self
            .core
            .cache
            .take_json(&format!("id:pk:reg:{challenge_id}"))
            .await?
            .ok_or_else(|| AppError::precondition("PASSKEY_EXPIRED", "the passkey request expired, try again"))?;
        ensure!(state.user_id == user_id, denied, "FORBIDDEN", "this passkey request belongs to another account");
        let credential: RegisterPublicKeyCredential = serde_json::from_str(credential_json)
            .map_err(|e| AppError::invalid("PASSKEY_INVALID", format!("malformed credential: {e}")))?;
        let passkey = self.webauthn.finish_passkey_registration(&credential, &state.state).map_err(webauthn_err)?;
        let credential_id: &[u8] = passkey.cred_id().as_ref();
        let stored =
            serde_json::to_value(&passkey).map_err(|e| AppError::internal(anyhow::anyhow!("passkey encode: {e}")))?;
        let (id, created_at): (Uuid, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
            "INSERT INTO passkeys (user_id, name, credential_id, credential) VALUES ($1, $2, $3, $4) RETURNING id, created_at",
        )
        .bind(user_id)
        .bind(&state.name)
        .bind(credential_id)
        .bind(stored)
        .fetch_one(&self.core.db)
        .await
        .map_err(|e| {
            let e = AppError::from(e);
            if e.is_already_exists() { AppError::already_exists("PASSKEY_EXISTS", "this passkey is already registered") } else { e }
        })?;
        Ok(PasskeyMsg { id: id.to_string(), name: state.name, created_at: ts(created_at), ..Default::default() })
    }

    /// Starts a usernameless sign-in; returns `(challenge_id, request options JSON)`.
    pub(crate) async fn begin_passkey_sign_in(&self) -> AppResult<(String, String)> {
        let (options, state) = self.webauthn.start_discoverable_authentication().map_err(webauthn_err)?;
        let challenge_id = random_token(18);
        self.core.cache.set_json(&format!("id:pk:auth:{challenge_id}"), &state, Some(CEREMONY_TTL)).await?;
        let json = serde_json::to_string(&options.public_key)
            .map_err(|e| AppError::internal(anyhow::anyhow!("options encode: {e}")))?;
        Ok((challenge_id, json))
    }

    /// Verifies the assertion and returns the user it authenticates.
    pub(crate) async fn finish_passkey_sign_in(&self, challenge_id: &str, credential_json: &str) -> AppResult<UserRow> {
        let state: DiscoverableAuthentication = self
            .core
            .cache
            .take_json(&format!("id:pk:auth:{challenge_id}"))
            .await?
            .ok_or_else(|| AppError::precondition("PASSKEY_EXPIRED", "the sign-in request expired, try again"))?;
        let credential: PublicKeyCredential = serde_json::from_str(credential_json)
            .map_err(|e| AppError::invalid("PASSKEY_INVALID", format!("malformed credential: {e}")))?;
        let unknown = || AppError::unauthenticated("PASSKEY_UNKNOWN", "this passkey is not registered");
        let (user_id, credential_id) =
            self.webauthn.identify_discoverable_authentication(&credential).map_err(|_| unknown())?;
        let row: Option<(Uuid, serde_json::Value)> =
            sqlx::query_as("SELECT id, credential FROM passkeys WHERE user_id = $1 AND credential_id = $2")
                .bind(user_id)
                .bind(credential_id)
                .fetch_optional(&self.core.db)
                .await?;
        let (passkey_id, stored) = row.ok_or_else(unknown)?;
        let mut passkey: Passkey =
            serde_json::from_value(stored).map_err(|e| AppError::internal(anyhow::anyhow!("stored passkey: {e}")))?;
        let result = self
            .webauthn
            .finish_discoverable_authentication_v2(&credential, state, &passkey)
            .map_err(|e| AppError::unauthenticated("PASSKEY_FAILED", format!("passkey verification failed: {e}")))?;
        // Persist the signature counter / backup state when the authenticator reports a change.
        if passkey.update_credential(&result) == Some(true) {
            let updated = serde_json::to_value(&passkey)
                .map_err(|e| AppError::internal(anyhow::anyhow!("passkey encode: {e}")))?;
            sqlx::query("UPDATE passkeys SET credential = $2, last_used_at = now() WHERE id = $1")
                .bind(passkey_id)
                .bind(updated)
                .execute(&self.core.db)
                .await?;
        } else {
            sqlx::query("UPDATE passkeys SET last_used_at = now() WHERE id = $1")
                .bind(passkey_id)
                .execute(&self.core.db)
                .await?;
        }
        let user = crate::users::get(&self.core, user_id).await.map_err(|_| unknown())?;
        ensure!(user.deleted_at.is_none(), unauthenticated, "PASSKEY_UNKNOWN", "this passkey is not registered");
        Ok(user)
    }
}
