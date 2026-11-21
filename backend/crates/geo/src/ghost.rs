//! Ghost mode settings: the default for everyone and per-friend overrides,
//! optionally timed. What the rules *mean* is in [`crate::visibility`]; this
//! module stores them and takes the frozen copy of the pin when a FROZEN rule
//! begins.
use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use tokuly_core::prelude::*;
use tokuly_proto::geo::{GetGhostSettingsResponse, GhostMode, GhostSetting};
use tokuly_social::graph;

use crate::{
    Geo,
    ctx::{FrozenPin, OwnerCtx},
    state::Pin,
    visibility,
};

const PRECISE: i32 = GhostMode::GHOST_MODE_PRECISE as i32;
const BLURRED: i32 = GhostMode::GHOST_MODE_BLURRED as i32;
const FROZEN: i32 = GhostMode::GHOST_MODE_FROZEN as i32;
/// Longest a timed rule may last.
const MAX_DURATION_SECONDS: i32 = 30 * 24 * 3600;
const MAX_OVERRIDES_PER_CALL: usize = 200;

/// The copy of the pin a new FROZEN rule for `viewer` (`None` = the default
/// for everyone) must keep: what that audience sees right now.
fn frozen_copy(ctx: &OwnerCtx, viewer: Option<Uuid>, pin: Option<&Pin>, now: DateTime<Utc>) -> Option<FrozenPin> {
    let now_ms = now.timestamp_millis();
    let current = match viewer {
        Some(viewer) => ctx.ghost.rule_for(viewer, now_ms),
        None => ctx.ghost.default_rule(now_ms),
    };
    match current {
        // Already frozen (by this rule or, for an override, by the default):
        // the viewer keeps the pin they have. Freezing again must not refresh it.
        Some(rule) if rule.mode == FROZEN => rule.frozen.clone(),
        Some(rule) if rule.mode == BLURRED => pin.map(|pin| FrozenPin { pin: pin.clone(), base: BLURRED }),
        _ => pin.map(|pin| FrozenPin { pin: pin.clone(), base: PRECISE }),
    }
}

fn setting(mode: i32, since: Option<DateTime<Utc>>, until: Option<DateTime<Utc>>) -> GhostSetting {
    GhostSetting { mode: mode.into(), since: opt_ts(since), until: opt_ts(until), ..Default::default() }
}

impl Geo {
    /// The caller's ghost settings as stored (rules past their end left out).
    pub(crate) async fn ghost_settings(&self, owner: Uuid) -> AppResult<GetGhostSettingsResponse> {
        let rows: Vec<(Uuid, i16, DateTime<Utc>, Option<DateTime<Utc>>)> =
            sqlx::query_as("SELECT viewer_id, mode, since, until FROM geo_ghost_rules WHERE owner_id = $1 AND (until IS NULL OR until > now())")
                .bind(owner)
                .fetch_all(&self.core.db)
                .await?;
        let mut everyone = setting(PRECISE, None, None);
        let mut overrides = HashMap::new();
        for (viewer_id, mode, since, until) in rows {
            let rule = setting(mode as i32, Some(since), until);
            if viewer_id.is_nil() {
                everyone = rule;
            } else {
                overrides.insert(viewer_id.to_string(), rule);
            }
        }
        Ok(GetGhostSettingsResponse { everyone: everyone.into(), overrides: overrides.into_iter().collect(), ..Default::default() })
    }

    /// Sets the default ghost mode (`viewers` empty) or overrides for some friends.
    pub(crate) async fn set_ghost_mode(&self, owner: Uuid, viewers: &[String], mode: buffa::EnumValue<GhostMode>, duration_seconds: i32) -> AppResult<()> {
        let mode = match mode.as_known() {
            Some(GhostMode::GHOST_MODE_PRECISE) => PRECISE,
            Some(GhostMode::GHOST_MODE_BLURRED) => BLURRED,
            Some(GhostMode::GHOST_MODE_FROZEN) => FROZEN,
            _ => bail!(invalid, "INVALID_GHOST_MODE", "ghost mode must be precise, blurred or frozen"),
        };
        ensure!((0..=MAX_DURATION_SECONDS).contains(&duration_seconds), invalid, "INVALID_DURATION", "a ghost mode lasts at most 30 days");
        ensure!(viewers.len() <= MAX_OVERRIDES_PER_CALL, invalid, "TOO_MANY_USERS", "at most {MAX_OVERRIDES_PER_CALL} friends per call");
        let mut viewers = parse_ids(viewers)?;
        viewers.sort();
        viewers.dedup();
        if !viewers.is_empty() {
            let friends = graph::friend_ids(&self.core, owner).await?;
            ensure!(viewers.iter().all(|viewer| friends.contains(viewer)), precondition, "NOT_FRIENDS", "ghost mode can only be set for friends");
        }

        // Under the owner's pipeline lock: the frozen copy must be exactly the
        // pin friends were last shown, not one a concurrent sample replaces.
        let lock = self.lock_user(owner).await?;
        let stored: AppResult<()> = async {
            let now = Utc::now();
            let until = (duration_seconds > 0).then(|| now + Duration::seconds(duration_seconds as i64));
            let ctx = self.ctx_of(owner).await?;
            let state = self.load_state(owner).await?;
            let pin = state.as_ref().filter(|state| state.located()).map(|state| &state.pin);
            let frozen_json = |frozen: Option<FrozenPin>| frozen.and_then(|frozen| serde_json::to_value(frozen).ok());
            const UPSERT: &str = "INSERT INTO geo_ghost_rules (owner_id, viewer_id, mode, since, until, frozen) VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (owner_id, viewer_id) DO UPDATE SET mode = EXCLUDED.mode, since = EXCLUDED.since, until = EXCLUDED.until, frozen = EXCLUDED.frozen";
            let mut tx = self.core.db.begin().await?;
            if viewers.is_empty() {
                if mode == PRECISE {
                    // Back to precise for everyone: exceptions saying "precise" say nothing any more.
                    sqlx::query("DELETE FROM geo_ghost_rules WHERE owner_id = $1 AND (viewer_id = $2 OR mode = $3)")
                        .bind(owner)
                        .bind(Uuid::nil())
                        .bind(PRECISE as i16)
                        .execute(&mut *tx)
                        .await?;
                } else {
                    let frozen = if mode == FROZEN { frozen_copy(&ctx, None, pin, now) } else { None };
                    sqlx::query(UPSERT).bind(owner).bind(Uuid::nil()).bind(mode as i16).bind(now).bind(until).bind(frozen_json(frozen)).execute(&mut *tx).await?;
                }
            } else {
                let strict_default = ctx.ghost.default_rule(now.timestamp_millis()).is_some_and(|rule| rule.mode != PRECISE);
                for viewer in &viewers {
                    if mode == PRECISE && !strict_default {
                        sqlx::query("DELETE FROM geo_ghost_rules WHERE owner_id = $1 AND viewer_id = $2").bind(owner).bind(viewer).execute(&mut *tx).await?;
                    } else {
                        let frozen = if mode == FROZEN { frozen_copy(&ctx, Some(*viewer), pin, now) } else { None };
                        sqlx::query(UPSERT).bind(owner).bind(viewer).bind(mode as i16).bind(now).bind(until).bind(frozen_json(frozen)).execute(&mut *tx).await?;
                    }
                }
            }
            tx.commit().await?;
            Ok(())
        }
        .await;
        self.invalidate_ctx(&[owner]).await;
        self.unlock_user(lock).await;
        stored?;

        // Show the people affected their new view at once — except when
        // freezing: a frozen pin must not so much as twitch.
        if mode != FROZEN {
            if viewers.is_empty() {
                self.refresh_friends(owner).await?;
            } else {
                self.refresh_viewers(owner, &viewers).await?;
            }
        }
        Ok(())
    }

    /// Removes ghost rules whose time is up and shows the viewers concerned
    /// the pin they see now. Returns how many rules ended.
    pub async fn expire_ghost_rules(&self) -> AppResult<u64> {
        let ended: Vec<(Uuid, Uuid)> =
            sqlx::query_as("DELETE FROM geo_ghost_rules WHERE until <= now() RETURNING owner_id, viewer_id").fetch_all(&self.core.db).await?;
        let mut by_owner: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for (owner, viewer) in &ended {
            by_owner.entry(*owner).or_default().push(*viewer);
        }
        for (owner, viewers) in by_owner {
            let default_ended = viewers.iter().any(Uuid::is_nil);
            if default_ended {
                // With the default back to precise, "precise" exceptions are redundant.
                sqlx::query(
                    "DELETE FROM geo_ghost_rules WHERE owner_id = $1 AND mode = $2
                     AND NOT EXISTS (SELECT 1 FROM geo_ghost_rules d WHERE d.owner_id = $1 AND d.viewer_id = $3)",
                )
                .bind(owner)
                .bind(PRECISE as i16)
                .bind(Uuid::nil())
                .execute(&self.core.db)
                .await?;
            }
            self.invalidate_ctx(&[owner]).await;
            let refreshed = if default_ended { self.refresh_friends(owner).await } else { self.refresh_viewers(owner, &viewers).await };
            if let Err(e) = refreshed {
                tracing::warn!(%owner, error = %e, "could not refresh pins after a ghost rule ended");
            }
        }
        Ok(ended.len() as u64)
    }

    /// Forgets the overrides two users had for each other (they are no longer friends).
    pub(crate) async fn drop_ghost_overrides(&self, a: Uuid, b: Uuid) -> AppResult<()> {
        sqlx::query("DELETE FROM geo_ghost_rules WHERE (owner_id = $1 AND viewer_id = $2) OR (owner_id = $2 AND viewer_id = $1)")
            .bind(a)
            .bind(b)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// The `GhostMode` number `viewer` is under for `owner` right now.
    pub(crate) async fn ghost_mode_between(&self, owner: Uuid, viewer: Uuid) -> AppResult<i32> {
        Ok(visibility::mode_for(&self.ctx_of(owner).await?, viewer, Utc::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::{GhostConfig, GhostRule};

    fn pin(lat: f64) -> Pin {
        Pin { lat, lng: 139.7, recorded_ms: 1, sharing: true, ..Default::default() }
    }

    fn rule(mode: i32, frozen: Option<FrozenPin>) -> GhostRule {
        GhostRule { mode, since_ms: 0, until_ms: None, frozen }
    }

    fn ctx(ghost: GhostConfig) -> OwnerCtx {
        OwnerCtx { active: true, ghost, ..Default::default() }
    }

    #[test]
    fn freezing_from_precise_or_blurred_copies_the_live_pin() {
        let now = Utc::now();
        let live = pin(35.0);
        let copy = frozen_copy(&ctx(GhostConfig::default()), None, Some(&live), now).unwrap();
        assert_eq!((copy.pin.lat, copy.base), (35.0, PRECISE));
        let blurred = ctx(GhostConfig { everyone: Some(rule(BLURRED, None)), ..Default::default() });
        assert_eq!(frozen_copy(&blurred, None, Some(&live), now).unwrap().base, BLURRED);
        // A friend with their own override is frozen from what the override showed.
        let viewer = Uuid::now_v7();
        let mut mixed = GhostConfig { everyone: Some(rule(BLURRED, None)), ..Default::default() };
        mixed.overrides.insert(viewer, rule(PRECISE, None));
        assert_eq!(frozen_copy(&ctx(mixed), Some(viewer), Some(&live), now).unwrap().base, PRECISE);
    }

    #[test]
    fn freezing_again_keeps_the_first_copy() {
        let now = Utc::now();
        let original = FrozenPin { pin: pin(35.0), base: PRECISE };
        let frozen = ctx(GhostConfig { everyone: Some(rule(FROZEN, Some(original.clone()))), ..Default::default() });
        let moved = pin(36.0);
        assert_eq!(frozen_copy(&frozen, None, Some(&moved), now), Some(original.clone()));
        // An override on top of a frozen default starts from the default's copy too.
        assert_eq!(frozen_copy(&frozen, Some(Uuid::now_v7()), Some(&moved), now), Some(original));
    }

    #[test]
    fn freezing_without_a_position_keeps_nothing() {
        assert_eq!(frozen_copy(&ctx(GhostConfig::default()), None, None, Utc::now()), None);
    }

    #[test]
    fn an_expired_frozen_rule_does_not_carry_over() {
        let now = Utc::now();
        let stale = FrozenPin { pin: pin(35.0), base: PRECISE };
        let expired = GhostRule { mode: FROZEN, since_ms: 0, until_ms: Some(now.timestamp_millis() - 1), frozen: Some(stale) };
        let config = ctx(GhostConfig { everyone: Some(expired), ..Default::default() });
        let fresh = frozen_copy(&config, None, Some(&pin(36.0)), now).unwrap();
        assert_eq!(fresh.pin.lat, 36.0);
    }
}
