//! Integration tests of the developer platform against real PostgreSQL,
//! Valkey, NATS and ClickHouse (one test binary, so the modules share a
//! database). Identity's services are registered next to this domain's, so
//! tokens are exercised against RPCs of another domain through the real
//! auth middleware.
mod apps;
mod bots;
mod cleanup;
mod common;
mod concurrency;
mod hardening;
mod mini_apps;
mod oauth;
mod reference;
mod secret_rotation;
mod tokens;
mod transfers;
mod webhooks;
