//! Integration tests of the quest domain against real PostgreSQL, Valkey,
//! NATS and ClickHouse (one test binary, so the modules share a database).
//!
//! Events are fed through `Quest::handle_event` / `handle_analytics` the way
//! the consumers would, and `Quest::flush_user` writes what was collected,
//! as the flush worker does a few seconds later.
mod badges;
mod boards;
mod common;
mod concurrency;
mod lifecycle;
mod missions;
mod privacy;
mod streaks;
mod xp;
