//! Administration and moderation.
//!
//! A map of the crate, by file:
//!
//! - **Moderation** (`moderation_svc.rs`): reports from the engagement domain
//!   are grouped into cases (`intake.rs`, `cases.rs`); moderators resolve
//!   them by dismissing, taking content down, warning, suspending or banning
//!   (`resolution.rs`, `sanctions.rs`). Every action lands in the moderation
//!   log (`actions.rs`), and whatever users are told is worded in `notices.rs`.
//! - **Accounts** (`users_svc.rs`, `accounts.rs`): search, the staff view of
//!   an account, sanctions, roles, the verified badge.
//! - **Feature flags** (`flags_svc.rs`), **announcements** — the product
//!   update log (`announcements_svc.rs`, [`announcements`]) — and **system**
//!   stats plus the audit log (`system_svc.rs`, `stats.rs`, `audit.rs`).
//! - **Background work** (`workers.rs`) and personal-data handling
//!   (`cleanup.rs`).
//!
//! Every RPC except the public announcement reads is gated on the
//! `moderator` or `admin` role (`staff.rs`), and every change is written to
//! the audit log in the transaction that makes it.
//!
//! Other domains never call into this crate. They hear about takedowns
//! through the `admin.content.removed` event ([`events::ContentRemoved`]) and
//! about account locks through identity's own events. See
//! `docs/events/admin.md`.
use std::{ops::Deref, sync::Arc};

use tokuly_core::prelude::*;
use tokuly_identity::Identity;

mod accounts;
mod actions;
pub mod announcements;
mod announcements_svc;
mod audit;
mod cases;
mod cleanup;
pub mod config;
pub mod events;
mod flags_svc;
mod intake;
mod moderation_svc;
mod notices;
mod resolution;
mod sanctions;
mod staff;
mod stats;
mod system_svc;
mod users_svc;
mod workers;

pub use config::AdminConfig;

/// The first `limit` rows of a listing, for previews embedded in other responses.
pub(crate) fn first_page(limit: i64) -> Page {
    Page { limit, cursor: None }
}

pub struct AdminInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: AdminConfig,
}

/// The admin domain. Cheap to clone.
#[derive(Clone)]
pub struct Admin(Arc<AdminInner>);

impl Deref for Admin {
    type Target = AdminInner;

    fn deref(&self) -> &AdminInner {
        &self.0
    }
}

impl Admin {
    pub fn new(core: Core, identity: Identity, cfg: AdminConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        core.exporters.register(Arc::new(cleanup::ModerationExporter { core: core.clone() }));
        Ok(Self(Arc::new(AdminInner { core, identity, cfg })))
    }

    /// Registers this domain's RPC services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::admin::{AnnouncementServiceExt, FeatureFlagServiceExt, ModerationServiceExt, SystemServiceExt, UserAdminServiceExt};
        let me = Arc::new(self.clone());
        let router = ModerationServiceExt::register(me.clone(), router);
        let router = UserAdminServiceExt::register(me.clone(), router);
        let router = FeatureFlagServiceExt::register(me.clone(), router);
        let router = AnnouncementServiceExt::register(me.clone(), router);
        SystemServiceExt::register(me, router)
    }

    /// First-deployment bootstrap: grants the `admin` role to the account
    /// named by `TOKULY_ADMIN_USER`. The server calls this at startup and the
    /// worker calls it whenever somebody signs up, so the owner of a fresh
    /// deployment only has to create their account.
    ///
    /// Returns `true` when the role was granted by this call.
    pub async fn grant_bootstrap_admin(&self) -> AppResult<bool> {
        self.ensure_admin(&self.cfg.bootstrap_admin).await
    }

    /// Grants the `admin` role to an existing account, identified by handle
    /// or email address.
    ///
    /// Returns `true` when the role was granted by this call, `false` when
    /// the account already was an admin or does not exist (yet).
    pub async fn ensure_admin(&self, handle_or_email: &str) -> AppResult<bool> {
        let identifier = handle_or_email.trim();
        if identifier.is_empty() {
            return Ok(false);
        }
        let Some(user) = tokuly_identity::users::find_by_identifier(&self.core, identifier).await? else {
            tracing::debug!(identifier, "bootstrap admin: no such account yet; it is promoted when it signs up");
            return Ok(false);
        };
        if user.roles.iter().any(|r| r == "admin") {
            return Ok(false);
        }
        let mut roles = user.roles.clone();
        roles.push("admin".to_owned());
        self.identity.set_roles(user.id, &roles).await?;
        audit::Audit::system("user.roles_changed")
            .user(user.id)
            .reason("bootstrap admin from server configuration")
            .before(serde_json::json!({ "roles": user.roles }))
            .after(serde_json::json!({ "roles": roles }))
            .record(&self.core.db)
            .await?;
        tracing::info!(user_id = %user.id, handle = %user.handle, "bootstrap admin: admin role granted");
        Ok(true)
    }
}
