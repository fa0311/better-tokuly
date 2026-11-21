//! Integration tests of the search domain against the development
//! infrastructure (PostgreSQL, Valkey and a real Meilisearch).
mod common;

mod admin;
mod history;
mod indexing;
mod queries;
mod safety;
mod suggest;
mod trending;
mod users;
