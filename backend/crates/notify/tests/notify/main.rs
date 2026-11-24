//! Integration tests of the notify domain against real PostgreSQL, Valkey
//! and NATS (one test binary, so the modules share a database).
mod blocks;
mod channels;
mod common;
mod concurrency;
mod consumer;
mod consumer_preferences;
mod consumer_urgent;
mod delivery;
mod devices;
mod inbox;
mod inbox_changes;
mod maintenance;
mod mutes;
mod preferences;
mod withdrawals;
