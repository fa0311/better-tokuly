//! Ergonomic access to the caller's identity inside RPC handlers.
use connectrpc::RequestContext;
use uuid::Uuid;

use crate::{
    auth::{AuthContext, AuthState, ClientMeta},
    error::{AppError, AppResult},
};

/// Extension methods on the Connect request context.
pub trait CtxExt {
    /// The authenticated caller, or an `unauthenticated` error.
    fn auth(&self) -> AppResult<&AuthContext>;
    /// The caller if signed in; `None` for anonymous requests. An invalid or
    /// expired token is still an error so clients refresh instead of silently
    /// acting as a guest.
    fn maybe_auth(&self) -> AppResult<Option<&AuthContext>>;
    fn user_id(&self) -> AppResult<Uuid> {
        self.auth().map(|a| a.user_id)
    }
    fn maybe_user_id(&self) -> AppResult<Option<Uuid>> {
        self.maybe_auth().map(|a| a.map(|a| a.user_id))
    }
    /// The caller, who must hold `role` (admins hold every role).
    fn require_role(&self, role: &str) -> AppResult<&AuthContext> {
        let auth = self.auth()?;
        if auth.has_role(role) {
            Ok(auth)
        } else {
            Err(AppError::denied(
                "FORBIDDEN",
                format!("requires role {role}"),
            ))
        }
    }
    fn client(&self) -> ClientMeta;
}

impl CtxExt for RequestContext {
    fn auth(&self) -> AppResult<&AuthContext> {
        match self.extensions().get::<AuthState>() {
            Some(AuthState::Authenticated(ctx)) => Ok(ctx),
            Some(AuthState::Invalid(reason)) => Err(AppError::unauthenticated(
                *reason,
                "access token is not valid",
            )),
            _ => Err(AppError::unauthenticated(
                "SIGN_IN_REQUIRED",
                "sign in required",
            )),
        }
    }

    fn maybe_auth(&self) -> AppResult<Option<&AuthContext>> {
        match self.extensions().get::<AuthState>() {
            Some(AuthState::Authenticated(ctx)) => Ok(Some(ctx)),
            Some(AuthState::Invalid(reason)) => Err(AppError::unauthenticated(
                *reason,
                "access token is not valid",
            )),
            _ => Ok(None),
        }
    }

    fn client(&self) -> ClientMeta {
        self.extensions()
            .get::<ClientMeta>()
            .cloned()
            .unwrap_or_default()
    }
}
