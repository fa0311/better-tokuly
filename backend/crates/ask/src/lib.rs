//! Anonymous question boxes (質問箱), prompts (お題) and compliment polls
//! (ほめ投票).
//!
//! # Anonymity
//!
//! Three kinds of things are sent anonymously: questions, prompt responses
//! and compliment votes ("flames"). For all three:
//!
//! - who sent it is **stored** (for moderators), and **never returned** to
//!   whoever receives it, unless the sender signed. The row types the
//!   recipient's queries use select the sender's ID only for signed items
//!   and compare it with the viewer inside the database;
//! - the recipient can **block "whoever sent this"** without learning who it
//!   is. From then on that sender's submissions are accepted exactly like
//!   anyone's and dropped: same limits, same row, same event, same response
//!   (`senders.rs`);
//! - the sender sees of what they sent only "waiting" or "answered";
//! - what a recipient has does **not follow the sender's account**. An
//!   anonymous item that vanished when someone was suspended, or a flame
//!   whose hints changed when someone switched them off, would point at
//!   that person. Items become visible when the delivery worker lets them
//!   through and stay as they were from then on (`delivery.rs`,
//!   `cleanup.rs`); hints are fixed when the vote is cast (`polls/hints.rs`);
//! - anonymity needs a **crowd**: where only friends can send, the
//!   recipient needs three of them before anything anonymous is accepted
//!   (`senders.rs`).
//!
//! # Map of the crate
//!
//! | module | what lives there |
//! |---|---|
//! | [`senders`] | the rules every submission passes (limits, word filter, drop) and sender blocks |
//! | [`questions`], [`answers`] | the question box: rows, who sees what, lists; writing answers and what a publication sets off |
//! | [`prompts`] | prompts and their responses |
//! | [`polls`] | compliment polls: bank, fair draw, rounds, flames, hints |
//! | [`friends`] | the two facts about friendships this domain keeps its own copy of (mutes, since when) |
//! | [`settings`], [`filter`], [`text`], [`config`] | per-user settings, the word filter, text validation, deployment settings |
//! | [`delivery`] | the worker that lets what was sent through to its recipient and tells them, after the sender's request is over |
//! | [`cleanup`] | reactions to deleted and banned accounts, blocks, takedowns and deleted uploads; the purge |
//! | [`workers`] | the event consumer and the purge timer |
//! | [`subjects`], [`profile`], [`export`] | what this domain plugs into comments / likes / reports, profiles, and the personal data export |
//! | [`events`] | the events this domain emits and consumes |
//! | `*_svc` | the Connect services: thin handlers over the modules above; `moderation_svc` is the one place that names a sender |
//!
//! # Connections
//!
//! No code here asks the pool for a connection while it holds a
//! transaction: whatever a transaction needs is read before it begins.
//! (A request that waits for a second connection while holding one is
//! harmless alone and stalls the whole service when a pool's worth of them
//! do it at once; `polls/rounds.rs` has the case that made this a rule.)
use std::{ops::Deref, sync::Arc};

use tokuly_core::prelude::*;
use tokuly_identity::{Identity, UserRow, users};
use tokuly_proto::{
    ask::{AskEvent, ask_event::Event as Push},
    realtime::server_frame::Body,
};
use tokuly_social::graph;

mod answers;
mod box_svc;
mod cleanup;
mod compliment_svc;
pub mod config;
mod delivery;
pub mod events;
mod export;
mod filter;
mod friends;
mod moderation_svc;
mod polls;
mod profile;
mod prompt_svc;
mod prompts;
mod questions;
mod senders;
mod settings;
mod subjects;
mod text;
mod workers;

pub use config::AskConfig;

use crate::filter::WordFilter;

pub struct AskInner {
    pub core: Core,
    pub cfg: AskConfig,
    /// The server-wide muted words (`TOKULY_ASK_FILTER_WORDS`).
    pub(crate) filter: WordFilter,
    /// Price of the first and the second hint; empty when none are sold.
    pub(crate) hint_prices: Vec<i64>,
    /// Set once the built-in poll questions are in the database.
    pub(crate) bank_ready: tokio::sync::OnceCell<()>,
}

/// The ask domain. Cheap to clone.
#[derive(Clone)]
pub struct Ask(Arc<AskInner>);

impl Deref for Ask {
    type Target = AskInner;

    fn deref(&self) -> &AskInner {
        &self.0
    }
}

impl Ask {
    /// Builds the domain and plugs it into the others: questions, prompts
    /// and prompt responses become subjects (comments, likes, reports,
    /// takedowns), profiles gain the question box card, and the personal
    /// data export gains an `ask` section.
    pub fn new(core: Core, identity: Identity, cfg: AskConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        let filter = WordFilter::new(cfg.filter_word_list());
        let hint_prices = cfg.hint_price_list()?;
        let ask = Self(Arc::new(AskInner {
            core: core.clone(),
            cfg,
            filter,
            hint_prices,
            bank_ready: tokio::sync::OnceCell::new(),
        }));
        subjects::register(&core);
        core.exporters.register(Arc::new(export::AskExporter::new(&core, ask.retention_days())));
        identity.add_profile_extension(Arc::new(ask.clone()));
        Ok(ask)
    }

    /// Registers this domain's RPC services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::ask::{
            AskModerationServiceExt, ComplimentServiceExt, PromptServiceExt, QuestionBoxServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = QuestionBoxServiceExt::register(me.clone(), router);
        let router = PromptServiceExt::register(me.clone(), router);
        let router = ComplimentServiceExt::register(me.clone(), router);
        AskModerationServiceExt::register(me, router)
    }

    /// Pushes an ask event to every connected device of one user.
    pub(crate) async fn push(&self, user_id: Uuid, event: Push) {
        self.core
            .rt
            .notify_user(user_id, Body::Ask(Box::new(AskEvent { event: Some(event), ..Default::default() })))
            .await;
    }

    /// The account named by `reference` (a user ID or a handle), as far as
    /// `viewer` may know of it: active, and with no block between the two.
    pub(crate) async fn visible_user(&self, reference: &str, viewer: Option<Uuid>) -> AppResult<Option<UserRow>> {
        let reference = reference.trim();
        let user = match Uuid::parse_str(reference) {
            Ok(id) => users::find_by_id(&self.core, id).await?,
            Err(_) => users::find_by_handle(&self.core, reference).await?,
        };
        let Some(user) = user.filter(UserRow::is_active) else { return Ok(None) };
        if let Some(viewer) = viewer
            && viewer != user.id
            && graph::is_blocked_either(&self.core, viewer, user.id).await?
        {
            return Ok(None);
        }
        Ok(Some(user))
    }
}
