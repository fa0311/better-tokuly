//! Stamps (スタンプ): sticker packs, the store, each user's library and the
//! usage log.
//!
//! # For other domains
//!
//! Chat, live chat, comments, notes and map reactions embed stamps as
//! `tokuly.common.v1.StampRef` and go through three functions:
//!
//! - [`ensure_usable`] — may this user send this stamp? Returns the
//!   `StampRef` to embed, or `STAMP_NOT_USABLE`.
//! - [`record_send`] — the stamp was sent in a [`SendContext`]: feeds the
//!   usage log, "recently used", rankings and creator statistics.
//! - [`stamp_refs`] — resolves stored stamp IDs for display.
//!
//! # Map of the crate
//!
//! | module | what lives there |
//! |---|---|
//! | [`access`] | every rule about who may see, send and edit what |
//! | [`validate`] | validation of names, tags, keywords, prices |
//! | [`model`] | database rows and their API representations |
//! | [`packs`], [`stamps`], [`creators`] | loading and changing the three core entities |
//! | [`lifecycle`] | pack status transitions (submit, publish, reject, unpublish, take down) |
//! | [`store`], [`installs`], [`purchases`], [`collections`], [`personal`], [`stats`] | the queries and flows behind the RPCs |
//! | `*_svc` | the four Connect services: thin handlers over the modules above |
//! | [`usage`] | the cross-domain API |
//! | [`workers`], [`cleanup`] | background workers; what happens when accounts, uploads or content disappear |
//! | [`subjects`] | subject resolvers (likes, comments, reports) and data export |
//! | [`official`] | the built-in official packs, drawn in code |
use std::{ops::Deref, sync::Arc};

use tokuly_core::Core;
use tokuly_identity::Identity;

pub mod access;
pub mod cleanup;
pub mod collections;
pub mod config;
pub mod creators;
pub mod events;
pub mod installs;
pub mod lifecycle;
pub mod model;
pub mod official;
pub mod packs;
pub mod personal;
pub mod purchases;
pub mod stamps;
pub mod stats;
pub mod store;
pub mod subjects;
pub mod usage;
pub mod validate;
pub mod workers;

mod library_svc;
mod review_svc;
mod store_svc;
mod studio_svc;

pub use config::StampConfig;
pub use usage::{SendContext, ensure_usable, record_send, stamp_refs};

pub struct StampInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: StampConfig,
}

/// The stamp domain. Cheap to clone.
#[derive(Clone)]
pub struct Stamp(Arc<StampInner>);

impl Deref for Stamp {
    type Target = StampInner;

    fn deref(&self) -> &StampInner {
        &self.0
    }
}

impl Stamp {
    pub fn new(core: Core, identity: Identity, cfg: StampConfig) -> anyhow::Result<Self> {
        anyhow::ensure!(
            cfg.fee_percent <= 100,
            "TOKULY_STAMP_FEE_PERCENT must be between 0 and 100, got {}",
            cfg.fee_percent
        );
        anyhow::ensure!(
            cfg.max_packs > 0 && cfg.max_personal_stamps > 0,
            "the stamp pack and personal stamp limits must be positive"
        );
        subjects::register(&core);
        Ok(Self(Arc::new(StampInner { core, identity, cfg })))
    }

    /// Registers the four stamp services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::stamp::{
            StampLibraryServiceExt, StampReviewServiceExt, StampStoreServiceExt, StampStudioServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = StampStoreServiceExt::register(me.clone(), router);
        let router = StampLibraryServiceExt::register(me.clone(), router);
        let router = StampStudioServiceExt::register(me.clone(), router);
        StampReviewServiceExt::register(me, router)
    }
}
