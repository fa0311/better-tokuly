//! What the pipeline and the visibility function need to know about a user
//! besides their live state: privacy flags, ghost rules, own places, an
//! active SOS / trip / live shares, and the place alerts watching them.
//!
//! All of it lives in PostgreSQL. [`OwnerCtx`] is a short-lived projection
//! cached in Valkey so that processing a location sample costs one read
//! instead of seven queries. Every code path that changes one of the
//! underlying rows calls [`Geo::invalidate_ctx`] after committing.
use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokuly_core::{
    geoindex::{self, Sight, SightRule},
    prelude::*,
};
use tokuly_identity::settings;
use tokuly_proto::identity::Status;

use crate::{Geo, state::Pin};

const CTX_TTL: Duration = Duration::from_secs(300);
/// After an invalidation, loads that may have read the old rows do not
/// re-cache for this long (same idea as the social domain's ID cache).
const DIRTY_TTL: Duration = Duration::from_secs(5);

/// The privacy settings the geo domain acts on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Privacy {
    pub share_battery: bool,
    pub share_activity: bool,
    pub share_online_status: bool,
    pub share_place_labels: bool,
    /// Record visits, footprints and raw history.
    pub record_history: bool,
    /// Days of raw history to keep (0 = forever).
    pub history_days: i32,
    /// identity `Audience` number: who may see the scratch map in full.
    pub footprints_audience: i32,
}

impl Default for Privacy {
    fn default() -> Self {
        Self::from(&settings::default_privacy())
    }
}

impl From<&tokuly_proto::identity::PrivacySettings> for Privacy {
    fn from(p: &tokuly_proto::identity::PrivacySettings) -> Self {
        Self {
            share_battery: p.share_battery,
            share_activity: p.share_activity,
            share_online_status: p.share_online_status,
            share_place_labels: p.share_place_labels,
            record_history: p.record_history,
            history_days: p.location_history_days,
            footprints_audience: p.footprints_visible_to.to_i32(),
        }
    }
}

/// The status shown next to a pin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusLite {
    pub emoji: String,
    pub text: String,
    pub set_ms: Option<i64>,
    pub expires_ms: Option<i64>,
}

impl StatusLite {
    /// The status as a message, unless it has expired.
    pub fn current(&self, now: DateTime<Utc>) -> Option<Status> {
        if self.expires_ms.is_some_and(|expires| expires <= now.timestamp_millis()) {
            return None;
        }
        Some(Status {
            emoji: self.emoji.clone(),
            text: self.text.clone(),
            set_at: opt_ts(self.set_ms.and_then(DateTime::from_timestamp_millis)),
            expires_at: opt_ts(self.expires_ms.and_then(DateTime::from_timestamp_millis)),
            ..Default::default()
        })
    }
}

/// A FROZEN rule's copy of the pin, and how it was shown when it froze.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrozenPin {
    pub pin: Pin,
    /// `GhostMode` the viewer saw before freezing: 1 precise or 2 blurred.
    pub base: i32,
}

/// One ghost rule: the default for everyone, or an override for one friend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GhostRule {
    /// `GhostMode` number.
    pub mode: i32,
    pub since_ms: i64,
    pub until_ms: Option<i64>,
    /// Set for FROZEN rules made while a position was known.
    pub frozen: Option<FrozenPin>,
    /// Since when the viewers of this rule have not seen the owner precisely
    /// (earlier than `since_ms` when it replaced another non-precise rule).
    /// What the owner did after this moment is not shown to them.
    #[serde(default)]
    pub hidden_since_ms: i64,
}

impl GhostRule {
    pub fn active(&self, now_ms: i64) -> bool {
        self.until_ms.is_none_or(|until| until > now_ms)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GhostConfig {
    pub everyone: Option<GhostRule>,
    pub overrides: HashMap<Uuid, GhostRule>,
    /// What a timed, non-precise override turns into when it ends while the
    /// default is not precise either (see [`GhostConfig::lapse`]): the
    /// default's mode, but the override's copy of the pin and its watermark.
    #[serde(default)]
    pub lapsed: HashMap<Uuid, GhostRule>,
}

impl GhostConfig {
    /// The rule that applies to `viewer` right now: their override, else the
    /// default. Rules past their end no longer apply even if the sweeper has
    /// not removed them yet.
    pub fn rule_for(&self, viewer: Uuid, now_ms: i64) -> Option<&GhostRule> {
        self.applied(viewer, now_ms).map(|(rule, _)| rule)
    }

    /// Works out, for every timed override that hides the owner, what
    /// applies to that friend from the moment it ends, if the default hides
    /// the owner too. The friend goes on under the default's mode, but must
    /// not be handed the default's (newer) copy of the pin or its (later)
    /// watermark: both are from a time the friend was already shut out.
    /// The sweeper stores the same thing when it gets to the expired row;
    /// this makes it true from the first millisecond.
    pub fn lapse(&mut self) {
        self.lapsed.clear();
        let Some(default) = self.everyone.as_ref().filter(|rule| rule.mode != PRECISE) else { return };
        for (viewer, rule) in &self.overrides {
            let Some(until_ms) = rule.until_ms.filter(|_| rule.mode != PRECISE) else { continue };
            let frozen = if default.mode == FROZEN && rule.mode == FROZEN { rule.frozen.clone() } else { None };
            self.lapsed.insert(
                *viewer,
                GhostRule {
                    mode: default.mode,
                    since_ms: until_ms,
                    until_ms: default.until_ms,
                    frozen,
                    hidden_since_ms: rule.hidden_since_ms,
                },
            );
        }
    }

    /// The default rule, if it still applies.
    pub fn default_rule(&self, now_ms: i64) -> Option<&GhostRule> {
        self.everyone.as_ref().filter(|rule| rule.active(now_ms))
    }

    /// Of the rules hiding the owner from someone right now, the one that
    /// has been doing so the longest, with the `viewer_id` of its row. Its
    /// watermark is the last moment every friend saw the owner precisely.
    pub fn longest_hidden(&self, now_ms: i64) -> Option<(&GhostRule, Uuid)> {
        let default = self.default_rule(now_ms).map(|rule| (rule, Uuid::nil()));
        let overrides = self.overrides.keys().filter_map(|viewer| self.applied(*viewer, now_ms));
        default
            .into_iter()
            .chain(overrides)
            .filter(|(rule, _)| rule.mode != PRECISE)
            .min_by_key(|(rule, _)| rule.hidden_since_ms)
    }

    /// The rule that applies to `viewer` and the `viewer_id` of its row (the
    /// nil UUID for the default).
    pub fn applied(&self, viewer: Uuid, now_ms: i64) -> Option<(&GhostRule, Uuid)> {
        if let Some(rule) = self.overrides.get(&viewer).filter(|rule| rule.active(now_ms)) {
            return Some((rule, viewer));
        }
        // An override that ran out under a default that hides the owner as well.
        if let Some(rule) = self.lapsed.get(&viewer).filter(|rule| rule.active(now_ms)) {
            return Some((rule, viewer));
        }
        self.default_rule(now_ms).map(|rule| (rule, Uuid::nil()))
    }
}

const PRECISE: i32 = tokuly_proto::geo::GhostMode::GHOST_MODE_PRECISE as i32;
const FROZEN: i32 = tokuly_proto::geo::GhostMode::GHOST_MODE_FROZEN as i32;

/// One of the user's own places, for matching stays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnPlace {
    pub id: Uuid,
    pub label: i32,
    pub name: String,
    pub emoji: String,
    pub lat: f64,
    pub lng: f64,
    pub radius_m: f64,
    pub visibility: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SosCtx {
    pub id: Uuid,
    pub message: String,
    pub notify: Vec<Uuid>,
    pub started_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TripCtx {
    pub id: Uuid,
    /// Set when the destination is a friend (followed live).
    pub dest_user_id: Option<Uuid>,
    pub dest_lat: f64,
    pub dest_lng: f64,
    pub arrive_radius_m: f64,
    /// `TravelMode` number.
    pub mode: i32,
    pub started_ms: i64,
    pub plan_distance_m: f64,
    pub plan_duration_s: i32,
    /// Empty = all friends.
    pub shared_with: Vec<Uuid>,
    pub notify_on_arrival: bool,
}

impl TripCtx {
    pub fn shared_with(&self, viewer: Uuid) -> bool {
        self.shared_with.is_empty() || self.shared_with.contains(&viewer)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShareCtx {
    pub token: String,
    pub expires_ms: i64,
}

/// An ACTIVE place alert watching this user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertCtx {
    pub id: Uuid,
    pub owner_id: Uuid,
    /// Fixed target; `None` for alerts on the subject's own labelled place.
    pub target: Option<(f64, f64, f64)>,
    /// `PlaceLabel` number, 0 when `target` is set.
    pub label: i32,
    /// `AlertTrigger` number.
    pub trigger: i32,
    pub recurring: bool,
    pub inside: Option<bool>,
    pub last_fired_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OwnerCtx {
    /// False for deleted, banned and unknown accounts: nothing of them is shown.
    pub active: bool,
    pub privacy: Privacy,
    /// The account's language is Japanese.
    pub ja: bool,
    /// The account's time zone (fallback when a position has none).
    pub time_zone: String,
    pub status: Option<StatusLite>,
    pub ghost: GhostConfig,
    pub places: Vec<OwnPlace>,
    pub sos: Option<SosCtx>,
    pub trip: Option<TripCtx>,
    pub shares: Vec<ShareCtx>,
    pub alerts: Vec<AlertCtx>,
}

impl OwnerCtx {
    /// Whether `viewer` was alerted by an SOS that is still active.
    pub fn sos_for(&self, viewer: Uuid) -> bool {
        self.sos.as_ref().is_some_and(|sos| sos.notify.contains(&viewer))
    }

    /// Who sees this user's precise location, in the form the shared
    /// position index keeps (so that what other domains ask it about a pair
    /// of users follows ghost mode too).
    pub fn sight(&self) -> Sight {
        if !self.active {
            // Nobody sees an account that is gone or banned.
            return Sight { everyone: Some(SightRule { precise: false, until_ms: None }), ..Default::default() };
        }
        let rule = |rule: &GhostRule| SightRule { precise: rule.mode == PRECISE, until_ms: rule.until_ms };
        Sight {
            everyone: self.ghost.everyone.as_ref().map(rule),
            overrides: self.ghost.overrides.iter().map(|(viewer, ghost)| (*viewer, rule(ghost))).collect(),
            alerted: self.sos.as_ref().map(|sos| sos.notify.clone()).unwrap_or_default(),
        }
    }

    /// The user's own place with a label (home, work, school).
    pub fn labelled_place(&self, label: i32) -> Option<&OwnPlace> {
        self.places.iter().find(|place| place.label == label)
    }
}

fn ctx_key(user_id: Uuid) -> String {
    format!("geo:ctx:{}", user_id.simple())
}

fn dirty_key(user_id: Uuid) -> String {
    format!("geo:ctx:dirty:{}", user_id.simple())
}

fn millis(time: DateTime<Utc>) -> i64 {
    time.timestamp_millis()
}

#[derive(sqlx::FromRow)]
struct UserBits {
    id: Uuid,
    privacy: serde_json::Value,
    locale: String,
    time_zone: String,
    status_emoji: String,
    status_text: String,
    status_set_at: Option<DateTime<Utc>>,
    status_expires_at: Option<DateTime<Utc>>,
    active: bool,
}

#[derive(sqlx::FromRow)]
struct GhostRow {
    owner_id: Uuid,
    viewer_id: Uuid,
    mode: i16,
    since: DateTime<Utc>,
    until: Option<DateTime<Utc>>,
    frozen: Option<serde_json::Value>,
    hidden_since: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct PlaceBits {
    id: Uuid,
    owner_id: Uuid,
    label: i16,
    name: String,
    emoji: String,
    lat: f64,
    lng: f64,
    radius_m: f64,
    visibility: i16,
}

#[derive(sqlx::FromRow)]
struct SosBits {
    id: Uuid,
    user_id: Uuid,
    message: String,
    notify_user_ids: Vec<Uuid>,
    started_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct TripBits {
    id: Uuid,
    user_id: Uuid,
    dest_user_id: Option<Uuid>,
    dest_lat: f64,
    dest_lng: f64,
    arrive_radius_m: f64,
    mode: i16,
    started_at: DateTime<Utc>,
    plan_distance_m: f64,
    plan_duration_s: i32,
    shared_with: Vec<Uuid>,
    notify_on_arrival: bool,
}

#[derive(sqlx::FromRow)]
struct AlertBits {
    id: Uuid,
    owner_id: Uuid,
    subject_id: Uuid,
    lat: Option<f64>,
    lng: Option<f64>,
    radius_m: Option<f64>,
    subject_place_label: i16,
    trigger: i16,
    recurring: bool,
    inside: Option<bool>,
    last_fired_at: Option<DateTime<Utc>>,
}

impl Geo {
    /// The context of one user.
    pub(crate) async fn ctx_of(&self, user_id: Uuid) -> AppResult<OwnerCtx> {
        Ok(self.ctx_many(&[user_id]).await?.remove(&user_id).unwrap_or_default())
    }

    /// The contexts of many users. Every requested user is in the map
    /// (unknown accounts as an inactive default).
    pub(crate) async fn ctx_many(&self, user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, OwnerCtx>> {
        let mut out = HashMap::with_capacity(user_ids.len());
        if user_ids.is_empty() {
            return Ok(out);
        }
        let cache = &self.core.cache;
        let keys: Vec<String> = user_ids.iter().map(|id| ctx_key(*id)).collect();
        let cached: Vec<Option<Vec<u8>>> = redis::cmd("MGET").arg(&keys).query_async(&mut cache.conn()).await?;
        let mut missing = Vec::new();
        for (user_id, bytes) in user_ids.iter().zip(cached) {
            match bytes.and_then(|bytes| serde_json::from_slice::<OwnerCtx>(&bytes).ok()) {
                Some(ctx) => {
                    out.insert(*user_id, ctx);
                }
                None => missing.push(*user_id),
            }
        }
        if missing.is_empty() {
            return Ok(out);
        }
        missing.sort();
        missing.dedup();
        let loaded = self.load_ctx(&missing).await?;
        let dirty_keys: Vec<String> = missing.iter().map(|id| dirty_key(*id)).collect();
        let dirty: Vec<Option<String>> = redis::cmd("MGET").arg(&dirty_keys).query_async(&mut cache.conn()).await?;
        for (user_id, dirty) in missing.iter().zip(dirty) {
            let ctx = loaded.get(user_id).cloned().unwrap_or_default();
            // The shared position index answers questions about pairs of
            // users by the same rules: it gets them whenever they are read anew.
            geoindex::set_sight(cache, *user_id, &ctx.sight()).await?;
            // A change committed while we were reading: serve what we read, but do not cache it.
            if dirty.is_none() {
                cache.set_json(&ctx_key(*user_id), &ctx, Some(CTX_TTL)).await?;
                if cache.exists(&dirty_key(*user_id)).await? {
                    cache.del(&ctx_key(*user_id)).await?;
                }
            }
            out.insert(*user_id, ctx);
        }
        Ok(out)
    }

    /// Drops cached contexts after a change to the rows they are built from.
    pub(crate) async fn invalidate_ctx(&self, user_ids: &[Uuid]) {
        for user_id in user_ids {
            let result: AppResult<()> = async {
                self.core.cache.set_string(&dirty_key(*user_id), "1", Some(DIRTY_TTL)).await?;
                self.core.cache.del(&ctx_key(*user_id)).await
            }
            .await;
            if let Err(e) = result {
                // The entry expires by itself; until then readers may see the old rules.
                tracing::warn!(%user_id, error = %e, "could not drop a cached location context");
            }
        }
        // Read them again at once: that is what hands the new rules to the
        // position index, which other domains ask without coming through here.
        if let Err(e) = self.ctx_many(user_ids).await {
            tracing::warn!(error = %e, "could not refresh location contexts after a change");
        }
    }

    async fn load_ctx(&self, user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, OwnerCtx>> {
        let db = &self.core.db;
        let mut out: HashMap<Uuid, OwnerCtx> = HashMap::with_capacity(user_ids.len());
        let users: Vec<UserBits> = sqlx::query_as(
            "SELECT id, privacy, locale, time_zone, status_emoji, status_text, status_set_at, status_expires_at,
                    (deleted_at IS NULL AND banned_at IS NULL) AS active
             FROM users WHERE id = ANY($1)",
        )
        .bind(user_ids)
        .fetch_all(db)
        .await?;
        for user in users {
            let status = (!user.status_emoji.is_empty() || !user.status_text.is_empty()).then(|| StatusLite {
                emoji: user.status_emoji,
                text: user.status_text,
                set_ms: user.status_set_at.map(millis),
                expires_ms: user.status_expires_at.map(millis),
            });
            out.insert(
                user.id,
                OwnerCtx {
                    active: user.active,
                    privacy: Privacy::from(&settings::privacy_from_json(&user.privacy)),
                    ja: user.locale.to_ascii_lowercase().starts_with("ja"),
                    time_zone: user.time_zone,
                    status,
                    ..Default::default()
                },
            );
        }

        let rules: Vec<GhostRow> = sqlx::query_as(
            "SELECT owner_id, viewer_id, mode, since, until, frozen, hidden_since FROM geo_ghost_rules WHERE owner_id = ANY($1)",
        )
        .bind(user_ids)
        .fetch_all(db)
        .await?;
        for row in rules {
            let Some(ctx) = out.get_mut(&row.owner_id) else { continue };
            let rule = GhostRule {
                mode: row.mode as i32,
                since_ms: millis(row.since),
                until_ms: row.until.map(millis),
                frozen: row.frozen.and_then(|value| serde_json::from_value(value).ok()),
                hidden_since_ms: millis(row.hidden_since),
            };
            if row.viewer_id.is_nil() {
                ctx.ghost.everyone = Some(rule);
            } else {
                ctx.ghost.overrides.insert(row.viewer_id, rule);
            }
        }
        for ctx in out.values_mut() {
            ctx.ghost.lapse();
        }

        let places: Vec<PlaceBits> = sqlx::query_as(
            "SELECT id, owner_id, label, name, emoji, lat, lng, radius_m, visibility FROM geo_places WHERE owner_id = ANY($1) ORDER BY created_at",
        )
        .bind(user_ids)
        .fetch_all(db)
        .await?;
        for place in places {
            if let Some(ctx) = out.get_mut(&place.owner_id) {
                ctx.places.push(OwnPlace {
                    id: place.id,
                    label: place.label as i32,
                    name: place.name,
                    emoji: place.emoji,
                    lat: place.lat,
                    lng: place.lng,
                    radius_m: place.radius_m,
                    visibility: place.visibility as i32,
                });
            }
        }

        let alarms: Vec<SosBits> =
            sqlx::query_as("SELECT id, user_id, message, notify_user_ids, started_at FROM geo_sos WHERE user_id = ANY($1) AND resolved_at IS NULL")
                .bind(user_ids)
                .fetch_all(db)
                .await?;
        for sos in alarms {
            if let Some(ctx) = out.get_mut(&sos.user_id) {
                ctx.sos = Some(SosCtx {
                    id: sos.id,
                    message: sos.message,
                    notify: sos.notify_user_ids,
                    started_ms: millis(sos.started_at),
                });
            }
        }

        let trips: Vec<TripBits> = sqlx::query_as(
            "SELECT id, user_id, dest_user_id, dest_lat, dest_lng, arrive_radius_m, mode, started_at, plan_distance_m, plan_duration_s,
                    shared_with, notify_on_arrival
             FROM geo_trips WHERE user_id = ANY($1) AND status = 1",
        )
        .bind(user_ids)
        .fetch_all(db)
        .await?;
        for trip in trips {
            if let Some(ctx) = out.get_mut(&trip.user_id) {
                ctx.trip = Some(TripCtx {
                    id: trip.id,
                    dest_user_id: trip.dest_user_id,
                    dest_lat: trip.dest_lat,
                    dest_lng: trip.dest_lng,
                    arrive_radius_m: trip.arrive_radius_m,
                    mode: trip.mode as i32,
                    started_ms: millis(trip.started_at),
                    plan_distance_m: trip.plan_distance_m,
                    plan_duration_s: trip.plan_duration_s,
                    shared_with: trip.shared_with,
                    notify_on_arrival: trip.notify_on_arrival,
                });
            }
        }

        let shares: Vec<(Uuid, Vec<u8>, DateTime<Utc>)> = sqlx::query_as(
            "SELECT user_id, token_sealed, expires_at FROM geo_live_shares WHERE user_id = ANY($1) AND ended_at IS NULL AND expires_at > now()",
        )
        .bind(user_ids)
        .fetch_all(db)
        .await?;
        for (user_id, sealed, expires_at) in shares {
            if let (Some(ctx), Some(token)) = (out.get_mut(&user_id), self.open_share_token(&sealed)) {
                ctx.shares.push(ShareCtx { token, expires_ms: millis(expires_at) });
            }
        }

        let alerts: Vec<AlertBits> = sqlx::query_as(
            "SELECT id, owner_id, subject_id, lat, lng, radius_m, subject_place_label, trigger, recurring, inside, last_fired_at
             FROM geo_place_alerts WHERE subject_id = ANY($1) AND status = 2",
        )
        .bind(user_ids)
        .fetch_all(db)
        .await?;
        for alert in alerts {
            if let Some(ctx) = out.get_mut(&alert.subject_id) {
                ctx.alerts.push(AlertCtx {
                    id: alert.id,
                    owner_id: alert.owner_id,
                    target: alert.lat.zip(alert.lng).zip(alert.radius_m).map(|((lat, lng), radius)| (lat, lng, radius)),
                    label: alert.subject_place_label as i32,
                    trigger: alert.trigger as i32,
                    recurring: alert.recurring,
                    inside: alert.inside,
                    last_fired_ms: alert.last_fired_at.map(millis),
                });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(mode: i32, until_ms: Option<i64>) -> GhostRule {
        GhostRule { mode, since_ms: 0, until_ms, frozen: None, hidden_since_ms: 0 }
    }

    #[test]
    fn overrides_win_over_the_default_until_they_expire() {
        let (aoi, ren) = (Uuid::now_v7(), Uuid::now_v7());
        let mut ghost = GhostConfig { everyone: Some(rule(2, None)), ..Default::default() };
        ghost.overrides.insert(aoi, rule(3, Some(1_000)));
        assert_eq!(ghost.rule_for(aoi, 500).unwrap().mode, 3);
        assert_eq!(ghost.rule_for(ren, 500).unwrap().mode, 2);
        // Past its end the override no longer applies, even before the sweeper removes it.
        assert_eq!(ghost.rule_for(aoi, 1_000).unwrap().mode, 2);
        let timed_default = GhostConfig { everyone: Some(rule(3, Some(1_000))), ..Default::default() };
        assert!(timed_default.rule_for(ren, 999).is_some());
        assert!(timed_default.rule_for(ren, 1_000).is_none());
        assert!(GhostConfig::default().rule_for(ren, 0).is_none());
    }

    #[test]
    fn expired_statuses_are_not_shown() {
        let status =
            StatusLite { emoji: "🍜".into(), text: "ラーメン".into(), set_ms: Some(0), expires_ms: Some(10_000) };
        let before = DateTime::from_timestamp_millis(9_000).unwrap();
        let after = DateTime::from_timestamp_millis(10_000).unwrap();
        assert_eq!(status.current(before).unwrap().emoji, "🍜");
        assert!(status.current(after).is_none());
        let forever = StatusLite { expires_ms: None, ..status };
        assert!(forever.current(after).is_some());
    }

    #[test]
    fn privacy_defaults_match_identity() {
        let privacy = Privacy::default();
        assert!(
            privacy.share_battery
                && privacy.share_activity
                && privacy.share_online_status
                && privacy.share_place_labels
                && privacy.record_history
        );
        assert_eq!(privacy.history_days, 0);
        assert_eq!(privacy.footprints_audience, tokuly_proto::identity::Audience::AUDIENCE_FRIENDS as i32);
    }
}
