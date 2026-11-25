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
//! - **Server settings** (`settings_svc.rs`): the console layer of
//!   `tokuly_core::settings` (「サーバー設定」).
//! - **Background work** (`workers.rs`) and personal-data handling
//!   (`cleanup.rs`).
//!
//! Every RPC except the public announcement reads is gated on the
//! `moderator` or `admin` role (`staff.rs`), and every change is written to
//! the audit log in the transaction that makes it.
//!
//! Content that is reported without an owner (posts in anonymous channels)
//! gets one from a lookup the server hands in (`owners.rs`), and the uploads
//! of reported content are held for the case through another
//! ([`evidence`]).
//!
//! Other domains never call into this crate. They hear about takedowns
//! through the `admin.content.removed` event ([`events::ContentRemoved`]) and
//! about account locks through identity's own events. See
//! `docs/events/admin.md`.
use std::{
    ops::Deref,
    sync::{Arc, OnceLock},
};

use tokuly_core::prelude::*;
use tokuly_identity::Identity;

mod accounts;
mod actions;
pub mod announcements;
mod announcements_svc;
mod audit;
mod cases;
mod cleanup;
pub mod evidence;
pub mod config;
pub mod events;
mod flags_svc;
mod intake;
mod moderation_svc;
mod notices;
mod owners;
pub mod reported;
mod resolution;
mod sanctions;
mod settings_svc;
mod staff;
mod stats;
mod system_svc;
mod users_svc;
mod workers;

pub use config::AdminConfig;
pub use evidence::ReportEvidence;
pub use owners::HiddenOwners;
pub use reported::ReportedSubjects;

/// The day (UTC) of a moment, for what is shown about an anonymous post
/// without its author: an exact time would line up with the one account
/// suspended or warned at that moment.
pub(crate) fn day_of(at: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    use chrono::DurationRound as _;
    at.duration_trunc(chrono::TimeDelta::days(1)).unwrap_or(at)
}

/// The first `limit` rows of a listing, for previews embedded in other responses.
pub(crate) fn first_page(limit: i64) -> Page {
    Page { limit, cursor: None }
}

pub struct AdminInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: AdminConfig,
    /// Asked about content that is reported without an owner.
    hidden_owners: OnceLock<Arc<dyn HiddenOwners>>,
    /// Holds the uploads of reported content.
    evidence: OnceLock<Arc<dyn ReportEvidence>>,
    /// Resolves reported content the way its reporters see it.
    reported: OnceLock<Arc<dyn ReportedSubjects>>,
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
        Ok(Self(Arc::new(AdminInner { core, identity, cfg, hidden_owners: OnceLock::new(), evidence: OnceLock::new(), reported: OnceLock::new() })))
    }

    /// Registers this domain's RPC services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::admin::{
            AnnouncementServiceExt, FeatureFlagServiceExt, ModerationServiceExt, ServerSettingsServiceExt, SystemServiceExt, UserAdminServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = ModerationServiceExt::register(me.clone(), router);
        let router = UserAdminServiceExt::register(me.clone(), router);
        let router = FeatureFlagServiceExt::register(me.clone(), router);
        let router = AnnouncementServiceExt::register(me.clone(), router);
        let router = ServerSettingsServiceExt::register(me.clone(), router);
        SystemServiceExt::register(me, router)
    }

    /// First-deployment bootstrap: grants the `admin` role to the account
    /// whose **verified** e-mail address is `TOKULY_ADMIN_USER`. The server
    /// calls this at startup and the worker whenever somebody signs up or
    /// confirms an address, so the owner of a fresh deployment only has to
    /// create their account and open the verification link.
    ///
    /// It happens once per deployment and address, recorded in the audit log
    /// (`admin.bootstrapped`): whoever holds the address later (after the
    /// owner moved to another one) is never promoted by it, and a demoted
    /// bootstrap admin is not promoted again at the next start. A handle is
    /// refused: an unverified name would go to whoever claimed it first.
    ///
    /// Returns `true` when the role was granted by this call.
    pub async fn grant_bootstrap_admin(&self) -> AppResult<bool> {
        let address = self.cfg.bootstrap_admin.trim().to_ascii_lowercase();
        if address.is_empty() {
            return Ok(false);
        }
        if !address.contains('@') || address.starts_with('@') {
            tracing::warn!(value = %address, "TOKULY_ADMIN_USER must be an e-mail address; nobody is promoted");
            return Ok(false);
        }
        // The record and the grant are one transaction: either the account is
        // admin and that is on record, or neither (and a later call tries
        // again). Everything here runs on that transaction's connection.
        let mut tx = self.core.db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(format!("admin.bootstrap:{address}"))
            .execute(&mut *tx)
            .await?;
        let done: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM admin_audit_log WHERE target_type = 'bootstrap' AND target_id = $1 AND action = 'admin.bootstrapped')",
        )
        .bind(&address)
        .fetch_one(&mut *tx)
        .await?;
        if done {
            return Ok(false);
        }
        let owner: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM users WHERE email = $1::citext AND email_verified_at IS NOT NULL AND deleted_at IS NULL AND banned_at IS NULL",
        )
        .bind(&address)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(owner) = owner else {
            tracing::debug!("bootstrap admin: no verified account with that address yet");
            return Ok(false);
        };
        audit::Audit::system("admin.bootstrapped")
            .bootstrap(&address, owner)
            .reason("first admin of this deployment, from TOKULY_ADMIN_USER")
            .record(&mut *tx)
            .await?;
        let granted = self.grant_admin_on(&mut tx, owner, "bootstrap admin from server configuration").await?;
        tx.commit().await?;
        Ok(granted)
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
            tracing::debug!(identifier, "ensure admin: no such account");
            return Ok(false);
        };
        self.ensure_admin_by_id(user.id).await
    }

    async fn ensure_admin_by_id(&self, user_id: Uuid) -> AppResult<bool> {
        let mut tx = self.core.db.begin().await?;
        let granted = self.grant_admin_on(&mut tx, user_id, "admin granted from the server (tokuly promote-admin)").await?;
        tx.commit().await?;
        Ok(granted)
    }

    /// Adds the `admin` role and its audit entry on `tx`; `false` when the
    /// account already is an admin or is gone.
    async fn grant_admin_on(&self, tx: &mut sqlx::PgConnection, user_id: Uuid, reason: &str) -> AppResult<bool> {
        let Some((before, after)) = tokuly_identity::users::add_role(&mut *tx, user_id, "admin").await? else {
            return Ok(false);
        };
        audit::Audit::system("user.roles_changed")
            .user(user_id)
            .reason(reason)
            .before(serde_json::json!({ "roles": before }))
            .after(serde_json::json!({ "roles": after }))
            .record(&mut *tx)
            .await?;
        tracing::info!(%user_id, "admin role granted");
        Ok(true)
    }
}
