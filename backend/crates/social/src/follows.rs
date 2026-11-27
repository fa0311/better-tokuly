//! Follows: following and unfollowing, locked accounts and their follow
//! requests, removing followers, follow lists and counts, and each
//! follower's private preferences (mute, notifications).
//!
//! Rules worth knowing:
//! - A follow is one way and independent of friendship: becoming friends
//!   follows nobody, unfollowing unfriends nobody. Both count as a tie for
//!   circles and as an audience (see [`crate::audience`]).
//! - A **locked** account approves its followers. Accounts of minors are
//!   stored as locked when they are created (never derived from the date, so
//!   an 18th birthday changes nothing visible). Declining a request
//!   is silent: the requester keeps seeing it as pending. Unlocking approves
//!   every request that is still waiting and drops the declined ones.
//! - Blocks: a block ends follows and requests in both directions and keeps
//!   new ones out; to the blocked user the blocker does not exist.
//! - Lists and counts show active accounts only and leave out people in a
//!   block with the viewer, so a count always matches its list.
//! - Every change to a pair runs under [`graph::lock_pair`]; adding to what
//!   a user follows also under [`graph::lock_following_of`] (the limit), and
//!   anything that reads whether the followee is locked under their shared
//!   [`graph::lock_follow_settings`]. No transaction takes a pooled
//!   connection: what needs the pool is read before `begin()`.
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use chrono::{DateTime, Utc};
use sqlx::{PgExecutor, Postgres, Transaction};
use tokuly_core::{
    events,
    outbound::{self, NotificationRequest},
    prelude::*,
};
use tokuly_identity::users;
use tokuly_proto::{
    common::{PageInfo, SubjectType, UserSummary},
    social::{
        FollowEntry, FollowList, FollowNotifyLevel, FollowPrefs, FollowRelation, FollowRequestDirection,
        FollowRequestEntry, FollowSettings, FollowState, FollowSummary, social_event::Event as Push,
    },
};

use crate::{
    Social,
    audience::locked_sql,
    circles::{self, CircleExits},
    copy,
    events::{
        FOLLOW_CREATED, FOLLOW_PREFS_CHANGED, FOLLOW_REMOVED, FOLLOW_REQUEST_WITHDRAWN, FOLLOW_REQUESTED,
        FOLLOW_SETTINGS_CHANGED, FollowCreated, FollowEndReason, FollowPrefsChanged, FollowRemoved,
        FollowRequestWithdrawn, FollowRequested, FollowSettingsChanged,
    },
    graph, idcache, people,
};

const HOUR: Duration = Duration::from_secs(3600);
const MAX_PAGE: i64 = 200;
const MAX_QUERY_CHARS: usize = 64;
/// "Followed by A, B and N others": people shown by name.
const KNOWN_PREVIEW: i64 = 3;
/// Shortest and longest timed mute.
const MIN_MUTE_SECONDS: i64 = 3600;
const MAX_MUTE_SECONDS: i64 = 365 * 24 * 3600;

/// What became of a `Follow`.
pub(crate) enum FollowOutcome {
    Following,
    Requested(FollowRequestEntry),
}

/// Everything that decides what a follow from `me` to `them` may do.
#[derive(Debug, sqlx::FromRow)]
struct PairState {
    blocked_by_me: bool,
    blocked_by_them: bool,
    following: bool,
    request_id: Option<Uuid>,
}

impl PairState {
    async fn load<'e, E: PgExecutor<'e>>(executor: E, me: Uuid, them: Uuid) -> AppResult<Self> {
        Ok(sqlx::query_as(
            "SELECT
                 EXISTS(SELECT 1 FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2) AS blocked_by_me,
                 EXISTS(SELECT 1 FROM user_blocks WHERE blocker_id = $2 AND blocked_id = $1) AS blocked_by_them,
                 EXISTS(SELECT 1 FROM follows WHERE follower_id = $1 AND followee_id = $2) AS following,
                 (SELECT id FROM follow_requests WHERE follower_id = $1 AND followee_id = $2) AS request_id",
        )
        .bind(me)
        .bind(them)
        .fetch_one(executor)
        .await?)
    }

    fn ensure_followable(&self) -> AppResult<()> {
        // Being blocked is indistinguishable from the user not existing.
        ensure!(!self.blocked_by_them, not_found, "USER_NOT_FOUND", "user not found");
        ensure!(!self.blocked_by_me, precondition, "USER_BLOCKED", "unblock this user before following them");
        Ok(())
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct RequestRow {
    id: Uuid,
    follower_id: Uuid,
    followee_id: Uuid,
    created_at: DateTime<Utc>,
}

/// A follower's preferences as stored.
#[derive(Debug, Clone, Copy, Default, sqlx::FromRow)]
struct PrefsRow {
    muted: bool,
    muted_until: Option<DateTime<Utc>>,
    notify_level: i16,
}

impl PrefsRow {
    /// As the API shows them: a timed mute that ran out is no mute.
    fn to_proto(self, now: DateTime<Utc>) -> FollowPrefs {
        let muted = self.muted && self.muted_until.is_none_or(|until| until > now);
        FollowPrefs {
            muted,
            muted_until: if muted { opt_ts(self.muted_until) } else { Default::default() },
            notify_level: i32::from(self.notify_level).into(),
            ..Default::default()
        }
    }
}

/// What a block ended between two users, for after the commit.
#[derive(Debug, Default)]
pub(crate) struct FollowsEnded {
    /// (follower, followee) of every follow that ended.
    follows: Vec<(Uuid, Uuid)>,
    /// (request, follower, followee) of requests the followee could still see.
    withdrawn: Vec<(Uuid, Uuid, Uuid)>,
    circles: CircleExits,
}

impl FollowsEnded {
    pub(crate) async fn finish(self, social: &Social) {
        let followers: Vec<Uuid> = self.follows.iter().map(|(follower, _)| *follower).collect();
        let followees: Vec<Uuid> = self.follows.iter().map(|(_, followee)| *followee).collect();
        idcache::follows_changed(&social.core, &followers, &followees).await;
        self.circles.invalidate(&social.core).await;
        for (request_id, _, followee) in self.withdrawn {
            social.push(followee, Push::FollowRequestCancelledId(request_id.to_string())).await;
        }
    }
}

/// Inside `tx` (under the pair lock): ends every follow and follow request
/// between `a` and `b`, and every circle membership between them. For blocks:
/// `a` is the one who blocked.
///
/// Takes both users' follow-settings locks (shared) first: an unlock of one
/// of them that is approving a request between the two right now either
/// commits before (and its follow is deleted here) or waits until the block
/// is in (and finds no request left). Without them, the follow it inserts
/// would outlive the block.
pub(crate) async fn end_follows_between(
    tx: &mut Transaction<'_, Postgres>,
    a: Uuid,
    b: Uuid,
) -> AppResult<FollowsEnded> {
    let (first, second) = if a < b { (a, b) } else { (b, a) };
    graph::lock_follow_settings(tx, first, false).await?;
    graph::lock_follow_settings(tx, second, false).await?;
    let follows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "DELETE FROM follows WHERE (follower_id = $1 AND followee_id = $2) OR (follower_id = $2 AND followee_id = $1)
         RETURNING follower_id, followee_id",
    )
    .bind(a)
    .bind(b)
    .fetch_all(&mut **tx)
    .await?;
    for (follower_id, followee_id) in &follows {
        let removed =
            FollowRemoved { follower_id: *follower_id, followee_id: *followee_id, reason: FollowEndReason::Blocked };
        events::emit(&mut **tx, FOLLOW_REMOVED, Some(a), &removed).await?;
    }
    let requests: Vec<(Uuid, Uuid, Uuid, bool)> = sqlx::query_as(
        "DELETE FROM follow_requests WHERE (follower_id = $1 AND followee_id = $2) OR (follower_id = $2 AND followee_id = $1)
         RETURNING id, follower_id, followee_id, declined_at IS NOT NULL",
    )
    .bind(a)
    .bind(b)
    .fetch_all(&mut **tx)
    .await?;
    let mut withdrawn = Vec::new();
    for (request_id, follower_id, followee_id, declined) in requests {
        if !declined {
            events::emit(
                &mut **tx,
                FOLLOW_REQUEST_WITHDRAWN,
                Some(a),
                &FollowRequestWithdrawn { request_id, follower_id, followee_id },
            )
            .await?;
            // The blocked side's devices are not told anything; the blocker's
            // own request list simply loses it.
            if followee_id == a {
                withdrawn.push((request_id, follower_id, followee_id));
            }
        }
    }
    let circles = circles::drop_memberships_between(tx, a, b).await?;
    Ok(FollowsEnded { follows, withdrawn, circles })
}

/// A `LIKE` pattern matching `query` anywhere, with its wildcards escaped.
fn contains_pattern(query: &str) -> String {
    format!("%{}%", query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))
}

/// Whether `user_id`'s account is locked, read on `executor`.
async fn is_locked<'e, E: PgExecutor<'e>>(executor: E, user_id: Uuid) -> AppResult<bool> {
    Ok(sqlx::query_scalar(concat!("SELECT ", locked_sql!("$1"))).bind(user_id).fetch_one(executor).await?)
}

/// Checks a mute duration: 0 (until unmuted) or one hour to a year.
fn mute_until(seconds: i64, now: DateTime<Utc>) -> AppResult<Option<DateTime<Utc>>> {
    if seconds == 0 {
        return Ok(None);
    }
    ensure!(
        (MIN_MUTE_SECONDS..=MAX_MUTE_SECONDS).contains(&seconds),
        invalid,
        "INVALID_MUTE_DURATION",
        "a timed mute lasts from an hour to a year"
    );
    Ok(Some(now + chrono::Duration::seconds(seconds)))
}

impl Social {
    /// Stores a locked setting for the account of a minor, unless the account
    /// already has a setting of its own. `minor` is the age group on the day
    /// the birthdate was given (`identity.user.created`,
    /// `identity.user.birthdate_given`): the lock then stays until its owner
    /// lifts it, whatever the date.
    pub(crate) async fn lock_if_minor(&self, user_id: Uuid, minor: bool) -> AppResult<()> {
        if minor {
            sqlx::query("INSERT INTO follow_settings (user_id, locked) VALUES ($1, true) ON CONFLICT (user_id) DO NOTHING")
                .bind(user_id)
                .execute(&self.core.db)
                .await?;
        }
        Ok(())
    }

    /// Follows `followee`, or asks to when their account is locked.
    pub(crate) async fn follow(&self, follower: Uuid, followee: Uuid) -> AppResult<FollowOutcome> {
        ensure!(follower != followee, invalid, "CANNOT_FOLLOW_SELF", "you cannot follow yourself");
        let core = &self.core;
        people::ensure_visible(core, follower, followee).await?;
        let state = PairState::load(&core.db, follower, followee).await?;
        state.ensure_followable()?;
        if state.following {
            return Ok(FollowOutcome::Following);
        }
        if let Some(request_id) = state.request_id {
            return Ok(FollowOutcome::Requested(self.follow_request(follower, request_id).await?));
        }
        core.cache.rate_limit(&format!("follow:{follower}"), 200, HOUR).await?;
        core.cache.rate_limit(&format!("follow-day:{follower}"), 1000, 24 * HOUR).await?;
        // Following and unfollowing again must not become a way to ping someone.
        core.cache.rate_limit(&format!("follow-pair:{follower}:{followee}"), 10, 24 * HOUR).await?;
        let sender = users::summary(core, follower).await?;

        let mut tx = core.db.begin().await?;
        graph::lock_pair(&mut tx, follower, followee).await?;
        graph::lock_following_of(&mut tx, follower).await?;
        graph::lock_follow_settings(&mut tx, followee, false).await?;
        let state = PairState::load(&mut *tx, follower, followee).await?;
        state.ensure_followable()?;
        if state.following {
            return Ok(FollowOutcome::Following);
        }
        if let Some(request_id) = state.request_id {
            tx.commit().await?;
            return Ok(FollowOutcome::Requested(self.follow_request(follower, request_id).await?));
        }
        // Pending requests count too, so approving one can never go over.
        let used: i64 = sqlx::query_scalar(
            "SELECT (SELECT count(*) FROM follows WHERE follower_id = $1) + (SELECT count(*) FROM follow_requests WHERE follower_id = $1)",
        )
        .bind(follower)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(
            used < self.cfg.max_following,
            precondition,
            "FOLLOW_LIMIT_REACHED",
            "you follow as many accounts as you can"
        );
        let name = people::name_of(&sender);

        if is_locked(&mut *tx, followee).await? {
            let request: RequestRow = sqlx::query_as(
                "INSERT INTO follow_requests (follower_id, followee_id) VALUES ($1, $2) RETURNING id, follower_id, followee_id, created_at",
            )
            .bind(follower)
            .bind(followee)
            .fetch_one(&mut *tx)
            .await?;
            events::emit(
                &mut *tx,
                FOLLOW_REQUESTED,
                Some(follower),
                &FollowRequested { request_id: request.id, follower_id: follower, followee_id: followee },
            )
            .await?;
            let text = copy::follow_request(&name);
            outbound::notify(
                &mut *tx,
                &NotificationRequest::new(
                    followee,
                    "social.follow_request",
                    text.title,
                    text.body,
                    "/friends/follow-requests",
                )
                .actor(follower)
                .subject(SubjectType::SUBJECT_TYPE_USER, follower)
                .collapse(format!("social.follow_request:{follower}")),
            )
            .await?;
            tx.commit().await?;
            let for_followee = self.follow_requests_to_proto(followee, vec![request.clone()]).await?.remove(0);
            self.push(followee, Push::FollowRequestReceived(Box::new(for_followee))).await;
            return Ok(FollowOutcome::Requested(
                self.follow_requests_to_proto(follower, vec![request]).await?.remove(0),
            ));
        }

        sqlx::query("INSERT INTO follows (follower_id, followee_id) VALUES ($1, $2)")
            .bind(follower)
            .bind(followee)
            .execute(&mut *tx)
            .await?;
        events::emit(
            &mut *tx,
            FOLLOW_CREATED,
            Some(follower),
            &FollowCreated { follower_id: follower, followee_id: followee, approved: false },
        )
        .await?;
        let text = copy::new_follower(&name);
        outbound::notify(
            &mut *tx,
            &NotificationRequest::new(
                followee,
                "social.new_follower",
                text.title,
                text.body,
                format!("/u/{}", sender.handle),
            )
            .actor(follower)
            .subject(SubjectType::SUBJECT_TYPE_USER, follower)
            .collapse(format!("social.new_follower:{follower}")),
        )
        .await?;
        tx.commit().await?;
        idcache::follows_changed(core, &[follower], &[followee]).await;
        self.push_follow_entry(followee, follower, Push::NewFollower).await;
        Ok(FollowOutcome::Following)
    }

    /// Pushes `about` as `to` sees them in a follow list.
    async fn push_follow_entry(&self, to: Uuid, about: Uuid, event: impl FnOnce(Box<FollowEntry>) -> Push) {
        match self.follow_entries(to, vec![(about, Utc::now(), None)]).await {
            Ok(mut entries) => self.push(to, event(Box::new(entries.remove(0)))).await,
            Err(e) => tracing::warn!(error = %e, "could not push a follow; clients catch up on their next refresh"),
        }
    }

    /// Stops following `followee`, or withdraws the request to them.
    pub(crate) async fn unfollow(&self, follower: Uuid, followee: Uuid) -> AppResult<()> {
        let core = &self.core;
        let mut tx = core.db.begin().await?;
        graph::lock_pair(&mut tx, follower, followee).await?;
        let unfollowed = sqlx::query("DELETE FROM follows WHERE follower_id = $1 AND followee_id = $2")
            .bind(follower)
            .bind(followee)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
        if unfollowed {
            let removed =
                FollowRemoved { follower_id: follower, followee_id: followee, reason: FollowEndReason::Unfollowed };
            events::emit(&mut *tx, FOLLOW_REMOVED, Some(follower), &removed).await?;
        }
        let withdrawn = self.withdraw_request_in(&mut tx, follower, followee).await?;
        let exits = circles::untie_from_circles(&mut tx, followee, follower).await?;
        tx.commit().await?;
        if unfollowed {
            idcache::follows_changed(core, &[follower], &[followee]).await;
        }
        exits.invalidate(core).await;
        if let Some(request_id) = withdrawn {
            self.push(followee, Push::FollowRequestCancelledId(request_id.to_string())).await;
        }
        Ok(())
    }

    /// Inside `tx`: deletes `follower`'s request to `followee`. Returns its ID
    /// when the followee could still see it (it was not declined).
    async fn withdraw_request_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        follower: Uuid,
        followee: Uuid,
    ) -> AppResult<Option<Uuid>> {
        let deleted: Option<(Uuid, bool)> =
            sqlx::query_as("DELETE FROM follow_requests WHERE follower_id = $1 AND followee_id = $2 RETURNING id, declined_at IS NOT NULL")
                .bind(follower)
                .bind(followee)
                .fetch_optional(&mut **tx)
                .await?;
        match deleted {
            Some((request_id, false)) => {
                events::emit(
                    &mut **tx,
                    FOLLOW_REQUEST_WITHDRAWN,
                    Some(follower),
                    &FollowRequestWithdrawn { request_id, follower_id: follower, followee_id: followee },
                )
                .await?;
                Ok(Some(request_id))
            }
            _ => Ok(None),
        }
    }

    /// Withdraws a follow request `user_id` sent.
    pub(crate) async fn cancel_follow_request(&self, user_id: Uuid, request_id: Uuid) -> AppResult<()> {
        let followee: Option<Uuid> =
            sqlx::query_scalar("SELECT followee_id FROM follow_requests WHERE id = $1 AND follower_id = $2")
                .bind(request_id)
                .bind(user_id)
                .fetch_optional(&self.core.db)
                .await?;
        let followee = followee.or_not_found("REQUEST_NOT_FOUND", "follow request not found")?;
        let mut tx = self.core.db.begin().await?;
        graph::lock_pair(&mut tx, user_id, followee).await?;
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM follow_requests WHERE id = $1)")
            .bind(request_id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(exists, not_found, "REQUEST_NOT_FOUND", "follow request not found");
        let withdrawn = self.withdraw_request_in(&mut tx, user_id, followee).await?;
        tx.commit().await?;
        if let Some(request_id) = withdrawn {
            self.push(followee, Push::FollowRequestCancelledId(request_id.to_string())).await;
        }
        Ok(())
    }

    /// Removes `follower` from `owner`'s followers, silently.
    pub(crate) async fn remove_follower(&self, owner: Uuid, follower: Uuid) -> AppResult<()> {
        let core = &self.core;
        core.cache.rate_limit(&format!("remove-follower:{owner}"), 300, HOUR).await?;
        let mut tx = core.db.begin().await?;
        graph::lock_pair(&mut tx, owner, follower).await?;
        let removed = sqlx::query("DELETE FROM follows WHERE follower_id = $1 AND followee_id = $2")
            .bind(follower)
            .bind(owner)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
        if removed {
            let event = FollowRemoved { follower_id: follower, followee_id: owner, reason: FollowEndReason::Removed };
            events::emit(&mut *tx, FOLLOW_REMOVED, Some(owner), &event).await?;
        }
        let exits = circles::untie_from_circles(&mut tx, owner, follower).await?;
        tx.commit().await?;
        if removed {
            idcache::follows_changed(core, &[follower], &[owner]).await;
        }
        exits.invalidate(core).await;
        Ok(())
    }

    /// Approves or (silently) declines a request to follow `user_id`.
    /// Returns the new follower when approved.
    pub(crate) async fn respond_follow_request(
        &self,
        user_id: Uuid,
        request_id: Uuid,
        approve: bool,
    ) -> AppResult<Option<FollowEntry>> {
        let core = &self.core;
        let requester: Option<Uuid> = sqlx::query_scalar(
            "SELECT follower_id FROM follow_requests WHERE id = $1 AND followee_id = $2 AND declined_at IS NULL",
        )
        .bind(request_id)
        .bind(user_id)
        .fetch_optional(&core.db)
        .await?;
        let requester = requester.or_not_found("REQUEST_NOT_FOUND", "follow request not found")?;
        let approver = users::summary(core, user_id).await?;

        let mut tx = core.db.begin().await?;
        graph::lock_pair(&mut tx, user_id, requester).await?;
        let still_pending: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM follow_requests WHERE id = $1 AND declined_at IS NULL)")
                .bind(request_id)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(still_pending, not_found, "REQUEST_NOT_FOUND", "follow request not found");
        if !approve {
            sqlx::query("UPDATE follow_requests SET declined_at = now() WHERE id = $1")
                .bind(request_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(None);
        }
        approve_requests(&mut tx, &approver, &[(request_id, requester)]).await?;
        tx.commit().await?;
        idcache::follows_changed(core, &[requester], &[user_id]).await;
        self.push_follow_entry(requester, user_id, Push::FollowRequestApproved).await;
        Ok(self.follow_entries(user_id, vec![(requester, Utc::now(), None)]).await?.pop())
    }

    /// One page of follow requests to (`INCOMING`, the default) or from
    /// `user_id`, newest first, and how many wait for their answer.
    pub(crate) async fn list_follow_requests(
        &self,
        user_id: Uuid,
        direction: FollowRequestDirection,
        page: &Page,
    ) -> AppResult<(Vec<FollowRequestEntry>, PageInfo, i32)> {
        let before: Option<Uuid> = page.decode()?;
        let outgoing = direction == FollowRequestDirection::FOLLOW_REQUEST_DIRECTION_OUTGOING;
        // Outgoing requests include declined ones: the sender is not told.
        let sql = if outgoing {
            "SELECT r.id, r.follower_id, r.followee_id, r.created_at FROM follow_requests r JOIN users u ON u.id = r.followee_id
             WHERE r.follower_id = $1 AND u.deleted_at IS NULL AND ($2::uuid IS NULL OR r.id < $2) ORDER BY r.id DESC LIMIT $3"
        } else {
            "SELECT r.id, r.follower_id, r.followee_id, r.created_at FROM follow_requests r JOIN users u ON u.id = r.follower_id
             WHERE r.followee_id = $1 AND r.declined_at IS NULL AND u.deleted_at IS NULL AND u.banned_at IS NULL
               AND ($2::uuid IS NULL OR r.id < $2) ORDER BY r.id DESC LIMIT $3"
        };
        let mut rows: Vec<RequestRow> =
            sqlx::query_as(sql).bind(user_id).bind(before).bind(page.fetch()).fetch_all(&self.core.db).await?;
        let info = page.finish(&mut rows, |r| r.id);
        let incoming = self.incoming_follow_request_count(user_id).await?;
        Ok((self.follow_requests_to_proto(user_id, rows).await?, info, incoming))
    }

    async fn incoming_follow_request_count(&self, user_id: Uuid) -> AppResult<i32> {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM follow_requests r JOIN users u ON u.id = r.follower_id
             WHERE r.followee_id = $1 AND r.declined_at IS NULL AND u.deleted_at IS NULL AND u.banned_at IS NULL",
        )
        .bind(user_id)
        .fetch_one(&self.core.db)
        .await?;
        Ok(count as i32)
    }

    /// One request as `viewer` (one of its two parties) sees it.
    async fn follow_request(&self, viewer: Uuid, request_id: Uuid) -> AppResult<FollowRequestEntry> {
        let row: RequestRow =
            sqlx::query_as("SELECT id, follower_id, followee_id, created_at FROM follow_requests WHERE id = $1")
                .bind(request_id)
                .fetch_one(&self.core.db)
                .await?;
        Ok(self.follow_requests_to_proto(viewer, vec![row]).await?.remove(0))
    }

    async fn follow_requests_to_proto(
        &self,
        viewer: Uuid,
        rows: Vec<RequestRow>,
    ) -> AppResult<Vec<FollowRequestEntry>> {
        let mut ids: Vec<Uuid> = rows.iter().flat_map(|r| [r.follower_id, r.followee_id]).collect();
        ids.sort_unstable();
        ids.dedup();
        let summaries = tokuly_identity::summaries(&self.core, &ids).await?;
        let others: Vec<Uuid> =
            rows.iter().map(|r| if r.follower_id == viewer { r.followee_id } else { r.follower_id }).collect();
        let known = self.known_follower_counts(viewer, &others).await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let other = if row.follower_id == viewer { row.followee_id } else { row.follower_id };
                FollowRequestEntry {
                    id: row.id.to_string(),
                    from: users::pick(&summaries, row.follower_id).into(),
                    to: users::pick(&summaries, row.followee_id).into(),
                    created_at: ts(row.created_at),
                    known_follower_count: known.get(&other).copied().unwrap_or(0),
                    ..Default::default()
                }
            })
            .collect())
    }

    /// How many people `viewer` follows follow each of `others`, for those
    /// whose follow lists the viewer may see.
    async fn known_follower_counts(&self, viewer: Uuid, others: &[Uuid]) -> AppResult<HashMap<Uuid, i32>> {
        if others.is_empty() {
            return Ok(HashMap::new());
        }
        let visible = self.follow_lists_visible_to(viewer, others).await?;
        let visible: Vec<Uuid> = visible.into_iter().collect();
        let hidden = graph::blocked_ids_either(&self.core, viewer).await?;
        let rows: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT f.followee_id, count(*) FROM follows f
             JOIN follows mine ON mine.follower_id = $1 AND mine.followee_id = f.follower_id
             JOIN users u ON u.id = f.follower_id
             WHERE f.followee_id = ANY($2) AND u.deleted_at IS NULL AND u.banned_at IS NULL AND NOT (f.follower_id = ANY($3))
             GROUP BY f.followee_id",
        )
        .bind(viewer)
        .bind(&visible)
        .bind(&hidden)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows.into_iter().map(|(id, count)| (id, count as i32)).collect())
    }

    /// Which of `owners` let `viewer` see their follow lists: unlocked
    /// accounts to everyone not in a block with them, locked ones to their
    /// followers and friends. Everyone sees their own.
    pub(crate) async fn follow_lists_visible_to(&self, viewer: Uuid, owners: &[Uuid]) -> AppResult<HashSet<Uuid>> {
        if owners.is_empty() {
            return Ok(HashSet::new());
        }
        let visible: Vec<Uuid> = sqlx::query_scalar(concat!(
            "SELECT o.id FROM unnest($2::uuid[]) AS o(id)
             WHERE o.id = $1 OR (
                 NOT EXISTS (SELECT 1 FROM user_blocks b WHERE (b.blocker_id = $1 AND b.blocked_id = o.id) OR (b.blocker_id = o.id AND b.blocked_id = $1))
                 AND (NOT ",
            locked_sql!("o.id"),
            " OR EXISTS (SELECT 1 FROM follows f WHERE f.follower_id = $1 AND f.followee_id = o.id)
                    OR EXISTS (SELECT 1 FROM friendships f WHERE f.user_id = $1 AND f.friend_id = o.id)))"
        ))
        .bind(viewer)
        .bind(owners)
        .fetch_all(&self.core.db)
        .await?;
        Ok(visible.into_iter().collect())
    }

    /// What `viewer` is to each of `others`, as far as follows go.
    pub(crate) async fn follow_relations(
        &self,
        viewer: Uuid,
        others: &[Uuid],
    ) -> AppResult<HashMap<Uuid, FollowRelation>> {
        const FOLLOWING: i32 = 1;
        const REQUESTED: i32 = 2;
        const FOLLOWS_YOU: i32 = 3;
        const FRIENDS: i32 = 4;
        const BLOCKED: i32 = 5;
        let facts: Vec<(Uuid, i32)> = sqlx::query_as(
            "SELECT followee_id, 1 FROM follows WHERE follower_id = $1 AND followee_id = ANY($2)
             UNION ALL SELECT followee_id, 2 FROM follow_requests WHERE follower_id = $1 AND followee_id = ANY($2)
             UNION ALL SELECT follower_id, 3 FROM follows WHERE followee_id = $1 AND follower_id = ANY($2)
             UNION ALL SELECT friend_id, 4 FROM friendships WHERE user_id = $1 AND friend_id = ANY($2)
             UNION ALL SELECT blocked_id, 5 FROM user_blocks WHERE blocker_id = $1 AND blocked_id = ANY($2)",
        )
        .bind(viewer)
        .bind(others)
        .fetch_all(&self.core.db)
        .await?;
        let mut relations: HashMap<Uuid, FollowRelation> = others
            .iter()
            .map(|id| {
                (
                    *id,
                    FollowRelation {
                        following: FollowState::FOLLOW_STATE_NONE.into(),
                        self_: *id == viewer,
                        ..Default::default()
                    },
                )
            })
            .collect();
        for (other, fact) in facts {
            let Some(relation) = relations.get_mut(&other) else { continue };
            match fact {
                FOLLOWING => relation.following = FollowState::FOLLOW_STATE_FOLLOWING.into(),
                REQUESTED if relation.following.as_known() != Some(FollowState::FOLLOW_STATE_FOLLOWING) => {
                    relation.following = FollowState::FOLLOW_STATE_REQUESTED.into()
                }
                FOLLOWS_YOU => relation.follows_you = true,
                FRIENDS => relation.friends = true,
                BLOCKED => relation.blocked = true,
                _ => {}
            }
        }
        Ok(relations)
    }

    /// Rows of a follow list as `viewer` sees them: (user, since, own prefs).
    async fn follow_entries(
        &self,
        viewer: Uuid,
        rows: Vec<(Uuid, DateTime<Utc>, Option<PrefsRow>)>,
    ) -> AppResult<Vec<FollowEntry>> {
        let ids: Vec<Uuid> = rows.iter().map(|(id, _, _)| *id).collect();
        let summaries = tokuly_identity::summaries(&self.core, &ids).await?;
        let relations = self.follow_relations(viewer, &ids).await?;
        let now = Utc::now();
        Ok(rows
            .into_iter()
            .map(|(id, since, prefs)| FollowEntry {
                user: users::pick(&summaries, id).into(),
                followed_at: ts(since),
                relation: relations.get(&id).cloned().unwrap_or_default().into(),
                prefs: prefs.map(|p| p.to_proto(now)).into(),
                ..Default::default()
            })
            .collect())
    }

    /// Fails unless `viewer` may see `owner`'s follow lists.
    async fn ensure_can_see_follows_of(&self, viewer: Uuid, owner: Uuid) -> AppResult<()> {
        if viewer == owner {
            return Ok(());
        }
        people::ensure_visible(&self.core, viewer, owner).await?;
        ensure!(
            !self.follow_lists_visible_to(viewer, &[owner]).await?.is_empty(),
            denied,
            "FOLLOW_LIST_PRIVATE",
            "this account is locked: only its followers see whom it follows"
        );
        Ok(())
    }

    /// One page of one of `owner`'s follow lists as `viewer` sees it, plus
    /// the total matching `query`.
    pub(crate) async fn list_follows(
        &self,
        viewer: Uuid,
        owner: Uuid,
        list: FollowList,
        query: &str,
        page: &Page,
    ) -> AppResult<(Vec<FollowEntry>, PageInfo, i32)> {
        self.ensure_can_see_follows_of(viewer, owner).await?;
        let query = query.trim().trim_start_matches('@');
        ensure!(query.chars().count() <= MAX_QUERY_CHARS, invalid, "QUERY_TOO_LONG", "search query is too long");
        let hidden = if viewer == owner { Vec::new() } else { graph::blocked_ids_either(&self.core, viewer).await? };
        let cursor: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
        let (after_time, after_id) = cursor.unzip();

        // (the user in the row, the rows) for each list. $1 owner, $2 viewer.
        let (user, from) = match list {
            FollowList::FOLLOW_LIST_FOLLOWING => ("f.followee_id", "follows f WHERE f.follower_id = $1"),
            FollowList::FOLLOW_LIST_MUTUALS => (
                "f.followee_id",
                "follows f WHERE f.follower_id = $1 AND EXISTS (SELECT 1 FROM follows back_ WHERE back_.follower_id = f.followee_id AND back_.followee_id = $1)",
            ),
            FollowList::FOLLOW_LIST_KNOWN_FOLLOWERS => (
                "f.follower_id",
                "follows f WHERE f.followee_id = $1 AND EXISTS (SELECT 1 FROM follows mine_ WHERE mine_.follower_id = $2 AND mine_.followee_id = f.follower_id)",
            ),
            FollowList::FOLLOW_LIST_FOLLOWERS | FollowList::FOLLOW_LIST_UNSPECIFIED => {
                ("f.follower_id", "follows f WHERE f.followee_id = $1")
            }
        };
        let filter = format!(
            "u.id = {user} AND u.deleted_at IS NULL AND u.banned_at IS NULL AND NOT ({user} = ANY($3))
             AND ($4 OR u.display_name ILIKE $5 OR u.handle::text ILIKE $5)"
        );
        let own_following = viewer == owner && list == FollowList::FOLLOW_LIST_FOLLOWING;
        #[derive(sqlx::FromRow)]
        struct Row {
            user_id: Uuid,
            created_at: DateTime<Utc>,
            muted: bool,
            muted_until: Option<DateTime<Utc>>,
            notify_level: i16,
        }
        let sql = format!(
            "SELECT {user} AS user_id, f.created_at, f.muted, f.muted_until, f.notify_level FROM {from}
               AND EXISTS (SELECT 1 FROM users u WHERE {filter})
               AND ($6::timestamptz IS NULL OR (f.created_at, {user}) < ($6, $7))
             ORDER BY f.created_at DESC, {user} DESC
             LIMIT $8"
        );
        let pattern = contains_pattern(query);
        let mut rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(owner)
            .bind(viewer)
            .bind(&hidden)
            .bind(query.is_empty())
            .bind(&pattern)
            .bind(after_time)
            .bind(after_id)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {from} AND EXISTS (SELECT 1 FROM users u WHERE {filter})"
        )))
        .bind(owner)
        .bind(viewer)
        .bind(&hidden)
        .bind(query.is_empty())
        .bind(&pattern)
        .fetch_one(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |r| (r.created_at, r.user_id));
        let rows = rows
            .into_iter()
            .map(|r| {
                let prefs = own_following.then_some(PrefsRow {
                    muted: r.muted,
                    muted_until: r.muted_until,
                    notify_level: r.notify_level,
                });
                (r.user_id, r.created_at, prefs)
            })
            .collect();
        Ok((self.follow_entries(viewer, rows).await?, info, total as i32))
    }

    pub(crate) fn follows_page(page: Option<&tokuly_proto::common::PageRequestView<'_>>) -> Page {
        Page::from_view(page, MAX_PAGE)
    }

    /// Follower and following counts of `owner` as `viewer` sees them.
    pub(crate) async fn follow_counts(&self, viewer: Uuid, owner: Uuid) -> AppResult<(i32, i32)> {
        let hidden = if viewer == owner { Vec::new() } else { graph::blocked_ids_either(&self.core, viewer).await? };
        let (followers, following): (i64, i64) = sqlx::query_as(
            "SELECT
                 (SELECT count(*) FROM follows f JOIN users u ON u.id = f.follower_id
                  WHERE f.followee_id = $1 AND u.deleted_at IS NULL AND u.banned_at IS NULL AND NOT (f.follower_id = ANY($2))),
                 (SELECT count(*) FROM follows f JOIN users u ON u.id = f.followee_id
                  WHERE f.follower_id = $1 AND u.deleted_at IS NULL AND u.banned_at IS NULL AND NOT (f.followee_id = ANY($2)))",
        )
        .bind(owner)
        .bind(&hidden)
        .fetch_one(&self.core.db)
        .await?;
        Ok((followers as i32, following as i32))
    }

    /// What a profile header shows about follows.
    pub(crate) async fn follow_summary(&self, viewer: Uuid, owner: Uuid) -> AppResult<FollowSummary> {
        let core = &self.core;
        if viewer != owner {
            people::ensure_visible(core, viewer, owner).await?;
        }
        let (follower_count, following_count) = self.follow_counts(viewer, owner).await?;
        let relation = self.follow_relations(viewer, &[owner]).await?.remove(&owner).unwrap_or_default();
        let locked = is_locked(&core.db, owner).await?;
        let mut summary =
            FollowSummary { user_id: owner.to_string(), follower_count, following_count, locked, ..Default::default() };
        if viewer != owner && !self.follow_lists_visible_to(viewer, &[owner]).await?.is_empty() {
            let page = Page::new(KNOWN_PREVIEW as i32, "", KNOWN_PREVIEW, KNOWN_PREVIEW);
            let (known, _, total) =
                self.list_follows(viewer, owner, FollowList::FOLLOW_LIST_KNOWN_FOLLOWERS, "", &page).await?;
            summary.known_follower_count = total;
            summary.known_followers = known.into_iter().filter_map(|entry| entry.user.into_option()).collect();
        }
        if relation.following.as_known() == Some(FollowState::FOLLOW_STATE_FOLLOWING) {
            let prefs: Option<PrefsRow> = sqlx::query_as(
                "SELECT muted, muted_until, notify_level FROM follows WHERE follower_id = $1 AND followee_id = $2",
            )
            .bind(viewer)
            .bind(owner)
            .fetch_optional(&core.db)
            .await?;
            summary.prefs = prefs.map(|p| p.to_proto(Utc::now())).into();
        }
        summary.relation = relation.into();
        Ok(summary)
    }

    /// Changes what `follower` keeps about an account they follow.
    pub(crate) async fn update_follow_prefs(
        &self,
        follower: Uuid,
        followee: Uuid,
        muted: Option<bool>,
        mute_seconds: i64,
        notify_level: Option<FollowNotifyLevel>,
    ) -> AppResult<FollowPrefs> {
        let now = Utc::now();
        let until = if muted == Some(true) { mute_until(mute_seconds, now)? } else { None };
        self.core.cache.rate_limit(&format!("follow-prefs:{follower}"), 600, HOUR).await?;
        let mut tx = self.core.db.begin().await?;
        let current: Option<PrefsRow> =
            sqlx::query_as("SELECT muted, muted_until, notify_level FROM follows WHERE follower_id = $1 AND followee_id = $2 FOR UPDATE")
                .bind(follower)
                .bind(followee)
                .fetch_optional(&mut *tx)
                .await?;
        let current = current.or_not_found("NOT_FOLLOWING", "you do not follow this account")?;
        let mut next = current;
        if let Some(muted) = muted {
            next.muted = muted;
            next.muted_until = until;
        }
        if let Some(level) = notify_level.filter(|level| *level != FollowNotifyLevel::FOLLOW_NOTIFY_LEVEL_UNSPECIFIED) {
            next.notify_level = level as i16;
        }
        sqlx::query("UPDATE follows SET muted = $3, muted_until = $4, notify_level = $5 WHERE follower_id = $1 AND followee_id = $2")
            .bind(follower)
            .bind(followee)
            .bind(next.muted)
            .bind(next.muted_until)
            .bind(next.notify_level)
            .execute(&mut *tx)
            .await?;
        if (next.muted, next.muted_until, next.notify_level)
            != (current.muted, current.muted_until, current.notify_level)
        {
            let event = FollowPrefsChanged {
                follower_id: follower,
                followee_id: followee,
                muted: next.muted,
                muted_until: next.muted_until,
                notify_level: i32::from(next.notify_level),
            };
            events::emit(&mut *tx, FOLLOW_PREFS_CHANGED, Some(follower), &event).await?;
        }
        tx.commit().await?;
        Ok(next.to_proto(now))
    }

    /// The caller's follow settings and how many requests wait for them.
    pub(crate) async fn follow_settings(&self, user_id: Uuid) -> AppResult<(FollowSettings, i32)> {
        let locked = is_locked(&self.core.db, user_id).await?;
        Ok((FollowSettings { locked, ..Default::default() }, self.incoming_follow_request_count(user_id).await?))
    }

    /// Locks or unlocks `user_id`'s account. Unlocking approves the requests
    /// that are still waiting; returns how many.
    pub(crate) async fn update_follow_settings(&self, user_id: Uuid, locked: bool) -> AppResult<(FollowSettings, i32)> {
        let core = &self.core;
        core.cache.rate_limit(&format!("follow-settings:{user_id}"), 30, HOUR).await?;
        let owner = users::summary(core, user_id).await?;
        let mut tx = core.db.begin().await?;
        graph::lock_follow_settings(&mut tx, user_id, true).await?;
        let was_locked = is_locked(&mut *tx, user_id).await?;
        sqlx::query("INSERT INTO follow_settings (user_id, locked) VALUES ($1, $2) ON CONFLICT (user_id) DO UPDATE SET locked = EXCLUDED.locked")
            .bind(user_id)
            .bind(locked)
            .execute(&mut *tx)
            .await?;
        let mut approved = Vec::new();
        if was_locked != locked {
            events::emit(&mut *tx, FOLLOW_SETTINGS_CHANGED, Some(user_id), &FollowSettingsChanged { user_id, locked })
                .await?;
        }
        if !locked {
            let waiting: Vec<(Uuid, Uuid)> =
                sqlx::query_as("SELECT id, follower_id FROM follow_requests WHERE followee_id = $1 AND declined_at IS NULL ORDER BY id FOR UPDATE")
                    .bind(user_id)
                    .fetch_all(&mut *tx)
                    .await?;
            approve_requests(&mut tx, &owner, &waiting).await?;
            approved = waiting.into_iter().map(|(_, follower)| follower).collect();
            // Requests declined earlier go too, without a word: their
            // senders now see an open account they do not follow, which is
            // also what a request withdrawn or a follower removed looks like.
            // Keeping them "pending" on an account that takes no requests
            // would tell them they had been declined.
            sqlx::query("DELETE FROM follow_requests WHERE followee_id = $1 AND declined_at IS NOT NULL")
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        if !approved.is_empty() {
            idcache::follows_changed(core, &approved, &[user_id]).await;
            for requester in &approved {
                self.push_follow_entry(*requester, user_id, Push::FollowRequestApproved).await;
            }
        }
        Ok((FollowSettings { locked, ..Default::default() }, approved.len() as i32))
    }
}

/// Inside `tx`: turns requests (id, requester) to `approver` into follows,
/// with the event and the requester's notification for each.
async fn approve_requests(
    tx: &mut Transaction<'_, Postgres>,
    approver: &UserSummary,
    requests: &[(Uuid, Uuid)],
) -> AppResult<()> {
    if requests.is_empty() {
        return Ok(());
    }
    let approver_id: Uuid = approver.id.parse().map_err(|_| AppError::invalid("INVALID_ID", "invalid user id"))?;
    let ids: Vec<Uuid> = requests.iter().map(|(id, _)| *id).collect();
    sqlx::query("DELETE FROM follow_requests WHERE id = ANY($1)").bind(&ids).execute(&mut **tx).await?;
    let requesters: Vec<Uuid> = requests.iter().map(|(_, requester)| *requester).collect();
    let created: Vec<Uuid> = sqlx::query_scalar(
        "INSERT INTO follows (follower_id, followee_id) SELECT r, $2 FROM unnest($1::uuid[]) AS r ON CONFLICT DO NOTHING RETURNING follower_id",
    )
    .bind(&requesters)
    .bind(approver_id)
    .fetch_all(&mut **tx)
    .await?;
    let text = copy::follow_approved(&people::name_of(approver));
    for follower in created {
        events::emit(
            &mut **tx,
            FOLLOW_CREATED,
            Some(approver_id),
            &FollowCreated { follower_id: follower, followee_id: approver_id, approved: true },
        )
        .await?;
        outbound::notify(
            &mut **tx,
            &NotificationRequest::new(
                follower,
                "social.follow_approved",
                text.title.clone(),
                text.body.clone(),
                format!("/u/{}", approver.handle),
            )
            .actor(approver_id)
            .subject(SubjectType::SUBJECT_TYPE_USER, approver_id),
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTIFY_NONE: i16 = FollowNotifyLevel::FOLLOW_NOTIFY_LEVEL_NONE as i16;

    #[test]
    fn mutes_last_from_an_hour_to_a_year_or_until_unmuted() {
        let now = Utc::now();
        assert_eq!(mute_until(0, now).unwrap(), None);
        assert_eq!(mute_until(3600, now).unwrap(), Some(now + chrono::Duration::hours(1)));
        for bad in [-1, 59, 3599, MAX_MUTE_SECONDS + 1] {
            assert_eq!(mute_until(bad, now).unwrap_err().reason, "INVALID_MUTE_DURATION", "{bad}");
        }
    }

    #[test]
    fn a_timed_mute_that_ran_out_reads_as_no_mute() {
        let now = Utc::now();
        let ran_out =
            PrefsRow { muted: true, muted_until: Some(now - chrono::Duration::minutes(1)), notify_level: NOTIFY_NONE };
        assert!(!ran_out.to_proto(now).muted);
        let running = PrefsRow { muted_until: Some(now + chrono::Duration::minutes(1)), ..ran_out };
        assert!(running.to_proto(now).muted && running.to_proto(now).muted_until.is_set());
        let forever = PrefsRow { muted_until: None, ..ran_out };
        assert!(forever.to_proto(now).muted && !forever.to_proto(now).muted_until.is_set());
    }
}
