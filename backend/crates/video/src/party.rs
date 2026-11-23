//! Watch parties (一緒に見る): the data model, who hears what, and the
//! membership rules. The RPC handlers are in `party_svc`.
//!
//! # Realtime topics
//!
//! Every party event is a `VideoEvent` published on
//! - `party:<party_id>` — followed by the signed-in members. The
//!   [`PartyGuard`] admits a connection only if its user has a seat;
//! - `party:<party_id>:<key>` — one topic per guest, where `key` is a secret
//!   of that guest's seat handed out by `JoinParty`. The guard admits anyone
//!   presenting a key that belongs to a seat of the party.
//!
//! So knowing a party's ID is not enough to listen, and a guest who leaves
//! or is removed is cut off at once (their topic is no longer published to).
//! The gateway checks a subscription once, when it is made: a signed-in
//! member who is removed keeps an already open subscription until their
//! connection resubscribes.
//!
//! # Playback
//!
//! The server is the clock. A playback state is "the video was at
//! `position_seconds` at `state_at` (server time), playing at `rate` or
//! paused"; `state_seq` grows with every change so clients can drop stale
//! pushes. Clients learn their clock offset from `server_time` in responses.
use std::sync::Arc;

use async_trait::async_trait;
use tokuly_core::{prelude::*, realtime::TopicGuard};
use tokuly_proto::{
    common::StampRef,
    video::{
        PartyEndReason, PartyEnded, PartyLeaveReason, PartyMember, PartyMemberLeft, PartyMessage, PartyPlayback,
        PartyRole, VideoEvent as VideoPush, video_event::Event as Push,
    },
};

pub const ROLE_HOST: i32 = PartyRole::PARTY_ROLE_HOST as i32;
pub const ROLE_COHOST: i32 = PartyRole::PARTY_ROLE_COHOST as i32;
pub const ROLE_MEMBER: i32 = PartyRole::PARTY_ROLE_MEMBER as i32;

/// Slowest and fastest playback speed a party may run at.
pub const MIN_RATE: f64 = 0.25;
pub const MAX_RATE: f64 = 2.0;

/// How many recent chat messages a joining member receives.
pub const RECENT_MESSAGES: i64 = 50;

type Tx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

/// Lets party chat carry stamps without this crate depending on the stamp
/// domain (which sits above it in the dependency graph). The server wires an
/// implementation backed by `tokuly_stamp::{ensure_usable, record_send}`
/// with `Video::set_party_stamps`; without one, stamps are refused.
#[async_trait]
pub trait PartyStamps: Send + Sync + 'static {
    /// Checks that `user_id` may send `stamp_id`, records the send, and
    /// returns the stamp as it is embedded in messages. Fails with
    /// `STAMP_NOT_USABLE` when the user may not send it.
    async fn send_stamp(&self, user_id: Uuid, stamp_id: Uuid) -> AppResult<StampRef>;
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PartyRow {
    pub id: Uuid,
    /// What invite links carry.
    pub code: String,
    pub video_id: Uuid,
    pub playlist_id: Option<Uuid>,
    pub allow_guests: bool,
    pub playing: bool,
    pub position_seconds: f64,
    pub rate: f64,
    pub state_at: Time,
    pub state_seq: i64,
    /// Seat that last changed the playback state.
    pub state_by: Option<Uuid>,
    pub created_by: Uuid,
    pub created_at: Time,
    pub ended_at: Option<Time>,
    pub end_reason: i32,
}

macro_rules! party_columns {
    () => {
        "p.id, p.code, p.video_id, p.playlist_id, p.allow_guests, p.playing, p.position_seconds, p.rate, p.state_at, \
         p.state_seq, p.state_by, p.created_by, p.created_at, p.ended_at, p.end_reason"
    };
}
pub(crate) use party_columns;

impl PartyRow {
    pub fn is_open(&self) -> bool {
        self.ended_at.is_none()
    }

    pub fn playback(&self) -> PartyPlayback {
        PartyPlayback {
            playing: self.playing,
            position_seconds: self.position_seconds,
            rate: self.rate,
            updated_at: ts(self.state_at),
            sequence: self.state_seq,
            changed_by_member_id: self.state_by.map(|id| id.to_string()).unwrap_or_default(),
            ..Default::default()
        }
    }

    /// Path of the invite page in the web app.
    pub fn path(&self) -> String {
        format!("/party/{}", self.code)
    }
}

/// A seat: a signed-in member or a guest.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MemberRow {
    pub id: Uuid,
    pub party_id: Uuid,
    /// `None` for guests.
    pub user_id: Option<Uuid>,
    pub guest_name: String,
    /// Guests: the secret segment of their realtime topic.
    pub topic_key: Option<String>,
    /// Guests: keyed hash of the address they joined from.
    pub ip_hash: String,
    pub role: i32,
    pub joined_at: Time,
    pub last_seen_at: Time,
}

macro_rules! member_columns {
    () => {
        "m.id, m.party_id, m.user_id, m.guest_name, m.topic_key, m.ip_hash, m.role, m.joined_at, m.last_seen_at"
    };
}

impl MemberRow {
    pub fn is_guest(&self) -> bool {
        self.user_id.is_none()
    }

    pub fn is_host(&self) -> bool {
        self.role == ROLE_HOST
    }

    /// Host or co-host: may control playback and remove members.
    pub fn can_control(&self) -> bool {
        self.role == ROLE_HOST || self.role == ROLE_COHOST
    }

    /// The realtime topic this seat follows.
    pub fn topic(&self) -> String {
        match &self.topic_key {
            Some(key) => guest_topic(self.party_id, key),
            None => member_topic(self.party_id),
        }
    }
}

pub fn member_topic(party_id: Uuid) -> String {
    format!("party:{party_id}")
}

pub fn guest_topic(party_id: Uuid, key: &str) -> String {
    format!("party:{party_id}:{key}")
}

// ───────────────────────────── lookups ─────────────────────────────

pub fn not_found() -> AppError {
    AppError::not_found("PARTY_NOT_FOUND", "watch party not found")
}

pub fn not_a_member() -> AppError {
    AppError::denied("NOT_A_PARTY_MEMBER", "you are not in this watch party")
}

pub async fn find(core: &Core, party_id: Uuid) -> AppResult<Option<PartyRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", party_columns!(), " FROM watch_parties p WHERE p.id = $1"))
        .bind(party_id)
        .fetch_optional(&core.db)
        .await?)
}

pub async fn find_by_code(core: &Core, code: &str) -> AppResult<Option<PartyRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", party_columns!(), " FROM watch_parties p WHERE p.code = $1"))
        .bind(code)
        .fetch_optional(&core.db)
        .await?)
}

/// Locks an open party for the rest of the transaction: every change to its
/// membership or state goes through this, so they happen one at a time.
pub(crate) async fn lock_open(tx: &mut Tx<'_>, party_id: Uuid) -> AppResult<PartyRow> {
    let party: Option<PartyRow> = sqlx::query_as(concat!(
        "SELECT ",
        party_columns!(),
        " FROM watch_parties p WHERE p.id = $1 AND p.ended_at IS NULL FOR UPDATE"
    ))
    .bind(party_id)
    .fetch_optional(&mut **tx)
    .await?;
    party.ok_or_else(not_found)
}

/// Who is asking, as far as seats are concerned.
#[derive(Debug, Clone)]
pub enum Caller {
    User(Uuid),
    /// A guest, proving their seat with the token `JoinParty` gave them.
    Guest(String),
}

impl Caller {
    /// A signed-in caller acts with their account; anyone else needs a guest token.
    pub fn new(user_id: Option<Uuid>, guest_token: &str) -> AppResult<Self> {
        match user_id {
            Some(user_id) => Ok(Self::User(user_id)),
            None if !guest_token.trim().is_empty() => Ok(Self::Guest(guest_token.trim().to_owned())),
            None => Err(not_a_member()),
        }
    }
}

/// The caller's seat in a party, if they have one.
pub(crate) async fn seat_in(
    tx: &mut Tx<'_>,
    core: &Core,
    party_id: Uuid,
    caller: &Caller,
) -> AppResult<Option<MemberRow>> {
    Ok(match caller {
        Caller::User(user_id) => {
            sqlx::query_as(concat!(
                "SELECT ",
                member_columns!(),
                " FROM watch_party_members m WHERE m.party_id = $1 AND m.user_id = $2"
            ))
            .bind(party_id)
            .bind(user_id)
            .fetch_optional(&mut **tx)
            .await?
        }
        Caller::Guest(token) => {
            sqlx::query_as(concat!(
                "SELECT ",
                member_columns!(),
                " FROM watch_party_members m WHERE m.party_id = $1 AND m.guest_token_hash = $2"
            ))
            .bind(party_id)
            .bind(core.keys.token_hash(token))
            .fetch_optional(&mut **tx)
            .await?
        }
    })
}

pub async fn seat(core: &Core, party_id: Uuid, caller: &Caller) -> AppResult<Option<MemberRow>> {
    let mut tx = core.db.begin().await?;
    let seat = seat_in(&mut tx, core, party_id, caller).await?;
    tx.commit().await?;
    Ok(seat)
}

pub(crate) async fn member_in(tx: &mut Tx<'_>, party_id: Uuid, member_id: Uuid) -> AppResult<Option<MemberRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        member_columns!(),
        " FROM watch_party_members m WHERE m.party_id = $1 AND m.id = $2"
    ))
    .bind(party_id)
    .bind(member_id)
    .fetch_optional(&mut **tx)
    .await?)
}

/// Everyone present, host first, then co-hosts, then by arrival.
pub async fn members(core: &Core, party_id: Uuid) -> AppResult<Vec<MemberRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        member_columns!(),
        " FROM watch_party_members m WHERE m.party_id = $1 ORDER BY m.role, m.joined_at, m.id"
    ))
    .bind(party_id)
    .fetch_all(&core.db)
    .await?)
}

pub async fn member_count(core: &Core, party_id: Uuid) -> AppResult<i32> {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM watch_party_members WHERE party_id = $1")
        .bind(party_id)
        .fetch_one(&core.db)
        .await?;
    Ok(count as i32)
}

/// Seats as API messages, in the given order.
pub async fn member_protos(core: &Core, rows: &[MemberRow]) -> AppResult<Vec<PartyMember>> {
    let user_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.user_id).collect();
    let users = tokuly_identity::summaries(core, &user_ids).await?;
    Ok(rows
        .iter()
        .map(|row| {
            let user = row.user_id.map(|id| tokuly_identity::users::pick(&users, id));
            PartyMember {
                id: row.id.to_string(),
                name: user.as_ref().map(|user| user.display_name.clone()).unwrap_or_else(|| row.guest_name.clone()),
                user: user.into(),
                role: row.role.into(),
                is_guest: row.is_guest(),
                joined_at: ts(row.joined_at),
                ..Default::default()
            }
        })
        .collect())
}

pub async fn member_proto(core: &Core, row: &MemberRow) -> AppResult<PartyMember> {
    Ok(member_protos(core, std::slice::from_ref(row)).await?.pop().unwrap_or_default())
}

#[derive(sqlx::FromRow)]
struct MessageRow {
    id: Uuid,
    member_id: Uuid,
    user_id: Option<Uuid>,
    name: String,
    role: i32,
    text: String,
    stamp: Option<serde_json::Value>,
    created_at: Time,
}

/// The latest chat messages, oldest first.
pub async fn recent_messages(core: &Core, party_id: Uuid) -> AppResult<Vec<PartyMessage>> {
    let mut rows: Vec<MessageRow> = sqlx::query_as(
        "SELECT id, member_id, user_id, name, role, text, stamp, created_at FROM watch_party_messages
         WHERE party_id = $1 ORDER BY id DESC LIMIT $2",
    )
    .bind(party_id)
    .bind(RECENT_MESSAGES)
    .fetch_all(&core.db)
    .await?;
    rows.reverse();
    let user_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.user_id).collect();
    let users = tokuly_identity::summaries(core, &user_ids).await?;
    Ok(rows
        .into_iter()
        .map(|row| PartyMessage {
            id: row.id.to_string(),
            sender: PartyMember {
                id: row.member_id.to_string(),
                user: row.user_id.map(|id| tokuly_identity::users::pick(&users, id)).into(),
                name: row.name,
                role: row.role.into(),
                is_guest: row.user_id.is_none(),
                ..Default::default()
            }
            .into(),
            text: row.text,
            stamp: row.stamp.and_then(|stamp| serde_json::from_value::<StampRef>(stamp).ok()).into(),
            sent_at: ts(row.created_at),
            ..Default::default()
        })
        .collect())
}

// ───────────────────────────── who hears what ─────────────────────────────

/// The realtime audience of a party: the member topic plus one topic per guest.
#[derive(Debug, Clone)]
pub struct Audience {
    pub party_id: Uuid,
    pub guest_keys: Vec<String>,
}

pub(crate) async fn audience_in(tx: &mut Tx<'_>, party_id: Uuid) -> AppResult<Audience> {
    let guest_keys: Vec<String> =
        sqlx::query_scalar("SELECT topic_key FROM watch_party_members WHERE party_id = $1 AND topic_key IS NOT NULL")
            .bind(party_id)
            .fetch_all(&mut **tx)
            .await?;
    Ok(Audience { party_id, guest_keys })
}

pub async fn audience(core: &Core, party_id: Uuid) -> AppResult<Audience> {
    let guest_keys: Vec<String> =
        sqlx::query_scalar("SELECT topic_key FROM watch_party_members WHERE party_id = $1 AND topic_key IS NOT NULL")
            .bind(party_id)
            .fetch_all(&core.db)
            .await?;
    Ok(Audience { party_id, guest_keys })
}

/// The audiences of the open parties watching a video (to tell them it is
/// going away).
pub(crate) async fn audiences_of_video_in(tx: &mut Tx<'_>, video_id: Uuid) -> AppResult<Vec<Audience>> {
    let party_ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM watch_parties WHERE video_id = $1 AND ended_at IS NULL")
            .bind(video_id)
            .fetch_all(&mut **tx)
            .await?;
    let mut audiences = Vec::with_capacity(party_ids.len());
    for party_id in party_ids {
        audiences.push(audience_in(tx, party_id).await?);
    }
    Ok(audiences)
}

/// Publishes an event to everyone in a party.
pub async fn push(core: &Core, audience: &Audience, event: Push) {
    let event = VideoPush { event: Some(event), ..Default::default() };
    core.rt.notify_topic(&member_topic(audience.party_id), &event).await;
    for key in &audience.guest_keys {
        core.rt.notify_topic(&guest_topic(audience.party_id, key), &event).await;
    }
}

/// [`push`] to the party's current audience.
pub async fn push_to_party(core: &Core, party_id: Uuid, event: Push) {
    match audience(core, party_id).await {
        Ok(audience) => push(core, &audience, event).await,
        Err(error) => tracing::warn!(%party_id, %error, "could not load a party's audience"),
    }
}

pub async fn push_ended(core: &Core, audience: &Audience, reason: PartyEndReason) {
    let ended = PartyEnded { party_id: audience.party_id.to_string(), reason: reason.into(), ..Default::default() };
    push(core, audience, Push::PartyEnded(Box::new(ended))).await;
    // After `party_ended`, which travels on the same topics: they are over.
    close_topics(core, audience).await;
}

/// Ends the party's member topic and every guest topic for everyone.
async fn close_topics(core: &Core, audience: &Audience) {
    core.rt.close_topic(&member_topic(audience.party_id)).await;
    for key in &audience.guest_keys {
        core.rt.close_topic(&guest_topic(audience.party_id, key)).await;
    }
}

/// Ends what the member who left followed of the party: a signed-in
/// member's subscription to the member topic (on all their devices), a
/// guest's own topic.
async fn revoke_seat(core: &Core, left: &MemberRow) {
    match (&left.user_id, &left.topic_key) {
        (Some(user_id), _) => core.rt.revoke(*user_id, &member_topic(left.party_id)).await,
        (None, Some(key)) => core.rt.close_topic(&guest_topic(left.party_id, key)).await,
        (None, None) => {}
    }
}

/// Admits subscribers to `party:` topics: signed-in members by user ID,
/// guests by the secret key of their seat (see the module documentation).
pub struct PartyGuard {
    pub db: sqlx::PgPool,
}

#[async_trait]
impl TopicGuard for PartyGuard {
    async fn may_subscribe(&self, subscriber: Option<Uuid>, topic: &str) -> AppResult<bool> {
        let mut segments = topic.split(':');
        let (Some("party"), Some(party_id)) = (segments.next(), segments.next()) else { return Ok(false) };
        let Ok(party_id) = Uuid::parse_str(party_id) else { return Ok(false) };
        let key = segments.next();
        if segments.next().is_some() {
            return Ok(false);
        }
        let admitted: bool = match (key, subscriber) {
            (Some(key), _) => {
                sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM watch_party_members m JOIN watch_parties p ON p.id = m.party_id
                                    WHERE m.party_id = $1 AND m.topic_key = $2 AND p.ended_at IS NULL)",
                )
                .bind(party_id)
                .bind(key)
                .fetch_one(&self.db)
                .await?
            }
            (None, Some(user_id)) => {
                sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM watch_party_members m JOIN watch_parties p ON p.id = m.party_id
                                    WHERE m.party_id = $1 AND m.user_id = $2 AND p.ended_at IS NULL)",
                )
                .bind(party_id)
                .bind(user_id)
                .fetch_one(&self.db)
                .await?
            }
            (None, None) => false,
        };
        Ok(admitted)
    }
}

pub fn guard(core: &Core) -> Arc<PartyGuard> {
    Arc::new(PartyGuard { db: core.db.clone() })
}

// ───────────────────────────── membership changes ─────────────────────────────

/// What a member's departure led to.
#[derive(Debug)]
pub struct Departure {
    pub left: MemberRow,
    /// Who heard about it: the audience as it was with the member still in.
    pub audience: Audience,
    /// The member who took over as host, when the host left.
    pub new_host: Option<MemberRow>,
    /// The party ended because no signed-in member remained.
    pub ended: bool,
    pub member_count: i32,
}

/// Removes a seat from a locked party. When the host goes, the
/// longest-present co-host — or else signed-in member — takes over; when no
/// signed-in member is left, the party ends.
pub(crate) async fn remove_member_in(tx: &mut Tx<'_>, party: &PartyRow, member: &MemberRow) -> AppResult<Departure> {
    let audience = audience_in(tx, party.id).await?;
    sqlx::query("DELETE FROM watch_party_members WHERE id = $1").bind(member.id).execute(&mut **tx).await?;
    let mut new_host = None;
    let mut ended = false;
    if member.is_host() {
        let successor: Option<MemberRow> = sqlx::query_as(concat!(
            "SELECT ",
            member_columns!(),
            " FROM watch_party_members m WHERE m.party_id = $1 AND m.user_id IS NOT NULL ORDER BY m.role, m.joined_at, m.id LIMIT 1"
        ))
        .bind(party.id)
        .fetch_optional(&mut **tx)
        .await?;
        match successor {
            Some(mut successor) => {
                sqlx::query("UPDATE watch_party_members SET role = $2 WHERE id = $1")
                    .bind(successor.id)
                    .bind(ROLE_HOST)
                    .execute(&mut **tx)
                    .await?;
                successor.role = ROLE_HOST;
                new_host = Some(successor);
            }
            None => {
                end_in(tx, party.id, PartyEndReason::PARTY_END_REASON_EMPTY).await?;
                ended = true;
            }
        }
    }
    let member_count: i64 = sqlx::query_scalar("SELECT count(*) FROM watch_party_members WHERE party_id = $1")
        .bind(party.id)
        .fetch_one(&mut **tx)
        .await?;
    Ok(Departure { left: member.clone(), audience, new_host, ended, member_count: member_count as i32 })
}

/// Tells the party about a departure (after the transaction committed).
pub async fn announce_departure(core: &Core, departure: &Departure, reason: PartyLeaveReason) {
    let left = PartyMemberLeft {
        member_id: departure.left.id.to_string(),
        reason: reason.into(),
        member_count: departure.member_count,
        ..Default::default()
    };
    push(core, &departure.audience, Push::PartyMemberLeft(Box::new(left))).await;
    if let Some(host) = &departure.new_host {
        match member_proto(core, host).await {
            Ok(member) => push(core, &departure.audience, Push::PartyMemberUpdated(Box::new(member))).await,
            Err(error) => tracing::warn!(party_id = %host.party_id, %error, "could not announce a new party host"),
        }
    }
    if departure.ended {
        push_ended(core, &departure.audience, PartyEndReason::PARTY_END_REASON_EMPTY).await;
    } else {
        revoke_seat(core, &departure.left).await;
    }
}

/// Marks a party as over. Returns whether it was open.
pub(crate) async fn end_in(tx: &mut Tx<'_>, party_id: Uuid, reason: PartyEndReason) -> AppResult<bool> {
    let ended = sqlx::query("UPDATE watch_parties SET ended_at = now(), end_reason = $2, playing = false WHERE id = $1 AND ended_at IS NULL")
        .bind(party_id)
        .bind(reason as i32)
        .execute(&mut **tx)
        .await?;
    Ok(ended.rows_affected() == 1)
}

/// Ends a party and tells everyone in it.
pub async fn end(core: &Core, party_id: Uuid, reason: PartyEndReason) -> AppResult<bool> {
    let mut tx = core.db.begin().await?;
    let audience = audience_in(&mut tx, party_id).await?;
    let ended = end_in(&mut tx, party_id, reason).await?;
    tx.commit().await?;
    if ended {
        push_ended(core, &audience, reason).await;
    }
    Ok(ended)
}

/// Ends every open party on a video (it was unpublished or became private).
pub async fn end_for_video(core: &Core, video_id: Uuid) -> AppResult<()> {
    let party_ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM watch_parties WHERE video_id = $1 AND ended_at IS NULL")
            .bind(video_id)
            .fetch_all(&core.db)
            .await?;
    for party_id in party_ids {
        end(core, party_id, PartyEndReason::PARTY_END_REASON_VIDEO_UNAVAILABLE).await?;
    }
    Ok(())
}

/// What one sweep of the party worker did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Sweep {
    pub timed_out: u64,
    pub expired: u64,
    pub purged: u64,
}

/// Housekeeping, run every few seconds by the worker: drops members whose
/// client stopped sending heartbeats, ends parties that ran longer than
/// `max_hours`, and deletes parties a day after they ended.
pub async fn sweep(core: &Core, member_timeout_seconds: u64, max_hours: i64) -> AppResult<Sweep> {
    let mut sweep = Sweep::default();
    let stale: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT m.party_id, m.id FROM watch_party_members m JOIN watch_parties p ON p.id = m.party_id
         WHERE p.ended_at IS NULL AND m.last_seen_at < now() - make_interval(secs => $1)
         ORDER BY m.last_seen_at LIMIT 500",
    )
    .bind(member_timeout_seconds as f64)
    .fetch_all(&core.db)
    .await?;
    for (party_id, member_id) in stale {
        let mut tx = core.db.begin().await?;
        let Ok(party) = lock_open(&mut tx, party_id).await else { continue };
        // A heartbeat may have arrived while we waited for the lock.
        let still_stale: Option<MemberRow> = sqlx::query_as(concat!(
            "SELECT ",
            member_columns!(),
            " FROM watch_party_members m WHERE m.id = $1 AND m.last_seen_at < now() - make_interval(secs => $2)"
        ))
        .bind(member_id)
        .bind(member_timeout_seconds as f64)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(member) = still_stale else { continue };
        let departure = remove_member_in(&mut tx, &party, &member).await?;
        tx.commit().await?;
        announce_departure(core, &departure, PartyLeaveReason::PARTY_LEAVE_REASON_TIMED_OUT).await;
        sweep.timed_out += 1;
    }

    let expired: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM watch_parties WHERE ended_at IS NULL AND created_at < now() - make_interval(hours => $1) LIMIT 500")
            .bind(max_hours as i32)
            .fetch_all(&core.db)
            .await?;
    for party_id in expired {
        if end(core, party_id, PartyEndReason::PARTY_END_REASON_EXPIRED).await? {
            sweep.expired += 1;
        }
    }

    sweep.purged = sqlx::query("DELETE FROM watch_parties WHERE ended_at < now() - interval '1 day'")
        .execute(&core.db)
        .await?
        .rows_affected();
    Ok(sweep)
}

/// Whether `position` and `rate` describe a playback state a party can be in
/// for a video of `duration` seconds.
pub fn valid_playback(position: f64, rate: f64, duration: f64) -> bool {
    position.is_finite()
        && position >= 0.0
        && position <= duration + 1.0
        && rate.is_finite()
        && (MIN_RATE..=MAX_RATE).contains(&rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_keep_guests_apart() {
        let party = Uuid::now_v7();
        assert_eq!(member_topic(party), format!("party:{party}"));
        assert_eq!(guest_topic(party, "Ab3x"), format!("party:{party}:Ab3x"));
        let member = |topic_key: Option<&str>| MemberRow {
            id: Uuid::now_v7(),
            party_id: party,
            user_id: topic_key.is_none().then(Uuid::now_v7),
            guest_name: String::new(),
            topic_key: topic_key.map(str::to_owned),
            ip_hash: String::new(),
            role: ROLE_MEMBER,
            joined_at: now(),
            last_seen_at: now(),
        };
        assert_eq!(member(None).topic(), member_topic(party));
        assert_eq!(member(Some("k1")).topic(), guest_topic(party, "k1"));
        assert!(member(Some("k1")).is_guest() && !member(None).is_guest());
    }

    #[test]
    fn playback_states_are_bounded() {
        assert!(valid_playback(0.0, 1.0, 60.0));
        assert!(valid_playback(60.5, 2.0, 60.0));
        assert!(!valid_playback(62.0, 1.0, 60.0));
        assert!(!valid_playback(-1.0, 1.0, 60.0));
        assert!(!valid_playback(f64::NAN, 1.0, 60.0));
        assert!(!valid_playback(1.0, 0.1, 60.0) && !valid_playback(1.0, 4.0, 60.0));
    }

    #[test]
    fn callers_are_accounts_or_guests_with_a_token() {
        let user = Uuid::now_v7();
        assert!(matches!(Caller::new(Some(user), "ignored").unwrap(), Caller::User(id) if id == user));
        assert!(matches!(Caller::new(None, " tok ").unwrap(), Caller::Guest(token) if token == "tok"));
        assert_eq!(Caller::new(None, "  ").unwrap_err().reason, "NOT_A_PARTY_MEMBER");
    }
}
