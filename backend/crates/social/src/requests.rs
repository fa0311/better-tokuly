//! Friend requests and the moment two users become friends.
//!
//! Rules worth knowing:
//! - A request to someone who already requested you is not stored: the two
//!   become friends on the spot.
//! - Declining is silent. The request stays (hidden from the recipient), so
//!   the sender still sees it as pending and cannot simply send it again.
//! - Every change to a pair runs under [`graph::lock_pair`], and nothing
//!   inside such a transaction takes a second pooled connection: all
//!   pool-backed checks happen before `begin()` and are re-validated under
//!   the lock where a race would matter.
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::{PgExecutor, Postgres, Transaction};
use tokuly_core::{
    events,
    outbound::{self, ActivityAudience, ActivityEntry, NotificationRequest},
    prelude::*,
};
use tokuly_identity::{settings, users};
use tokuly_proto::{
    common::{PageInfo, SubjectType, UserSummary},
    identity::Audience,
    social::{Friend, FriendRequest, FriendRequestDirection, FriendSource, social_event::Event as Push},
};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    Social, copy,
    events::{FRIEND_ACCEPTED, FRIEND_REQUESTED, FriendAccepted, FriendRequested},
    graph, idcache, people,
};

const MAX_MESSAGE_GRAPHEMES: usize = 140;
/// Most unanswered requests one user can have out at a time.
const MAX_PENDING_OUTGOING: i64 = 500;
const MAX_REQUESTS_PAGE: i64 = 100;

/// Who is asking for a friendship to be requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestVia {
    /// The user, through `SendFriendRequest`: the target's privacy and the
    /// rate limits apply.
    User,
    /// The server, because both users just did something that shows they
    /// want to connect (bumped phones, joined through the other's invite).
    /// The target's "who may send me requests" audience does not apply.
    Encounter,
}

/// What became of an attempt to request a friendship.
pub(crate) enum RequestOutcome {
    /// A request is pending. `created` is false when it had been sent before.
    Pending { request: FriendRequest, created: bool },
    /// The other side had already asked: the two are friends now.
    Friends,
}

/// Columns of [`RequestRow`].
macro_rules! request_columns {
    () => {
        "id, from_user_id, to_user_id, source, message, created_at"
    };
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct RequestRow {
    id: Uuid,
    from_user_id: Uuid,
    to_user_id: Uuid,
    source: i32,
    message: String,
    created_at: DateTime<Utc>,
}

/// Everything that decides what a request from `me` to `them` may do.
#[derive(Debug, sqlx::FromRow)]
struct PairState {
    blocked_by_me: bool,
    blocked_by_them: bool,
    friends: bool,
    outgoing_id: Option<Uuid>,
    incoming_id: Option<Uuid>,
}

impl PairState {
    async fn load<'e, E: PgExecutor<'e>>(executor: E, me: Uuid, them: Uuid) -> AppResult<Self> {
        Ok(sqlx::query_as(
            "SELECT
                 EXISTS(SELECT 1 FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2) AS blocked_by_me,
                 EXISTS(SELECT 1 FROM user_blocks WHERE blocker_id = $2 AND blocked_id = $1) AS blocked_by_them,
                 EXISTS(SELECT 1 FROM friendships WHERE user_id = $1 AND friend_id = $2) AS friends,
                 (SELECT id FROM friend_requests WHERE from_user_id = $1 AND to_user_id = $2) AS outgoing_id,
                 (SELECT id FROM friend_requests WHERE from_user_id = $2 AND to_user_id = $1) AS incoming_id",
        )
        .bind(me)
        .bind(them)
        .fetch_one(executor)
        .await?)
    }

    /// The reasons a request can never go through, whoever asks.
    fn ensure_requestable(&self) -> AppResult<()> {
        // Being blocked is indistinguishable from the user not existing.
        ensure!(!self.blocked_by_them, not_found, "USER_NOT_FOUND", "user not found");
        ensure!(!self.blocked_by_me, precondition, "USER_BLOCKED", "unblock this user before sending a friend request");
        ensure!(!self.friends, already_exists, "ALREADY_FRIENDS", "you are already friends");
        Ok(())
    }
}

/// One of the two people of a new friendship.
struct Party {
    id: Uuid,
    summary: UserSummary,
    /// Who may read "became friends with ..." in this user's activity: their
    /// friends, unless they keep their friend list to themselves.
    announce_to: ActivityAudience,
}

/// The two people of a new friendship, loaded before the transaction that
/// creates it (which must not touch the pool).
struct Introductions {
    accepter: Party,
    requester: Party,
}

impl Introductions {
    async fn load(core: &Core, accepter: Uuid, requester: Uuid) -> AppResult<Self> {
        let summaries = tokuly_identity::summaries(core, &[accepter, requester]).await?;
        let privacy = settings::privacy_of_many(core, &[accepter, requester]).await?;
        let party = |id: Uuid| Party {
            id,
            summary: users::pick(&summaries, id),
            announce_to: match privacy.get(&id).map(|p| people::audience(&p.friend_list_visible_to)) {
                Some(Audience::AUDIENCE_NOBODY) => ActivityAudience::Private,
                _ => ActivityAudience::Friends,
            },
        };
        Ok(Self { accepter: party(accepter), requester: party(requester) })
    }
}

fn normalize_message(input: &str) -> AppResult<String> {
    let message: String = input.chars().filter(|c| !c.is_control() || *c == '\n').collect();
    let message = message.trim();
    ensure!(
        message.graphemes(true).count() <= MAX_MESSAGE_GRAPHEMES,
        invalid,
        "MESSAGE_TOO_LONG",
        "the greeting is at most {MAX_MESSAGE_GRAPHEMES} characters"
    );
    Ok(message.to_owned())
}

impl Social {
    /// Requests a friendship from `from` to `to`, or completes one when `to`
    /// had already asked. Sending the same request twice is harmless.
    pub(crate) async fn request_friendship(&self, from: Uuid, to: Uuid, source: FriendSource, message: &str, via: RequestVia) -> AppResult<RequestOutcome> {
        ensure!(from != to, invalid, "CANNOT_FRIEND_SELF", "you cannot befriend yourself");
        let message = normalize_message(message)?;
        let core = &self.core;

        // Pre-flight on the pool; the decisive checks are repeated under the lock.
        let state = PairState::load(&core.db, from, to).await?;
        state.ensure_requestable()?;
        let is_new_request = state.incoming_id.is_none() && state.outgoing_id.is_none();
        if is_new_request {
            self.ensure_may_send_request(from, to, via).await?;
        }
        // `from` takes the accepter's seat: that is what they are if `to`
        // turns out to have asked first.
        let parties = Introductions::load(core, from, to).await?;

        let mut tx = core.db.begin().await?;
        graph::lock_pair(&mut tx, from, to).await?;
        let state = PairState::load(&mut *tx, from, to).await?;
        state.ensure_requestable()?;
        let is_new_after_all = state.incoming_id.is_none() && state.outgoing_id.is_none();
        if is_new_after_all && !is_new_request {
            // The request this call was about to answer or repeat was
            // withdrawn in the meantime. Storing a new one now would skip the
            // privacy and limit checks, so the caller has to come again.
            return Err(AppError::new(
                connectrpc::ErrorCode::Aborted,
                "REQUEST_CHANGED",
                "the friend request changed while it was being processed; try again",
            ));
        }

        if state.incoming_id.is_some() {
            // Crossed requests become a friendship on the spot.
            self.befriend(&mut tx, source, &parties).await?;
            tx.commit().await?;
            self.after_befriended(from, to).await;
            return Ok(RequestOutcome::Friends);
        }

        // Either the request already exists, or it is stored now.
        let (row, is_news): (RequestRow, bool) = match state.outgoing_id {
            Some(request_id) => {
                // An encounter (a bump, an invite) is a fresh sign from both
                // sides, so it puts a request the recipient once declined back
                // in front of them. Asking again by hand does not.
                let revived = via == RequestVia::Encounter
                    && sqlx::query("UPDATE friend_requests SET declined_at = NULL, source = $2 WHERE id = $1 AND declined_at IS NOT NULL")
                        .bind(request_id)
                        .bind(source as i32)
                        .execute(&mut *tx)
                        .await?
                        .rows_affected()
                        > 0;
                let row = sqlx::query_as(concat!("SELECT ", request_columns!(), " FROM friend_requests WHERE id = $1"))
                    .bind(request_id)
                    .fetch_one(&mut *tx)
                    .await?;
                (row, revived)
            }
            None => {
                let row = sqlx::query_as(concat!(
                    "INSERT INTO friend_requests (from_user_id, to_user_id, source, message) VALUES ($1, $2, $3, $4) RETURNING ",
                    request_columns!()
                ))
                .bind(from)
                .bind(to)
                .bind(source as i32)
                .bind(&message)
                .fetch_one(&mut *tx)
                .await?;
                (row, true)
            }
        };
        if is_news {
            let requested = FriendRequested { request_id: row.id, from_user_id: from, to_user_id: to, source: source as i32 };
            events::emit(&mut *tx, FRIEND_REQUESTED, Some(from), &requested).await?;
            let sender = &parties.accepter.summary; // `from`, see above
            let text = copy::friend_request(&people::name_of(sender), source);
            outbound::notify(
                &mut *tx,
                &NotificationRequest::new(to, "social.friend_request", text.title, text.body, format!("/u/{}", sender.handle))
                    .actor(from)
                    .subject(SubjectType::SUBJECT_TYPE_USER, from)
                    .collapse(format!("social.friend_request:{from}")),
            )
            .await?;
        }
        tx.commit().await?;

        if is_news {
            // The recipient sees the request with their own mutual-friend count.
            let for_recipient = self.requests_to_proto(to, vec![row.clone()]).await?.remove(0);
            self.push(to, Push::FriendRequestReceived(Box::new(for_recipient))).await;
        }
        let request = self.requests_to_proto(from, vec![row]).await?.remove(0);
        Ok(RequestOutcome::Pending { request, created: is_news })
    }

    /// The checks that only apply to a brand-new request: the target's
    /// privacy, both sides' room for another friend, and the sender's limits.
    async fn ensure_may_send_request(&self, from: Uuid, to: Uuid, via: RequestVia) -> AppResult<()> {
        let core = &self.core;
        if via == RequestVia::User {
            let privacy = settings::privacy_of(core, to).await?;
            let allowed = graph::allows(core, to, Some(from), people::audience(&privacy.friend_requests_from)).await?;
            ensure!(allowed, denied, "FRIEND_REQUESTS_RESTRICTED", "this user does not accept friend requests from you");
        }
        ensure!(graph::friend_count(core, from).await? < self.cfg.max_friends, precondition, "FRIEND_LIMIT_REACHED", "you have reached the friend limit");
        let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM friend_requests WHERE from_user_id = $1").bind(from).fetch_one(&core.db).await?;
        ensure!(pending < MAX_PENDING_OUTGOING, precondition, "TOO_MANY_PENDING_REQUESTS", "too many friend requests are waiting for an answer");
        if via == RequestVia::User {
            let hour = Duration::from_secs(3600);
            core.cache.rate_limit(&format!("friend-request:{from}"), 60, hour).await?;
            core.cache.rate_limit(&format!("friend-request-day:{from}"), 200, 24 * hour).await?;
            // Cancelling and re-sending must not become a way to ping someone.
            core.cache.rate_limit(&format!("friend-request-pair:{from}:{to}"), 5, 24 * hour).await?;
        }
        Ok(())
    }

    /// Makes the two parties friends inside `tx`: both friendship rows, no
    /// more requests between them, the event, the requester's notification
    /// and both activity entries. Call [`Self::after_befriended`] once the
    /// transaction has committed.
    async fn befriend(&self, tx: &mut Transaction<'_, Postgres>, source: FriendSource, parties: &Introductions) -> AppResult<()> {
        let (accepter, requester) = (&parties.accepter, &parties.requester);
        for party in [accepter, requester] {
            let friends: i64 = sqlx::query_scalar("SELECT count(*) FROM friendships WHERE user_id = $1").bind(party.id).fetch_one(&mut **tx).await?;
            ensure!(friends < self.cfg.max_friends, precondition, "FRIEND_LIMIT_REACHED", "one of you has reached the friend limit");
        }
        sqlx::query("INSERT INTO friendships (user_id, friend_id, source) VALUES ($1, $2, $3), ($2, $1, $3)")
            .bind(accepter.id)
            .bind(requester.id)
            .bind(source as i32)
            .execute(&mut **tx)
            .await?;
        sqlx::query("DELETE FROM friend_requests WHERE (from_user_id = $1 AND to_user_id = $2) OR (from_user_id = $2 AND to_user_id = $1)")
            .bind(accepter.id)
            .bind(requester.id)
            .execute(&mut **tx)
            .await?;
        let accepted = FriendAccepted { user_id: accepter.id, friend_id: requester.id, source: source as i32 };
        events::emit(&mut **tx, FRIEND_ACCEPTED, Some(accepter.id), &accepted).await?;

        let text = copy::friend_accepted(&people::name_of(&accepter.summary));
        outbound::notify(
            &mut **tx,
            &NotificationRequest::new(requester.id, "social.friend_accepted", text.title, text.body, format!("/u/{}", accepter.summary.handle))
                .actor(accepter.id)
                .subject(SubjectType::SUBJECT_TYPE_USER, accepter.id),
        )
        .await?;
        for (user, friend) in [(accepter, requester), (requester, accepter)] {
            outbound::activity(
                &mut **tx,
                &ActivityEntry::new(user.id, "social.friend_added", user.announce_to, copy::became_friends(&people::name_of(&friend.summary)))
                    .subject(SubjectType::SUBJECT_TYPE_USER, friend.id)
                    .with([friend.id])
                    .path(format!("/u/{}", friend.summary.handle)),
            )
            .await?;
        }
        Ok(())
    }

    /// After a new friendship committed: refresh cached friend lists and tell
    /// the requester's devices that they were accepted.
    async fn after_befriended(&self, accepter: Uuid, requester: Uuid) {
        idcache::FRIENDS.invalidate(&self.core, &[accepter, requester]).await;
        match self.friend_as_seen_by(requester, accepter).await {
            Ok(friend) => self.push(requester, Push::FriendRequestAccepted(Box::new(friend))).await,
            Err(e) => tracing::warn!(error = %e, "could not push the accepted friendship; clients catch up on their next refresh"),
        }
    }

    /// Accepts or (silently) declines a request addressed to `user_id`.
    /// Returns the new friend when accepted.
    pub(crate) async fn respond_to_request(&self, user_id: Uuid, request_id: Uuid, accept: bool) -> AppResult<Option<Friend>> {
        let core = &self.core;
        let pending: Option<(Uuid, i32)> =
            sqlx::query_as("SELECT from_user_id, source FROM friend_requests WHERE id = $1 AND to_user_id = $2 AND declined_at IS NULL")
                .bind(request_id)
                .bind(user_id)
                .fetch_optional(&core.db)
                .await?;
        let (requester, source) = pending.or_not_found("REQUEST_NOT_FOUND", "friend request not found")?;
        let parties = Introductions::load(core, user_id, requester).await?;

        let mut tx = core.db.begin().await?;
        graph::lock_pair(&mut tx, user_id, requester).await?;
        // Under the lock: it may have been cancelled or answered meanwhile.
        let still_pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM friend_requests WHERE id = $1 AND declined_at IS NULL)")
            .bind(request_id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(still_pending, not_found, "REQUEST_NOT_FOUND", "friend request not found");
        if !accept {
            sqlx::query("UPDATE friend_requests SET declined_at = now() WHERE id = $1").bind(request_id).execute(&mut *tx).await?;
            tx.commit().await?;
            return Ok(None);
        }
        let source = buffa::EnumValue::<FriendSource>::from(source).as_known().unwrap_or(FriendSource::FRIEND_SOURCE_UNSPECIFIED);
        self.befriend(&mut tx, source, &parties).await?;
        tx.commit().await?;
        self.after_befriended(user_id, requester).await;
        Ok(Some(self.friend_as_seen_by(user_id, requester).await?))
    }

    /// Withdraws a request `user_id` sent.
    pub(crate) async fn cancel_request(&self, user_id: Uuid, request_id: Uuid) -> AppResult<()> {
        let cancelled: Option<(Uuid, bool)> =
            sqlx::query_as("DELETE FROM friend_requests WHERE id = $1 AND from_user_id = $2 RETURNING to_user_id, declined_at IS NOT NULL")
                .bind(request_id)
                .bind(user_id)
                .fetch_optional(&self.core.db)
                .await?;
        let (recipient, was_declined) = cancelled.or_not_found("REQUEST_NOT_FOUND", "friend request not found")?;
        if !was_declined {
            // A declined request is already gone from the recipient's screen.
            self.push(recipient, Push::FriendRequestCancelledId(request_id.to_string())).await;
        }
        Ok(())
    }

    /// One page of requests to (`INCOMING`, the default) or from `user_id`,
    /// newest first, and the number of requests waiting for their answer.
    pub(crate) async fn list_requests(&self, user_id: Uuid, direction: FriendRequestDirection, page: &Page) -> AppResult<(Vec<FriendRequest>, PageInfo, i32)> {
        let before: Option<Uuid> = page.decode()?;
        let outgoing = direction == FriendRequestDirection::FRIEND_REQUEST_DIRECTION_OUTGOING;
        // Outgoing requests include declined ones: the sender is not told.
        let sql = if outgoing {
            concat!(
                "SELECT ",
                request_columns!(),
                " FROM friend_requests WHERE from_user_id = $1 AND ($2::uuid IS NULL OR id < $2) ORDER BY id DESC LIMIT $3"
            )
        } else {
            concat!(
                "SELECT ",
                request_columns!(),
                " FROM friend_requests WHERE to_user_id = $1 AND declined_at IS NULL AND ($2::uuid IS NULL OR id < $2) ORDER BY id DESC LIMIT $3"
            )
        };
        let mut rows: Vec<RequestRow> = sqlx::query_as(sql).bind(user_id).bind(before).bind(page.fetch()).fetch_all(&self.core.db).await?;
        let info = page.finish(&mut rows, |r| r.id);
        let incoming_count: i64 = sqlx::query_scalar("SELECT count(*) FROM friend_requests WHERE to_user_id = $1 AND declined_at IS NULL")
            .bind(user_id)
            .fetch_one(&self.core.db)
            .await?;
        Ok((self.requests_to_proto(user_id, rows).await?, info, incoming_count as i32))
    }

    pub(crate) fn requests_page(page: Option<&tokuly_proto::common::PageRequestView<'_>>) -> Page {
        Page::from_view(page, MAX_REQUESTS_PAGE)
    }

    /// Request rows as `viewer` (one of the two parties) sees them.
    async fn requests_to_proto(&self, viewer: Uuid, rows: Vec<RequestRow>) -> AppResult<Vec<FriendRequest>> {
        let mut user_ids: Vec<Uuid> = rows.iter().flat_map(|r| [r.from_user_id, r.to_user_id]).collect();
        user_ids.sort_unstable();
        user_ids.dedup();
        let summaries = tokuly_identity::summaries(&self.core, &user_ids).await?;
        let mutual = graph::mutual_friend_counts(&self.core, viewer, &user_ids).await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let other = if row.from_user_id == viewer { row.to_user_id } else { row.from_user_id };
                FriendRequest {
                    id: row.id.to_string(),
                    from: users::pick(&summaries, row.from_user_id).into(),
                    to: users::pick(&summaries, row.to_user_id).into(),
                    source: row.source.into(),
                    message: row.message,
                    created_at: ts(row.created_at),
                    mutual_friend_count: mutual.get(&other).copied().unwrap_or(0),
                    ..Default::default()
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greetings_are_trimmed_and_limited() {
        assert_eq!(normalize_message("  よろしく!\n\u{0000} ").unwrap(), "よろしく!");
        assert!(normalize_message(&"あ".repeat(140)).is_ok());
        assert_eq!(normalize_message(&"あ".repeat(141)).unwrap_err().reason, "MESSAGE_TOO_LONG");
    }
}
