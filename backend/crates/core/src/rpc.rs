//! Glue between domain code (which speaks [`AppError`]) and Connect handlers
//! (which must return `ConnectError`).
//!
//! Write every handler body inside [`handle`], so `?` works on database,
//! cache and any other error, and finish with [`ok`]:
//!
//! ```ignore
//! async fn get_thing(&self, ctx: RequestContext, request: ServiceRequest<'_, GetThingRequest>) -> ServiceResult<GetThingResponse> {
//!     handle(async {
//!         let user_id = ctx.user_id()?;
//!         let thing = self.load(parse_id(request.thing_id)?, user_id).await?;
//!         ok(GetThingResponse { thing: thing.into(), ..Default::default() })
//!     })
//!     .await
//! }
//! ```
use std::future::Future;

use connectrpc::{ConnectError, Response};

use crate::error::AppResult;

/// Runs a handler body written against [`AppResult`] and converts its error
/// into the Connect error clients receive.
pub async fn handle<T>(body: impl Future<Output = AppResult<T>>) -> Result<T, ConnectError> {
    body.await.map_err(ConnectError::from)
}

/// A successful response with no extra headers.
pub fn ok<T>(body: T) -> AppResult<Response<T>> {
    Ok(Response::new(body))
}
