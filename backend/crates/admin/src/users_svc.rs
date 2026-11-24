//! `UserAdminService`: account administration. Needs the `moderator` role,
//! except `SetUserRoles` which needs `admin`.
use chrono::Utc;
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::admin::*;

use crate::{
    Admin,
    accounts::{ROLES, UserFilter, UserQuery, normalize_roles},
    actions::ActionFilter,
    audit::Audit,
    first_page,
    sanctions::{Sanction, Target},
    staff::{Staff, note_text, reason_text, suspension_duration},
};

/// Moderation log entries included with `GetUser`.
const RECENT_ACTIONS: i64 = 10;

impl UserAdminService for Admin {
    async fn search_users(&self, ctx: RequestContext, request: ServiceRequest<'_, SearchUsersRequest>) -> ServiceResult<SearchUsersResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let role = request.role.trim();
            ensure!(role.is_empty() || ROLES.contains(&role), invalid, "ROLE_INVALID", "unknown role {role:?}");
            let filter = UserFilter {
                query: UserQuery::parse(request.query)?,
                state: request.state.as_known().unwrap_or(AccountState::ACCOUNT_STATE_UNSPECIFIED),
                role: role.to_owned(),
                verified_only: request.verified_only,
            };
            let page = Page::from_view(request.page.as_option(), 100);
            let (rows, info) = self.search_accounts(&filter, &page).await?;
            ok(SearchUsersResponse { users: self.accounts_to_proto(&staff, &rows).await?, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_user(&self, ctx: RequestContext, request: ServiceRequest<'_, GetUserRequest>) -> ServiceResult<GetUserResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let user_id = parse_id(request.user_id)?;
            let user = self.admin_user(&staff, user_id).await?;
            let history = ActionFilter { target_user_id: Some(user_id), ..Default::default() };
            let (actions, _) = self.list_actions(&history, &first_page(RECENT_ACTIONS)).await?;
            ok(GetUserResponse {
                user: user.into(),
                moderation: self.moderation_summary(&staff, user_id).await?.into(),
                // Not what was done about an anonymous post the viewer is involved in:
                // here it would say whose post that was.
                recent_actions: self.actions_for(&staff, &actions, Some(user_id)).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn warn_user(&self, ctx: RequestContext, request: ServiceRequest<'_, WarnUserRequest>) -> ServiceResult<WarnUserResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let reason = reason_text(request.reason, true)?;
            let note = note_text(request.note)?;
            let target = self.target(parse_id(request.user_id)?).await?;
            Self::check_may_sanction(&staff, &target)?;
            self.limit_sanctions(&staff).await?;
            let mut tx = self.core.db.begin().await?;
            let action = self.warn(&mut tx, &staff, &target, Sanction { reason: &reason, note: &note, case: None }).await?;
            tx.commit().await?;
            ok(WarnUserResponse { action: self.action_to_proto(&action).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn suspend_user(&self, ctx: RequestContext, request: ServiceRequest<'_, SuspendUserRequest>) -> ServiceResult<SuspendUserResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let reason = reason_text(request.reason, true)?;
            let note = note_text(request.note)?;
            // Exactly one of "permanent" and a duration, so a forgotten
            // duration can never turn into a permanent ban.
            let expires_at = match (request.permanent, request.suspension_seconds) {
                (true, 0) => None,
                (false, seconds) => Some(Utc::now() + suspension_duration(seconds)?),
                (true, _) => bail!(invalid, "SUSPENSION_DURATION_INVALID", "a permanent ban takes no duration"),
            };
            let target = self.target(parse_id(request.user_id)?).await?;
            Self::check_may_sanction(&staff, &target)?;
            self.limit_sanctions(&staff).await?;
            let slot = self.identity_slot().await?;
            let mut tx = self.core.db.begin().await?;
            let action = self.suspend(&mut tx, &staff, &target, Sanction { reason: &reason, note: &note, case: None }, expires_at).await?;
            tx.commit().await?;
            drop(slot);
            self.mail_suspension(&target, &reason, expires_at);
            ok(SuspendUserResponse {
                user: self.admin_user(&staff, target.id).await?.into(),
                action: self.action_to_proto(&action).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn unsuspend_user(&self, ctx: RequestContext, request: ServiceRequest<'_, UnsuspendUserRequest>) -> ServiceResult<UnsuspendUserResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let note = note_text(request.note)?;
            let target = self.target(parse_id(request.user_id)?).await?;
            ensure!(target.banned_at.is_some(), precondition, "USER_NOT_SUSPENDED", "this account is not suspended");
            // A moderator must not undo what an admin imposed on a staff account.
            ensure!(staff.may_sanction(&target.roles), denied, "TARGET_IS_STAFF", "only admins can act on staff accounts");
            self.lift_suspension(&staff, &target, &note).await?;
            ok(UnsuspendUserResponse { user: self.admin_user(&staff, target.id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn set_user_verified(&self, ctx: RequestContext, request: ServiceRequest<'_, SetUserVerifiedRequest>) -> ServiceResult<SetUserVerifiedResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let note = note_text(request.note)?;
            let target = self.target(parse_id(request.user_id)?).await?;
            // The badge says Tokuly vouches for the account: a moderator does not
            // hand it to themselves or a colleague, nor take it from an admin.
            ensure!(staff.may_sanction(&target.roles), denied, "TARGET_IS_STAFF", "only admins can change the badge of staff accounts");
            let was_verified: bool = sqlx::query_scalar("SELECT verified FROM users WHERE id = $1").bind(target.id).fetch_one(&self.core.db).await?;
            self.identity.set_verified(target.id, request.verified).await?;
            Audit::by(&staff, "user.verified_changed")
                .user(target.id)
                .reason(note)
                .before(serde_json::json!({ "verified": was_verified }))
                .after(serde_json::json!({ "verified": request.verified }))
                .record(&self.core.db)
                .await?;
            ok(SetUserVerifiedResponse { user: self.admin_user(&staff, target.id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn set_user_roles(&self, ctx: RequestContext, request: ServiceRequest<'_, SetUserRolesRequest>) -> ServiceResult<SetUserRolesResponse> {
        handle(async {
            let staff = Staff::admin(&ctx)?;
            let request = request.to_owned_message();
            let roles = normalize_roles(&request.roles)?;
            let note = note_text(&request.note)?;
            let target_id = parse_id(&request.user_id)?;
            // Keeps at least one admin around: the last one cannot step down.
            ensure!(target_id != staff.id || roles.iter().any(|r| r == "admin"), denied, "CANNOT_DEMOTE_SELF", "you cannot remove your own admin role");
            // Role changes take turns, and each checks under the lock that the
            // caller still is an admin: the role travels in access tokens, so
            // two admins demoting each other at once would otherwise both
            // succeed and leave nobody to manage the deployment.
            let slot = self.identity_slot().await?;
            let mut tx = self.core.db.begin().await?;
            sqlx::query("SELECT pg_advisory_xact_lock(1600, hashtext('admin.roles'))").execute(&mut *tx).await?;
            let still_admin: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND 'admin' = ANY(roles) AND deleted_at IS NULL AND banned_at IS NULL)",
            )
            .bind(staff.id)
            .fetch_one(&mut *tx)
            .await?;
            ensure!(still_admin, denied, "FORBIDDEN", "requires role admin");
            let target = Target::load(&mut *tx, target_id).await?;
            self.identity.set_roles(target.id, &roles).await?;
            Audit::by(&staff, "user.roles_changed")
                .user(target.id)
                .reason(note)
                .before(serde_json::json!({ "roles": target.roles }))
                .after(serde_json::json!({ "roles": roles }))
                .record(&mut *tx)
                .await?;
            tx.commit().await?;
            drop(slot);
            // Roles travel inside access tokens. A removed role must stop
            // working now, not when the token expires, so the account is
            // signed out. (Not for the caller: they keep `admin`, which
            // already implies every other role.)
            let lost_a_role = target.roles.iter().any(|r| !roles.contains(r));
            if lost_a_role && target.id != staff.id {
                self.identity.revoke_all_sessions(target.id, None).await?;
            }
            ok(SetUserRolesResponse { user: self.admin_user(&staff, target.id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn revoke_user_sessions(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RevokeUserSessionsRequest>,
    ) -> ServiceResult<RevokeUserSessionsResponse> {
        handle(async {
            let staff = Staff::moderator(&ctx)?;
            let note = note_text(request.note)?;
            let target = self.target(parse_id(request.user_id)?).await?;
            ensure!(target.id == staff.id || staff.may_sanction(&target.roles), denied, "TARGET_IS_STAFF", "only admins can act on staff accounts");
            let revoked = self.identity.revoke_all_sessions(target.id, None).await?;
            Audit::by(&staff, "user.sessions_revoked")
                .user(target.id)
                .reason(note)
                .after(serde_json::json!({ "revoked": revoked }))
                .record(&self.core.db)
                .await?;
            ok(RevokeUserSessionsResponse { revoked_count: revoked as i32, ..Default::default() })
        })
        .await
    }
}
