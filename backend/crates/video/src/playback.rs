//! Playback sessions: watch time, view counting, resume positions and the
//! watch history.
//!
//! A session lives in Valkey for as long as the client keeps reporting (two
//! hours after the last report). The server measures watch time itself: the
//! time between two reports counts when the earlier one said "playing",
//! capped so that a client that stops reporting does not keep accruing.
//! Positions are only used for the resume position and for the audience
//! retention curve (which hundredths of the video a session covered).
//!
//! A *view* is counted once a session accumulated the minimum watch time,
//! at most once per viewer (account, or network address when anonymous) and
//! video within the de-duplication period — a Valkey key per viewer/video
//! decides, so reloading the page or opening a second tab does not count
//! again. The channel owner watching their own video never counts.
//!
//! Analytics events (ClickHouse `events`, owner = channel owner, subject =
//! `("video", id)`; see `analytics`):
//! `video.play` when a session starts, `video.watch` per report with the
//! seconds watched and the covered range, `video.view` when the view counts.
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokuly_core::{analytics::AnalyticsEvent, prelude::*};

use crate::videos::VideoRow;

/// How long a session survives without a report.
pub const SESSION_TTL: Duration = Duration::from_secs(2 * 3600);
/// The retention curve has this many buckets.
pub const RETENTION_BUCKETS: usize = 100;
/// A video counts as watched to the end from this share of its length.
const COMPLETED_AT: f64 = 0.95;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub video_id: Uuid,
    pub channel_id: Uuid,
    pub owner_id: Uuid,
    pub user_id: Option<Uuid>,
    /// Who is watching, for de-duplication and unique-viewer counts: the
    /// user ID, or `ip:<keyed hash>` for anonymous viewers.
    pub viewer: String,
    /// `tokuly.video.v1.TrafficSource` as its number.
    pub source: i32,
    pub duration: f64,
    /// Unix milliseconds of the last report (or the start).
    pub last_report_ms: i64,
    pub last_position: f64,
    /// The player was playing at the last report.
    pub playing: bool,
    /// Seconds watched in this session so far.
    pub watched: f64,
    /// This session's view has been counted (or found to be a repeat).
    pub counted: bool,
}

fn session_key(session_id: &str) -> String {
    format!("video:playback:{session_id}")
}

fn dedupe_key(video_id: Uuid, viewer: &str) -> String {
    format!("video:viewed:{}:{viewer}", video_id.simple())
}

/// The viewer key of a caller: their account, or a keyed hash of their
/// network address (raw addresses are not stored).
pub fn viewer_key(core: &Core, user_id: Option<Uuid>, ip: &str) -> String {
    match user_id {
        Some(user_id) => user_id.to_string(),
        None => format!("ip:{}", &hex::encode(core.keys.mac("video/viewer-ip", ip.as_bytes()))[..24]),
    }
}

/// Seconds of watching after which a session counts as a view: the
/// configured minimum, or half the video for very short ones.
pub fn min_view_seconds(configured: f64, duration: f64) -> f64 {
    if duration > 0.0 { configured.min(duration / 2.0) } else { configured }
}

/// What one progress report adds to a session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    /// Seconds watched since the previous report.
    pub watched: f64,
    /// Inclusive range of retention buckets the viewer just watched.
    pub covered: Option<(u8, u8)>,
}

/// Advances a session by one report at `now_ms`. Time counts only while the
/// previous report said "playing", and at most `max_gap` seconds of it; the
/// stretch between the two positions counts as covered when it is what
/// playing forward for that long can produce (not a seek).
pub fn advance(session: &mut Session, now_ms: i64, position: f64, playing: bool, max_gap: f64) -> Progress {
    let elapsed = ((now_ms - session.last_report_ms) as f64 / 1000.0).clamp(0.0, max_gap);
    let position = if position.is_finite() { position.clamp(0.0, session.duration.max(0.0)) } else { session.last_position };
    let mut progress = Progress { watched: 0.0, covered: None };
    if session.playing {
        progress.watched = elapsed;
        let moved = position - session.last_position;
        // Up to double speed, plus slack for timer jitter.
        let plausible = moved > 0.0 && moved <= elapsed * 2.0 + 2.0;
        if plausible && session.duration > 0.0 {
            let bucket = |at: f64| ((at / session.duration * RETENTION_BUCKETS as f64) as usize).min(RETENTION_BUCKETS - 1) as u8;
            // The end position is exclusive: a report at exactly 50% has not watched bucket 50 yet.
            let last = bucket((position - 0.001).max(session.last_position));
            progress.covered = Some((bucket(session.last_position), last));
        }
    }
    session.watched += progress.watched;
    session.last_report_ms = now_ms;
    session.last_position = position;
    session.playing = playing;
    progress
}

/// Whether a position counts as having watched the video to the end.
pub fn is_completed(position: f64, duration: f64, ended: bool) -> bool {
    ended || (duration > 0.0 && position >= duration * COMPLETED_AT)
}

pub async fn load(core: &Core, session_id: &str) -> AppResult<Option<Session>> {
    let plausible = (8..=64).contains(&session_id.len()) && session_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !plausible {
        return Ok(None);
    }
    core.cache.get_json(&session_key(session_id)).await
}

pub async fn save(core: &Core, session_id: &str, session: &Session) -> AppResult<()> {
    core.cache.set_json(&session_key(session_id), session, Some(SESSION_TTL)).await
}

/// Opens a session for `video` and returns its ID.
pub async fn start(core: &Core, video: &VideoRow, user_id: Option<Uuid>, ip: &str, source: i32, position: f64) -> AppResult<String> {
    let session = Session {
        video_id: video.id,
        channel_id: video.channel_id,
        owner_id: video.owner_id,
        user_id,
        viewer: viewer_key(core, user_id, ip),
        source,
        duration: video.duration_seconds,
        last_report_ms: now().timestamp_millis(),
        last_position: if position.is_finite() { position.clamp(0.0, video.duration_seconds.max(0.0)) } else { 0.0 },
        playing: true,
        watched: 0.0,
        counted: false,
    };
    let session_id = tokuly_core::keys::random_token(18);
    save(core, &session_id, &session).await?;
    if user_id != Some(video.owner_id) {
        core.analytics.track(event("video.play", &session, &session_id)).await;
    }
    Ok(session_id)
}

fn event(name: &str, session: &Session, session_id: &str) -> AnalyticsEvent {
    AnalyticsEvent::new(name).user(session.user_id).subject("video", session.video_id).owner(session.owner_id).props(serde_json::json!({
        "channel_id": session.channel_id,
        "session": session_id,
        "viewer": session.viewer,
        "source": session.source,
    }))
}

/// Records what a report added: the `video.watch` analytics event.
pub async fn track_progress(core: &Core, session: &Session, session_id: &str, progress: Progress) {
    if progress.watched <= 0.0 || session.user_id == Some(session.owner_id) {
        return;
    }
    let mut watch = event("video.watch", session, session_id).value(progress.watched);
    if let (Some((from, to)), Some(props)) = (progress.covered, watch.props.as_object_mut()) {
        props.insert("from".to_owned(), from.into());
        props.insert("to".to_owned(), to.into());
    }
    core.analytics.track(watch).await;
}

/// Counts the session's view if it is due: enough watch time, not the
/// owner, and no counted view by the same viewer within the de-duplication
/// period. Marks the session so the check is not repeated.
pub async fn count_view(core: &Core, session: &mut Session, session_id: &str, min_seconds: f64, dedupe: Duration) -> AppResult<()> {
    if session.counted || session.watched < min_seconds {
        return Ok(());
    }
    session.counted = true;
    if session.user_id == Some(session.owner_id) {
        return Ok(());
    }
    if !core.cache.set_nx(&dedupe_key(session.video_id, &session.viewer), "1", dedupe).await? {
        return Ok(()); // a repeat within the period
    }
    let mut tx = core.db.begin().await?;
    let counted = sqlx::query("UPDATE videos SET view_count = view_count + 1 WHERE id = $1").bind(session.video_id).execute(&mut *tx).await?;
    if counted.rows_affected() == 1 {
        sqlx::query("UPDATE channels SET view_count = view_count + 1 WHERE id = $1").bind(session.channel_id).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO video_hourly_stats (video_id, hour, views) VALUES ($1, date_trunc('hour', now()), 1)
             ON CONFLICT (video_id, hour) DO UPDATE SET views = video_hourly_stats.views + 1",
        )
        .bind(session.video_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    core.analytics.track(event("video.view", session, session_id).value(1.0)).await;
    Ok(())
}

/// Notes that a signed-in viewer started watching (unless their history is paused).
pub async fn touch_history(core: &Core, user_id: Uuid, video_id: Uuid) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO watch_history (user_id, video_id)
         SELECT $1, $2 WHERE NOT EXISTS (SELECT 1 FROM video_user_settings WHERE user_id = $1 AND history_paused)
         ON CONFLICT (user_id, video_id) DO UPDATE SET last_watched_at = now()",
    )
    .bind(user_id)
    .bind(video_id)
    .execute(&core.db)
    .await?;
    Ok(())
}

/// Stores where a signed-in viewer is in a video and how long they watched
/// (unless their history is paused).
pub async fn record_history(core: &Core, user_id: Uuid, session: &Session, position: f64, watched: f64, ended: bool) -> AppResult<()> {
    let completed = is_completed(position, session.duration, ended);
    sqlx::query(
        "INSERT INTO watch_history (user_id, video_id, position_seconds, watched_seconds, completed)
         SELECT $1, $2, $3, $4, $5 WHERE NOT EXISTS (SELECT 1 FROM video_user_settings WHERE user_id = $1 AND history_paused)
         ON CONFLICT (user_id, video_id) DO UPDATE SET
             position_seconds = EXCLUDED.position_seconds,
             watched_seconds = watch_history.watched_seconds + EXCLUDED.watched_seconds,
             completed = watch_history.completed OR EXCLUDED.completed,
             last_watched_at = now()",
    )
    .bind(user_id)
    .bind(session.video_id)
    .bind(if completed { 0.0 } else { position })
    .bind(watched)
    .bind(completed)
    .execute(&core.db)
    .await?;
    Ok(())
}

/// Where `user_id` left off in a video; 0 when unknown or watched to the end.
pub async fn resume_position(core: &Core, user_id: Uuid, video_id: Uuid) -> AppResult<f64> {
    let position: Option<f64> = sqlx::query_scalar(
        "SELECT CASE WHEN completed THEN 0 ELSE position_seconds END FROM watch_history WHERE user_id = $1 AND video_id = $2",
    )
    .bind(user_id)
    .bind(video_id)
    .fetch_optional(&core.db)
    .await?;
    Ok(position.unwrap_or(0.0))
}

pub async fn history_paused(core: &Core, user_id: Uuid) -> AppResult<bool> {
    let paused: Option<bool> =
        sqlx::query_scalar("SELECT history_paused FROM video_user_settings WHERE user_id = $1").bind(user_id).fetch_optional(&core.db).await?;
    Ok(paused.unwrap_or(false))
}

pub async fn set_history_paused(core: &Core, user_id: Uuid, paused: bool) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO video_user_settings (user_id, history_paused) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET history_paused = EXCLUDED.history_paused, updated_at = now()",
    )
    .bind(user_id)
    .bind(paused)
    .execute(&core.db)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(duration: f64) -> Session {
        Session {
            video_id: Uuid::now_v7(),
            channel_id: Uuid::now_v7(),
            owner_id: Uuid::now_v7(),
            user_id: None,
            viewer: "ip:test".into(),
            source: 0,
            duration,
            last_report_ms: 1_000_000,
            last_position: 0.0,
            playing: true,
            watched: 0.0,
            counted: false,
        }
    }

    #[test]
    fn time_counts_only_while_playing_and_is_capped() {
        let mut s = session(100.0);
        let p = advance(&mut s, 1_010_000, 10.0, true, 30.0);
        assert_eq!((p.watched, p.covered), (10.0, Some((0, 9))));
        // Paused at this report: the next stretch does not count.
        advance(&mut s, 1_015_000, 15.0, false, 30.0);
        let p = advance(&mut s, 1_075_000, 15.0, true, 30.0);
        assert_eq!((p.watched, p.covered), (0.0, None));
        // A client that went silent for ten minutes gets the cap, not ten minutes.
        let p = advance(&mut s, 1_675_000, 20.0, true, 30.0);
        assert_eq!(p.watched, 30.0);
        assert_eq!(s.watched, 10.0 + 5.0 + 30.0);
        // Clocks running backwards never subtract.
        assert_eq!(advance(&mut s, 0, 20.0, true, 30.0).watched, 0.0);
    }

    #[test]
    fn seeks_are_not_coverage() {
        let mut s = session(200.0);
        // Jumped 150 s ahead in 5 s: watched 5 s, covered nothing.
        let p = advance(&mut s, 1_005_000, 150.0, true, 30.0);
        assert_eq!((p.watched, p.covered), (5.0, None));
        // Seeking backwards covers nothing either.
        assert_eq!(advance(&mut s, 1_010_000, 20.0, true, 30.0).covered, None);
        // Double speed is real playback: 10 s of wall time, 20 s of video (10%).
        let p = advance(&mut s, 1_020_000, 40.0, true, 30.0);
        assert_eq!((p.watched, p.covered), (10.0, Some((10, 19))));
        // Positions past the end are clamped; the last bucket is 99.
        let mut s = session(10.0);
        assert_eq!(advance(&mut s, 1_010_000, 99.0, false, 30.0).covered, Some((0, 99)));
        assert_eq!(s.last_position, 10.0);
    }

    #[test]
    fn views_need_less_time_on_short_videos() {
        assert_eq!(min_view_seconds(10.0, 600.0), 10.0);
        assert_eq!(min_view_seconds(10.0, 6.0), 3.0);
        assert_eq!(min_view_seconds(10.0, 0.0), 10.0);
    }

    #[test]
    fn completion_is_the_end_or_nearly() {
        assert!(is_completed(10.0, 100.0, true));
        assert!(is_completed(96.0, 100.0, false));
        assert!(!is_completed(50.0, 100.0, false));
        assert!(!is_completed(50.0, 0.0, false));
    }
}
