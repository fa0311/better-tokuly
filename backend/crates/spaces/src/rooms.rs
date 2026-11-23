//! A room's life — created, scheduled, started, ended — and the building
//! blocks every change to a room uses.
//!
//! Every change happens in a transaction that first locks the room's row
//! ([`lock_space`]), so two people acting on the same room never see half
//! of each other's change. The counters on the row and the two clocks that
//! decide when a room is over (`host_absent_since`, `empty_since`) are
//! recomputed in that transaction ([`recount`]). What follows a change —
//! realtime pushes, LiveKit — happens after the commit.
use sqlx::{Postgres, Transaction};
use tokuly_core::{events, outbound, prelude::*};
use tokuly_proto::spaces as pb;
use tokuly_social::graph;

use crate::{
    Spaces,
    events::{ROOM_ENDED, ROOM_STARTED, RoomEnded, RoomStarted},
    invites::Invited,
    limits::Limit,
    model::{PARTICIPANT, ParticipantRow, SPACE, SpaceRow, Venue, audience, end, leave, role, state},
    notices,
    rules::{self, Access},
};

pub(crate) type Tx<'a> = Transaction<'a, Postgres>;

pub(crate) fn space_not_found() -> AppError {
    AppError::not_found("SPACE_NOT_FOUND", "room not found")
}

pub(crate) fn not_host() -> AppError {
    AppError::denied("NOT_HOST", "only the host can do this")
}

pub(crate) fn not_live() -> AppError {
    AppError::precondition("SPACE_NOT_LIVE", "this room is not live")
}

pub(crate) fn space_ended() -> AppError {
    AppError::precondition("SPACE_ENDED", "this room is over")
}

fn not_scheduled() -> AppError {
    AppError::precondition("SPACE_NOT_SCHEDULED", "this room is not waiting to start any more")
}

pub(crate) fn already_hosting() -> AppError {
    AppError::precondition("ALREADY_HOSTING", "a host runs one live room at a time")
}

/// Reason of the error [`ensure_nowhere_else`] fails with.
const ENTERING_ELSEWHERE: &str = "ENTERING_ELSEWHERE";
/// How often entering is tried again after taking the person out of
/// wherever else they turned up.
const ENTER_RETRIES: usize = 3;

fn entering_elsewhere() -> AppError {
    AppError::unavailable(ENTERING_ELSEWHERE, "you are entering another room right now; try again in a moment")
}

/// Serializes, for the rest of the transaction, everything that puts
/// `user_id` inside a room or hangout: a person is in one place at a time,
/// and two requests entering two rooms at once must not both pass the
/// check ([`ensure_nowhere_else`]). Taken before the lock of the room or
/// hangout itself.
pub(crate) async fn lock_presence(tx: &mut Tx<'_>, user_id: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("spaces:inside:{}", user_id.simple()))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Fails with `ENTERING_ELSEWHERE` when `user_id` is inside something
/// other than `venue`. Call it under [`lock_presence`], once everything
/// else about entering is settled, right before the person is put inside;
/// [`Spaces::entering`] then takes them out of the other place and tries
/// again.
pub(crate) async fn ensure_nowhere_else(tx: &mut Tx<'_>, user_id: Uuid, venue: Venue) -> AppResult<()> {
    let (space_id, hangout_id) = match venue {
        Venue::Space(id) => (Some(id), None),
        Venue::Hangout(id) => (None, Some(id)),
    };
    let elsewhere: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM space_participants
                        WHERE user_id = $1 AND left_at IS NULL AND space_id IS DISTINCT FROM $2)
             OR EXISTS (SELECT 1 FROM hangout_presence WHERE user_id = $1 AND hangout_id IS DISTINCT FROM $3)",
    )
    .bind(user_id)
    .bind(space_id)
    .bind(hangout_id)
    .fetch_one(&mut **tx)
    .await?;
    if elsewhere { Err(entering_elsewhere()) } else { Ok(()) }
}

/// Someone who was inside and is not any more.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct Departed {
    pub user_id: Uuid,
    /// The role they had inside.
    pub role: i16,
    pub hand_raised: bool,
    /// The raised hand was one everyone in the room could see.
    pub hand_public: bool,
}

/// A room that just ended, with the people it sent home.
#[derive(Debug, Clone)]
pub(crate) struct Finished {
    pub space: SpaceRow,
    pub departed: Vec<Departed>,
}

/// Locks a room for the rest of the transaction.
pub(crate) async fn lock_space(tx: &mut Tx<'_>, space_id: Uuid) -> AppResult<SpaceRow> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {SPACE} FROM spaces s WHERE s.id = $1 FOR UPDATE")))
        .bind(space_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(space_not_found)
}

/// Recomputes a room's counters from who is inside, and starts or stops
/// the clocks for "the host is away" and "nobody is inside". Returns the
/// room as it is now.
pub(crate) async fn recount(tx: &mut Tx<'_>, space_id: Uuid) -> AppResult<SpaceRow> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "UPDATE spaces s SET
             listener_count = c.inside, speaker_count = c.stage, hand_count = c.hands,
             peak_listeners = GREATEST(s.peak_listeners, c.inside), total_listeners = c.total,
             host_absent_since = CASE WHEN s.state <> $2 OR c.host_inside THEN NULL
                                      ELSE COALESCE(s.host_absent_since, now()) END,
             empty_since = CASE WHEN s.state <> $2 OR c.inside > 0 THEN NULL ELSE COALESCE(s.empty_since, now()) END
         FROM (SELECT count(*) FILTER (WHERE left_at IS NULL)::int AS inside,
                      count(*) FILTER (WHERE left_at IS NULL AND role >= $3)::int AS stage,
                      count(*) FILTER (WHERE left_at IS NULL AND hand_raised_at IS NOT NULL)::int AS hands,
                      count(*)::int AS total,
                      COALESCE(bool_or(left_at IS NULL AND role = $4), false) AS host_inside
               FROM space_participants WHERE space_id = $1) c
         WHERE s.id = $1
         RETURNING {SPACE}"
    )))
    .bind(space_id)
    .bind(state::LIVE)
    .bind(role::SPEAKER)
    .bind(role::HOST)
    .fetch_one(&mut **tx)
    .await?)
}

/// Someone's row in a room, whether they are inside or were.
pub(crate) async fn participant_in_tx(
    tx: &mut Tx<'_>,
    space_id: Uuid,
    user_id: Uuid,
) -> AppResult<Option<ParticipantRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {PARTICIPANT} FROM space_participants p WHERE p.space_id = $1 AND p.user_id = $2"
    )))
    .bind(space_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?)
}

/// Marks people inside a room as having left: everyone (`who` = `None`)
/// or those named. Adds up their time inside, drops raised hands and stage
/// invitations and sets the role they would return with (see
/// [`rules::role_after_leaving`]). Returns who actually was inside.
///
/// Call [`recount`] afterwards.
pub(crate) async fn depart(
    tx: &mut Tx<'_>,
    space_id: Uuid,
    who: Option<&[Uuid]>,
    reason: i16,
) -> AppResult<Vec<Departed>> {
    let inside: Vec<Departed> = sqlx::query_as(
        "SELECT user_id, role, hand_raised_at IS NOT NULL AS hand_raised, hand_raised_at IS NOT NULL AND hand_public AS hand_public
         FROM space_participants
         WHERE space_id = $1 AND left_at IS NULL AND ($2::uuid[] IS NULL OR user_id = ANY($2))
         ORDER BY user_id
         FOR UPDATE",
    )
    .bind(space_id)
    .bind(who)
    .fetch_all(&mut **tx)
    .await?;
    if inside.is_empty() {
        return Ok(inside);
    }
    let ids: Vec<Uuid> = inside.iter().map(|departed| departed.user_id).collect();
    let roles: Vec<i16> = inside.iter().map(|departed| rules::role_after_leaving(departed.role, reason)).collect();
    sqlx::query(
        "UPDATE space_participants p SET
             left_at = now(), leave_reason = $2, role = gone.role, left_stage_at = CASE WHEN p.role >= $5 THEN now() ELSE p.left_stage_at END,
             left_stage_reason = CASE WHEN p.role >= $5 THEN $2 ELSE p.left_stage_reason END,
             hand_public = false, stage_since = NULL,
             seconds = p.seconds + GREATEST(extract(epoch FROM now() - p.joined_at)::int, 0),
             hand_raised_at = NULL, stage_invited_at = NULL, stage_invited_by = NULL, seen_at = NULL, missing_since = NULL
         FROM unnest($3::uuid[], $4::smallint[]) AS gone(user_id, role)
         WHERE p.space_id = $1 AND p.user_id = gone.user_id",
    )
    .bind(space_id)
    .bind(reason)
    .bind(&ids)
    .bind(&roles)
    .bind(role::SPEAKER)
    .execute(&mut **tx)
    .await?;
    Ok(inside)
}

/// The audience named in a request; unset means friends, the cautious
/// default.
fn audience_of(value: buffa::EnumValue<pb::SpaceAudience>) -> AppResult<i16> {
    match value.as_known() {
        Some(pb::SpaceAudience::SPACE_AUDIENCE_UNSPECIFIED) => Ok(audience::FRIENDS),
        Some(pb::SpaceAudience::SPACE_AUDIENCE_PUBLIC) => Ok(audience::PUBLIC),
        Some(pb::SpaceAudience::SPACE_AUDIENCE_FRIENDS) => Ok(audience::FRIENDS),
        Some(pb::SpaceAudience::SPACE_AUDIENCE_INVITED) => Ok(audience::INVITED),
        None => Err(AppError::invalid("INVALID_AUDIENCE", "unknown audience")),
    }
}

impl Spaces {
    /// A room that exists as far as its users are concerned: one its host
    /// deleted is gone.
    pub(crate) async fn find_space(&self, space_id: Uuid) -> AppResult<Option<SpaceRow>> {
        Ok(self.find_space_record(space_id).await?.filter(|space| space.deleted_at.is_none()))
    }

    /// A room as it is on record, deleted or not: for reports and
    /// moderation.
    pub(crate) async fn find_space_record(&self, space_id: Uuid) -> AppResult<Option<SpaceRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {SPACE} FROM spaces s WHERE s.id = $1")))
            .bind(space_id)
            .fetch_optional(&self.core.db)
            .await?)
    }

    /// Puts `user_id` inside `venue` with `enter`: a transaction that
    /// takes [`lock_presence`], decides whether the person may enter at
    /// all and then calls [`ensure_nowhere_else`]. Only when that is the
    /// one thing in the way are they taken out of wherever else they are
    /// and `enter` is run again — so a request that would be refused
    /// anyway costs the person nothing.
    pub(crate) async fn entering<T, F, Fut>(&self, user_id: Uuid, venue: Venue, enter: F) -> AppResult<T>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = AppResult<T>>,
    {
        for _ in 0..ENTER_RETRIES {
            match enter().await {
                Err(e) if e.reason == ENTERING_ELSEWHERE => self.leave_everything(user_id, Some(venue)).await?,
                result => return result,
            }
        }
        enter().await
    }

    /// What decides whether `viewer` may see `space` (see [`Access`]).
    pub(crate) async fn access(&self, viewer: Option<Uuid>, space: &SpaceRow) -> AppResult<Access> {
        let core = &self.core;
        let mut access = Access { audience: space.audience, ..Default::default() };
        let Some(viewer) = viewer else { return Ok(access) };
        if viewer == space.host_id {
            access.host = true;
            return Ok(access);
        }
        access.blocked = graph::is_blocked_either(core, viewer, space.host_id).await?;
        if access.blocked {
            return Ok(access);
        }
        (access.inside, access.invited, access.banned) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM space_participants WHERE space_id = $1 AND user_id = $2 AND left_at IS NULL),
                    EXISTS (SELECT 1 FROM space_invites WHERE space_id = $1 AND user_id = $2 AND revoked_at IS NULL),
                    EXISTS (SELECT 1 FROM space_bans WHERE space_id = $1 AND user_id = $2)",
        )
        .bind(space.id)
        .bind(viewer)
        .fetch_one(&core.db)
        .await?;
        if space.audience == audience::FRIENDS && !access.inside && !access.invited {
            access.friend_of_host = graph::are_friends(core, space.host_id, viewer).await?;
        }
        Ok(access)
    }

    /// A room `viewer` may see, or `SPACE_NOT_FOUND`.
    pub(crate) async fn visible_space(&self, viewer: Option<Uuid>, space_id: Uuid) -> AppResult<SpaceRow> {
        let space = self.find_space(space_id).await?.ok_or_else(space_not_found)?;
        if self.access(viewer, &space).await?.may_see() { Ok(space) } else { Err(space_not_found()) }
    }

    /// A room `user_id` hosts: `SPACE_NOT_FOUND` if they may not see it,
    /// `NOT_HOST` if it is somebody else's.
    pub(crate) async fn hosted_space(&self, user_id: Uuid, space_id: Uuid) -> AppResult<SpaceRow> {
        let space = self.visible_space(Some(user_id), space_id).await?;
        if space.host_id == user_id { Ok(space) } else { Err(not_host()) }
    }

    async fn is_hosting(&self, user_id: Uuid) -> AppResult<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM spaces WHERE host_id = $1 AND state = $2)")
            .bind(user_id)
            .bind(state::LIVE)
            .fetch_one(&self.core.db)
            .await?)
    }

    /// Puts the host inside a room that just went live and tells the other
    /// domains: `spaces.room.started`, the activity entry of a public room
    /// and its search document.
    async fn go_live(&self, tx: &mut Tx<'_>, space_id: Uuid, host_id: Uuid, scheduled: bool) -> AppResult<SpaceRow> {
        sqlx::query(
            "INSERT INTO space_participants AS p (space_id, user_id, role, top_role) VALUES ($1, $2, $3, $3)
             ON CONFLICT (space_id, user_id) DO UPDATE SET
                 role = EXCLUDED.role, top_role = GREATEST(p.top_role, EXCLUDED.role), joined_at = now(),
                 left_at = NULL, leave_reason = 0",
        )
        .bind(space_id)
        .bind(host_id)
        .bind(role::HOST)
        .execute(&mut **tx)
        .await?;
        let space = recount(tx, space_id).await?;
        let started = RoomStarted {
            space_id,
            host_id,
            title: space.title.clone(),
            tags: space.tags.clone(),
            language: space.language.clone(),
            audience: i32::from(space.audience),
            scheduled,
        };
        events::emit(&mut **tx, ROOM_STARTED, Some(host_id), &started).await?;
        if space.is_public() {
            outbound::activity(&mut **tx, &notices::started_activity(&space)).await?;
            outbound::index(&mut **tx, notices::search_doc(&space)).await?;
        }
        Ok(space)
    }

    // ───────────────────────────── creating ─────────────────────────────

    /// Creates a room: scheduled, or live at once with the host inside.
    pub(crate) async fn create_space(
        &self,
        user_id: Uuid,
        request: pb::CreateSpaceRequest,
    ) -> AppResult<(SpaceRow, Option<pb::AudioCredentials>)> {
        let core = &self.core;
        let title = rules::title(&request.title)?;
        let tags = rules::tags(&request.tags)?;
        let language = rules::language(&request.language)?;
        let audience = audience_of(request.audience)?;
        let scheduled_at = match request.scheduled_at.as_option() {
            Some(at) => Some(rules::schedule(from_ts(at), now(), self.cfg.max_schedule_days)?),
            None => None,
        };
        let invitees = self.invitable(user_id, user_id, &request.invitee_ids).await?;
        if scheduled_at.is_none() {
            self.require_audio()?;
            ensure!(
                !self.is_hosting(user_id).await?,
                precondition,
                "ALREADY_HOSTING",
                "a host runs one live room at a time"
            );
        }
        self.limit(Limit::CreateRoom, user_id).await?;
        let host = tokuly_identity::summary(core, user_id).await?;
        let space_id = new_id();
        let venue = Venue::Space(space_id);

        let Some(scheduled_at) = scheduled_at else {
            // Live at once: the host is told off if they do this all the
            // time, and the audio must be there before anyone is told.
            self.limit(Limit::GoLive, user_id).await?;
            self.open_room(venue).await?;
            let enter = || async {
                let mut tx = core.db.begin().await?;
                lock_presence(&mut tx, user_id).await?;
                // Settled before anything is given up for the new room: a
                // second request racing this one must not take the host out
                // of the room the first one opened.
                let hosting: bool =
                    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM spaces WHERE host_id = $1 AND state = $2)")
                        .bind(user_id)
                        .bind(state::LIVE)
                        .fetch_one(&mut *tx)
                        .await?;
                if hosting {
                    return Err(already_hosting());
                }
                ensure_nowhere_else(&mut tx, user_id, venue).await?;
                sqlx::query(
                    "INSERT INTO spaces (id, host_id, title, tags, language, audience, state, room, started_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now())",
                )
                .bind(space_id)
                .bind(user_id)
                .bind(&title)
                .bind(&tags)
                .bind(&language)
                .bind(audience)
                .bind(state::LIVE)
                .bind(venue.room())
                .execute(&mut *tx)
                .await?;
                let space = self.go_live(&mut tx, space_id, user_id, false).await?;
                let invited = self.invite_in_tx(&mut tx, &space, user_id, &host, &invitees).await?;
                tx.commit().await?;
                Ok((space, invited))
            };
            let created: AppResult<(SpaceRow, Invited)> = self.entering(user_id, venue, enter).await;
            let (space, invited) = match created {
                Ok(created) => created,
                Err(e) => {
                    // Nobody will ever enter the room that was prepared.
                    self.close_room(venue).await;
                    // The index on live rooms caught a second one racing in.
                    return Err(if e.is_already_exists() { already_hosting() } else { e });
                }
            };
            self.announce_invitations(&space, &host, &invited.announced).await;
            let credentials = self.credentials(venue, user_id, &notices::name_of(&host), role::HOST)?;
            return Ok((space, Some(credentials)));
        };

        let mut tx = core.db.begin().await?;
        // Serializes one host's scheduling, so the limit holds.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("spaces:schedule:{}", user_id.simple()))
            .execute(&mut *tx)
            .await?;
        let upcoming: i64 = sqlx::query_scalar("SELECT count(*) FROM spaces WHERE host_id = $1 AND state = $2")
            .bind(user_id)
            .bind(state::SCHEDULED)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            upcoming < self.cfg.max_scheduled,
            precondition,
            "TOO_MANY_SPACES",
            "at most {} rooms can be waiting to start",
            self.cfg.max_scheduled
        );
        let space: SpaceRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO spaces AS s (id, host_id, title, tags, language, audience, state, room, scheduled_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING {SPACE}"
        )))
        .bind(space_id)
        .bind(user_id)
        .bind(&title)
        .bind(&tags)
        .bind(&language)
        .bind(audience)
        .bind(state::SCHEDULED)
        .bind(venue.room())
        .bind(scheduled_at)
        .fetch_one(&mut *tx)
        .await?;
        if space.is_public() {
            outbound::index(&mut *tx, notices::search_doc(&space)).await?;
        }
        let invited = self.invite_in_tx(&mut tx, &space, user_id, &host, &invitees).await?;
        tx.commit().await?;
        self.announce_invitations(&space, &host, &invited.announced).await;
        Ok((space, None))
    }

    /// Changes a room's title, tags and language, and — while it is still
    /// upcoming — its audience and planned start.
    pub(crate) async fn update_space(&self, user_id: Uuid, request: pb::UpdateSpaceRequest) -> AppResult<SpaceRow> {
        let space_id = parse_id(&request.space_id)?;
        let current = self.hosted_space(user_id, space_id).await?;
        if current.is_over() {
            return Err(space_ended());
        }
        self.limit(Limit::UpdateRoom, user_id).await?;
        let title = request.title.as_deref().map(rules::title).transpose()?;
        let tags = request.set_tags.then(|| rules::tags(&request.tags)).transpose()?;
        let language = request.language.as_deref().map(rules::language).transpose()?;
        let audience = request.audience.map(audience_of).transpose()?;
        let scheduled_at = match request.scheduled_at.as_option() {
            Some(at) => Some(rules::schedule(from_ts(at), now(), self.cfg.max_schedule_days)?),
            None => None,
        };

        let mut tx = self.core.db.begin().await?;
        let locked = lock_space(&mut tx, space_id).await?;
        if locked.host_id != user_id {
            return Err(not_host());
        }
        if locked.is_over() {
            return Err(space_ended());
        }
        if (audience.is_some() || scheduled_at.is_some()) && !locked.is_scheduled() {
            return Err(not_scheduled());
        }
        let space: SpaceRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE spaces s SET
                 title = COALESCE($2, s.title), tags = COALESCE($3, s.tags), language = COALESCE($4, s.language),
                 audience = COALESCE($5, s.audience), scheduled_at = COALESCE($6, s.scheduled_at),
                 host_nudged_at = CASE WHEN $6::timestamptz IS NULL THEN s.host_nudged_at ELSE NULL END
             WHERE s.id = $1
             RETURNING {SPACE}"
        )))
        .bind(space_id)
        .bind(&title)
        .bind(&tags)
        .bind(&language)
        .bind(audience)
        .bind(scheduled_at)
        .fetch_one(&mut *tx)
        .await?;
        // Keeps the search document in step, and removes it when the room
        // stopped being public.
        if space.is_public() || locked.is_public() {
            outbound::index(&mut *tx, notices::search_doc(&space)).await?;
        }
        tx.commit().await?;
        self.push_space(&space).await;
        // Public > friends > invited only: only a narrower audience leaves someone out.
        let reach = |value: i16| match value {
            audience::PUBLIC => 2,
            audience::FRIENDS => 1,
            _ => 0,
        };
        if reach(space.audience) < reach(locked.audience) {
            // A narrower audience: whoever is no longer in it stops following the room.
            self.core.rt.recheck_topic(&crate::model::space_topic(space.id)).await;
        }
        Ok(space)
    }

    // ───────────────────────────── starting and ending ─────────────────────────────

    /// Starts an upcoming room with its host inside.
    pub(crate) async fn start_space(
        &self,
        user_id: Uuid,
        space_id: Uuid,
    ) -> AppResult<(SpaceRow, pb::AudioCredentials)> {
        let core = &self.core;
        let current = self.hosted_space(user_id, space_id).await?;
        if !current.is_scheduled() {
            return Err(not_scheduled());
        }
        self.require_audio()?;
        if self.is_hosting(user_id).await? {
            return Err(already_hosting());
        }
        self.limit(Limit::GoLive, user_id).await?;
        let host = tokuly_identity::summary(core, user_id).await?;
        let venue = Venue::Space(space_id);
        self.open_room(venue).await?;

        let enter = || async {
            let mut tx = core.db.begin().await?;
            lock_presence(&mut tx, user_id).await?;
            let locked = lock_space(&mut tx, space_id).await?;
            if locked.host_id != user_id {
                return Err(not_host());
            }
            if !locked.is_scheduled() {
                return Err(not_scheduled());
            }
            let hosting: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM spaces WHERE host_id = $1 AND state = $2)")
                    .bind(user_id)
                    .bind(state::LIVE)
                    .fetch_one(&mut *tx)
                    .await?;
            if hosting {
                return Err(already_hosting());
            }
            ensure_nowhere_else(&mut tx, user_id, venue).await?;
            sqlx::query("UPDATE spaces SET state = $2, started_at = now() WHERE id = $1")
                .bind(space_id)
                .bind(state::LIVE)
                .execute(&mut *tx)
                .await?;
            let space = self.go_live(&mut tx, space_id, user_id, true).await?;
            tx.commit().await?;
            Ok(space)
        };
        let started: AppResult<SpaceRow> = self.entering(user_id, venue, enter).await;
        let space = match started {
            Ok(space) => space,
            Err(e) => {
                self.close_room(venue).await;
                return Err(if e.is_already_exists() { already_hosting() } else { e });
            }
        };
        self.push_space(&space).await;
        let credentials = self.credentials(venue, user_id, &notices::name_of(&host), role::HOST)?;
        Ok((space, credentials))
    }

    /// Calls an upcoming room off.
    pub(crate) async fn cancel_space(&self, user_id: Uuid, space_id: Uuid) -> AppResult<SpaceRow> {
        self.hosted_space(user_id, space_id).await?;
        let mut tx = self.core.db.begin().await?;
        let locked = lock_space(&mut tx, space_id).await?;
        if locked.host_id != user_id {
            return Err(not_host());
        }
        if !locked.is_scheduled() {
            return Err(not_scheduled());
        }
        let space = self.call_off(&mut tx, &locked, end::HOST_ENDED).await?;
        tx.commit().await?;
        self.push_space(&space).await;
        Ok(space)
    }

    /// Cancels an upcoming room, inside the transaction that holds its lock.
    pub(crate) async fn call_off(&self, tx: &mut Tx<'_>, space: &SpaceRow, reason: i16) -> AppResult<SpaceRow> {
        let cancelled: SpaceRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE spaces s SET state = $2, end_reason = $3, ended_at = now() WHERE s.id = $1 RETURNING {SPACE}"
        )))
        .bind(space.id)
        .bind(state::CANCELLED)
        .bind(reason)
        .fetch_one(&mut **tx)
        .await?;
        if space.is_public() {
            outbound::unindex(&mut **tx, notices::SEARCH_KIND, space.id).await?;
        }
        Ok(cancelled)
    }

    /// Ends a live room for everyone. A room that already ended stays as
    /// it is.
    pub(crate) async fn end_space(&self, user_id: Uuid, space_id: Uuid) -> AppResult<SpaceRow> {
        let current = self.hosted_space(user_id, space_id).await?;
        if current.is_scheduled() {
            return Err(not_live());
        }
        let mut tx = self.core.db.begin().await?;
        let locked = lock_space(&mut tx, space_id).await?;
        if locked.host_id != user_id {
            return Err(not_host());
        }
        if !locked.is_live() {
            return Ok(locked);
        }
        let finished = self.finish(&mut tx, &locked, end::HOST_ENDED).await?;
        tx.commit().await?;
        self.after_finish(&finished).await;
        Ok(finished.space)
    }

    /// Ends a live room inside the transaction that holds its lock:
    /// everyone leaves, the room is marked ended, `spaces.room.ended` is
    /// emitted and a public room leaves the search index. Follow the commit
    /// with [`Self::after_finish`].
    pub(crate) async fn finish(&self, tx: &mut Tx<'_>, space: &SpaceRow, reason: i16) -> AppResult<Finished> {
        let departed = depart(tx, space.id, None, leave::ENDED).await?;
        sqlx::query("UPDATE spaces SET state = $2, end_reason = $3, ended_at = now() WHERE id = $1")
            .bind(space.id)
            .bind(state::ENDED)
            .bind(reason)
            .execute(&mut **tx)
            .await?;
        let ended = recount(tx, space.id).await?;
        let event = RoomEnded {
            space_id: ended.id,
            host_id: ended.host_id,
            audience: i32::from(ended.audience),
            reason: i32::from(reason),
            minutes: ended.minutes(),
            peak_listeners: ended.peak_listeners,
            total_listeners: ended.total_listeners,
        };
        events::emit(&mut **tx, ROOM_ENDED, Some(ended.host_id), &event).await?;
        if ended.is_public() {
            outbound::unindex(&mut **tx, notices::SEARCH_KIND, ended.id).await?;
        }
        Ok(Finished { space: ended, departed })
    }

    /// Announces an ended room and closes its LiveKit room.
    pub(crate) async fn after_finish(&self, finished: &Finished) {
        let space = &finished.space;
        self.push_space(space).await;
        // Also to each person, for clients that do not follow the topic.
        let left = pb::SpaceParticipantLeft {
            space_id: space.id.to_string(),
            reason: pb::SpaceLeaveReason::SPACE_LEAVE_REASON_ENDED.into(),
            ..Default::default()
        };
        for departed in &finished.departed {
            self.push_user(
                departed.user_id,
                pb::SpaceParticipantLeft { user_id: departed.user_id.to_string(), ..left.clone() },
            )
            .await;
        }
        self.close_ended_room(space.id).await;
    }

    /// Closes the LiveKit room of an ended room and remembers that it is
    /// done; until then the sweep keeps trying.
    pub(crate) async fn close_ended_room(&self, space_id: Uuid) {
        if !self.close_room(Venue::Space(space_id)).await {
            return;
        }
        if let Err(e) = sqlx::query("UPDATE spaces SET room_closed_at = now() WHERE id = $1 AND room_closed_at IS NULL")
            .bind(space_id)
            .execute(&self.core.db)
            .await
        {
            tracing::warn!(%space_id, error = %e, "closed room could not be recorded");
        }
    }

    /// Removes a finished room from its host's history. From then on it
    /// reads as gone; the record itself stays until the sweep purges it
    /// (see [`crate::presence`]), so that the people who were in the room
    /// can still report it and moderators can see who was there.
    pub(crate) async fn delete_space(&self, user_id: Uuid, space_id: Uuid) -> AppResult<()> {
        self.hosted_space(user_id, space_id).await?;
        let mut tx = self.core.db.begin().await?;
        let locked = lock_space(&mut tx, space_id).await?;
        if locked.host_id != user_id {
            return Err(not_host());
        }
        ensure!(locked.is_over(), precondition, "SPACE_NOT_ENDED", "end or cancel the room before deleting it");
        sqlx::query("UPDATE spaces SET deleted_at = now() WHERE id = $1").bind(space_id).execute(&mut *tx).await?;
        tx.commit().await?;
        self.core.rt.close_topic(&crate::model::space_topic(space_id)).await;
        Ok(())
    }
}
