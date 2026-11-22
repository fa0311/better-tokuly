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

/// What one new rule is built from.
struct NewRule {
    /// The friend it is for; `None` = the default for everyone.
    viewer: Option<Uuid>,
    mode: i32,
    until: Option<DateTime<Utc>>,
    /// The viewer saw everything up to now (an SOS just ended): the rule
    /// starts from the present instead of continuing what applied before.
    fresh: bool,
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
        Ok(GetGhostSettingsResponse {
            everyone: everyone.into(),
            overrides: overrides.into_iter().collect(),
            ..Default::default()
        })
    }

    /// Stores a non-precise rule. The viewers it applies to keep what they
    /// have: a rule that follows another non-precise one inherits its
    /// watermark and footprint snapshot (and, for FROZEN, its copy of the
    /// pin), so replacing one ghost mode by another never shows anything new.
    async fn store_hidden_rule(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        owner: Uuid,
        ctx: &OwnerCtx,
        pin: Option<&Pin>,
        rule: &NewRule,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let now_ms = now.timestamp_millis();
        let row_viewer = rule.viewer.unwrap_or(Uuid::nil());
        // What these viewers are under right now, and the row it is stored in.
        let before = match rule.viewer {
            Some(viewer) => ctx.ghost.applied(viewer, now_ms),
            None => ctx.ghost.default_rule(now_ms).map(|rule| (rule, Uuid::nil())),
        }
        .filter(|(before, _)| before.mode != PRECISE && !rule.fresh);
        let frozen = match (rule.mode == FROZEN, rule.fresh) {
            (false, _) => None,
            (true, true) => pin.map(|pin| FrozenPin { pin: pin.clone(), base: PRECISE }),
            (true, false) => frozen_copy(ctx, rule.viewer, pin, now),
        };
        let frozen = frozen.and_then(|frozen| serde_json::to_value(frozen).ok());
        match before {
            Some((before, inherited_row)) => {
                sqlx::query(
                    "INSERT INTO geo_ghost_rules (owner_id, viewer_id, mode, since, until, frozen, hidden_since, footprint)
                     SELECT $1, $2, $3, $4, $5, $6, $7, (SELECT footprint FROM geo_ghost_rules WHERE owner_id = $1 AND viewer_id = $8)
                     ON CONFLICT (owner_id, viewer_id) DO UPDATE SET mode = EXCLUDED.mode, since = EXCLUDED.since, until = EXCLUDED.until,
                         frozen = EXCLUDED.frozen, hidden_since = EXCLUDED.hidden_since, footprint = EXCLUDED.footprint",
                )
                .bind(owner)
                .bind(row_viewer)
                .bind(rule.mode as i16)
                .bind(now)
                .bind(rule.until)
                .bind(frozen)
                .bind(DateTime::from_timestamp_millis(before.hidden_since_ms).unwrap_or(now))
                .bind(inherited_row)
                .execute(&mut **tx)
                .await?;
            }
            None => {
                let footprint = self.footprint_snapshot(tx, owner, None).await?;
                sqlx::query(
                    "INSERT INTO geo_ghost_rules (owner_id, viewer_id, mode, since, until, frozen, hidden_since, footprint)
                     VALUES ($1, $2, $3, $4, $5, $6, $4, $7)
                     ON CONFLICT (owner_id, viewer_id) DO UPDATE SET mode = EXCLUDED.mode, since = EXCLUDED.since, until = EXCLUDED.until,
                         frozen = EXCLUDED.frozen, hidden_since = EXCLUDED.hidden_since, footprint = EXCLUDED.footprint",
                )
                .bind(owner)
                .bind(row_viewer)
                .bind(rule.mode as i16)
                .bind(now)
                .bind(rule.until)
                .bind(frozen)
                .bind(sqlx::types::Json(footprint))
                .execute(&mut **tx)
                .await?;
            }
        }
        Ok(())
    }

    /// Sets the default ghost mode (`viewers` empty) or overrides for some friends.
    pub(crate) async fn set_ghost_mode(
        &self,
        owner: Uuid,
        viewers: &[String],
        mode: buffa::EnumValue<GhostMode>,
        duration_seconds: i32,
    ) -> AppResult<()> {
        let mode = match mode.as_known() {
            Some(GhostMode::GHOST_MODE_PRECISE) => PRECISE,
            Some(GhostMode::GHOST_MODE_BLURRED) => BLURRED,
            Some(GhostMode::GHOST_MODE_FROZEN) => FROZEN,
            _ => bail!(invalid, "INVALID_GHOST_MODE", "ghost mode must be precise, blurred or frozen"),
        };
        ensure!(
            (0..=MAX_DURATION_SECONDS).contains(&duration_seconds),
            invalid,
            "INVALID_DURATION",
            "a ghost mode lasts at most 30 days"
        );
        ensure!(
            viewers.len() <= MAX_OVERRIDES_PER_CALL,
            invalid,
            "TOO_MANY_USERS",
            "at most {MAX_OVERRIDES_PER_CALL} friends per call"
        );
        let mut viewers = parse_ids(viewers)?;
        viewers.sort();
        viewers.dedup();
        if !viewers.is_empty() {
            let friends = graph::friend_ids(&self.core, owner).await?;
            ensure!(
                viewers.iter().all(|viewer| friends.contains(viewer)),
                precondition,
                "NOT_FRIENDS",
                "ghost mode can only be set for friends"
            );
        }

        // Under the owner's pipeline lock: the frozen copy must be exactly the
        // pin friends were last shown, not one a concurrent sample replaces,
        // and the footprint snapshot exactly the totals of that moment.
        let lock = self.lock_user(owner).await?;
        let stored: AppResult<()> = async {
            let now = Utc::now();
            let until = (duration_seconds > 0).then(|| now + Duration::seconds(duration_seconds as i64));
            let ctx = self.ctx_of(owner).await?;
            let state = self.load_state(owner).await?;
            let pin = state.as_ref().filter(|state| state.located()).map(|state| &state.pin);
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
                    self.store_hidden_rule(&mut tx, owner, &ctx, pin, &NewRule { viewer: None, mode, until, fresh: false }, now).await?;
                }
            } else {
                let strict_default = ctx.ghost.default_rule(now.timestamp_millis()).is_some_and(|rule| rule.mode != PRECISE);
                for viewer in &viewers {
                    if mode != PRECISE {
                        self.store_hidden_rule(&mut tx, owner, &ctx, pin, &NewRule { viewer: Some(*viewer), mode, until, fresh: false }, now).await?;
                    } else if strict_default {
                        // An exception to a stricter default.
                        sqlx::query(
                            "INSERT INTO geo_ghost_rules (owner_id, viewer_id, mode, since, until, hidden_since) VALUES ($1, $2, $3, $4, $5, $4)
                             ON CONFLICT (owner_id, viewer_id) DO UPDATE SET mode = EXCLUDED.mode, since = EXCLUDED.since, until = EXCLUDED.until,
                                 frozen = NULL, hidden_since = EXCLUDED.hidden_since, footprint = NULL",
                        )
                        .bind(owner)
                        .bind(viewer)
                        .bind(PRECISE as i16)
                        .bind(now)
                        .bind(until)
                        .execute(&mut *tx)
                        .await?;
                    } else {
                        sqlx::query("DELETE FROM geo_ghost_rules WHERE owner_id = $1 AND viewer_id = $2").bind(owner).bind(viewer).execute(&mut *tx).await?;
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

        // Who sees the owner precisely changed: what they share with people
        // who no longer do ends now, whatever the owner does next.
        self.sight_changed(owner).await?;
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

    /// An SOS ended. The people it alerted saw the owner's live pin until
    /// now; those who are under a ghost rule go back under it from here:
    /// a frozen pin stays where they last saw it (not where it was before
    /// the SOS, which would send it back in time), and what they were shown
    /// meanwhile stays shown.
    pub(crate) async fn settle_after_sos(&self, owner: Uuid, alerted: &[Uuid]) -> AppResult<()> {
        let lock = self.lock_user(owner).await?;
        let stored: AppResult<()> = async {
            let now = Utc::now();
            let ctx = self.ctx_of(owner).await?;
            let state = self.load_state(owner).await?;
            let pin = state.as_ref().filter(|state| state.located()).map(|state| &state.pin);
            let mut tx = self.core.db.begin().await?;
            for viewer in alerted {
                let Some((rule, _)) =
                    ctx.ghost.applied(*viewer, now.timestamp_millis()).filter(|(rule, _)| rule.mode != PRECISE)
                else {
                    continue;
                };
                let until = rule.until_ms.and_then(DateTime::from_timestamp_millis);
                let new = NewRule { viewer: Some(*viewer), mode: rule.mode, until, fresh: true };
                self.store_hidden_rule(&mut tx, owner, &ctx, pin, &new, now).await?;
            }
            tx.commit().await?;
            Ok(())
        }
        .await;
        self.invalidate_ctx(&[owner]).await;
        self.unlock_user(lock).await;
        stored
    }

    /// Removes ghost rules whose time is up and shows the viewers concerned
    /// the pin they see now. Returns how many rules ended.
    pub async fn expire_ghost_rules(&self) -> AppResult<u64> {
        let due: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT owner_id, viewer_id FROM geo_ghost_rules WHERE until <= now() ORDER BY owner_id LIMIT 500",
        )
        .fetch_all(&self.core.db)
        .await?;
        let mut by_owner: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for (owner, viewer) in &due {
            by_owner.entry(*owner).or_default().push(*viewer);
        }
        let mut ended = 0;
        for (owner, viewers) in by_owner {
            match self.end_ghost_rules(owner, &viewers).await {
                Ok(count) => ended += count,
                Err(e) => tracing::warn!(%owner, error = %e, "could not end ghost rules; will retry"),
            }
        }
        Ok(ended)
    }

    /// Ends the rules of `owner` for `viewers` (the nil UUID = the default)
    /// whose time is up.
    async fn end_ghost_rules(&self, owner: Uuid, viewers: &[Uuid]) -> AppResult<u64> {
        let default_ended = viewers.iter().any(Uuid::is_nil);
        let lock = self.lock_user(owner).await?;
        let stored: AppResult<u64> = async {
            let now = Utc::now();
            let ctx = self.ctx_of(owner).await?;
            let state = self.load_state(owner).await?;
            let pin = state.as_ref().filter(|state| state.located()).map(|state| &state.pin);
            let default = ctx.ghost.default_rule(now.timestamp_millis()).filter(|rule| rule.mode != PRECISE);
            let mut tx = self.core.db.begin().await?;
            let mut ended = 0;
            for viewer in viewers {
                let expired = ctx.ghost.overrides.get(viewer).filter(|rule| rule.mode != PRECISE);
                match (viewer.is_nil(), expired, default) {
                    // A timed override ends under a stricter default: the friend
                    // falls under the default's mode, but stays where the
                    // override left them. Its watermark is the older one, and
                    // nothing from in between may come to light.
                    (false, Some(expired), Some(default)) => {
                        let frozen = match (default.mode == FROZEN, &expired.frozen) {
                            (false, _) => None,
                            (true, Some(copy)) if expired.mode == FROZEN => Some(copy.clone()),
                            (true, _) => pin.map(|pin| FrozenPin { pin: pin.clone(), base: BLURRED }),
                        };
                        ended += sqlx::query(
                            "UPDATE geo_ghost_rules SET mode = $3, since = $4, until = $5, frozen = $6 WHERE owner_id = $1 AND viewer_id = $2 AND until <= now()",
                        )
                        .bind(owner)
                        .bind(viewer)
                        .bind(default.mode as i16)
                        .bind(now)
                        .bind(default.until_ms.and_then(DateTime::from_timestamp_millis))
                        .bind(frozen.and_then(|frozen| serde_json::to_value(frozen).ok()))
                        .execute(&mut *tx)
                        .await?
                        .rows_affected();
                    }
                    _ => {
                        ended += sqlx::query("DELETE FROM geo_ghost_rules WHERE owner_id = $1 AND viewer_id = $2 AND until <= now()")
                            .bind(owner)
                            .bind(viewer)
                            .execute(&mut *tx)
                            .await?
                            .rows_affected();
                    }
                }
            }
            if default_ended {
                // With the default back to precise, "precise" exceptions are redundant.
                sqlx::query(
                    "DELETE FROM geo_ghost_rules WHERE owner_id = $1 AND mode = $2
                     AND NOT EXISTS (SELECT 1 FROM geo_ghost_rules d WHERE d.owner_id = $1 AND d.viewer_id = $3)",
                )
                .bind(owner)
                .bind(PRECISE as i16)
                .bind(Uuid::nil())
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            Ok(ended)
        }
        .await;
        self.invalidate_ctx(&[owner]).await;
        self.unlock_user(lock).await;
        let ended = stored?;

        self.sight_changed(owner).await?;
        let refreshed =
            if default_ended { self.refresh_friends(owner).await } else { self.refresh_viewers(owner, viewers).await };
        if let Err(e) = refreshed {
            tracing::warn!(%owner, error = %e, "could not refresh pins after a ghost rule ended");
        }
        Ok(ended)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::{GhostConfig, GhostRule};

    fn pin(lat: f64) -> Pin {
        Pin { lat, lng: 139.7, recorded_ms: 1, sharing: true, ..Default::default() }
    }

    fn rule(mode: i32, frozen: Option<FrozenPin>) -> GhostRule {
        GhostRule { mode, since_ms: 0, until_ms: None, frozen, hidden_since_ms: 0 }
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
        let expired = GhostRule {
            mode: FROZEN,
            since_ms: 0,
            until_ms: Some(now.timestamp_millis() - 1),
            frozen: Some(stale),
            hidden_since_ms: 0,
        };
        let config = ctx(GhostConfig { everyone: Some(expired), ..Default::default() });
        let fresh = frozen_copy(&config, None, Some(&pin(36.0)), now).unwrap();
        assert_eq!(fresh.pin.lat, 36.0);
    }
}
