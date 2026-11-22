//! Keeping "who is inside" true.
//!
//! People leave without saying so: an app crashes, a phone loses its
//! network. And LiveKit can disagree with the database the other way
//! round: a token stays valid for a few minutes after its holder left or
//! was removed. Three workers bring the two in line:
//!
//! - [`Spaces::reconcile`] compares every venue in use with LiveKit's
//!   list of participants. Someone marked as inside who is not connected
//!   gets time to connect or reconnect ([`rules::sighting`]) and is then
//!   taken out. A connection of someone who is not inside is cut. A
//!   connection whose permissions do not match the person's role is
//!   corrected. LiveKit's webhook ([`crate::webhook`]) triggers the same
//!   comparison for one venue the moment something happens there.
//! - [`Spaces::settle_rooms`] deals with rooms whose host stays away or
//!   that are empty ([`rules::settle`]): hand-over to a co-host, or the end.
//! - [`Spaces::sweep`] does the slow housekeeping: the nudge to hosts of
//!   scheduled rooms, upcoming rooms nobody started, LiveKit rooms of ended
//!   rooms that could not be closed, old history.
use std::collections::{HashMap, HashSet};

use tokuly_core::{livekit, outbound, prelude::*};

use crate::{
    Spaces, audio,
    audio::HANGOUT_ROLE,
    hangouts::{lock_hangout, vacate},
    model::{InsideRow, SPACE, SpaceRow, Venue, end, leave, member, role, state},
    notices,
    rooms::{depart, lock_space, recount},
    rules::{self, Sighting, Standing, Verdict},
};

/// Venues compared with LiveKit in one pass.
const MAX_VENUES_PER_PASS: i64 = 2000;
/// Rooms settled, nudged or cancelled in one pass.
const MAX_ROOMS_PER_SWEEP: i64 = 500;
/// An ended room is given this long before its LiveKit room is closed
/// again by the sweep (the first attempt happens when it ends).
const CLOSE_RETRY_AFTER_SECONDS: f64 = 30.0;

/// What a comparison with LiveKit changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reconciled {
    /// People marked as inside who were gone and have been taken out.
    pub ghosts: usize,
    /// Connections of people who are not inside that were cut.
    pub strays: usize,
    /// Connections whose permissions were brought in line with the role.
    pub repaired: usize,
}

impl Reconciled {
    fn add(&mut self, other: Reconciled) {
        self.ghosts += other.ghosts;
        self.strays += other.strays;
        self.repaired += other.repaired;
    }
}

/// How the people marked as inside a venue compare with LiveKit.
#[derive(Debug, Default)]
struct Comparison {
    /// Found connected for the first time, or again after being missed.
    seen: Vec<Uuid>,
    /// Was connected, is not: the reconnect clock starts.
    missing: Vec<Uuid>,
    /// Not connected for too long.
    gone: Vec<Uuid>,
}

impl Spaces {
    fn compare(&self, inside: &[InsideRow], connected: &HashSet<Uuid>) -> Comparison {
        let now = now();
        let mut comparison = Comparison::default();
        for row in inside {
            let sighting = rules::sighting(
                connected.contains(&row.user_id),
                row.joined_at,
                row.seen_at,
                row.missing_since,
                now,
                self.cfg.connect_grace_seconds,
                self.cfg.reconnect_grace_seconds,
            );
            match sighting {
                // Only a change is worth writing down.
                Sighting::Seen if row.seen_at.is_none() || row.missing_since.is_some() => {
                    comparison.seen.push(row.user_id)
                }
                Sighting::Seen | Sighting::Awaited => {}
                Sighting::Missing => comparison.missing.push(row.user_id),
                Sighting::Gone => comparison.gone.push(row.user_id),
            }
        }
        comparison
    }

    /// Cuts connections of people who are not inside and corrects the
    /// permissions of those who are. `roles` holds who is inside.
    async fn align_connections(
        &self,
        venue: Venue,
        connected: &[livekit::Participant],
        roles: &HashMap<Uuid, i16>,
    ) -> Reconciled {
        let mut changed = Reconciled::default();
        let room = venue.room();
        for participant in connected {
            let inside =
                Uuid::parse_str(&participant.identity).ok().and_then(|id| roles.get(&id).map(|role| (id, *role)));
            let Some((user_id, role)) = inside else {
                match self.livekit.remove_participant(&room, &participant.identity).await {
                    Ok(true) => changed.strays += 1,
                    Ok(false) => {}
                    Err(e) => tracing::warn!(?venue, error = %e, "stray connection could not be cut"),
                }
                continue;
            };
            let wanted = audio::permission(role);
            let differs = participant.permission.as_ref().is_some_and(|has| {
                has.can_publish != wanted.can_publish
                    || has.can_publish_data != wanted.can_publish_data
                    || has.can_subscribe != wanted.can_subscribe
            });
            if differs || participant.metadata != audio::metadata(role) {
                self.apply_role(venue, user_id, role).await;
                changed.repaired += 1;
            }
        }
        changed
    }

    /// Compares one venue with LiveKit. Does nothing when LiveKit is not
    /// configured or cannot be reached, or when the venue is not in this
    /// database (another deployment may share the LiveKit server).
    pub async fn reconcile_venue(&self, venue: Venue) -> AppResult<Reconciled> {
        if !self.audio_enabled() {
            return Ok(Reconciled::default());
        }
        // LiveKit first, the database second: whoever is connected without
        // being inside by then has left in between, or never was.
        let connected = match self.livekit.list_participants(&venue.room()).await {
            Ok(connected) => connected,
            Err(e) => {
                tracing::warn!(?venue, error = %e, "venue could not be compared with LiveKit");
                return Ok(Reconciled::default());
            }
        };
        let connected_ids: HashSet<Uuid> =
            connected.iter().filter_map(|participant| Uuid::parse_str(&participant.identity).ok()).collect();
        match venue {
            Venue::Space(space_id) => self.reconcile_space(space_id, &connected, &connected_ids).await,
            Venue::Hangout(hangout_id) => self.reconcile_hangout(hangout_id, &connected, &connected_ids).await,
        }
    }

    async fn reconcile_space(
        &self,
        space_id: Uuid,
        connected: &[livekit::Participant],
        connected_ids: &HashSet<Uuid>,
    ) -> AppResult<Reconciled> {
        let venue = Venue::Space(space_id);
        let mut tx = self.core.db.begin().await?;
        if lock_space(&mut tx, space_id).await.is_err() {
            return Ok(Reconciled::default());
        }
        let inside: Vec<InsideRow> = sqlx::query_as(
            "SELECT user_id, joined_at, seen_at, missing_since, role FROM space_participants
             WHERE space_id = $1 AND left_at IS NULL",
        )
        .bind(space_id)
        .fetch_all(&mut *tx)
        .await?;
        let comparison = self.compare(&inside, connected_ids);
        sqlx::query(
            "UPDATE space_participants SET
                 seen_at = CASE WHEN user_id = ANY($2) THEN now() ELSE seen_at END,
                 missing_since = CASE WHEN user_id = ANY($3) THEN now() END
             WHERE space_id = $1 AND left_at IS NULL AND (user_id = ANY($2) OR user_id = ANY($3))",
        )
        .bind(space_id)
        .bind(&comparison.seen)
        .bind(&comparison.missing)
        .execute(&mut *tx)
        .await?;
        // Under the same lock as the comparison: someone who asks to be let
        // in again a moment later is not thrown out on stale news.
        let departed = match comparison.gone.is_empty() {
            true => Vec::new(),
            false => depart(&mut tx, space_id, Some(&comparison.gone), leave::DISCONNECTED).await?,
        };
        let space = if departed.is_empty() { None } else { Some(recount(&mut tx, space_id).await?) };
        tx.commit().await?;

        let mut changed = Reconciled { ghosts: departed.len(), ..Default::default() };
        if let Some(space) = &space {
            self.after_departures(space, &departed, leave::DISCONNECTED).await;
        }
        let roles: HashMap<Uuid, i16> = inside
            .iter()
            .filter(|row| !comparison.gone.contains(&row.user_id))
            .map(|row| (row.user_id, row.role))
            .collect();
        changed.add(self.align_connections(venue, connected, &roles).await);
        Ok(changed)
    }

    async fn reconcile_hangout(
        &self,
        hangout_id: Uuid,
        connected: &[livekit::Participant],
        connected_ids: &HashSet<Uuid>,
    ) -> AppResult<Reconciled> {
        let venue = Venue::Hangout(hangout_id);
        let mut tx = self.core.db.begin().await?;
        if lock_hangout(&mut tx, hangout_id).await.is_err() {
            return Ok(Reconciled::default());
        }
        let inside: Vec<InsideRow> = sqlx::query_as(
            "SELECT user_id, joined_at, seen_at, missing_since, $2::smallint AS role FROM hangout_presence WHERE hangout_id = $1",
        )
        .bind(hangout_id)
        .bind(HANGOUT_ROLE)
        .fetch_all(&mut *tx)
        .await?;
        let comparison = self.compare(&inside, connected_ids);
        sqlx::query(
            "UPDATE hangout_presence SET
                 seen_at = CASE WHEN user_id = ANY($2) THEN now() ELSE seen_at END,
                 missing_since = CASE WHEN user_id = ANY($3) THEN now() END
             WHERE hangout_id = $1 AND (user_id = ANY($2) OR user_id = ANY($3))",
        )
        .bind(hangout_id)
        .bind(&comparison.seen)
        .bind(&comparison.missing)
        .execute(&mut *tx)
        .await?;
        // Under the same lock as the comparison (see `reconcile_space`).
        let vacated = match comparison.gone.is_empty() {
            true => None,
            false => vacate(&mut tx, hangout_id, &comparison.gone).await?,
        };
        tx.commit().await?;

        let mut changed = Reconciled::default();
        if let Some(vacated) = &vacated {
            changed.ghosts = vacated.gone.len();
            // Their connection is gone already: nothing to disconnect.
            self.after_vacate(vacated, false).await;
        }
        let roles: HashMap<Uuid, i16> = inside
            .iter()
            .filter(|row| !comparison.gone.contains(&row.user_id))
            .map(|row| (row.user_id, row.role))
            .collect();
        changed.add(self.align_connections(venue, connected, &roles).await);
        Ok(changed)
    }

    /// Compares every venue in use with LiveKit: live rooms, hangouts with
    /// somebody inside, and — for as long as an old token could still open
    /// a connection — rooms that just ended and hangouts that just emptied.
    pub async fn reconcile(&self) -> AppResult<Reconciled> {
        let mut total = Reconciled::default();
        if !self.audio_enabled() {
            return Ok(total);
        }
        let db = &self.core.db;
        // How long a token handed out before the end can still be used.
        let token_window = (self.cfg.token_ttl_seconds + 60) as f64;
        let rooms: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM spaces
             WHERE state = $1 OR (state = $2 AND ended_at > now() - make_interval(secs => $3))
             ORDER BY id LIMIT $4",
        )
        .bind(state::LIVE)
        .bind(state::ENDED)
        .bind(token_window)
        .bind(MAX_VENUES_PER_PASS)
        .fetch_all(db)
        .await?;
        let hangouts: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM hangouts
             WHERE deleted_at IS NULL AND (inside_count > 0 OR last_active_at > now() - make_interval(secs => $1))
             ORDER BY id LIMIT $2",
        )
        .bind(token_window)
        .bind(MAX_VENUES_PER_PASS)
        .fetch_all(db)
        .await?;
        let venues = rooms.into_iter().map(Venue::Space).chain(hangouts.into_iter().map(Venue::Hangout));
        for venue in venues {
            match self.reconcile_venue(venue).await {
                Ok(changed) => total.add(changed),
                Err(e) => tracing::warn!(?venue, error = %e, "venue could not be reconciled"),
            }
        }
        Ok(total)
    }

    /// Deals with live rooms whose host has been away, or that have been
    /// empty, for longer than allowed: the room is handed to the co-host
    /// who has been inside longest, or ended. Returns how many rooms
    /// changed.
    pub async fn settle_rooms(&self) -> AppResult<usize> {
        let due: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM spaces
             WHERE state = $1
               AND (host_absent_since <= now() - make_interval(secs => $2) OR empty_since <= now() - make_interval(secs => $3))
             ORDER BY id LIMIT $4",
        )
        .bind(state::LIVE)
        .bind(self.cfg.host_grace_seconds as f64)
        .bind(self.cfg.empty_grace_seconds as f64)
        .bind(MAX_ROOMS_PER_SWEEP)
        .fetch_all(&self.core.db)
        .await?;
        let mut changed = 0;
        for space_id in due {
            let mut tx = self.core.db.begin().await?;
            let Ok(space) = lock_space(&mut tx, space_id).await else { continue };
            if !space.is_live() {
                continue;
            }
            // A co-host who is inside and not running a live room of their own.
            let successor: Option<Uuid> = sqlx::query_scalar(
                "SELECT p.user_id FROM space_participants p
                 WHERE p.space_id = $1 AND p.left_at IS NULL AND p.role = $2
                   AND NOT EXISTS (SELECT 1 FROM spaces other WHERE other.host_id = p.user_id AND other.state = $3)
                 ORDER BY p.joined_at, p.user_id
                 LIMIT 1",
            )
            .bind(space_id)
            .bind(role::COHOST)
            .bind(state::LIVE)
            .fetch_optional(&mut *tx)
            .await?;
            let now = now();
            let standing = Standing {
                host_absent_seconds: space.host_absent_since.map(|since| (now - since).num_seconds()),
                empty_seconds: space.empty_since.map(|since| (now - since).num_seconds()),
                successor_inside: successor.is_some(),
            };
            match (rules::settle(standing, self.cfg.host_grace_seconds, self.cfg.empty_grace_seconds), successor) {
                // (A hand-over is only decided when there is a successor.)
                (Verdict::Continue, _) | (Verdict::HandOver, None) => continue,
                (Verdict::HandOver, Some(successor)) => {
                    let handed = self.hand_over(&mut tx, &space, successor).await?;
                    tx.commit().await?;
                    self.after_hand_over(&handed, space.host_id).await;
                }
                (Verdict::End(reason), _) => {
                    let finished = self.finish(&mut tx, &space, reason).await?;
                    tx.commit().await?;
                    self.after_finish(&finished).await;
                }
            }
            changed += 1;
        }
        Ok(changed)
    }

    /// The slow housekeeping (see the module documentation).
    pub async fn sweep(&self) -> AppResult<()> {
        self.nudge_hosts().await?;
        self.cancel_rooms_nobody_started().await?;
        self.close_leftover_rooms().await?;
        self.purge().await
    }

    /// Forgets what is old enough to be forgotten.
    ///
    /// A finished room goes `TOKULY_SPACES_RETENTION_DAYS` after it ended.
    /// Deleting something does not make it go sooner — it only hides it:
    /// a room its host deleted, a hangout its owner closed and the trace
    /// of a former member all stay for `TOKULY_SPACES_EVIDENCE_DAYS` after
    /// that moment, so that a report filed the day before the deletion can
    /// still be looked into.
    pub async fn purge(&self) -> AppResult<()> {
        let db = &self.core.db;
        let evidence_days = self.cfg.evidence_days as i32;
        sqlx::query(
            "DELETE FROM spaces
             WHERE state IN ($1, $2) AND ended_at < now() - make_interval(days => $3)
               AND (deleted_at IS NULL OR deleted_at < now() - make_interval(days => $4))",
        )
        .bind(state::ENDED)
        .bind(state::CANCELLED)
        .bind(self.cfg.retention_days as i32)
        .bind(evidence_days)
        .execute(db)
        .await?;
        sqlx::query("DELETE FROM hangouts WHERE deleted_at < now() - make_interval(days => $1)")
            .bind(evidence_days)
            .execute(db)
            .await?;
        sqlx::query("DELETE FROM hangout_members WHERE state = $1 AND left_at < now() - make_interval(days => $2)")
            .bind(member::FORMER)
            .bind(evidence_days)
            .execute(db)
            .await?;
        Ok(())
    }

    /// Tells the host of an upcoming room that its time has come. Once per
    /// room (again if it is rescheduled). Returns how many hosts were told.
    pub async fn nudge_hosts(&self) -> AppResult<usize> {
        let due: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM spaces WHERE state = $1 AND scheduled_at <= now() AND host_nudged_at IS NULL
             ORDER BY scheduled_at LIMIT $2",
        )
        .bind(state::SCHEDULED)
        .bind(MAX_ROOMS_PER_SWEEP)
        .fetch_all(&self.core.db)
        .await?;
        let mut nudged = 0;
        for space_id in due {
            let mut tx = self.core.db.begin().await?;
            // Claims the nudge, so two workers do not both send it.
            let space: Option<SpaceRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "UPDATE spaces s SET host_nudged_at = now()
                 WHERE s.id = $1 AND s.state = $2 AND s.scheduled_at <= now() AND s.host_nudged_at IS NULL
                 RETURNING {SPACE}"
            )))
            .bind(space_id)
            .bind(state::SCHEDULED)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(space) = space else { continue };
            outbound::notify(&mut *tx, &notices::time_to_start(&space)).await?;
            tx.commit().await?;
            nudged += 1;
        }
        Ok(nudged)
    }

    /// Cancels upcoming rooms nobody started within the allowed time after
    /// their planned start. Returns how many were cancelled.
    pub async fn cancel_rooms_nobody_started(&self) -> AppResult<usize> {
        let stale: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM spaces WHERE state = $1 AND scheduled_at <= now() - make_interval(hours => $2)
             ORDER BY scheduled_at LIMIT $3",
        )
        .bind(state::SCHEDULED)
        .bind(self.cfg.never_started_hours as i32)
        .bind(MAX_ROOMS_PER_SWEEP)
        .fetch_all(&self.core.db)
        .await?;
        let mut cancelled = 0;
        for space_id in stale {
            let mut tx = self.core.db.begin().await?;
            let Ok(space) = lock_space(&mut tx, space_id).await else { continue };
            if !space.is_scheduled() {
                continue;
            }
            let space = self.call_off(&mut tx, &space, end::NEVER_STARTED).await?;
            tx.commit().await?;
            self.push_space(&space).await;
            cancelled += 1;
        }
        Ok(cancelled)
    }

    /// Closes the LiveKit rooms of ended rooms where that did not work
    /// when they ended.
    async fn close_leftover_rooms(&self) -> AppResult<()> {
        let unclosed: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM spaces
             WHERE state = $1 AND room_closed_at IS NULL AND ended_at <= now() - make_interval(secs => $2)
             ORDER BY ended_at LIMIT $3",
        )
        .bind(state::ENDED)
        .bind(CLOSE_RETRY_AFTER_SECONDS)
        .bind(MAX_ROOMS_PER_SWEEP)
        .fetch_all(&self.core.db)
        .await?;
        for space_id in unclosed {
            self.close_ended_room(space_id).await;
        }
        Ok(())
    }
}
