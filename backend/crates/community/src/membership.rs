//! Membership: joining and leaving, join requests, invites, the member
//! directory, roles and each member's own settings.
use std::time::Duration;

use sqlx::{Postgres, Transaction};
use tokuly_core::{events as core_events, ids::random_base62, outbound, prelude::*};
use tokuly_proto::{
    common::PageInfo,
    community::{self as pb, MemberRole, MembershipStatus, ModerationAction, Permission},
};

use crate::{
    Community,
    access::{Access, Caller, community_not_found, not_a_member, permission_denied},
    communities::lock,
    events::{self, MemberChanged},
    model::{
        CommunityRow, INVITE_COLUMNS, InviteRow, MemberRow, community_columns, list_from_json, member_columns, to_json,
        visibility,
    },
    modlog::{self, LogEntry},
    notices::{self, MembershipNotice},
    permissions::{self, Role},
    text, validate,
};

/// Days a declined requester waits before asking again.
const REQUEST_COOLDOWN_DAYS: i32 = 7;
/// Usable invites one community may have at a time.
const MAX_ACTIVE_INVITES: i64 = 200;
const INVITE_CODE_LENGTH: usize = 10;

fn invite_invalid() -> AppError {
    AppError::not_found("INVITE_INVALID", "this invite does not exist or can no longer be used")
}

fn member_not_found() -> AppError {
    AppError::not_found("MEMBER_NOT_FOUND", "this user is not a member of the community")
}

/// What became of a member who left or was taken out.
pub(crate) struct Departure {
    /// Events whose attendance changed because the member's answers went with them.
    pub events: Vec<Uuid>,
}

#[derive(sqlx::FromRow)]
struct RequestRow {
    id: Uuid,
    community_id: Uuid,
    user_id: Uuid,
    answers: serde_json::Value,
    created_at: Time,
}

impl RequestRow {
    fn to_proto(&self, user: tokuly_proto::common::UserSummary) -> pb::JoinRequest {
        pb::JoinRequest {
            id: self.id.to_string(),
            community_id: self.community_id.to_string(),
            user: user.into(),
            answers: list_from_json(&self.answers),
            created_at: ts(self.created_at),
            ..Default::default()
        }
    }
}

impl Community {
    /// `JOIN_LIMIT_REACHED` when `user_id` is in as many communities as one
    /// account may be.
    pub(crate) async fn ensure_can_join_more(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: Uuid,
    ) -> AppResult<()> {
        let joined: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM community_members m JOIN communities c ON c.id = m.community_id
             WHERE m.user_id = $1 AND c.deleted_at IS NULL",
        )
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await?;
        ensure!(
            joined < i64::from(self.cfg.max_joined),
            precondition,
            "JOIN_LIMIT_REACHED",
            "one account is in at most {} communities",
            self.cfg.max_joined
        );
        Ok(())
    }

    /// Makes `user_id` a member inside `tx`, honouring the member cap, the
    /// join limit and bans. Returns false when they were a member already.
    pub(crate) async fn add_member(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community: &CommunityRow,
        user_id: Uuid,
        invite: Option<&InviteRow>,
    ) -> AppResult<bool> {
        // Locking the community row serializes joins, so the cap holds.
        let member_count: i32 =
            sqlx::query_scalar("SELECT member_count FROM communities WHERE id = $1 AND deleted_at IS NULL FOR UPDATE")
                .bind(community.id)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or_else(community_not_found)?;
        let already: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM community_members WHERE community_id = $1 AND user_id = $2)",
        )
        .bind(community.id)
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await?;
        if already {
            return Ok(false);
        }
        let banned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM community_bans
                           WHERE community_id = $1 AND user_id = $2 AND (expires_at IS NULL OR expires_at > now()))",
        )
        .bind(community.id)
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await?;
        ensure!(!banned, denied, "BANNED", "this account is banned from the community");
        ensure!(
            member_count < self.cfg.max_members as i32,
            precondition,
            "COMMUNITY_FULL",
            "the community has reached its member limit"
        );
        lock(tx, &format!("community:user:{user_id}")).await?;
        self.ensure_can_join_more(tx, user_id).await?;
        if let Some(invite) = invite {
            let used = sqlx::query(
                "UPDATE community_invites SET use_count = use_count + 1
                 WHERE id = $1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now())
                   AND (max_uses = 0 OR use_count < max_uses)",
            )
            .bind(invite.id)
            .execute(&mut **tx)
            .await?;
            if used.rows_affected() == 0 {
                return Err(invite_invalid());
            }
        }
        sqlx::query("INSERT INTO community_members (community_id, user_id, role) VALUES ($1, $2, $3)")
            .bind(community.id)
            .bind(user_id)
            .bind(Role::Member.code())
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE communities SET member_count = member_count + 1 WHERE id = $1")
            .bind(community.id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("DELETE FROM community_join_requests WHERE community_id = $1 AND user_id = $2")
            .bind(community.id)
            .bind(user_id)
            .execute(&mut **tx)
            .await?;
        // An expired ban is over; joining clears what is left of it.
        sqlx::query("DELETE FROM community_bans WHERE community_id = $1 AND user_id = $2")
            .bind(community.id)
            .bind(user_id)
            .execute(&mut **tx)
            .await?;
        core_events::emit(
            &mut **tx,
            events::MEMBER_JOINED,
            Some(user_id),
            &MemberChanged { community_id: community.id, user_id, public: community.is_public(), cause: String::new() },
        )
        .await?;
        outbound::activity(&mut **tx, &notices::joined_activity(user_id, community, false, false)).await?;
        Ok(true)
    }

    /// Takes `user_id` out of the community inside `tx`: the membership,
    /// and their answers to events that are not over. `None` when they were
    /// not a member — or, with `below`, when their role is not below that
    /// one: the role on file decides, not the one a request started with
    /// (an owner never leaves, nobody removes a peer who was just promoted).
    /// Call [`Self::after_departure`] once committed.
    pub(crate) async fn remove_member(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community: &CommunityRow,
        user_id: Uuid,
        cause: &str,
        below: Option<Role>,
    ) -> AppResult<Option<Departure>> {
        let removed = sqlx::query(
            "DELETE FROM community_members WHERE community_id = $1 AND user_id = $2 AND ($3::int IS NULL OR role < $3)",
        )
        .bind(community.id)
        .bind(user_id)
        .bind(below.map(Role::code))
        .execute(&mut **tx)
        .await?;
        if removed.rows_affected() == 0 {
            return Ok(None);
        }
        sqlx::query("UPDATE communities SET member_count = greatest(member_count - 1, 0) WHERE id = $1")
            .bind(community.id)
            .execute(&mut **tx)
            .await?;
        let events = self.withdraw_from_events(tx, community, user_id).await?;
        core_events::emit(
            &mut **tx,
            events::MEMBER_LEFT,
            Some(user_id),
            &MemberChanged {
                community_id: community.id,
                user_id,
                public: community.is_public(),
                cause: cause.to_owned(),
            },
        )
        .await?;
        Ok(Some(Departure { events }))
    }

    /// Pushes that follow a committed [`Self::remove_member`].
    pub(crate) async fn after_departure(&self, community: &CommunityRow, user_id: Uuid, departure: &Departure) {
        self.push_membership(community, user_id).await;
        self.push_community_changed(community.id).await;
        for event_id in &departure.events {
            self.push_event_changed(community, *event_id).await;
        }
    }

    /// Joins a community, or files a request to. Returns what the caller
    /// now is to it.
    pub(crate) async fn join(
        &self,
        caller: &Caller,
        user_id: Uuid,
        req: &pb::JoinCommunityRequest,
    ) -> AppResult<(MembershipStatus, CommunityRow)> {
        let core = &self.core;
        let code = req.invite_code.trim();
        let (community, invite) = if code.is_empty() {
            (self.access(caller, &req.community_id).await?.community, None)
        } else {
            let invite = self.usable_invite(code).await?;
            let community = self.community_by_id(invite.community_id).await?.ok_or_else(invite_invalid)?;
            if !req.community_id.trim().is_empty() {
                // An invite for another community than the one named is not an invite for this one.
                let named = self.find_community(&req.community_id).await?;
                ensure!(
                    named.is_some_and(|named| named.id == community.id),
                    not_found,
                    "INVITE_INVALID",
                    "this invite is for another community"
                );
            }
            (community, Some(invite))
        };
        let access = self.stance(caller, community).await?;
        if access.is_member() {
            return Ok((MembershipStatus::MEMBERSHIP_STATUS_MEMBER, access.community));
        }
        ensure!(access.ban.is_none(), denied, "BANNED", "this account is banned from the community");
        let community = access.community;
        self.limit("join", user_id, 30, Duration::from_secs(3600)).await?;

        if invite.is_some() || community.is_public() {
            let mut tx = core.db.begin().await?;
            let joined = self.add_member(&mut tx, &community, user_id, invite.as_ref()).await?;
            tx.commit().await?;
            if joined {
                self.push_membership(&community, user_id).await;
                self.push_community_changed(community.id).await;
            }
            return Ok((MembershipStatus::MEMBERSHIP_STATUS_MEMBER, community));
        }
        // A hidden community is reached by invite only (and `access` above
        // already answered "not found" for it).
        ensure!(
            community.visibility == visibility::RESTRICTED,
            not_found,
            "COMMUNITY_NOT_FOUND",
            "community not found"
        );

        let answers = validate::join_answers(&community.join_questions, &req.answers)?;
        // Each request filed rings the staff: asking, withdrawing and asking
        // again is not a way to ring them all day.
        self.limit("join-request", format!("{user_id}:{}", community.id), 3, Duration::from_secs(24 * 3600)).await?;
        // Read before the transaction opens: inside it nothing waits for a
        // second connection.
        let requester = tokuly_identity::summary(core, user_id).await?;
        let staff = self.staff_ids(community.id).await?;
        let mut tx = core.db.begin().await?;
        // A pending request stays as it is; a declined one may be asked
        // again once the cooldown is over. The requester is never told which.
        let filed: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO community_join_requests AS q (community_id, user_id, answers) VALUES ($1, $2, $3)
             ON CONFLICT (community_id, user_id) DO UPDATE
                 SET answers = EXCLUDED.answers, declined_at = NULL, declined_by = NULL, created_at = now()
                 WHERE q.declined_at < now() - make_interval(days => $4)
             RETURNING q.id",
        )
        .bind(community.id)
        .bind(user_id)
        .bind(to_json(&answers)?)
        .bind(REQUEST_COOLDOWN_DAYS)
        .fetch_optional(&mut *tx)
        .await?;
        if filed.is_some() {
            for staff_id in staff {
                outbound::notify(&mut *tx, &notices::join_request(staff_id, &requester, user_id, &community)).await?;
            }
        }
        tx.commit().await?;
        if filed.is_some() {
            self.push_staff_queues(community.id).await;
        }
        Ok((MembershipStatus::MEMBERSHIP_STATUS_REQUESTED, community))
    }

    /// Leaves a community. The owner cannot.
    pub(crate) async fn leave(&self, access: &Access) -> AppResult<()> {
        let member = access.require_member()?;
        ensure!(
            member.role() != Role::Owner,
            precondition,
            "OWNER_CANNOT_LEAVE",
            "hand the community to someone else (or delete it) before leaving"
        );
        let mut tx = self.core.db.begin().await?;
        // Whoever was made owner a moment ago stays: the role on file decides.
        let departure =
            self.remove_member(&mut tx, &access.community, member.user_id, "left", Some(Role::Owner)).await?;
        tx.commit().await?;
        match departure {
            Some(departure) => self.after_departure(&access.community, member.user_id, &departure).await,
            None => {
                let owner = self.member(access.community.id, member.user_id).await?.is_some();
                return Err(if owner {
                    AppError::precondition("OWNER_CANNOT_LEAVE", "hand the community to someone else before leaving")
                } else {
                    not_a_member()
                });
            }
        }
        Ok(())
    }

    /// Withdraws the caller's own join request.
    pub(crate) async fn cancel_request(&self, caller: &Caller, user_id: Uuid, reference: &str) -> AppResult<()> {
        // A request can only exist for a community the caller could see.
        let community = self.find_community(reference).await?.ok_or_else(community_not_found)?;
        let access = self.access_to(caller, community).await?;
        let cancelled = sqlx::query("DELETE FROM community_join_requests WHERE community_id = $1 AND user_id = $2")
            .bind(access.community.id)
            .bind(user_id)
            .execute(&self.core.db)
            .await?;
        ensure!(
            cancelled.rows_affected() > 0,
            not_found,
            "REQUEST_NOT_FOUND",
            "you have no request to join this community"
        );
        self.push_staff_queues(access.community.id).await;
        Ok(())
    }

    /// Pending join requests, oldest first. Needs MANAGE_MEMBERS.
    pub(crate) async fn list_requests(
        &self,
        access: &Access,
        page: &Page,
    ) -> AppResult<(Vec<pb::JoinRequest>, PageInfo)> {
        access.require(Permission::PERMISSION_MANAGE_MEMBERS)?;
        let after: Option<Uuid> = page.decode()?;
        let mut rows: Vec<RequestRow> = sqlx::query_as(
            "SELECT id, community_id, user_id, answers, created_at FROM community_join_requests
             WHERE community_id = $1 AND declined_at IS NULL AND ($2::uuid IS NULL OR id > $2)
             ORDER BY id LIMIT $3",
        )
        .bind(access.community.id)
        .bind(after)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        let user_ids: Vec<Uuid> = rows.iter().map(|row| row.user_id).collect();
        let users = tokuly_identity::summaries(&self.core, &user_ids).await?;
        Ok((rows.iter().map(|row| row.to_proto(tokuly_identity::users::pick(&users, row.user_id))).collect(), info))
    }

    /// Approves or declines a join request. Needs MANAGE_MEMBERS in the
    /// request's community. Returns the new member on approval.
    pub(crate) async fn respond_request(
        &self,
        caller: &Caller,
        request_id: Uuid,
        approve: bool,
    ) -> AppResult<Option<pb::Member>> {
        let not_found = || AppError::not_found("REQUEST_NOT_FOUND", "join request not found");
        let request: RequestRow = sqlx::query_as(
            "SELECT id, community_id, user_id, answers, created_at FROM community_join_requests
             WHERE id = $1 AND declined_at IS NULL",
        )
        .bind(request_id)
        .fetch_optional(&self.core.db)
        .await?
        .ok_or_else(not_found)?;
        let access = match self.access_by_id(caller, request.community_id).await {
            Ok(access) => access,
            Err(error) if error.is_not_found() => return Err(not_found()),
            Err(error) => return Err(error),
        };
        access.require(Permission::PERMISSION_MANAGE_MEMBERS)?;
        let actor = access.user_id.ok_or_else(permission_denied)?;
        let community = &access.community;

        let mut tx = self.core.db.begin().await?;
        // The request may have been answered or withdrawn in the meantime.
        let still_pending: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM community_join_requests WHERE id = $1 AND declined_at IS NULL FOR UPDATE)",
        )
        .bind(request.id)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(still_pending, not_found, "REQUEST_NOT_FOUND", "join request not found");
        let action = if approve {
            self.add_member(&mut tx, community, request.user_id, None).await?;
            outbound::notify(&mut *tx, &notices::join_approved(request.user_id, community).actor(actor)).await?;
            ModerationAction::MODERATION_ACTION_JOIN_REQUEST_APPROVED
        } else {
            sqlx::query("UPDATE community_join_requests SET declined_at = now(), declined_by = $2 WHERE id = $1")
                .bind(request.id)
                .bind(actor)
                .execute(&mut *tx)
                .await?;
            ModerationAction::MODERATION_ACTION_JOIN_REQUEST_DECLINED
        };
        modlog::record(&mut *tx, &LogEntry::new(community.id, action).by(actor).target(request.user_id)).await?;
        tx.commit().await?;

        self.push_staff_queues(community.id).await;
        if !approve {
            return Ok(None);
        }
        self.push_membership(community, request.user_id).await;
        self.push_community_changed(community.id).await;
        let member = self.member(community.id, request.user_id).await?.ok_or_else(member_not_found)?;
        let user = tokuly_identity::summary(&self.core, request.user_id).await?;
        Ok(Some(member.to_proto(user, true)))
    }

    /// The member directory: staff first, then by join date. Members only.
    pub(crate) async fn list_members(
        &self,
        access: &Access,
        query: &str,
        role: i32,
        page: &Page,
    ) -> AppResult<(Vec<pb::Member>, PageInfo)> {
        if !access.is_member() && !access.site_moderator {
            return Err(not_a_member());
        }
        let query = validate::query(query)?;
        if !query.is_empty() {
            let who = access.user_id.unwrap_or_default();
            self.limit("member-search", who, 120, Duration::from_secs(60)).await?;
        }
        let blocked = self.blocked(access.user_id).await?;
        let after: Option<(i32, Time, Uuid)> = page.decode()?;
        let (after_role, after_joined, after_user) = match after {
            Some((role, joined, user)) => (Some(role), Some(joined), Some(user)),
            None => (None, None, None),
        };
        let mut rows: Vec<MemberRow> = sqlx::query_as(concat!(
            "SELECT ",
            member_columns!(),
            " FROM community_members m JOIN users u ON u.id = m.user_id
              WHERE m.community_id = $1 AND u.deleted_at IS NULL
                AND ($2 = 0 OR m.role = $2)
                AND ($3 = '' OR u.display_name ILIKE $4 OR u.handle::text ILIKE $4)
                AND NOT (m.user_id = ANY($5))
                AND ($6::int IS NULL OR m.role < $6 OR (m.role = $6 AND (m.joined_at, m.user_id) > ($7, $8)))
              ORDER BY m.role DESC, m.joined_at, m.user_id
              LIMIT $9"
        ))
        .bind(access.community.id)
        .bind(role)
        .bind(&query)
        .bind(text::like_pattern(&query))
        .bind(&blocked)
        .bind(after_role)
        .bind(after_joined)
        .bind(after_user)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.role, row.joined_at, row.user_id));
        let user_ids: Vec<Uuid> = rows.iter().map(|row| row.user_id).collect();
        let users = tokuly_identity::summaries(&self.core, &user_ids).await?;
        let staff = access.is_staff() || access.site_moderator;
        Ok((
            rows.iter()
                .map(|row| {
                    let with_timeout = staff || access.user_id == Some(row.user_id);
                    row.to_proto(tokuly_identity::users::pick(&users, row.user_id), with_timeout)
                })
                .collect(),
            info,
        ))
    }

    /// One member, as the caller (a member) may see them.
    pub(crate) async fn get_member(&self, access: &Access, user_id: Uuid) -> AppResult<pb::Member> {
        if !access.is_member() && !access.site_moderator {
            return Err(not_a_member());
        }
        if self.blocked(access.user_id).await?.contains(&user_id) {
            return Err(member_not_found());
        }
        let member = self.member(access.community.id, user_id).await?.ok_or_else(member_not_found)?;
        let user = tokuly_identity::summary(&self.core, user_id).await?;
        let with_timeout = access.is_staff() || access.site_moderator || access.user_id == Some(user_id);
        Ok(member.to_proto(user, with_timeout))
    }

    /// The member `user_id`, who must rank below the caller for the caller
    /// to act on them.
    pub(crate) async fn require_lower_member(&self, access: &Access, user_id: Uuid) -> AppResult<MemberRow> {
        let actor = access.require_member()?;
        ensure!(actor.user_id != user_id, invalid, "CANNOT_TARGET_SELF", "you cannot do this to yourself");
        let target = self.member(access.community.id, user_id).await?.ok_or_else(member_not_found)?;
        if !actor.role().outranks(target.role()) {
            return Err(permission_denied());
        }
        Ok(target)
    }

    /// Changes a member's role. Needs MANAGE_ROLES and a higher role than
    /// both the member's current and new one.
    pub(crate) async fn set_member_role(
        &self,
        access: &Access,
        user_id: Uuid,
        role: MemberRole,
    ) -> AppResult<pb::Member> {
        access.require(Permission::PERMISSION_MANAGE_ROLES)?;
        let new_role = match role {
            MemberRole::MEMBER_ROLE_MEMBER => Role::Member,
            MemberRole::MEMBER_ROLE_MODERATOR => Role::Moderator,
            MemberRole::MEMBER_ROLE_ADMIN => Role::Admin,
            _ => bail!(invalid, "ROLE_INVALID", "choose member, moderator or admin"),
        };
        let actor = access.require_member()?;
        ensure!(actor.user_id != user_id, invalid, "CANNOT_TARGET_SELF", "you cannot change your own role");
        let target = self.member(access.community.id, user_id).await?.ok_or_else(member_not_found)?;
        if !permissions::may_assign(actor.role(), target.role(), new_role) {
            return Err(permission_denied());
        }
        // Every change tells the member: not something to flip back and forth.
        self.limit("role", actor.user_id, 60, Duration::from_secs(3600)).await?;
        let community = &access.community;
        let mut tx = self.core.db.begin().await?;
        // The roles on file decide, not the ones this request started
        // with: the target's must be unchanged, and so must the actor's.
        let updated: Option<MemberRow> = sqlx::query_as(concat!(
            "UPDATE community_members AS m SET role = $3
             WHERE m.community_id = $1 AND m.user_id = $2 AND m.role = $4
               AND EXISTS (SELECT 1 FROM community_members a
                           WHERE a.community_id = $1 AND a.user_id = $5 AND a.role = $6)
             RETURNING ",
            member_columns!()
        ))
        .bind(community.id)
        .bind(user_id)
        .bind(new_role.code())
        .bind(target.role)
        .bind(actor.user_id)
        .bind(actor.role)
        .fetch_optional(&mut *tx)
        .await?;
        let updated = updated.ok_or_else(member_not_found)?;
        if target.role() != new_role {
            let entry = LogEntry::new(community.id, ModerationAction::MODERATION_ACTION_ROLE_CHANGED)
                .by(actor.user_id)
                .target(user_id)
                .detail("from", target.role().name())
                .detail("to", new_role.name());
            modlog::record(&mut *tx, &entry).await?;
            let notice =
                notices::membership(user_id, community, &MembershipNotice::Role(new_role)).actor(actor.user_id);
            outbound::notify(&mut *tx, &notice).await?;
        }
        tx.commit().await?;
        self.push_membership(community, user_id).await;
        let user = tokuly_identity::summary(&self.core, user_id).await?;
        Ok(updated.to_proto(user, true))
    }

    /// The caller's own settings in a community.
    pub(crate) async fn update_membership(
        &self,
        access: &Access,
        notification_level: Option<i32>,
        show_on_profile: Option<bool>,
    ) -> AppResult<pb::CommunityViewer> {
        let member = access.require_member()?;
        let updated: MemberRow = sqlx::query_as(concat!(
            "UPDATE community_members AS m SET
                 notification_level = coalesce($3, notification_level),
                 show_on_profile = coalesce($4, show_on_profile)
             WHERE m.community_id = $1 AND m.user_id = $2
             RETURNING ",
            member_columns!()
        ))
        .bind(access.community.id)
        .bind(member.user_id)
        .bind(notification_level)
        .bind(show_on_profile)
        .fetch_optional(&self.core.db)
        .await?
        .ok_or_else(not_a_member)?;
        let access = Access { member: Some(updated), ..access.clone() };
        access.viewer().ok_or_else(not_a_member)
    }

    // ───────────────────────────── invites ─────────────────────────────

    /// The invite with this code, if it can still be used.
    async fn usable_invite(&self, code: &str) -> AppResult<InviteRow> {
        let plausible = code.len() == INVITE_CODE_LENGTH && code.bytes().all(|b| b.is_ascii_alphanumeric());
        if !plausible {
            return Err(invite_invalid());
        }
        let invite: Option<InviteRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {INVITE_COLUMNS} FROM community_invites WHERE code = $1"
        )))
        .bind(code)
        .fetch_optional(&self.core.db)
        .await?;
        invite.filter(|invite| invite.is_usable(now())).ok_or_else(invite_invalid)
    }

    fn invite_proto(&self, invite: &InviteRow, creator: tokuly_proto::common::UserSummary) -> pb::Invite {
        pb::Invite {
            id: invite.id.to_string(),
            community_id: invite.community_id.to_string(),
            code: invite.code.clone(),
            url: self.core.web_url(&format!("/c/join/{}", invite.code)),
            creator: creator.into(),
            expires_at: opt_ts(invite.expires_at),
            max_uses: invite.max_uses,
            use_count: invite.use_count,
            note: invite.note.clone(),
            created_at: ts(invite.created_at),
            ..Default::default()
        }
    }

    /// Creates an invite. Needs CREATE_INVITE.
    pub(crate) async fn create_invite(&self, access: &Access, req: &pb::CreateInviteRequest) -> AppResult<pb::Invite> {
        access.require(Permission::PERMISSION_CREATE_INVITE)?;
        let creator = access.user_id.ok_or_else(permission_denied)?;
        ensure!(
            (0..=validate::MAX_INVITE_SECONDS).contains(&req.expires_in_seconds),
            invalid,
            "EXPIRY_INVALID",
            "an invite expires within 30 days, or never (0)"
        );
        ensure!(
            (0..=validate::MAX_INVITE_USES).contains(&req.max_uses),
            invalid,
            "MAX_USES_INVALID",
            "an invite can be used up to {} times, or without limit (0)",
            validate::MAX_INVITE_USES
        );
        let note = text::clean_line(&req.note);
        ensure!(
            text::fits(&note, validate::MAX_INVITE_NOTE_CHARS),
            invalid,
            "NOTE_TOO_LONG",
            "the note has at most {} characters",
            validate::MAX_INVITE_NOTE_CHARS
        );
        let core = &self.core;
        self.limit("invite", creator, 30, Duration::from_secs(3600)).await?;
        let expires_at =
            (req.expires_in_seconds > 0).then(|| now() + chrono::Duration::seconds(i64::from(req.expires_in_seconds)));
        let mut tx = core.db.begin().await?;
        // Locking the community row serializes invite creation: the cap holds.
        sqlx::query("SELECT 1 FROM communities WHERE id = $1 FOR UPDATE")
            .bind(access.community.id)
            .execute(&mut *tx)
            .await?;
        let active: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM community_invites
             WHERE community_id = $1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now())
               AND (max_uses = 0 OR use_count < max_uses)",
        )
        .bind(access.community.id)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(
            active < MAX_ACTIVE_INVITES,
            precondition,
            "TOO_MANY_INVITES",
            "revoke an invite before creating another"
        );
        let invite: InviteRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO community_invites (community_id, code, created_by, note, max_uses, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING {INVITE_COLUMNS}"
        )))
        .bind(access.community.id)
        .bind(random_base62(INVITE_CODE_LENGTH))
        .bind(creator)
        .bind(&note)
        .bind(req.max_uses)
        .bind(expires_at)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        let user = tokuly_identity::summary(core, creator).await?;
        Ok(self.invite_proto(&invite, user))
    }

    /// The usable invites of a community: all of them for the staff, the
    /// caller's own otherwise. Needs CREATE_INVITE.
    pub(crate) async fn list_invites(&self, access: &Access, page: &Page) -> AppResult<(Vec<pb::Invite>, PageInfo)> {
        access.require(Permission::PERMISSION_CREATE_INVITE)?;
        let all = access.has(Permission::PERMISSION_MANAGE_MEMBERS);
        let after: Option<Uuid> = page.decode()?;
        let mut rows: Vec<InviteRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {INVITE_COLUMNS} FROM community_invites
             WHERE community_id = $1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now())
               AND (max_uses = 0 OR use_count < max_uses)
               AND ($2 OR created_by = $3) AND ($4::uuid IS NULL OR id < $4)
             ORDER BY id DESC LIMIT $5"
        )))
        .bind(access.community.id)
        .bind(all)
        .bind(access.user_id)
        .bind(after)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        let creators: Vec<Uuid> = rows.iter().map(|row| row.created_by).collect();
        let users = tokuly_identity::summaries(&self.core, &creators).await?;
        Ok((
            rows.iter()
                .map(|row| self.invite_proto(row, tokuly_identity::users::pick(&users, row.created_by)))
                .collect(),
            info,
        ))
    }

    /// Makes an invite unusable: its creator, or the staff.
    pub(crate) async fn revoke_invite(&self, caller: &Caller, invite_id: Uuid) -> AppResult<()> {
        let not_found = || AppError::not_found("INVITE_NOT_FOUND", "invite not found");
        let invite: InviteRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {INVITE_COLUMNS} FROM community_invites WHERE id = $1 AND revoked_at IS NULL"
        )))
        .bind(invite_id)
        .fetch_optional(&self.core.db)
        .await?
        .ok_or_else(not_found)?;
        let access = match self.access_by_id(caller, invite.community_id).await {
            Ok(access) => access,
            Err(error) if error.is_not_found() => return Err(not_found()),
            Err(error) => return Err(error),
        };
        let actor = access.require_member().map_err(|_| not_found())?.user_id;
        let own = invite.created_by == actor;
        if !own {
            access.require(Permission::PERMISSION_MANAGE_MEMBERS)?;
        }
        let mut tx = self.core.db.begin().await?;
        sqlx::query("UPDATE community_invites SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
            .bind(invite.id)
            .execute(&mut *tx)
            .await?;
        if !own {
            let entry = LogEntry::new(invite.community_id, ModerationAction::MODERATION_ACTION_INVITE_REVOKED)
                .by(actor)
                .target(invite.created_by);
            modlog::record(&mut *tx, &entry).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// What an invite leads to: the community (even a hidden one) as the
    /// caller stands to it, and who invites.
    pub(crate) async fn preview_invite(
        &self,
        caller: &Caller,
        code: &str,
    ) -> AppResult<(pb::Community, tokuly_proto::common::UserSummary)> {
        let invite = self.usable_invite(code.trim()).await?;
        let community = self.community_by_id(invite.community_id).await?.ok_or_else(invite_invalid)?;
        let access = self.stance(caller, community).await?;
        let inviter = tokuly_identity::summary(&self.core, invite.created_by).await?;
        Ok((self.render_community(&access).await?, inviter))
    }

    // ───────────────────────────── lists ─────────────────────────────

    /// The communities `user_id` is in, most recently active first.
    pub(crate) async fn my_communities(&self, user_id: Uuid, page: &Page) -> AppResult<(Vec<CommunityRow>, PageInfo)> {
        let after: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<CommunityRow> = sqlx::query_as(concat!(
            "SELECT ",
            community_columns!(),
            " FROM community_members m JOIN communities c ON c.id = m.community_id
              WHERE m.user_id = $1 AND c.deleted_at IS NULL
                AND ($2::timestamptz IS NULL OR (c.last_activity_at, c.id) < ($2, $3))
              ORDER BY c.last_activity_at DESC, c.id DESC LIMIT $4"
        ))
        .bind(user_id)
        .bind(after.map(|(at, _)| at))
        .bind(after.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.last_activity_at, row.id));
        Ok((rows, info))
    }

    /// The communities `user_id` asked to join and has no answer from.
    pub(crate) async fn requested_communities(&self, user_id: Uuid) -> AppResult<Vec<CommunityRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            community_columns!(),
            " FROM community_join_requests q JOIN communities c ON c.id = q.community_id
              WHERE q.user_id = $1 AND c.deleted_at IS NULL AND c.visibility <> 3
              ORDER BY q.created_at DESC LIMIT 50"
        ))
        .bind(user_id)
        .fetch_all(&self.core.db)
        .await?)
    }

    /// The public communities `user_id` shows on their profile, largest first.
    pub(crate) async fn shown_communities(
        &self,
        user_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<CommunityRow>, PageInfo)> {
        let after: Option<(i32, Uuid)> = page.decode()?;
        let mut rows: Vec<CommunityRow> = sqlx::query_as(concat!(
            "SELECT ",
            community_columns!(),
            " FROM community_members m JOIN communities c ON c.id = m.community_id
              WHERE m.user_id = $1 AND m.show_on_profile AND c.visibility = 1 AND c.deleted_at IS NULL
                AND ($2::int IS NULL OR (c.member_count, c.id) < ($2, $3))
              ORDER BY c.member_count DESC, c.id DESC LIMIT $4"
        ))
        .bind(user_id)
        .bind(after.map(|(count, _)| count))
        .bind(after.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.member_count, row.id));
        Ok((rows, info))
    }
}
