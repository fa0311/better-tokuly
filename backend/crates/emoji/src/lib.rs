//! Custom emoji (カスタム絵文字): small images with a `:shortcode:` that work
//! in text, display names and reactions everywhere, like on Misskey.
//!
//! # For other domains
//!
//! Anything that shows text resolves emoji through [`catalog`]:
//!
//! - [`catalog::exists`] — is there an emoji with this name?
//! - [`catalog::resolve`] — names → images, for rendering;
//! - [`catalog::lookup`] — names → whole catalogue entries;
//! - [`catalog::record_use`] — a user used these emoji.
//!
//! All four are answered from the cached catalogue and are cheap enough to
//! call on every request.
//!
//! # Map of the crate
//!
//! | module | what lives there |
//! |---|---|
//! | [`validate`] | validation and normalization of names, aliases, notes, queries, deck items |
//! | [`model`] | the emoji row, its API form, pictures, and who may see what |
//! | [`emojis`] | the life of an emoji: create, approve, reject, disable, edit, delete |
//! | [`categories`] | the managed list of categories |
//! | [`catalog`] | the approved set: versions, the cache, search, the cross-domain API |
//! | [`usage`], [`deck`] | recently used emoji, use counters and popularity; reaction decks |
//! | [`pack`], [`import`], [`export`] | Misskey's pack format; bringing packs in and writing them out |
//! | `*_svc` | the three Connect services: thin handlers over the modules above |
//! | [`workers`] | event consumers, jobs and periodic upkeep |
//! | [`subjects`] | subject resolver (reports), data export, profile card |
use std::{ops::Deref, sync::Arc};

use tokuly_core::Core;
use tokuly_identity::Identity;

pub mod catalog;
pub mod categories;
pub mod config;
pub mod deck;
pub mod emojis;
pub mod events;
pub mod export;
pub mod import;
pub mod model;
pub mod pack;
pub mod subjects;
pub mod usage;
pub mod validate;
pub mod workers;

mod admin_svc;
mod catalog_svc;
mod submission_svc;

pub use config::EmojiConfig;

pub struct EmojiInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: EmojiConfig,
}

/// The emoji domain. Cheap to clone.
#[derive(Clone)]
pub struct Emoji(Arc<EmojiInner>);

impl Deref for Emoji {
    type Target = EmojiInner;

    fn deref(&self) -> &EmojiInner {
        &self.0
    }
}

impl Emoji {
    pub fn new(core: Core, identity: Identity, cfg: EmojiConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        subjects::register(&core, &identity);
        Ok(Self(Arc::new(EmojiInner { core, identity, cfg })))
    }

    /// Registers the three emoji services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::emoji::{EmojiAdminServiceExt, EmojiServiceExt, EmojiSubmissionServiceExt};
        let me = Arc::new(self.clone());
        let router = EmojiServiceExt::register(me.clone(), router);
        let router = EmojiSubmissionServiceExt::register(me.clone(), router);
        EmojiAdminServiceExt::register(me, router)
    }
}
