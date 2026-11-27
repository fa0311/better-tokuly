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
    /// The caller, who must be signed in with a session: API tokens are
    /// turned away. The middleware already keeps tokens from the RPCs listed
    /// in [`crate::api_tokens`]; use this where a handler decides by itself.
    fn require_session(&self) -> AppResult<&AuthContext> {
        let auth = self.auth()?;
        if auth.is_session() {
            Ok(auth)
        } else {
            Err(AppError::denied("SESSION_REQUIRED", "API tokens cannot do this; it takes a signed-in session"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_tokens::{ApiToken, ApiTokenKind, RateTier, Scopes};

    fn context(state: AuthState) -> RequestContext {
        let mut ctx = RequestContext::default();
        ctx.extensions_mut().insert(state);
        ctx
    }

    #[test]
    fn a_session_is_required_where_a_handler_says_so() {
        let session = AuthContext { user_id: Uuid::now_v7(), session_id: Uuid::now_v7(), roles: Vec::new(), expires_at: chrono::Utc::now(), api_token: None, too_young: false };
        assert!(context(AuthState::Authenticated(session)).require_session().is_ok());

        let token = ApiToken {
            kind: ApiTokenKind::Personal,
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            app_id: None,
            scopes: Scopes::new(["identity:write"]),
            expires_at: None,
            rate: RateTier { name: "standard".into(), per_minute: 60 },
        };
        let ctx = context(AuthState::Authenticated(AuthContext::from_api_token(token.clone())));
        // The token is the caller for everything else.
        assert_eq!(ctx.user_id().unwrap(), token.user_id);
        let denied = ctx.require_session().unwrap_err();
        assert_eq!((denied.code, denied.reason.as_ref()), (connectrpc::ErrorCode::PermissionDenied, "SESSION_REQUIRED"));
        // Tokens hold no role, so role-gated RPCs stay closed to them.
        assert_eq!(ctx.require_role("moderator").unwrap_err().reason, "FORBIDDEN");

        assert_eq!(context(AuthState::Anonymous).require_session().unwrap_err().reason, "SIGN_IN_REQUIRED");
        assert_eq!(context(AuthState::Invalid("TOKEN_REVOKED")).require_session().unwrap_err().reason, "TOKEN_REVOKED");
    }
}
