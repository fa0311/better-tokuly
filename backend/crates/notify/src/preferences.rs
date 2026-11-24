//! Notification preferences: category toggles, the global push pause, quiet
//! hours and per-person mutes.
//!
//! Nothing is stored for a user until they change something: a missing row
//! means "everything on".
use chrono::Timelike as _;
use tokuly_core::prelude::*;
use tokuly_proto::notify::{CategoryPreference, Preferences, PushPause, QuietHours};

use crate::categories::{self, CATEGORIES};

pub const MINUTES_PER_DAY: i32 = 24 * 60;
/// Time zone assumed when an account's zone cannot be parsed (it is the
/// column's default too).
const FALLBACK_TIME_ZONE: chrono_tz::Tz = chrono_tz::Asia::Tokyo;

/// What a user asked for regarding one notification: read by the event
/// consumer in a single query (see [`delivery_context`]).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DeliveryContext {
    pub locale: String,
    pub time_zone: String,
    /// The category's inbox and push toggles.
    pub inbox: bool,
    pub push: bool,
    pub push_paused: bool,
    pub push_paused_until: Option<Time>,
    pub quiet_enabled: bool,
    pub quiet_start_minute: i16,
    pub quiet_end_minute: i16,
    /// The recipient muted the person who caused the notification.
    pub muted: bool,
    /// The recipient has at least one push registration.
    pub has_devices: bool,
}

impl DeliveryContext {
    pub fn push_paused_at(&self, now: Time) -> bool {
        is_paused(self.push_paused, self.push_paused_until, now)
    }

    pub fn quiet_at(&self, now: Time) -> bool {
        self.quiet_enabled && in_quiet_hours(&self.time_zone, self.quiet_start_minute, self.quiet_end_minute, now)
    }
}

/// Loads the recipient's preferences for a notification of `category` caused
/// by `actor_id`. `None` when the recipient does not exist (anymore).
pub async fn delivery_context(
    core: &Core,
    user_id: Uuid,
    category: &str,
    actor_id: Option<Uuid>,
) -> AppResult<Option<DeliveryContext>> {
    Ok(sqlx::query_as(
        "SELECT u.locale, u.time_zone,
                COALESCE(c.inbox, true) AS inbox,
                COALESCE(c.push, true) AS push,
                COALESCE(s.push_paused, false) AS push_paused,
                s.push_paused_until,
                COALESCE(s.quiet_enabled, false) AS quiet_enabled,
                COALESCE(s.quiet_start_minute, 0::smallint) AS quiet_start_minute,
                COALESCE(s.quiet_end_minute, 0::smallint) AS quiet_end_minute,
                EXISTS (SELECT 1 FROM notify_mutes m WHERE m.user_id = u.id AND m.muted_user_id = $3 AND m.muted) AS muted,
                EXISTS (SELECT 1 FROM push_devices d WHERE d.user_id = u.id) AS has_devices
         FROM users u
         LEFT JOIN notify_settings s ON s.user_id = u.id
         LEFT JOIN notify_category_prefs c ON c.user_id = u.id AND c.category = $2
         WHERE u.id = $1 AND u.deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(category)
    .bind(actor_id)
    .fetch_optional(&core.db)
    .await?)
}

/// Whether a push pause is in effect. A pause without an end lasts until the
/// user resumes; one with an end simply stops applying.
pub fn is_paused(paused: bool, until: Option<Time>, now: Time) -> bool {
    paused && until.is_none_or(|until| until > now)
}

/// Whether `now` falls inside the daily window `[start, end)` given in
/// minutes after midnight in `time_zone`.
pub fn in_quiet_hours(time_zone: &str, start_minute: i16, end_minute: i16, now: Time) -> bool {
    let zone: chrono_tz::Tz = time_zone.parse().unwrap_or(FALLBACK_TIME_ZONE);
    let local = now.with_timezone(&zone);
    window_contains(start_minute as i32, end_minute as i32, (local.hour() * 60 + local.minute()) as i32)
}

/// `[start, end)` on a 24-hour clock; when `end` is not after `start` the
/// window wraps past midnight (22:00-07:00).
pub fn window_contains(start: i32, end: i32, minute: i32) -> bool {
    if start < end { (start..end).contains(&minute) } else { minute >= start || minute < end }
}

/// Validates quiet hours sent by a client.
pub fn validate_quiet_hours(quiet: &QuietHours) -> AppResult<()> {
    let in_day = |minute: i32| (0..MINUTES_PER_DAY).contains(&minute);
    ensure!(
        in_day(quiet.start_minute) && in_day(quiet.end_minute),
        invalid,
        "QUIET_HOURS_INVALID",
        "quiet hours are minutes after midnight (0-1439)"
    );
    // An empty window would silently do nothing; a whole day is what the pause is for.
    ensure!(
        quiet.start_minute != quiet.end_minute,
        invalid,
        "QUIET_HOURS_INVALID",
        "quiet hours must start and end at different times"
    );
    Ok(())
}

#[derive(sqlx::FromRow)]
struct SettingsRow {
    locale: String,
    time_zone: String,
    push_paused: bool,
    push_paused_until: Option<Time>,
    quiet_enabled: bool,
    quiet_start_minute: i16,
    quiet_end_minute: i16,
}

/// The caller's preferences as the API presents them: every category (with
/// defaults filled in and names in the caller's language), quiet hours and
/// the pause state.
pub async fn load(core: &Core, user_id: Uuid) -> AppResult<Preferences> {
    // The defaults mirror the column defaults of `notify_settings`.
    let settings: SettingsRow = sqlx::query_as(
        "SELECT u.locale, u.time_zone,
                COALESCE(s.push_paused, false) AS push_paused,
                s.push_paused_until,
                COALESCE(s.quiet_enabled, false) AS quiet_enabled,
                COALESCE(s.quiet_start_minute, 1320::smallint) AS quiet_start_minute,
                COALESCE(s.quiet_end_minute, 420::smallint) AS quiet_end_minute
         FROM users u LEFT JOIN notify_settings s ON s.user_id = u.id
         WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_one(&core.db)
    .await?;
    let toggles: Vec<(String, bool, bool)> =
        sqlx::query_as("SELECT category, inbox, push FROM notify_category_prefs WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(&core.db)
            .await?;

    let now = now();
    let categories = CATEGORIES
        .iter()
        .map(|category| {
            let (inbox, push) = toggles
                .iter()
                .find(|(id, ..)| id == category.id)
                .map(|(_, inbox, push)| (*inbox, *push))
                .unwrap_or((true, true));
            CategoryPreference {
                category: category.id.to_owned(),
                label: category.label().pick(&settings.locale).to_owned(),
                description: category.description().pick(&settings.locale).to_owned(),
                inbox: inbox || category.inbox_locked,
                push,
                inbox_locked: category.inbox_locked,
                ..Default::default()
            }
        })
        .collect();
    let paused = is_paused(settings.push_paused, settings.push_paused_until, now);
    Ok(Preferences {
        categories,
        quiet_hours: QuietHours {
            enabled: settings.quiet_enabled,
            start_minute: settings.quiet_start_minute as i32,
            end_minute: settings.quiet_end_minute as i32,
            ..Default::default()
        }
        .into(),
        push_pause: PushPause {
            paused,
            until: opt_ts(settings.push_paused_until.filter(|_| paused)),
            ..Default::default()
        }
        .into(),
        quiet_now: settings.quiet_enabled
            && in_quiet_hours(&settings.time_zone, settings.quiet_start_minute, settings.quiet_end_minute, now),
        time_zone: settings.time_zone,
        ..Default::default()
    })
}

/// The category a client named, or `CATEGORY_UNKNOWN`.
pub fn require_category(id: &str) -> AppResult<&'static categories::Category> {
    match categories::find(id) {
        Some(category) => Ok(category),
        None => bail!(invalid, "CATEGORY_UNKNOWN", "unknown notification category {id:?}"),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    fn utc(hour: u32, minute: u32) -> Time {
        chrono::Utc.with_ymd_and_hms(2026, 11, 20, hour, minute, 0).unwrap()
    }

    #[test]
    fn windows_within_a_day_and_across_midnight() {
        // 09:00-17:00
        assert!(window_contains(540, 1020, 540));
        assert!(window_contains(540, 1020, 1019));
        assert!(!window_contains(540, 1020, 1020), "the end is exclusive");
        assert!(!window_contains(540, 1020, 0));
        // 22:00-07:00
        assert!(window_contains(1320, 420, 1320));
        assert!(window_contains(1320, 420, 1439));
        assert!(window_contains(1320, 420, 0));
        assert!(window_contains(1320, 420, 419));
        assert!(!window_contains(1320, 420, 420));
        assert!(!window_contains(1320, 420, 720));
    }

    #[test]
    fn quiet_hours_use_the_users_time_zone() {
        // 14:30 UTC is 23:30 in Tokyo and 15:30 in Paris (winter).
        let now = utc(14, 30);
        assert!(in_quiet_hours("Asia/Tokyo", 1320, 420, now));
        assert!(!in_quiet_hours("Europe/Paris", 1320, 420, now));
        // 22:10 UTC: 07:10 in Tokyo (just over), 23:10 in Paris (inside).
        let now = utc(22, 10);
        assert!(!in_quiet_hours("Asia/Tokyo", 1320, 420, now));
        assert!(in_quiet_hours("Europe/Paris", 1320, 420, now));
    }

    #[test]
    fn daylight_saving_shifts_the_window_with_the_wall_clock() {
        // New York is UTC-4 in July and UTC-5 in January: 02:30 UTC is 22:30
        // local in summer but 21:30 in winter.
        let summer = chrono::Utc.with_ymd_and_hms(2026, 7, 10, 2, 30, 0).unwrap();
        let winter = chrono::Utc.with_ymd_and_hms(2026, 1, 10, 2, 30, 0).unwrap();
        assert!(in_quiet_hours("America/New_York", 1320, 420, summer));
        assert!(!in_quiet_hours("America/New_York", 1320, 420, winter));
    }

    #[test]
    fn an_unknown_time_zone_falls_back_to_tokyo() {
        assert!(in_quiet_hours("Mars/Olympus", 1320, 420, utc(14, 30)));
    }

    #[test]
    fn pauses_end_by_themselves_or_never() {
        let now = utc(12, 0);
        assert!(!is_paused(false, None, now));
        assert!(is_paused(true, None, now), "no end: until resumed");
        assert!(is_paused(true, Some(utc(13, 0)), now));
        assert!(!is_paused(true, Some(utc(11, 0)), now), "the pause ran out");
    }

    #[test]
    fn quiet_hours_validation() {
        let quiet =
            |start_minute, end_minute| QuietHours { enabled: true, start_minute, end_minute, ..Default::default() };
        assert!(validate_quiet_hours(&quiet(1320, 420)).is_ok());
        assert!(validate_quiet_hours(&quiet(0, 1439)).is_ok());
        for bad in [quiet(-1, 10), quiet(10, 1440), quiet(600, 600)] {
            assert_eq!(validate_quiet_hours(&bad).unwrap_err().reason, "QUIET_HOURS_INVALID");
        }
    }
}
