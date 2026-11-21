//! News: articles collected from RSS/Atom feeds and ranked for each reader.
//!
//! ```text
//! news_sources ──(scheduler / RefreshNewsSources)──▶ feed.news.fetch job
//!                                                        │ refresh.rs
//!        guard.rs + fetch.rs: bounded, SSRF-safe, conditional GET
//!                                                        │
//!        parse.rs: feed-rs → articles (text.rs strips markup, tagging.rs tags)
//!                                                        │
//!        news_articles (one row per canonical URL) ──▶ search.changed
//!
//! readers: svc.rs ── clicks / feedback ──▶ news_tag_affinity (decaying)
//!                 ── ListNews ──▶ ranking.rs (interests + random picks)
//! ```
//!
//! Sources are managed by admins (`sources.rs`, `NewsAdminService`) and
//! seeded from `TOKULY_NEWS_FEEDS`.
pub mod articles;
pub mod fetch;
pub mod guard;
pub mod parse;
pub mod ranking;
pub mod refresh;
pub mod sources;
mod svc;
pub mod tagging;
pub mod text;
mod thumbnails;
