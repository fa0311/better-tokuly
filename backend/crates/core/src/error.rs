//! The error type every domain function returns.
//!
//! [`AppError`] carries a Connect status code, a machine-readable `reason`
//! (sent to clients in the `x-tokuly-error` header so they can localize) and a
//! human message for logs. Any `std::error::Error` converts into an internal
//! error with `?`; database "not found" and unique violations map to the
//! matching codes automatically.
use std::{borrow::Cow, fmt};

use connectrpc::{ConnectError, ErrorCode};

pub type AppResult<T> = Result<T, AppError>;

pub struct AppError {
    pub code: ErrorCode,
    pub reason: Cow<'static, str>,
    pub message: Cow<'static, str>,
    pub source: Option<anyhow::Error>,
}

/// Header (and trailer) carrying the machine-readable error reason.
pub const REASON_HEADER: &str = "x-tokuly-error";

impl AppError {
    pub fn new(
        code: ErrorCode,
        reason: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            code,
            reason: reason.into(),
            message: message.into(),
            source: None,
        }
    }

    /// 400: the request is malformed or fails validation.
    pub fn invalid(
        reason: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(ErrorCode::InvalidArgument, reason, message)
    }

    /// 404: the entity does not exist or the caller may not know it exists.
    pub fn not_found(
        reason: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(ErrorCode::NotFound, reason, message)
    }

    /// 409: something with the same identity already exists.
    pub fn already_exists(
        reason: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(ErrorCode::AlreadyExists, reason, message)
    }

    /// 403: authenticated but not allowed.
    pub fn denied(
        reason: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(ErrorCode::PermissionDenied, reason, message)
    }

    /// 401: no (valid) credentials.
    pub fn unauthenticated(
        reason: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(ErrorCode::Unauthenticated, reason, message)
    }

    /// 400/412: the request is valid but the system is not in a state to accept it.
    pub fn precondition(
        reason: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(ErrorCode::FailedPrecondition, reason, message)
    }

    /// 429: rate limit or quota exceeded.
    pub fn rate_limited(
        reason: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(ErrorCode::ResourceExhausted, reason, message)
    }

    /// 503: a dependency is down; the client may retry.
    pub fn unavailable(
        reason: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(ErrorCode::Unavailable, reason, message)
    }

    /// 501: the feature is not configured on this server.
    pub fn unimplemented(
        reason: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(ErrorCode::Unimplemented, reason, message)
    }

    /// 500 with the underlying cause kept for logs (never sent to clients).
    pub fn internal(source: impl Into<anyhow::Error>) -> Self {
        Self {
            code: ErrorCode::Internal,
            reason: Cow::Borrowed("INTERNAL"),
            message: Cow::Borrowed("internal error"),
            source: Some(source.into()),
        }
    }

    pub fn is_not_found(&self) -> bool {
        self.code == ErrorCode::NotFound
    }

    pub fn is_already_exists(&self) -> bool {
        self.code == ErrorCode::AlreadyExists
    }
}

impl fmt::Debug for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "AppError({:?}, {}, {:?}",
            self.code, self.reason, self.message
        )?;
        if let Some(source) = &self.source {
            write!(f, ", source: {source:#}")?;
        }
        write!(f, ")")
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.reason, self.message)?;
        if let Some(source) = &self.source {
            write!(f, ": {source:#}")?;
        }
        Ok(())
    }
}

// `AppError` deliberately does not implement `std::error::Error` so that this
// blanket conversion can exist (the same trick `anyhow` uses).
impl<E> From<E> for AppError
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn from(err: E) -> Self {
        let any: &(dyn std::error::Error + 'static) = &err;
        if let Some(db) = any.downcast_ref::<sqlx::Error>() {
            match db {
                sqlx::Error::RowNotFound => return Self::not_found("NOT_FOUND", "not found"),
                sqlx::Error::Database(d) if d.is_unique_violation() => {
                    return Self::already_exists("ALREADY_EXISTS", "already exists");
                }
                sqlx::Error::Database(d) if d.is_foreign_key_violation() => {
                    return Self::not_found("NOT_FOUND", "referenced entity not found");
                }
                _ => {}
            }
        }
        Self::internal(anyhow::Error::new(err))
    }
}

impl From<AppError> for ConnectError {
    fn from(err: AppError) -> Self {
        if err.code == ErrorCode::Internal || err.code == ErrorCode::Unknown {
            match &err.source {
                Some(source) => {
                    tracing::error!(reason = %err.reason, error = %format!("{source:#}"), "internal error")
                }
                None => {
                    tracing::error!(reason = %err.reason, message = %err.message, "internal error")
                }
            }
        } else {
            tracing::debug!(code = ?err.code, reason = %err.reason, message = %err.message, "request rejected");
        }
        let mut out = ConnectError::new(err.code, err.message.into_owned());
        if let Ok(value) = http::HeaderValue::from_str(&err.reason) {
            out.response_headers_mut()
                .insert(REASON_HEADER, value.clone());
            out.trailers_mut().insert(REASON_HEADER, value);
        }
        out
    }
}

/// Converts `Option<T>` into a not-found error.
pub trait OrNotFound<T> {
    fn or_not_found(self, reason: &'static str, message: &'static str) -> AppResult<T>;
}

impl<T> OrNotFound<T> for Option<T> {
    fn or_not_found(self, reason: &'static str, message: &'static str) -> AppResult<T> {
        self.ok_or_else(|| AppError::not_found(reason, message))
    }
}

/// Early-returns an [`AppError`] built with one of the constructor names:
/// `bail!(invalid, "HANDLE_TAKEN", "handle {handle} is taken")`.
#[macro_export]
macro_rules! bail {
    ($kind:ident, $reason:expr, $($arg:tt)+) => {
        return Err($crate::error::AppError::$kind($reason, format!($($arg)+)).into())
    };
}

/// Returns an error unless the condition holds:
/// `ensure!(n <= 10, invalid, "TOO_MANY", "at most 10 items")`.
#[macro_export]
macro_rules! ensure {
    ($cond:expr, $kind:ident, $reason:expr, $($arg:tt)+) => {
        if !($cond) {
            return Err($crate::error::AppError::$kind($reason, format!($($arg)+)).into());
        }
    };
}
