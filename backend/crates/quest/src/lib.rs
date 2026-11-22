//! XP and levels, missions, badges, friend streaks and leaderboards.
//!
//! The quest domain rewards using the app with friends. Nothing here is
//! "done" through an RPC: the domain listens to what every other domain
//! announces and turns it into XP, mission and badge progress and streak
//! days. The RPCs read that progress and collect rewards.
//!
//! How an event becomes a reward:
//!
//! ```text
//! EVENTS / ANALYTICS streams
//!     │  ingest.rs    the rule table (rules.rs) says what the event is worth;
//!     ▼               payloads.rs reads who did it
//! Valkey day counters (store.rs)   one script per deed: de-duplicated by
//!     │                            event ID, XP capped per source and day
//!     │  flush.rs     a few seconds later, in one transaction per player
//!     ▼
//! PostgreSQL (earnings.rs)   XP, level, counters, mission progress, badges
//!     │
//!     ▼
//! announce: level-up coins, realtime `QuestEvent`s, analytics
//! ```
//!
//! Things done towards a friend also go to `streaks`, which keeps the
//! streak between the two (PostgreSQL, once per friend and day).
//!
//! What other domains use:
//! - [`badges::award`] — give a player a badge of the catalogue.
//! - [`DirectChats`] / [`Quest::set_direct_chats`] — how the server lets
//!   this domain ask the chat domain who a direct conversation is between
//!   (optional; see the trait).
//! - [`Quest::handle_event`] / [`Quest::handle_analytics`] are the worker
//!   entry points (and what tests feed events through), with
//!   [`Quest::flush_user`] to apply what was collected right away.
//!
//! Layout: `rules.rs`, `missions/pool.rs` and `badges/catalog.rs` are the
//! data; `streaks/days.rs`, `missions/draw.rs`, `levels.rs` and `clock.rs`
//! the pure logic; one `*_svc.rs` per RPC service.
use std::{collections::HashSet, ops::Deref, sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono_tz::Tz;
use tokuly_core::prelude::*;
use tokuly_identity::Identity;
use tokuly_proto::{
    quest::{QuestEvent, quest_event::Event as Push},
    realtime::server_frame::Body,
};

pub mod badges;
pub mod clock;
pub mod config;
mod earnings;
mod export;
mod flush;
mod ingest;
mod leaderboards;
pub mod levels;
pub mod metrics;
pub mod missions;
pub mod payloads;
mod profile;
mod progress_svc;
pub mod rules;
mod seasons;
mod store;
pub mod streaks;
pub mod text;
mod workers;

pub use config::QuestConfig;

/// Who the two members of a direct conversation are.
///
/// The chat domain announces a message with its conversation, not with its
/// recipient, and a message only counts towards a streak when it is known
/// who it went to. This domain learns that from `chat.conversation.created`
/// by itself, which covers every conversation opened while it was
/// listening. Conversations older than that are unknown to it — unless the
/// server hands in a lookup backed by the chat domain (the two crates do
/// not depend on each other; the server sees both).
#[async_trait]
pub trait DirectChats: Send + Sync + 'static {
    /// The two members of `conversation_id` when it is a direct
    /// conversation; `None` for groups and unknown conversations.
    async fn members(&self, conversation_id: Uuid) -> AppResult<Option<(Uuid, Uuid)>>;
}

pub struct QuestInner {
    pub core: Core,
    pub cfg: QuestConfig,
    /// Asked about direct conversations this domain has not seen opened.
    direct_chat_lookup: parking_lot::RwLock<Option<Arc<dyn DirectChats>>>,
    /// Prefix of every quest key in Valkey (`quest:<namespace>:`).
    keyspace: String,
    scripts: store::Scripts,
    /// Time zone and privacy of accounts; `None` for accounts that do not
    /// exist any more (deleted, banned), which earn nothing.
    accounts: moka::future::Cache<Uuid, Option<Account>>,
    /// Members of direct conversations (see `quest_direct_chats`).
    direct_chats: moka::future::Cache<Uuid, (Uuid, Uuid)>,
    /// Conversations that were looked up and are not known direct ones;
    /// believed for a minute.
    unknown_chats: moka::future::Cache<Uuid, ()>,
    /// Keys of the rules that fired recently, re-read every minute.
    live_rules: moka::future::Cache<(), Arc<HashSet<&'static str>>>,
    /// Rarity of every badge, re-read every minute.
    badge_stats: moka::future::Cache<(), Arc<std::collections::HashMap<String, badges::BadgeStat>>>,
    /// When this process last recorded each rule as seen.
    rule_marks: parking_lot::Mutex<std::collections::HashMap<&'static str, std::time::Instant>>,
}

/// The quest domain. Cheap to clone.
#[derive(Clone)]
pub struct Quest(Arc<QuestInner>);

impl Deref for Quest {
    type Target = QuestInner;

    fn deref(&self) -> &QuestInner {
        &self.0
    }
}

/// How long an account's time zone is remembered before it is read again.
const ACCOUNT_TTL: Duration = Duration::from_secs(300);

/// What the quest domain needs to know about an account to credit it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Account {
    /// Defines the player's calendar day.
    pub zone: Tz,
}

impl Quest {
    /// Builds the domain and plugs it into identity: profiles gain the
    /// `quest.level` card (and `quest.streak` between friends), and the
    /// personal data export gains a `quest` section.
    pub fn new(core: Core, identity: Identity, cfg: QuestConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        let keyspace = if cfg.keyspace.is_empty() {
            derived_keyspace(&core.cfg.database_url, &core.cfg.nats_prefix)
        } else {
            cfg.keyspace.clone()
        };
        // The exporter lives inside `core`, so it must not hold a `Core` (or
        // this domain) back: that would be a reference cycle.
        core.exporters.register(Arc::new(export::QuestExporter::new(&core)));
        let quest = Self(Arc::new(QuestInner {
            core,
            cfg,
            direct_chat_lookup: parking_lot::RwLock::new(None),
            keyspace: format!("quest:{keyspace}:"),
            scripts: store::Scripts::new(),
            accounts: moka::future::Cache::builder().max_capacity(200_000).time_to_live(ACCOUNT_TTL).build(),
            direct_chats: moka::future::Cache::builder()
                .max_capacity(200_000)
                .time_to_live(Duration::from_secs(3600))
                .build(),
            unknown_chats: moka::future::Cache::builder()
                .max_capacity(200_000)
                .time_to_live(Duration::from_secs(60))
                .build(),
            live_rules: moka::future::Cache::builder().max_capacity(1).time_to_live(Duration::from_secs(60)).build(),
            badge_stats: moka::future::Cache::builder().max_capacity(1).time_to_live(Duration::from_secs(60)).build(),
            rule_marks: parking_lot::Mutex::new(std::collections::HashMap::new()),
        }));
        identity.add_profile_extension(Arc::new(quest.clone()));
        Ok(quest)
    }

    /// Registers this domain's RPC services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::quest::{
            BadgeServiceExt, LeaderboardServiceExt, MissionServiceExt, QuestAdminServiceExt, QuestServiceExt,
            StreakServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = QuestServiceExt::register(me.clone(), router);
        let router = MissionServiceExt::register(me.clone(), router);
        let router = BadgeServiceExt::register(me.clone(), router);
        let router = StreakServiceExt::register(me.clone(), router);
        let router = LeaderboardServiceExt::register(me.clone(), router);
        QuestAdminServiceExt::register(me, router)
    }

    /// Hands in the lookup for direct conversations (see [`DirectChats`]).
    /// Call once at startup.
    pub fn set_direct_chats(&self, lookup: Arc<dyn DirectChats>) {
        *self.direct_chat_lookup.write() = Some(lookup);
    }

    pub(crate) fn direct_chat_lookup(&self) -> Option<Arc<dyn DirectChats>> {
        self.direct_chat_lookup.read().clone()
    }

    /// What is known about an account that can earn: `None` when it does
    /// not exist, was deleted or is banned. Remembered for a few minutes.
    pub(crate) async fn account_of(&self, user_id: Uuid) -> AppResult<Option<Account>> {
        if let Some(account) = self.accounts.get(&user_id).await {
            return Ok(account);
        }
        let zone: Option<String> = sqlx::query_scalar(
            "SELECT time_zone FROM users WHERE id = $1 AND deleted_at IS NULL AND banned_at IS NULL",
        )
        .bind(user_id)
        .fetch_optional(&self.core.db)
        .await?;
        let account = zone.map(|zone| Account { zone: clock::parse_zone(&zone) });
        self.accounts.insert(user_id, account).await;
        Ok(account)
    }

    /// The time zone of an account that can earn (see [`Self::account_of`]).
    pub(crate) async fn zone_of(&self, user_id: Uuid) -> AppResult<Option<Tz>> {
        Ok(self.account_of(user_id).await?.map(|account| account.zone))
    }

    /// The caller's time zone, read fresh: an RPC should follow a zone the
    /// player changed a moment ago.
    pub(crate) async fn caller_zone(&self, user_id: Uuid) -> AppResult<Tz> {
        let zone: Option<String> = sqlx::query_scalar("SELECT time_zone FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.core.db)
            .await?;
        let account = Account { zone: zone.map_or(clock::DEFAULT_ZONE, |zone| clock::parse_zone(&zone)) };
        self.accounts.insert(user_id, Some(account)).await;
        Ok(account.zone)
    }

    /// Forgets what is cached about an account (it was deleted or changed).
    pub(crate) async fn forget_account(&self, user_id: Uuid) {
        self.accounts.invalidate(&user_id).await;
    }

    /// Pushes a quest event to every connected device of `user_id`.
    pub(crate) async fn push(&self, user_id: Uuid, event: Push) {
        push(&self.core, user_id, event).await;
    }
}

/// Pushes a quest event to every connected device of `user_id`.
pub(crate) async fn push(core: &Core, user_id: Uuid, event: Push) {
    let event = QuestEvent { event: Some(event), ..Default::default() };
    core.rt.notify_user(user_id, Body::Quest(Box::new(event))).await;
}

/// A short, stable name for "this database on this bus": FNV-1a over both,
/// so every process of one deployment computes the same.
fn derived_keyspace(database_url: &str, nats_prefix: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in database_url.bytes().chain([b'|']).chain(nats_prefix.bytes()) {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployments_get_their_own_keyspace() {
        let a = derived_keyspace("postgres://tokuly@db/tokuly", "");
        assert_eq!(a, derived_keyspace("postgres://tokuly@db/tokuly", ""));
        assert_ne!(a, derived_keyspace("postgres://tokuly@db/tokuly_test_1", ""));
        assert_ne!(a, derived_keyspace("postgres://tokuly@db/tokuly", "T1"));
        assert_eq!(a.len(), 16);
    }
}
