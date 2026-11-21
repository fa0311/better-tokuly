//! Shared foundation for every Tokuly backend crate.
//!
//! - [`Core`] bundles the infrastructure clients (database, cache, bus,
//!   storage, tokens, mail, analytics, search).
//! - [`error`] defines [`AppError`], which converts into Connect errors.
//! - [`ctx`] gives RPC handlers the caller identity.
//! - [`events`] / [`jobs`] / [`realtime`] are the three ways domains talk to
//!   each other and to clients asynchronously.
pub mod analytics;
pub mod auth;
pub mod bus;
pub mod cache;
pub mod config;
mod core;
pub mod ctx;
pub mod db;
pub mod error;
pub mod events;
pub mod export;
pub mod flags;
pub mod geoindex;
pub mod ids;
pub mod jobs;
pub mod keys;
pub mod mail;
pub mod outbound;
pub mod page;
pub mod presence;
pub mod proxies;
pub mod realtime;
pub mod rpc;
pub mod storage;
pub mod subject;
pub mod telemetry;
pub mod time;

pub use crate::core::{Core, CoreInner};
pub use crate::error::{AppError, AppResult};

/// The handful of names nearly every domain module needs.
pub mod prelude {
    pub use crate::auth::AuthContext;
    pub use crate::core::Core;
    pub use crate::ctx::CtxExt as _;
    pub use crate::error::{AppError, AppResult, OrNotFound as _};
    pub use crate::ids::{new_id, parse_id, parse_ids, parse_opt_id};
    pub use crate::page::Page;
    pub use crate::rpc::{handle, ok};
    pub use crate::time::{Time, from_ts, now, opt_ts, ts};
    pub use crate::{bail, ensure};
    pub use uuid::Uuid;
}
