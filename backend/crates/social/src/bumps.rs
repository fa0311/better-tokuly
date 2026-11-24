//! Bumps: two phones shaken together.
//!
//! A shake is an *attempt*, kept in Valkey for a few seconds (the position in
//! a live GEO set, the details as JSON). Two attempts match when they are
//! close enough in space and time; a match becomes a bump in PostgreSQL.
//!
//! Each caller publishes its attempt *before* looking for others, so of two
//! people shaking at the same moment at least one sees the other. When both
//! do, a `SET NX` on the pair decides who records the bump; the other caller
//! (and any retry during the cool-down) is handed that same bump. The
//! participant who was not calling learns of it through `SocialEvent.bump`.
//!
//! The shake's position is the client's word, so it is checked against the
//! account's own recent location fixes before anything is published (see
//! [`crate::claims`]).
use std::{collections::HashSet, time::Duration};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokuly_core::{
    events, geoindex,
    outbound::{self, ActivityAudience, ActivityEntry, NotificationRequest},
    prelude::*,
};
use tokuly_identity::{settings, users};
use tokuly_proto::{
    common::{LatLng, PageInfo, SubjectType, UserSummary},
    social::{Bump, FriendSource, social_event::Event as Push},
};

use crate::{
    Social,
    claims::Claim,
    copy,
    events::{BUMP_CREATED, BumpCreated},
    graph::{self, InteractionKind, Pair},
    livegeo::LiveGeoSet,
    people,
    requests::{RequestOutcome, RequestVia},
};

/// Combined GPS error of the two phones we are willing to add to the bump
/// radius, in meters. Indoors, where people bump, GPS is rarely better.
const MAX_ACCURACY_ALLOWANCE: f64 = 30.0;
/// How far back a device-reported shake time may lie (network latency) and
/// how far ahead (clock skew) before the arrival time is used instead.
const MAX_SHAKE_AGE_MS: i64 = 5000;
const MAX_SHAKE_AHEAD_MS: i64 = 1000;
/// Attempts outlive the match window by this much, covering request latency.
const ATTEMPT_SLACK: Duration = Duration::from_secs(2);
/// Bumps one shake can produce (a group of friends shaking together). At
/// most one of them can be with a stranger.
const MAX_BUMPS_PER_SHAKE: usize = 4;
/// Candidates examined around one shake.
const MAX_CANDIDATES: usize = 12;
/// Mutual friends told about one bump.
const MAX_BUMP_AUDIENCE: usize = 200;
/// Mutual friends hear about the same two people at most this often.
const ANNOUNCE_EVERY: Duration = Duration::from_secs(600);
const MAX_BUMPS_PAGE: i64 = 100;
const MAX_PLACE_NAME_CHARS: usize = 120;

/// One shake, as remembered for a few seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct Attempt {
    lat: f64,
    lng: f64,
    /// Reported GPS accuracy in meters.
    accuracy: f64,
    /// When the phone was shaken, Unix milliseconds.
    at_ms: i64,
    /// Peak acceleration in g.
    intensity: f64,
}

/// When two attempts count as one bump.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BumpRules {
    radius_meters: f64,
    window_ms: i64,
}

impl BumpRules {
    fn is_match(&self, a: &Attempt, b: &Attempt) -> bool {
        let apart = geoindex::distance_meters(a.lat, a.lng, b.lat, b.lng);
        let allowance = (a.accuracy + b.accuracy).clamp(0.0, MAX_ACCURACY_ALLOWANCE);
        (a.at_ms - b.at_ms).abs() <= self.window_ms && apart <= self.radius_meters + allowance
    }

    /// The widest circle in which a matching attempt can lie.
    fn search_radius(&self) -> f64 {
        self.radius_meters + MAX_ACCURACY_ALLOWANCE
    }

    fn attempt_ttl(&self) -> Duration {
        Duration::from_millis(self.window_ms.max(0) as u64) + ATTEMPT_SLACK
    }
}

/// When the shake happened: the device's clock when it is plausible,
/// otherwise the moment the request arrived.
fn shake_time(reported: Option<DateTime<Utc>>, received: DateTime<Utc>) -> DateTime<Utc> {
    match reported {
        Some(at) if (-MAX_SHAKE_AHEAD_MS..=MAX_SHAKE_AGE_MS).contains(&(received - at).num_milliseconds()) => at,
        _ => received,
    }
}

/// Puts the best partners for `mine` first: closest in time, then most
/// similar shake strength (two phones knocked together peak alike).
fn rank_matches(mine: &Attempt, candidates: &mut [(Uuid, Attempt)]) {
    candidates.sort_by(|(id_a, a), (id_b, b)| {
        let time = (a.at_ms - mine.at_ms).abs().cmp(&(b.at_ms - mine.at_ms).abs());
        let strength = (a.intensity - mine.intensity).abs().total_cmp(&(b.intensity - mine.intensity).abs());
        time.then(strength).then(id_a.cmp(id_b))
    });
}

fn attempt_key(user_id: Uuid) -> String {
    format!("social:bump:attempt:{}", user_id.simple())
}

fn pair_key(prefix: &str, pair: Pair) -> String {
    format!("social:bump:{prefix}:{}:{}", pair.lo.simple(), pair.hi.simple())
}

#[derive(Debug, sqlx::FromRow)]
struct BumpRow {
    id: Uuid,
    user_lo: Uuid,
    user_hi: Uuid,
    lat: f64,
    lng: f64,
    place_name: String,
    friend_request_created: bool,
    created_at: DateTime<Utc>,
    pair_count: i32,
    attrs: serde_json::Value,
}

/// Another person's shake that matches the caller's.
#[derive(Debug, Clone, Copy)]
struct Candidate {
    user_id: Uuid,
    attempt: Attempt,
    /// Already friends with the caller.
    friend: bool,
}

/// A bump about to be stored.
struct BumpRecord {
    id: Uuid,
    /// Who shook first, and who second.
    first: Uuid,
    second: Uuid,
    lat: f64,
    lng: f64,
    /// The two are friends (possibly since a moment ago).
    friends: bool,
    friend_request_created: bool,
    /// Where each of the two shook, for a bump between people who were not
    /// friends when they shook. Each of them only ever gets their own
    /// position back: the point between the two phones, together with
    /// one's own, would give away the other's.
    own_positions: Option<[(Uuid, f64, f64); 2]>,
}

/// Columns of [`BumpRow`], for queries over `bumps b` left-joined with `friend_pairs p`.
macro_rules! bump_columns {
    () => {
        "b.id, b.user_lo, b.user_hi, b.lat, b.lng, b.place_name, b.friend_request_created, b.created_at, \
         COALESCE(p.bump_count, 0) AS pair_count, b.attrs"
    };
}

/// `attrs` of a bump between people who were not friends: where each shook.
fn own_positions_attrs(positions: &[(Uuid, f64, f64); 2]) -> serde_json::Value {
    let positions: serde_json::Map<String, serde_json::Value> =
        positions.iter().map(|(user_id, lat, lng)| (user_id.to_string(), serde_json::json!({ "lat": lat, "lng": lng }))).collect();
    serde_json::json!({ "own_positions": positions })
}

impl BumpRow {
    /// Where `viewer` is told the bump happened: the point between the two
    /// phones for friends, the viewer's own shake for anyone else.
    fn location_for(&self, viewer: Uuid) -> LatLng {
        let own = &self.attrs["own_positions"];
        if own.is_object() {
            let mine = &own[viewer.to_string()];
            return LatLng { lat: mine["lat"].as_f64().unwrap_or_default(), lng: mine["lng"].as_f64().unwrap_or_default(), ..Default::default() };
        }
        LatLng { lat: self.lat, lng: self.lng, ..Default::default() }
    }

    /// The bump as `viewer` (one of the two) sees it.
    fn into_proto(self, viewer: Uuid, summaries: &std::collections::HashMap<Uuid, UserSummary>) -> Bump {
        Bump {
            id: self.id.to_string(),
            participants: vec![users::pick(summaries, self.user_lo), users::pick(summaries, self.user_hi)],
            location: self.location_for(viewer).into(),
            place_name: self.place_name,
            created_at: ts(self.created_at),
            pair_count: self.pair_count,
            friend_request_created: self.friend_request_created,
            ..Default::default()
        }
    }
}

/// Names the place of a bump. For the geo domain, which knows places and can
/// react to `social.bump.created`.
pub async fn set_bump_place(core: &Core, bump_id: Uuid, place_name: &str) -> AppResult<()> {
    let place_name: String = place_name.trim().chars().take(MAX_PLACE_NAME_CHARS).collect();
    sqlx::query("UPDATE bumps SET place_name = $2 WHERE id = $1").bind(bump_id).bind(place_name).execute(&core.db).await?;
    Ok(())
}

/// How many bumps a user has taken part in.
pub(crate) async fn bump_count(core: &Core, user_id: Uuid) -> AppResult<i64> {
    Ok(sqlx::query_scalar("SELECT COALESCE(sum(bump_count), 0)::bigint FROM friend_pairs WHERE user_lo = $1 OR user_hi = $1")
        .bind(user_id)
        .fetch_one(&core.db)
        .await?)
}

impl Social {
    fn bump_rules(&self) -> BumpRules {
        BumpRules { radius_meters: self.cfg.bump_radius_meters, window_ms: self.cfg.bump_window_ms }
    }

    /// The live set of people who just shook their phone.
    pub(crate) fn bump_attempts(&self) -> LiveGeoSet {
        LiveGeoSet::new("social:bump:geo", "social:bump:seen", self.bump_rules().attempt_ttl())
    }

    /// Registers a shake by `user_id` and returns the bumps it completes
    /// (empty when nobody else shook nearby just now).
    pub(crate) async fn bump(&self, user_id: Uuid, lat: f64, lng: f64, accuracy_meters: f64, shaken_at: Option<DateTime<Utc>>, intensity: f64) -> AppResult<Vec<Bump>> {
        let core = &self.core;
        ensure!(geoindex::valid_coordinates(lat, lng), invalid, "INVALID_LOCATION", "a valid location is required");
        core.cache.rate_limit(&format!("bump:{user_id}"), 30, Duration::from_secs(60)).await?;
        // Checked before the attempt is published: a shake from somewhere
        // the user is not can match nobody (see [`crate::claims`]).
        self.require_plausible(user_id, Claim { lat, lng, accuracy_meters }).await?;
        let mine = Attempt {
            lat,
            lng,
            accuracy: if accuracy_meters.is_finite() { accuracy_meters.max(0.0) } else { 0.0 },
            at_ms: shake_time(shaken_at, Utc::now()).timestamp_millis(),
            intensity: if intensity.is_finite() { intensity.max(0.0) } else { 0.0 },
        };

        // Publish first, look second (see the module docs).
        core.cache.set_json(&attempt_key(user_id), &mine, Some(self.bump_rules().attempt_ttl())).await?;
        self.bump_attempts().check_in(&core.cache, user_id, lat, lng).await?;

        let mut bumps = Vec::new();
        for (rank, candidate) in self.matching_attempts(user_id, &mine).await?.into_iter().enumerate() {
            if bumps.len() == MAX_BUMPS_PER_SHAKE {
                break;
            }
            // Friends shaking together all bump each other. A stranger only
            // when the two are each other's best match: someone lurking
            // nearby does not get to join in.
            if !candidate.friend && (rank > 0 || !self.is_best_match_of(candidate, user_id).await?) {
                continue;
            }
            if let Some(bump) = self.bump_pair(user_id, mine, candidate).await? {
                bumps.push(bump);
            }
        }
        Ok(bumps)
    }

    /// Whether `user_id`'s shake is the best match `candidate` has.
    async fn is_best_match_of(&self, candidate: Candidate, user_id: Uuid) -> AppResult<bool> {
        let theirs = self.matching_attempts(candidate.user_id, &candidate.attempt).await?;
        Ok(theirs.first().is_some_and(|best| best.user_id == user_id))
    }

    /// Other people's attempts that match `mine`, best partner first. People
    /// in a block with the user never match, and strangers only match when
    /// both are open to being found nearby.
    async fn matching_attempts(&self, user_id: Uuid, mine: &Attempt) -> AppResult<Vec<Candidate>> {
        let core = &self.core;
        let rules = self.bump_rules();
        let nearby: Vec<Uuid> = self
            .bump_attempts()
            .within(&core.cache, mine.lat, mine.lng, rules.search_radius(), MAX_CANDIDATES)
            .await?
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| *id != user_id)
            .collect();
        if nearby.is_empty() {
            return Ok(Vec::new());
        }
        let keys: Vec<String> = nearby.iter().map(|id| attempt_key(*id)).collect();
        let stored: Vec<Option<String>> = redis::cmd("MGET").arg(&keys).query_async(&mut core.cache.conn()).await?;
        let hidden = graph::blocked_ids_either(core, user_id).await?;
        let mut matches: Vec<(Uuid, Attempt)> = nearby
            .into_iter()
            .zip(stored)
            .filter_map(|(id, json)| Some((id, serde_json::from_str::<Attempt>(&json?).ok()?)))
            .filter(|(id, theirs)| !hidden.contains(id) && rules.is_match(mine, theirs))
            .collect();
        let candidates: Vec<Uuid> = matches.iter().map(|(id, _)| *id).collect();
        let (friends, strangers) = self.may_bump(user_id, &candidates).await?;
        matches.retain(|(id, _)| friends.contains(id) || strangers.contains(id));
        rank_matches(mine, &mut matches);
        Ok(matches.into_iter().map(|(id, attempt)| Candidate { user_id: id, attempt, friend: friends.contains(&id) }).collect())
    }

    /// Who among `candidates` `user_id` may bump: friends always; strangers
    /// only when both of them have nearby discovery on and both are minors
    /// or both adults. A bump with a stranger tells each who the other is
    /// and sends a friend request, so it is a way of being found nearby.
    /// Returns `(friends, strangers)` among the candidates who may.
    async fn may_bump(&self, user_id: Uuid, candidates: &[Uuid]) -> AppResult<(HashSet<Uuid>, HashSet<Uuid>)> {
        let core = &self.core;
        if candidates.is_empty() {
            return Ok(Default::default());
        }
        let friends: HashSet<Uuid> = sqlx::query_scalar::<_, Uuid>("SELECT friend_id FROM friendships WHERE user_id = $1 AND friend_id = ANY($2)")
            .bind(user_id)
            .bind(candidates)
            .fetch_all(&core.db)
            .await?
            .into_iter()
            .collect();
        let strangers: Vec<Uuid> = candidates.iter().copied().filter(|id| !friends.contains(id)).collect();
        if strangers.is_empty() {
            return Ok((friends, HashSet::new()));
        }
        let involved = [strangers.as_slice(), &[user_id]].concat();
        let privacy = settings::privacy_of_many(core, &involved).await?;
        let minors = people::minors_among(core, &involved).await?;
        let discoverable = |id: &Uuid| privacy.get(id).is_some_and(|p| p.nearby_discoverable);
        let same_age_group = |id: &Uuid| minors.contains(id) == minors.contains(&user_id);
        let allowed = if discoverable(&user_id) { strangers.into_iter().filter(|id| discoverable(id) && same_age_group(id)).collect() } else { HashSet::new() };
        Ok((friends, allowed))
    }

    /// Turns one matched pair of attempts into a bump, exactly once however
    /// many requests race for it.
    async fn bump_pair(&self, caller: Uuid, mine: Attempt, candidate: Candidate) -> AppResult<Option<Bump>> {
        let other = candidate.user_id;
        let cache = &self.core.cache;
        let claim = pair_key("pair", Pair::of(caller, other));
        let bump_id = new_id();
        let cooldown = Duration::from_secs(self.cfg.bump_cooldown_seconds.max(1));
        if cache.set_nx(&claim, &bump_id.to_string(), cooldown).await? {
            return match self.create_bump(bump_id, caller, mine, candidate).await {
                Ok(bump) => Ok(Some(bump)),
                Err(e) => {
                    // Let the next shake try again instead of waiting out the cool-down.
                    let _ = cache.del(&claim).await;
                    Err(e)
                }
            };
        }
        // The other phone's request won the claim, or the two bumped moments
        // ago: return that bump rather than counting a second one.
        let claimed = cache.get_string(&claim).await?.and_then(|id| Uuid::parse_str(&id).ok());
        match claimed {
            Some(bump_id) => self.await_bump(bump_id, caller).await,
            None => Ok(None),
        }
    }

    /// Loads a bump another request is creating right now, waiting briefly
    /// for its transaction. `None` if it does not show up; the caller's
    /// client still gets it as a realtime event.
    async fn await_bump(&self, bump_id: Uuid, viewer: Uuid) -> AppResult<Option<Bump>> {
        for _ in 0..10 {
            if let Some(bump) = self.find_bump(bump_id, viewer).await? {
                return Ok(Some(bump));
            }
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
        Ok(None)
    }

    /// A bump as `viewer` (one of the two) sees it.
    async fn find_bump(&self, bump_id: Uuid, viewer: Uuid) -> AppResult<Option<Bump>> {
        let row: Option<BumpRow> = sqlx::query_as(concat!(
            "SELECT ",
            bump_columns!(),
            " FROM bumps b LEFT JOIN friend_pairs p ON p.user_lo = b.user_lo AND p.user_hi = b.user_hi WHERE b.id = $1"
        ))
        .bind(bump_id)
        .fetch_optional(&self.core.db)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let summaries = tokuly_identity::summaries(&self.core, &[row.user_lo, row.user_hi]).await?;
        Ok(Some(row.into_proto(viewer, &summaries)))
    }

    /// Records the bump between `caller` and `other` and tells everyone
    /// concerned: both participants, and for two friends their mutual friends.
    async fn create_bump(&self, bump_id: Uuid, caller: Uuid, mine: Attempt, candidate: Candidate) -> AppResult<Bump> {
        let core = &self.core;
        let (other, theirs) = (candidate.user_id, candidate.attempt);
        // Who shook first. On a tie (same millisecond) it is the one whose
        // attempt was already waiting when the caller's request arrived.
        let (first, second) = if mine.at_ms < theirs.at_ms { (caller, other) } else { (other, caller) };
        let (friends, friend_request_created) = self.connect_bumpers(first, second).await?;
        let audience = if friends { self.bump_audience(Pair::of(first, second)).await? } else { Vec::new() };
        let record = BumpRecord {
            id: bump_id,
            first,
            second,
            // Between the two phones.
            lat: (mine.lat + theirs.lat) / 2.0,
            lng: (mine.lng + theirs.lng) / 2.0,
            friends,
            friend_request_created,
            own_positions: (!candidate.friend).then_some([(caller, mine.lat, mine.lng), (other, theirs.lat, theirs.lng)]),
        };
        self.store_bump(&record, &audience).await?;

        if friends && let Err(e) = graph::record_interaction(core, first, second, InteractionKind::Bump, 1).await {
            tracing::warn!(error = %e, "bump not counted towards the best-friend ranking");
        }
        let vanished = || AppError::internal(anyhow::anyhow!("bump {bump_id} vanished after commit"));
        let bump = self.find_bump(bump_id, caller).await?.ok_or_else(vanished)?;
        let for_other = self.find_bump(bump_id, other).await?.ok_or_else(vanished)?;
        self.push(caller, Push::Bump(Box::new(bump.clone()))).await;
        self.push(other, Push::Bump(Box::new(for_other))).await;
        // Mutual friends hear that the two met, not where: each of the two
        // may be hiding their position from some of them (ghost mode), which
        // only the geo domain knows about.
        let announced = Bump { location: Default::default(), ..bump.clone() };
        self.push_many(audience, Push::FriendsBumped(Box::new(announced))).await;
        Ok(bump)
    }

    /// What two people who just bumped are to each other: `(friends,
    /// friend_request_created)`. Strangers get a friend request, from
    /// whoever shook first (which makes them friends on the spot if the
    /// other had asked before).
    async fn connect_bumpers(&self, first: Uuid, second: Uuid) -> AppResult<(bool, bool)> {
        if graph::are_friends(&self.core, first, second).await? {
            return Ok((true, false));
        }
        // Both shook on purpose, so the "who may send me requests" audience
        // is moot. Other obstacles (a full friend list...) just mean no
        // request; the bump itself still counts.
        match self.request_friendship(first, second, FriendSource::FRIEND_SOURCE_BUMP, "", RequestVia::Encounter).await {
            Ok(RequestOutcome::Friends) => Ok((true, false)),
            Ok(RequestOutcome::Pending { created, .. }) => Ok((false, created)),
            Err(e) if e.code == connectrpc::ErrorCode::Internal || e.code == connectrpc::ErrorCode::Unavailable => Err(e),
            Err(e) => {
                tracing::debug!(reason = %e.reason, "bump between strangers did not produce a friend request");
                Ok((false, false))
            }
        }
    }

    /// Stores a bump in one transaction: the row, the pair's counter, the
    /// event, and for friends the activity entries and notifications (an
    /// inbox entry for each of the two, "A and B bumped!" for `audience`).
    async fn store_bump(&self, bump: &BumpRecord, audience: &[Uuid]) -> AppResult<()> {
        let core = &self.core;
        let pair = Pair::of(bump.first, bump.second);
        let people = tokuly_identity::summaries(core, &[bump.first, bump.second]).await?;
        let (first_user, second_user) = (users::pick(&people, bump.first), users::pick(&people, bump.second));

        let mut tx = core.db.begin().await?;
        sqlx::query("INSERT INTO bumps (id, user_lo, user_hi, lat, lng, friend_request_created, attrs) VALUES ($1, $2, $3, $4, $5, $6, $7)")
            .bind(bump.id)
            .bind(pair.lo)
            .bind(pair.hi)
            .bind(bump.lat)
            .bind(bump.lng)
            .bind(bump.friend_request_created)
            .bind(bump.own_positions.as_ref().map(own_positions_attrs).unwrap_or_else(|| serde_json::json!({})))
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO friend_pairs (user_lo, user_hi, bump_count, last_bump_at) VALUES ($1, $2, 1, now())
             ON CONFLICT (user_lo, user_hi) DO UPDATE SET bump_count = friend_pairs.bump_count + 1, last_bump_at = now()",
        )
        .bind(pair.lo)
        .bind(pair.hi)
        .execute(&mut *tx)
        .await?;
        let created = BumpCreated { bump_id: bump.id, user_ids: [bump.first, bump.second], friends: bump.friends, lat: bump.lat, lng: bump.lng };
        events::emit(&mut *tx, BUMP_CREATED, Some(bump.first), &created).await?;

        if bump.friends {
            for (user_id, partner_id, partner) in [(bump.first, bump.second, &second_user), (bump.second, bump.first, &first_user)] {
                let partner_name = people::name_of(partner);
                outbound::activity(
                    &mut *tx,
                    &ActivityEntry::new(user_id, "social.bumped", ActivityAudience::Friends, copy::bumped_activity(&partner_name))
                        .subject(SubjectType::SUBJECT_TYPE_USER, partner_id)
                        .with([partner_id])
                        .path(format!("/u/{}", partner.handle))
                        // No `.at(...)`: the friends reading this may be
                        // kept from the user's position by ghost mode.
                        .data(serde_json::json!({ "bump_id": bump.id })),
                )
                .await?;
                // Both are looking at their phones; the inbox keeps the memory.
                let text = copy::bumped(&partner_name);
                outbound::notify(
                    &mut *tx,
                    &NotificationRequest::new(user_id, "social.bump", text.title, text.body, format!("/u/{}", partner.handle))
                        .actor(partner_id)
                        .subject(SubjectType::SUBJECT_TYPE_USER, partner_id)
                        .inbox_only(),
                )
                .await?;
            }
            let text = copy::friends_bumped(&people::name_of(&first_user), &people::name_of(&second_user));
            for friend in audience {
                outbound::notify(
                    &mut *tx,
                    &NotificationRequest::new(*friend, "social.friends_bumped", text.title.clone(), text.body.clone(), "/map")
                        .actor(bump.first)
                        .collapse(format!("social.friends_bumped:{}:{}", pair.lo, pair.hi))
                        .data(serde_json::json!({ "bump_id": bump.id, "user_ids": [bump.first, bump.second] })),
                )
                .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// The mutual friends to tell about a bump of `pair` — nobody if they
    /// were told about these two a moment ago.
    async fn bump_audience(&self, pair: Pair) -> AppResult<Vec<Uuid>> {
        let core = &self.core;
        if !core.cache.set_nx(&pair_key("announced", pair), "1", ANNOUNCE_EVERY).await? {
            return Ok(Vec::new());
        }
        graph::mutual_friend_ids(core, pair.lo, pair.hi, MAX_BUMP_AUDIENCE as i64).await
    }

    /// One page of `user_id`'s bumps, newest first, optionally only those
    /// with one other user. Bumps with people now hidden by a block are left out.
    pub(crate) async fn list_bumps(&self, user_id: Uuid, with: Option<Uuid>, page: &Page) -> AppResult<(Vec<Bump>, PageInfo)> {
        let core = &self.core;
        let before: Option<Uuid> = page.decode()?;
        let hidden = graph::blocked_ids_either(core, user_id).await?;
        let mut rows: Vec<BumpRow> = sqlx::query_as(concat!(
            "SELECT ",
            bump_columns!(),
            " FROM bumps b LEFT JOIN friend_pairs p ON p.user_lo = b.user_lo AND p.user_hi = b.user_hi
              WHERE (b.user_lo = $1 OR b.user_hi = $1)
                AND ($2::uuid IS NULL OR b.id < $2)
                AND ($3::uuid IS NULL OR b.user_lo = $3 OR b.user_hi = $3)
                AND NOT (b.user_lo = ANY($4) OR b.user_hi = ANY($4))
              ORDER BY b.id DESC
              LIMIT $5"
        ))
        .bind(user_id)
        .bind(before)
        .bind(with)
        .bind(&hidden)
        .bind(page.fetch())
        .fetch_all(&core.db)
        .await?;
        let info = page.finish(&mut rows, |r| r.id);
        let mut user_ids: Vec<Uuid> = rows.iter().flat_map(|r| [r.user_lo, r.user_hi]).collect();
        user_ids.sort_unstable();
        user_ids.dedup();
        let summaries = tokuly_identity::summaries(core, &user_ids).await?;
        Ok((rows.into_iter().map(|row| row.into_proto(user_id, &summaries)).collect(), info))
    }

    pub(crate) fn bumps_page(page: Option<&tokuly_proto::common::PageRequestView<'_>>) -> Page {
        Page::from_view(page, MAX_BUMPS_PAGE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: BumpRules = BumpRules { radius_meters: 30.0, window_ms: 4000 };
    /// One meter of latitude, in degrees.
    const METER: f64 = 1.0 / 111_195.0;

    fn attempt(meters_north: f64, at_ms: i64) -> Attempt {
        Attempt { lat: 35.0 + meters_north * METER, lng: 139.0, accuracy: 0.0, at_ms, intensity: 2.0 }
    }

    #[test]
    fn attempts_match_within_the_radius_and_window() {
        let me = attempt(0.0, 10_000);
        assert!(RULES.is_match(&me, &attempt(20.0, 12_000)));
        assert!(RULES.is_match(&me, &attempt(29.0, 6_000)), "the window applies in both directions");
        assert!(!RULES.is_match(&me, &attempt(40.0, 10_000)), "too far apart");
        assert!(!RULES.is_match(&me, &attempt(5.0, 14_500)), "too late");
    }

    #[test]
    fn reported_gps_error_widens_the_radius_up_to_a_limit() {
        let me = attempt(0.0, 0);
        let mut far = attempt(50.0, 0);
        assert!(!RULES.is_match(&me, &far));
        far.accuracy = 25.0;
        assert!(RULES.is_match(&me, &far), "50 m apart, but 25 m of that may be GPS error");
        let mut very_far = attempt(100.0, 0);
        very_far.accuracy = 500.0;
        assert!(!RULES.is_match(&me, &very_far), "a huge reported error does not make the whole block a match");
        assert!(RULES.search_radius() >= 60.0);
    }

    #[test]
    fn device_time_is_trusted_only_when_plausible() {
        let received = Utc::now();
        let ms = chrono::Duration::milliseconds;
        assert_eq!(shake_time(Some(received - ms(800)), received), received - ms(800));
        assert_eq!(shake_time(Some(received + ms(500)), received), received + ms(500), "slight clock skew is fine");
        assert_eq!(shake_time(Some(received - ms(60_000)), received), received, "a stale shake counts as now");
        assert_eq!(shake_time(Some(received + ms(30_000)), received), received);
        assert_eq!(shake_time(None, received), received);
    }

    #[test]
    fn the_closest_shake_in_time_is_the_best_partner() {
        let me = attempt(0.0, 10_000);
        let (early, close, twin) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let mut gentle = attempt(1.0, 10_100);
        gentle.intensity = 0.5;
        let mut candidates = vec![(early, attempt(1.0, 7_000)), (close, gentle), (twin, attempt(1.0, 9_900))];
        rank_matches(&me, &mut candidates);
        // `close` and `twin` are equally near in time; `twin` shook as hard as we did.
        assert_eq!(candidates.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![twin, close, early]);
    }

    #[test]
    fn attempts_outlive_the_window() {
        assert_eq!(RULES.attempt_ttl(), Duration::from_secs(6));
    }
}
