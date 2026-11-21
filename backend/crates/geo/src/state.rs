//! A user's live state in Valkey, and the per-user lock that serializes
//! changes to it.
//!
//! [`LiveState`] is what the pipeline knows right now: the latest sample, the
//! current stay, pending distance totals, who is near. Its [`Pin`] is the part
//! other people may get to see (through [`crate::visibility`]). Everything
//! durable is derived from it and stored in PostgreSQL / ClickHouse; losing a
//! state only costs the continuity of the current stay.
use std::time::Duration;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use tokuly_core::prelude::*;

use crate::{
    Geo,
    detect::{ProximityTracker, StayTracker},
    math::Point,
    odometer::Odometer,
};

/// States of users who stopped reporting are dropped after this long.
const STATE_TTL: Duration = Duration::from_secs(60 * 24 * 3600);
/// A lock nobody released (a crashed worker) frees itself after this long.
const LOCK_TTL: Duration = Duration::from_secs(30);
/// How long to wait for another worker holding a user's lock.
const LOCK_WAIT: Duration = Duration::from_secs(8);

/// Coarse description of where a point is, in both languages.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Area {
    /// Municipality or nearest populated place, e.g. "Shibuya" / 「渋谷区」.
    pub locality: String,
    pub locality_ja: String,
    /// First-level subdivision, e.g. "Tokyo" / 「東京都」.
    pub region: String,
    pub region_ja: String,
    /// ISO 3166-1 alpha-2.
    pub country_code: String,
    /// IANA time zone.
    pub time_zone: String,
}

impl Area {
    /// The area name shown next to a pin: 「渋谷区」 or "Shibuya, Tokyo".
    pub fn label(&self, ja: bool) -> String {
        if ja {
            return if self.locality_ja.is_empty() { self.locality.clone() } else { self.locality_ja.clone() };
        }
        if self.locality.is_empty() {
            return self.region.clone();
        }
        // "Kita, Osaka" already names its region.
        if self.region.is_empty() || self.locality == self.region || self.locality.ends_with(&format!(", {}", self.region)) {
            return self.locality.clone();
        }
        format!("{}, {}", self.locality, self.region)
    }

    /// The locality alone in the viewer's language.
    pub fn locality_in(&self, ja: bool) -> &str {
        if ja && !self.locality_ja.is_empty() { &self.locality_ja } else { &self.locality }
    }
}

/// The place a stay matched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StayPlace {
    pub id: Uuid,
    pub owner_id: Uuid,
    /// `PlaceLabel` number.
    pub label: i32,
    pub name: String,
    pub emoji: String,
    /// `Visibility` number of the place.
    pub visibility: i32,
}

/// What can be shown of a user's position: the input of the visibility
/// function, and what a FROZEN ghost rule keeps a copy of.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Pin {
    pub lat: f64,
    pub lng: f64,
    pub accuracy: f64,
    /// Ground speed in m/s (reported or derived).
    pub speed: f64,
    pub heading: Option<f64>,
    /// Device time of the sample, Unix milliseconds.
    pub recorded_ms: i64,
    pub battery: Option<i32>,
    pub charging: bool,
    /// `Activity` number.
    pub activity: i32,
    /// False after `StopSharing`, until the next sample.
    pub sharing: bool,
    /// Start and center of the confirmed stay, if the user is in one.
    pub stationary_since_ms: Option<i64>,
    pub stay_center: Option<Point>,
    pub place: Option<StayPlace>,
    pub area: Area,
    /// When a sample last came from the app in the foreground.
    pub last_foreground_ms: i64,
}

impl Pin {
    pub fn position(&self) -> Point {
        (self.lat, self.lng)
    }
}

/// The regions of the user's latest footprint cell.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RegionRef {
    pub country_id: String,
    pub admin1_id: String,
}

/// What was last pushed to friends, to skip pushes that change nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pushed {
    pub at_ms: i64,
    pub lat: f64,
    pub lng: f64,
    /// Everything else a friend could notice, folded into one value.
    pub fingerprint: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LiveState {
    pub pin: Pin,
    pub altitude: Option<f64>,
    pub mocked: bool,
    /// Device time of the latest sample accurate enough for stays, distance and footprints.
    pub last_fine_ms: i64,
    pub stay: StayTracker,
    /// The ongoing visit row, when history is recorded.
    pub visit_id: Option<Uuid>,
    /// Friends already recorded on the ongoing visit.
    #[serde(default)]
    pub visit_with: Vec<Uuid>,
    /// Where `pin.area` was looked up.
    pub area_at: Option<Point>,
    #[serde(default)]
    pub region: RegionRef,
    /// The latest footprint cells (H3 indexes), newest last: cells in here need no database round trip.
    #[serde(default)]
    pub recent_cells: Vec<i64>,
    #[serde(default)]
    pub odometer: Odometer,
    #[serde(default)]
    pub near: ProximityTracker,
    /// Sample time of the last proximity check.
    #[serde(default)]
    pub proximity_checked_ms: i64,
    /// The friends this user was together with when gatherings were last synced (sorted).
    #[serde(default)]
    pub gathering_with: Vec<Uuid>,
    /// When this user's gathering membership was last confirmed in the database.
    #[serde(default)]
    pub gathering_seen_ms: i64,
    /// The latest night already recorded for the current stay.
    #[serde(default)]
    pub night: Option<NaiveDate>,
    /// Trip progress last stored and pushed: sample time and ETA.
    #[serde(default)]
    pub trip_pushed_ms: i64,
    #[serde(default)]
    pub trip_eta_ms: i64,
    #[serde(default)]
    pub pushed: Option<Pushed>,
}

impl LiveState {
    /// Whether the state holds a position at all.
    pub fn located(&self) -> bool {
        self.pin.recorded_ms > 0
    }

    /// Remembers a footprint cell as the current one. Returns false if it
    /// was already known (it just moves to the end).
    pub fn remember_cell(&mut self, cell: i64) -> bool {
        const KEPT: usize = 48;
        if let Some(index) = self.recent_cells.iter().position(|known| *known == cell) {
            self.recent_cells.remove(index);
            self.recent_cells.push(cell);
            return false;
        }
        if self.recent_cells.len() >= KEPT {
            self.recent_cells.remove(0);
        }
        self.recent_cells.push(cell);
        true
    }
}

fn state_key(user_id: Uuid) -> String {
    format!("geo:state:{}", user_id.simple())
}

fn lock_key(user_id: Uuid) -> String {
    format!("geo:lock:{}", user_id.simple())
}

/// Holds a user's pipeline lock until released (or until it times out).
pub(crate) struct UserLock {
    key: String,
    token: String,
}

const RELEASE: &str = "if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) else return 0 end";

impl Geo {
    /// The live state of a user (`None` if they never reported a location, or not for a long time).
    pub(crate) async fn load_state(&self, user_id: Uuid) -> AppResult<Option<LiveState>> {
        self.core.cache.get_json(&state_key(user_id)).await
    }

    /// The live states of many users, in the order asked.
    pub(crate) async fn load_states(&self, user_ids: &[Uuid]) -> AppResult<Vec<Option<LiveState>>> {
        if user_ids.is_empty() {
            return Ok(Vec::new());
        }
        let keys: Vec<String> = user_ids.iter().map(|id| state_key(*id)).collect();
        let raw: Vec<Option<Vec<u8>>> = redis::cmd("MGET").arg(&keys).query_async(&mut self.core.cache.conn()).await?;
        Ok(raw.into_iter().map(|bytes| bytes.and_then(|bytes| serde_json::from_slice(&bytes).ok())).collect())
    }

    pub(crate) async fn save_state(&self, user_id: Uuid, state: &LiveState) -> AppResult<()> {
        self.core.cache.set_json(&state_key(user_id), state, Some(STATE_TTL)).await
    }

    pub(crate) async fn drop_state(&self, user_id: Uuid) -> AppResult<()> {
        self.core.cache.del(&state_key(user_id)).await
    }

    /// Takes the lock that serializes everything changing one user's live
    /// state: the pipeline (stream and RPC path, in any process) and the RPCs
    /// that edit the state. Waits for a holder; gives up with `unavailable`.
    pub(crate) async fn lock_user(&self, user_id: Uuid) -> AppResult<UserLock> {
        let key = lock_key(user_id);
        let token = tokuly_core::ids::random_base62(16);
        let deadline = tokio::time::Instant::now() + LOCK_WAIT;
        let mut pause = Duration::from_millis(5);
        loop {
            if self.core.cache.set_nx(&key, &token, LOCK_TTL).await? {
                return Ok(UserLock { key, token });
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(AppError::unavailable("LOCATION_BUSY", "location updates for this account are busy; try again"));
            }
            tokio::time::sleep(pause).await;
            pause = (pause * 2).min(Duration::from_millis(80));
        }
    }

    /// Releases a lock taken with [`Self::lock_user`]. A lock that already
    /// timed out and was taken by someone else is left alone.
    pub(crate) async fn unlock_user(&self, lock: UserLock) {
        let released: Result<i64, _> =
            redis::cmd("EVAL").arg(RELEASE).arg(1).arg(&lock.key).arg(&lock.token).query_async(&mut self.core.cache.conn()).await;
        if let Err(e) = released {
            tracing::warn!(error = %e, "could not release a location lock; it expires by itself");
        }
    }

    /// Runs `work` on a user's live state under their lock and stores the
    /// state afterwards if `work` succeeded. `work` receives `None` turned
    /// into a default state when the user has none yet.
    pub(crate) async fn with_state<T, F>(&self, user_id: Uuid, work: F) -> AppResult<T>
    where
        F: AsyncFnOnce(&mut LiveState) -> AppResult<T>,
    {
        let lock = self.lock_user(user_id).await?;
        let result = async {
            let mut state = self.load_state(user_id).await?.unwrap_or_default();
            let before = state.clone();
            let out = work(&mut state).await?;
            if state != before {
                self.save_state(user_id, &state).await?;
            }
            Ok(out)
        }
        .await;
        self.unlock_user(lock).await;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_labels_follow_the_language() {
        let shibuya = Area {
            locality: "Shibuya".into(),
            locality_ja: "渋谷区".into(),
            region: "Tokyo".into(),
            region_ja: "東京都".into(),
            country_code: "JP".into(),
            time_zone: "Asia/Tokyo".into(),
        };
        assert_eq!(shibuya.label(true), "渋谷区");
        assert_eq!(shibuya.label(false), "Shibuya, Tokyo");
        let ward = Area { locality: "Kita, Osaka".into(), region: "Osaka".into(), ..Default::default() };
        assert_eq!(ward.label(false), "Kita, Osaka", "the region is not repeated");
        assert_eq!(ward.label(true), "Kita, Osaka", "falls back to the English name");
        let nowhere = Area { region: "Nunavut".into(), ..Default::default() };
        assert_eq!(nowhere.label(false), "Nunavut");
        assert_eq!(Area::default().label(true), "");
    }

    #[test]
    fn recent_cells_are_a_bounded_set() {
        let mut state = LiveState::default();
        assert!(state.remember_cell(1));
        assert!(!state.remember_cell(1));
        for cell in 2..=60 {
            assert!(state.remember_cell(cell));
        }
        assert_eq!(state.recent_cells.len(), 48);
        assert!(state.remember_cell(1), "the oldest cells were forgotten");
        assert!(!state.remember_cell(40));
        assert_eq!(state.recent_cells.last(), Some(&40), "a known cell becomes the current one");
        assert_eq!(state.recent_cells.len(), 48);
    }

    #[test]
    fn states_survive_serialization_and_tolerate_missing_fields() {
        let mut state = LiveState::default();
        state.pin.lat = 35.0;
        state.pin.recorded_ms = 1;
        state.night = NaiveDate::from_ymd_opt(2026, 11, 20);
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(serde_json::from_value::<LiveState>(json).unwrap(), state);
        // A state written by an older version without the newer fields.
        let old = serde_json::json!({ "pin": state.pin, "altitude": null, "mocked": false, "last_fine_ms": 0, "stay": {}, "visit_id": null, "area_at": null });
        let loaded: LiveState = serde_json::from_value(old).unwrap();
        assert!(loaded.located() && loaded.recent_cells.is_empty());
    }
}
