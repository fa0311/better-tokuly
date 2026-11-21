//! Unified search: indexing every domain's public documents into Meilisearch
//! and answering queries, typeahead, trending and search history.
//!
//! Other domains never call this crate. They publish what everyone may find
//! with `tokuly_core::outbound::{index, unindex}` inside their own
//! transaction, and the consumer here picks the change up from the event
//! stream:
//!
//! ```text
//! search.changed ─┐
//! identity.user.* ─┼─▶ consumer ─▶ search_documents ─▶ indexer ─▶ Meilisearch
//! admin.content.* ─┘   (consumer.rs)   (registry.rs)   (indexer.rs)   one index per kind
//! ```
//!
//! **The registry is the truth, the engine is a copy.** Every document is
//! first stored in PostgreSQL (`search_documents`) together with the ID of
//! the event that produced it. Event IDs are time-ordered, so an event older
//! than what is stored is dropped: redeliveries change nothing and a late
//! upsert cannot undo a newer delete. The indexer then writes whatever is
//! pending to Meilisearch in batches, one process at a time, and waits for
//! the engine's tasks before marking rows as written. Because the registry
//! holds the documents, an index can be rebuilt or repaired without asking
//! the owning domains for anything.
//!
//! **One index per kind** (`<prefix>_user`, `<prefix>_video`, ...), not one
//! index with a `kind` filter. Kinds differ by orders of magnitude in size
//! and write rate, and Meilisearch indexes one batch per index at a time:
//! with separate indexes a burst of notes does not delay user typeahead, a
//! settings change or a rebuild touches one kind, and a tab searches only
//! its own data. The price would be the "all" tab, but federated
//! multi-search merges the indexes in one request, per-kind counts included
//! (a merged facet on `kind`), and since every index has the same ranking
//! rules the merge honours the shared `rank` attribute across kinds.
//!
//! Layout:
//! - [`kinds`], [`document`], [`text`] — what a document is and how text is
//!   cleaned, ranked and highlighted.
//! - [`registry`], [`indexer`], [`indexes`] — the write path.
//! - [`consumer`], [`users`] — events in; users are indexed by this crate
//!   from identity's data.
//! - `query`, `search_svc` — the read path (`SearchService`).
//! - [`history`], [`trending`] — recent searches, trending queries,
//!   completions and the blocklist.
//! - `admin_svc` — `SearchAdminService`.
//! - [`maintenance`] — workers, jobs, sweeps, account deletion, data export.
use std::{ops::Deref, sync::Arc};

use tokuly_core::prelude::*;
use tokuly_identity::Identity;

mod admin_svc;
pub mod config;
pub mod consumer;
pub mod document;
pub mod events;
pub mod history;
pub mod indexer;
pub mod indexes;
pub mod kinds;
pub mod maintenance;
mod query;
pub mod registry;
mod search_svc;
pub mod text;
pub mod trending;
pub mod users;

pub use config::SearchConfig;

pub struct SearchInner {
    pub core: Core,
    pub cfg: SearchConfig,
    /// Wakes this process's indexer as soon as a change is stored.
    pub(crate) indexer_wake: tokio::sync::Notify,
    /// The moderators' blocklist, cached for a few seconds.
    pub(crate) blocklist: trending::BlocklistCache,
}

/// The search domain. Cheap to clone.
#[derive(Clone)]
pub struct Search(Arc<SearchInner>);

impl Deref for Search {
    type Target = SearchInner;

    fn deref(&self) -> &SearchInner {
        &self.0
    }
}

impl Search {
    /// `_identity` is unused: user data comes from `tokuly_identity`'s
    /// functions, which only need the database. The parameter keeps the
    /// constructor shape every domain shares.
    pub fn new(core: Core, _identity: Identity, cfg: SearchConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        core.exporters.register(Arc::new(maintenance::SearchExporter { core: core.clone() }));
        Ok(Self(Arc::new(SearchInner {
            core,
            cfg,
            indexer_wake: tokio::sync::Notify::new(),
            blocklist: trending::BlocklistCache::default(),
        })))
    }

    /// Registers this domain's RPC services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::search::{SearchAdminServiceExt, SearchServiceExt};
        let me = Arc::new(self.clone());
        let router = SearchServiceExt::register(me.clone(), router);
        SearchAdminServiceExt::register(me, router)
    }

    /// One-time and idempotent setup, called by the server's migrate step:
    /// creates the indexes and applies their settings.
    pub async fn setup(&self) -> AppResult<()> {
        self.ensure_indexes().await
    }
}
