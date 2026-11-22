//! Study sessions.
//!
//! - `sessions`: the session row and how study time is counted and logged.
//! - `focus`: the timer the user drives (start, pause, resume, skip, stop,
//!   heartbeat) and the sweep that ends sessions nobody is behind anymore.
//! - `log`: logging by hand, editing, deleting and listing sessions.
//! - `stats`: totals per day and subject, the goal and streak, the heatmap,
//!   the weekly ranking.
//! - `presence`: who is studying right now, and cheers.
//! - `svc`: the RPC handlers.
pub(crate) mod focus;
mod log;
mod presence;
pub(crate) mod sessions;
mod stats;
mod svc;
