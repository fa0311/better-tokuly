//! Account sanctions: warnings, suspensions and bans.
//!
//! Identity owns the lock on an account (`users.banned_at`, changed through
//! `Identity::set_banned`); this module adds who imposed a suspension, why and
//! until when (`user_suspensions`), writes the moderation and audit logs, and
//! lifts timed suspensions when they run out.
//!
//! Identity commits `set_banned` in its own transaction, so a suspension is
//! applied in this order: take the per-account sanction lock, call identity,
//! then record our rows. If recording fails the account is locked without a
//! record, which a retry of the same request repairs (`set_banned` is
//! idempotent). The opposite order could leave a record claiming a suspension
//! that never took effect.
use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use tokuly_core::{outbound, prelude::*};
use tokuly_proto::admin::ModerationActionKind;

use crate::{
    Admin,
    actions::{ActionRow, NewAction},
    audit::Audit,
    cases::CaseRow,
    notices::{self, Mail},
    staff::{self, Staff},
};

/// Why a suspension stopped applying (`user_suspensions.end_cause`).
const END_LIFTED: &str = "lifted";
const END_EXPIRED: &str = "expired";
const END_SUPERSEDED: &str = "superseded";

/// Suspensions lifted per pass of the expiry worker.
const EXPIRY_BATCH: i64 = 200;

/// The account a sanction is aimed at.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct Target {
    pub id: Uuid,
    pub roles: Vec<String>,
    pub email: Option<String>,
    pub locale: String,
    pub time_zone: String,
    pub banned_at: Option<DateTime<Utc>>,
}

impl Target {
    /// An existing (not deleted) account, or `USER_NOT_FOUND`.
    pub async fn load<'e, E: sqlx::PgExecutor<'e>>(executor: E, user_id: Uuid) -> AppResult<Self> {
        sqlx::query_as("SELECT id, roles, email::text AS email, locale, time_zone, banned_at FROM users WHERE id = $1 AND deleted_at IS NULL")
            .bind(user_id)
            .fetch_optional(executor)
            .await?
            .or_not_found("USER_NOT_FOUND", "user not found")
    }
}

/// What a sanction says and where it comes from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sanction<'a> {
    /// Shown to the user.
    pub reason: &'a str,
    /// Internal.
    pub note: &'a str,
    /// The case being resolved, when the sanction is its outcome.
    pub case: Option<&'a CaseRow>,
}

impl<'a> Sanction<'a> {
    fn action(&self, kind: ModerationActionKind, staff: &Staff, target: &Target) -> NewAction<'a> {
        let action = NewAction::new(kind, Some(staff.id)).target(Some(target.id)).texts(self.reason, self.note);
        match self.case {
            Some(case) => action
                .case(Some(case.id))
                .subject(case.subject_type, case.subject_id, &case.subject_title)
                .hidden_target(case.owner_hidden),
            None => action,
        }
    }
}

/// Serializes every suspension change of one account (suspend, lift, expiry)
/// for the rest of the transaction. Without it the expiry worker could lift a
/// suspension a moderator has just replaced with a longer one.
async fn lock_sanctions(conn: &mut PgConnection, user_id: Uuid) -> AppResult<()> {
    // 1600 namespaces the lock to this domain (its migration number).
    sqlx::query("SELECT pg_advisory_xact_lock(1600, hashtext($1))").bind(user_id.to_string()).execute(conn).await?;
    Ok(())
}

/// Locks staff out of the deployment only one change at a time, on the lock
/// role changes take (`SetUserRoles`): the caller must still be an active
/// admin at that moment, and the target must not be the last one. Without
/// it two admins suspending each other at once would both succeed.
async fn guard_staff_sanction(conn: &mut PgConnection, staff: &Staff, target: &Target) -> AppResult<()> {
    lock_admin_roles(conn).await?;
    let (caller_active, others_active): (bool, bool) = sqlx::query_as(
        "SELECT
             EXISTS(SELECT 1 FROM users WHERE id = $1 AND 'admin' = ANY(roles) AND deleted_at IS NULL AND banned_at IS NULL),
             EXISTS(SELECT 1 FROM users WHERE id <> $2 AND 'admin' = ANY(roles) AND deleted_at IS NULL AND banned_at IS NULL)",
    )
    .bind(staff.id)
    .bind(target.id)
    .fetch_one(&mut *conn)
    .await?;
    ensure!(caller_active, denied, "FORBIDDEN", "requires role admin");
    ensure!(
        others_active || !target.roles.iter().any(|r| r == "admin"),
        precondition,
        "LAST_ADMIN",
        "this is the last active admin of the deployment"
    );
    Ok(())
}

/// Serializes changes that could leave the deployment without an admin
/// (role changes, sanctions on staff) for the rest of the transaction.
pub(crate) async fn lock_admin_roles(conn: &mut PgConnection) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(1600, hashtext('admin.roles'))").execute(conn).await?;
    Ok(())
}

impl Admin {
    /// An existing (not deleted) account, or `USER_NOT_FOUND`.
    pub(crate) async fn target(&self, user_id: Uuid) -> AppResult<Target> {
        Target::load(&self.core.db, user_id).await
    }

    /// Rejects sanctions a staff member may not impose: on themselves, and —
    /// for moderators — on other staff.
    pub(crate) fn check_may_sanction(staff: &Staff, target: &Target) -> AppResult<()> {
        ensure!(staff.id != target.id, denied, "CANNOT_SANCTION_SELF", "you cannot sanction your own account");
        ensure!(staff.may_sanction(&target.roles), denied, "TARGET_IS_STAFF", "only admins can act against staff accounts");
        Ok(())
    }

    /// Caps how many sanctions a moderator hands out per hour.
    pub(crate) async fn limit_sanctions(&self, staff: &Staff) -> AppResult<()> {
        if staff.is_admin {
            return Ok(());
        }
        self.core.cache.rate_limit(&format!("admin-sanction:{}", staff.id), self.cfg.moderator_sanctions_per_hour, std::time::Duration::from_secs(3600)).await
    }

    /// Warns an account inside the caller's transaction: notification,
    /// moderation log entry and audit entry.
    pub(crate) async fn warn(&self, conn: &mut PgConnection, staff: &Staff, target: &Target, sanction: Sanction<'_>) -> AppResult<ActionRow> {
        outbound::notify(&mut *conn, &notices::warning(target.id, sanction.reason)).await?;
        let action = sanction.action(ModerationActionKind::MODERATION_ACTION_KIND_WARN, staff, target).insert(conn).await?;
        Audit::by(staff, "user.warned")
            .user(target.id)
            .for_case(sanction.case)
            .reason(sanction.reason)
            .after(serde_json::json!({ "note": sanction.note, "case_id": sanction.case.map(|c| c.id) }))
            .record(&mut *conn)
            .await?;
        Ok(action)
    }

    /// Suspends an account until `expires_at`, or bans it when `None`, inside
    /// the caller's transaction. An existing suspension is replaced. The
    /// caller holds [`Admin::identity_slot`] from before it opened the
    /// transaction. Call [`Self::mail_suspension`] once it has committed.
    pub(crate) async fn suspend(
        &self,
        conn: &mut PgConnection,
        staff: &Staff,
        target: &Target,
        sanction: Sanction<'_>,
        expires_at: Option<DateTime<Utc>>,
    ) -> AppResult<ActionRow> {
        if staff::is_staff(&target.roles) {
            guard_staff_sanction(conn, staff, target).await?;
        }
        lock_sanctions(conn, target.id).await?;
        self.identity.set_banned(target.id, Some(sanction.reason)).await?;

        let replaced: Option<Uuid> = sqlx::query_scalar(
            "UPDATE user_suspensions SET ended_at = now(), end_cause = $2, ended_by = $3 WHERE user_id = $1 AND ended_at IS NULL RETURNING id",
        )
        .bind(target.id)
        .bind(END_SUPERSEDED)
        .bind(staff.id)
        .fetch_optional(&mut *conn)
        .await?;
        let suspension_id: Uuid =
            sqlx::query_scalar("INSERT INTO user_suspensions (user_id, moderator_id, case_id, reason, expires_at) VALUES ($1, $2, $3, $4, $5) RETURNING id")
                .bind(target.id)
                .bind(staff.id)
                .bind(sanction.case.map(|c| c.id))
                .bind(sanction.reason)
                .bind(expires_at)
                .fetch_one(&mut *conn)
                .await?;

        let kind = match expires_at {
            Some(_) => ModerationActionKind::MODERATION_ACTION_KIND_SUSPEND,
            None => ModerationActionKind::MODERATION_ACTION_KIND_BAN,
        };
        let action = sanction.action(kind, staff, target).expires(expires_at).insert(conn).await?;
        Audit::by(staff, "user.suspended")
            .user(target.id)
            .for_case(sanction.case)
            .reason(sanction.reason)
            .before(serde_json::json!({ "suspended": target.banned_at.is_some(), "replaced_suspension_id": replaced }))
            .after(serde_json::json!({
                "suspension_id": suspension_id,
                "permanent": expires_at.is_none(),
                "expires_at": expires_at,
                "note": sanction.note,
                "case_id": sanction.case.map(|c| c.id),
            }))
            .record(&mut *conn)
            .await?;
        Ok(action)
    }

    /// Lifts an account's suspension or ban on a staff member's request.
    pub(crate) async fn lift_suspension(&self, staff: &Staff, target: &Target, note: &str) -> AppResult<()> {
        let slot = self.identity_slot().await?;
        let mut tx = self.core.db.begin().await?;
        lock_sanctions(&mut tx, target.id).await?;
        // Re-read under the lock: the expiry worker may have lifted it already.
        let banned: bool = sqlx::query_scalar("SELECT banned_at IS NOT NULL FROM users WHERE id = $1").bind(target.id).fetch_one(&mut *tx).await?;
        ensure!(banned, precondition, "USER_NOT_SUSPENDED", "this account is not suspended");
        self.identity.set_banned(target.id, None).await?;
        let ended: Option<Uuid> = sqlx::query_scalar(
            "UPDATE user_suspensions SET ended_at = now(), end_cause = $2, ended_by = $3 WHERE user_id = $1 AND ended_at IS NULL RETURNING id",
        )
        .bind(target.id)
        .bind(END_LIFTED)
        .bind(staff.id)
        .fetch_optional(&mut *tx)
        .await?;
        NewAction::new(ModerationActionKind::MODERATION_ACTION_KIND_UNSUSPEND, Some(staff.id)).target(Some(target.id)).texts("", note).insert(&mut tx).await?;
        Audit::by(staff, "user.unsuspended")
            .user(target.id)
            .reason(note)
            .before(serde_json::json!({ "suspended": true, "suspension_id": ended }))
            .after(serde_json::json!({ "suspended": false }))
            .record(&mut *tx)
            .await?;
        tx.commit().await?;
        drop(slot);
        self.send_mail(target.email.clone(), notices::suspension_lifted_mail(&target.locale));
        Ok(())
    }

    /// Lifts every timed suspension that has run out. Returns how many were
    /// lifted. Safe to run from several workers at once.
    pub async fn expire_due_suspensions(&self) -> AppResult<u64> {
        let due: Vec<(Uuid, Uuid)> =
            sqlx::query_as("SELECT id, user_id FROM user_suspensions WHERE ended_at IS NULL AND expires_at <= now() ORDER BY expires_at LIMIT $1")
                .bind(EXPIRY_BATCH)
                .fetch_all(&self.core.db)
                .await?;
        let mut lifted = 0;
        for (suspension_id, user_id) in due {
            match self.expire_suspension(suspension_id, user_id).await {
                Ok(true) => lifted += 1,
                Ok(false) => {}
                Err(e) => {
                    tracing::error!(%suspension_id, %user_id, error = %e, "lifting an expired suspension failed")
                }
            }
        }
        Ok(lifted)
    }

    /// Lifts one expired suspension unless it ended or was replaced meanwhile.
    async fn expire_suspension(&self, suspension_id: Uuid, user_id: Uuid) -> AppResult<bool> {
        let slot = self.identity_slot().await?;
        let mut tx = self.core.db.begin().await?;
        lock_sanctions(&mut tx, user_id).await?;
        let still_due: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_suspensions WHERE id = $1 AND ended_at IS NULL AND expires_at <= now())")
            .bind(suspension_id)
            .fetch_one(&mut *tx)
            .await?;
        if !still_due {
            return Ok(false);
        }
        match self.identity.set_banned(user_id, None).await {
            Ok(()) => {}
            // The account was deleted while suspended: nothing left to unlock.
            Err(e) if e.is_not_found() => {}
            Err(e) => return Err(e),
        }
        sqlx::query("UPDATE user_suspensions SET ended_at = now(), end_cause = $2 WHERE id = $1")
            .bind(suspension_id)
            .bind(END_EXPIRED)
            .execute(&mut *tx)
            .await?;
        NewAction::new(ModerationActionKind::MODERATION_ACTION_KIND_UNSUSPEND, None)
            .target(Some(user_id))
            .texts("", "suspension expired")
            .insert(&mut tx)
            .await?;
        Audit::system("user.suspension_expired")
            .user(user_id)
            .before(serde_json::json!({ "suspended": true, "suspension_id": suspension_id }))
            .after(serde_json::json!({ "suspended": false }))
            .record(&mut *tx)
            .await?;
        tx.commit().await?;
        drop(slot);
        if let Ok(target) = self.target(user_id).await {
            self.send_mail(target.email, notices::suspension_lifted_mail(&target.locale));
        }
        Ok(true)
    }

    /// Tells a user by email that their account was suspended. Call after
    /// the suspension has committed.
    pub(crate) fn mail_suspension(&self, target: &Target, reason: &str, expires_at: Option<DateTime<Utc>>) {
        self.send_mail(target.email.clone(), notices::suspension_mail(&target.locale, &target.time_zone, reason, expires_at));
    }

    /// Sends a mail in the background; a delivery failure is logged and never
    /// fails the moderation action it reports.
    fn send_mail(&self, to: Option<String>, mail: Mail) {
        let Some(to) = to else { return };
        let core = self.core.clone();
        tokio::spawn(async move {
            if let Err(e) = core.mail.send(&to, &mail.subject, &mail.text, &mail.html).await {
                tracing::warn!(error = %e, subject = %mail.subject, "moderation mail could not be sent");
            }
        });
    }
}
