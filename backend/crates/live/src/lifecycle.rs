//! Going on air, ending, and following the media server.
//!
//! The media server is the source of truth for "is an encoder publishing":
//! [`Live::reconcile`] reads its control API on a timer and brings the
//! streams in line with what it sees ([`Live::apply_media_state`]):
//!
//! - a path came online → the stream's encoder is connected; with
//!   `auto_start` an UPCOMING stream goes on air;
//! - a path went away → the encoder is gone; a stream on air gets the
//!   reconnect grace to come back and is ended when it does not;
//! - a path is online for a stream that is over or unknown → its publisher
//!   is disconnected;
//! - streams on air longer than the maximum are ended.
//!
//! Because this is a comparison of states and not a reaction to events,
//! nothing is lost when the media server or the worker restarts: the next
//! look sees the paths that are there (or not), and streams whose encoder
//! never comes back run into the grace period. When the media server cannot
//! be reached at all for [`OUTAGE_GRACE_SECONDS`], its encoders are taken
//! for gone.
//!
//! The same timer samples the viewer counts ([`Live::sample_viewers`]).
//!
//! # The media timeline
//!
//! A stream's timeline starts when it goes on air and only advances while
//! the encoder is connected: it is the time covered by the stream's ingest
//! sessions from `started_at` on. Chat offsets, clip positions and the
//! archive video all use it, so they line up even after reconnects
//! ([`offset_ms_now`], [`time_at_offset`]).
use std::collections::{HashMap, HashSet};

use chrono::Duration;
use sqlx::PgExecutor;
use tokuly_core::{
    events as core_events,
    outbound::{self, ActivityAudience, ActivityEntry, Localized},
    prelude::*,
};
use tokuly_proto::live::{EndReason, live_event::Event};
use tokuly_video::channels;

use crate::{
    Live,
    access::{FRIENDS, PUBLIC},
    audience,
    events::{STREAM_ENDED, STREAM_STARTED},
    media_server::MediaPath,
    polls, recordings, search,
    streams::{self, ARCHIVE_NONE, ARCHIVE_PENDING, Aftermath, StreamRow, stream_columns},
};

/// After this long without an answer from the media server its encoders
/// are taken for gone (and their streams run into the reconnect grace).
pub const OUTAGE_GRACE_SECONDS: i64 = 300;

const MEDIA_DOWN_KEY: &str = "live:media-down-since";
/// Every this many looks at the media server, the readers of restricted
/// streams are checked against who may watch.
const ENFORCE_EVERY: u64 = 10;

/// What one look at the media server changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Reconciled {
    /// Encoders that connected / went away.
    pub connected: u32,
    pub disconnected: u32,
    /// Streams put on air / ended.
    pub started: u32,
    pub ended: u32,
    /// Publishers disconnected from paths nobody may publish to.
    pub kicked: u32,
}

const OFFSET_MS_SQL: &str = "SELECT (CASE
         WHEN s.started_at IS NULL THEN 0
         WHEN NOT EXISTS (SELECT 1 FROM live_ingest_sessions i WHERE i.stream_id = s.id)
             THEN EXTRACT(EPOCH FROM (LEAST(now(), COALESCE(s.ended_at, now())) - s.started_at)) * 1000
         ELSE COALESCE((
             SELECT SUM(EXTRACT(EPOCH FROM (LEAST(COALESCE(i.ended_at, now()), COALESCE(s.ended_at, now()))
                                            - GREATEST(i.started_at, s.started_at))))
             FROM live_ingest_sessions i
             WHERE i.stream_id = s.id AND COALESCE(i.ended_at, now()) > s.started_at), 0) * 1000
     END)::bigint
     FROM live_streams s WHERE s.id = $1";

/// Where a stream's media timeline stands right now, in milliseconds (0
/// before it went on air).
pub async fn offset_ms_now<'e, E: PgExecutor<'e>>(executor: E, stream_id: Uuid) -> AppResult<i64> {
    let offset: Option<i64> = sqlx::query_scalar(OFFSET_MS_SQL).bind(stream_id).fetch_optional(executor).await?;
    Ok(offset.unwrap_or(0).max(0))
}

/// The wall-clock time at which a stream's media timeline stood at
/// `offset_seconds`, or `None` when the timeline has not come that far.
pub async fn time_at_offset(core: &Core, stream: &StreamRow, offset_seconds: f64) -> AppResult<Option<Time>> {
    let Some(started_at) = stream.started_at else { return Ok(None) };
    let end = stream.ended_at.unwrap_or_else(now);
    let sessions: Vec<(Time, Option<Time>)> = sqlx::query_as(
        "SELECT started_at, ended_at FROM live_ingest_sessions WHERE stream_id = $1 ORDER BY started_at",
    )
    .bind(stream.id)
    .fetch_all(&core.db)
    .await?;
    let spans: Vec<(Time, Time)> = if sessions.is_empty() {
        vec![(started_at, end)]
    } else {
        sessions
            .into_iter()
            .map(|(from, to)| (from.max(started_at), to.unwrap_or(end).min(end)))
            .filter(|(from, to)| to > from)
            .collect()
    };
    Ok(locate(&spans, offset_seconds))
}

/// Walks the spans the encoder was connected for and finds the moment
/// `offset_seconds` into their sum.
fn locate(spans: &[(Time, Time)], offset_seconds: f64) -> Option<Time> {
    let mut remaining = Duration::milliseconds((offset_seconds * 1000.0).round() as i64);
    if remaining < Duration::zero() {
        return None;
    }
    for (from, to) in spans {
        let length = *to - *from;
        if remaining <= length {
            return Some(*from + remaining);
        }
        remaining -= length;
    }
    None
}

impl Live {
    /// Puts an UPCOMING stream on air. Returns the stream when this call
    /// did it, `None` when it was not upcoming (any more).
    pub async fn go_live(&self, stream_id: Uuid) -> AppResult<Option<StreamRow>> {
        let core = &self.core;
        // Read before the transaction: while it holds the stream's row,
        // nothing asks the pool for a second connection.
        let Some(known) = streams::find_by_id(core, stream_id).await? else { return Ok(None) };
        let channel = channels::find_by_id(core, known.channel_id).await?;
        let mut tx = core.db.begin().await?;
        let Some(stream) = streams::lock(&mut tx, stream_id).await? else { return Ok(None) };
        if !stream.is_upcoming() {
            return Ok(None);
        }
        sqlx::query("UPDATE live_streams SET status = 2, started_at = now(), encoder_lost_at = NULL WHERE id = $1")
            .bind(stream.id)
            .execute(&mut *tx)
            .await?;
        let stream = streams::lock(&mut tx, stream_id).await?.ok_or_else(streams::not_found)?;
        core_events::emit(&mut *tx, STREAM_STARTED, Some(stream.owner_id), &stream.event()).await?;
        if matches!(stream.visibility, PUBLIC | FRIENDS) && !stream.suspended {
            // Friends only, also for public streams: a channel does not
            // reveal its owner to everyone.
            let mut entry = ActivityEntry::new(
                stream.owner_id,
                "live.started",
                ActivityAudience::Friends,
                Localized::new(
                    format!("ライブ配信を始めたよ!「{}」", stream.title),
                    format!("Went live: {}", stream.title),
                ),
            )
            .subject(streams::SUBJECT, stream.id)
            .path(stream.path())
            .data(serde_json::json!({
                "stream_id": stream.id,
                "short_id": stream.short_id,
                "channel_id": stream.channel_id,
                "channel_handle": channel.as_ref().map(|c| c.handle.clone()),
                "channel_name": channel.as_ref().map(|c| c.name.clone()),
            }));
            if let Some(url) = stream.thumbnail_url() {
                entry = entry.image(url);
            }
            outbound::activity(&mut *tx, &entry).await?;
        }
        search::sync_stream(&mut tx, &stream, channel.as_ref()).await?;
        tx.commit().await?;

        if stream.visibility == PUBLIC
            && !stream.game.is_empty()
            && let Err(error) = tokuly_video::games::record_use(core, &stream.game).await
        {
            tracing::warn!(stream_id = %stream.id, %error, "could not record the game of a stream");
        }
        self.push_stream_updated(&stream).await;
        Ok(Some(stream))
    }

    /// Ends a stream (or cancels one that never went on air). Returns the
    /// stream when this call ended it, `None` when it was over already.
    ///
    /// With `auto_archive`, a stream that was on air is marked for
    /// archiving; the archive job starts from the `live.stream.ended` event.
    /// `raid_target` is the stream the audience was sent to, if any.
    pub async fn finish_stream(
        &self,
        stream_id: Uuid,
        reason: EndReason,
        raid_target: Option<Uuid>,
        actor: Option<Uuid>,
    ) -> AppResult<Option<StreamRow>> {
        let core = &self.core;
        let mut tx = core.db.begin().await?;
        let Some(stream) = streams::lock(&mut tx, stream_id).await? else { return Ok(None) };
        if stream.is_ended() {
            return Ok(None);
        }
        let archive = stream.auto_archive && stream.started_at.is_some() && reason != EndReason::END_REASON_REMOVED;
        sqlx::query(
            "UPDATE live_streams SET status = 3, ended_at = now(), end_reason = $2, encoder_connected = false,
                    encoder_protocol = '', encoder_lost_at = NULL, viewer_count = 0, ranked_viewer_count = 0, archive_status = $3,
                    archive_changed_at = now(), raid_target_id = $4
             WHERE id = $1",
        )
        .bind(stream.id)
        .bind(reason as i32)
        .bind(if archive { ARCHIVE_PENDING } else { ARCHIVE_NONE })
        .bind(raid_target)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE live_ingest_sessions SET ended_at = now() WHERE stream_id = $1 AND ended_at IS NULL")
            .bind(stream.id)
            .execute(&mut *tx)
            .await?;
        let closed_polls = polls::close_open_in(&mut tx, stream.id).await?;
        let ended = streams::lock(&mut tx, stream_id).await?.ok_or_else(streams::not_found)?;
        core_events::emit(&mut *tx, STREAM_ENDED, actor, &ended.event()).await?;
        // An ended stream is not searchable: this only removes its document.
        search::sync_stream(&mut tx, &ended, None).await?;
        tx.commit().await?;

        if let Err(error) = audience::clear(core, ended.id).await {
            tracing::warn!(stream_id = %ended.id, %error, "could not clear the audience of an ended stream");
        }
        for poll in closed_polls {
            self.push_poll(&ended, &poll).await;
        }
        self.push_stream_updated(&ended).await;
        {
            // The encoder must not keep sending into a stream that is over,
            // whether or not we had noticed it connect (an upcoming stream
            // deleted moments after its encoder came). Not awaited: a slow
            // media server should not hold up the caller.
            let live = self.clone();
            let kicked = ended.clone();
            tokio::spawn(async move { live.kick_quietly(&kicked).await });
        }
        Ok(Some(ended))
    }

    /// Notes that a stream's encoder connected; with `auto_start` an
    /// upcoming stream goes on air. Returns whether the stream went live.
    pub async fn encoder_connected(&self, stream: &StreamRow, protocol: &str) -> AppResult<bool> {
        let mut tx = self.core.db.begin().await?;
        let changed = sqlx::query(
            "UPDATE live_streams SET encoder_connected = true, encoder_protocol = $2, encoder_lost_at = NULL
             WHERE id = $1 AND status < 3 AND NOT encoder_connected",
        )
        .bind(stream.id)
        .bind(protocol)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if !changed {
            return Ok(false);
        }
        sqlx::query(
            "INSERT INTO live_ingest_sessions (stream_id, protocol) VALUES ($1, $2)
             ON CONFLICT (stream_id) WHERE ended_at IS NULL DO NOTHING",
        )
        .bind(stream.id)
        .bind(protocol)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.push_encoder_state(stream, true, protocol).await;
        if stream.is_upcoming() && stream.auto_start {
            return Ok(self.go_live(stream.id).await?.is_some());
        }
        Ok(false)
    }

    /// Notes that a stream's encoder went away. A stream on air starts its
    /// reconnect grace.
    pub async fn encoder_disconnected(&self, stream: &StreamRow) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let changed = sqlx::query(
            "UPDATE live_streams SET encoder_connected = false, encoder_protocol = '',
                    encoder_lost_at = CASE WHEN status = 2 THEN now() END
             WHERE id = $1 AND encoder_connected",
        )
        .bind(stream.id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        sqlx::query("UPDATE live_ingest_sessions SET ended_at = now() WHERE stream_id = $1 AND ended_at IS NULL")
            .bind(stream.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        if changed {
            self.push_encoder_state(stream, false, "").await;
        }
        Ok(())
    }

    /// One look at the media server: reads its paths and applies them.
    /// Without a configured media server only the checks that need none run
    /// (viewer samples, the maximum stream length).
    pub async fn reconcile(&self) -> AppResult<Reconciled> {
        if !self.cfg.media_server_configured() {
            return self.apply_media_state(None).await;
        }
        let cache = &self.core.cache;
        match self.media_paths().await {
            Ok(paths) => {
                cache.del(MEDIA_DOWN_KEY).await?;
                let report = self.apply_media_state(Some(&paths)).await?;
                // Every so often, look whether everyone reading a restricted
                // stream may still do so (the events that take access away
                // are acted on at once; this catches what they missed).
                let run = self.reconcile_runs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if run % ENFORCE_EVERY == ENFORCE_EVERY - 1 {
                    let restricted: Vec<StreamRow> = sqlx::query_as(concat!(
                        "SELECT ",
                        stream_columns!(),
                        " FROM live_streams s WHERE (s.status = 2 AND s.visibility IN (4, 5)) OR (s.status = 1 AND s.encoder_connected)"
                    ))
                    .fetch_all(&self.core.db)
                    .await?;
                    for stream in restricted
                        .iter()
                        .filter(|stream| paths.iter().any(|path| path.name == stream.media_path() && path.readers > 0))
                    {
                        if let Err(error) = self.enforce_access(stream).await {
                            tracing::warn!(stream_id = %stream.id, %error, "could not check who is reading a restricted stream");
                        }
                    }
                }
                Ok(report)
            }
            Err(error) => {
                let now = now().timestamp();
                let since = match cache.get_string(MEDIA_DOWN_KEY).await?.and_then(|at| at.parse::<i64>().ok()) {
                    Some(since) => since,
                    None => {
                        cache
                            .set_string(MEDIA_DOWN_KEY, &now.to_string(), Some(std::time::Duration::from_secs(86_400)))
                            .await?;
                        now
                    }
                };
                tracing::warn!(%error, down_for_seconds = now - since, "the media server cannot be reached");
                if now - since > OUTAGE_GRACE_SECONDS {
                    self.apply_media_state(Some(&[])).await
                } else {
                    self.apply_media_state(None).await
                }
            }
        }
    }

    /// Brings the streams in line with the paths the media server reports
    /// (`None`: it could not be asked, so encoders are left as they are).
    pub async fn apply_media_state(&self, observed: Option<&[MediaPath]>) -> AppResult<Reconciled> {
        let core = &self.core;
        let mut report = Reconciled::default();
        let mut readers: HashMap<Uuid, i64> = HashMap::new();

        if let Some(paths) = observed {
            let online: HashMap<&str, &MediaPath> = paths
                .iter()
                .filter(|path| path.online)
                .filter_map(|path| Some((streams::short_id_of_path(&path.name)?, path)))
                .collect();
            let active: Vec<StreamRow> = sqlx::query_as(concat!(
                "SELECT ",
                stream_columns!(),
                " FROM live_streams s WHERE s.status = 2 OR s.encoder_connected"
            ))
            .fetch_all(&core.db)
            .await?;
            let known: HashSet<&str> = active.iter().map(|stream| stream.short_id.as_str()).collect();
            let unknown: Vec<String> =
                online.keys().filter(|short_id| !known.contains(*short_id)).map(|s| (*s).to_owned()).collect();
            let newly: Vec<StreamRow> = if unknown.is_empty() {
                Vec::new()
            } else {
                sqlx::query_as(concat!("SELECT ", stream_columns!(), " FROM live_streams s WHERE s.short_id = ANY($1)"))
                    .bind(&unknown)
                    .fetch_all(&core.db)
                    .await?
            };
            let by_short_id: HashMap<&str, &StreamRow> =
                active.iter().chain(newly.iter()).map(|stream| (stream.short_id.as_str(), stream)).collect();

            for (short_id, path) in &online {
                match by_short_id.get(short_id) {
                    Some(stream) if !stream.is_ended() && !stream.suspended => {
                        if !stream.encoder_connected {
                            report.connected += 1;
                            if self.encoder_connected(stream, path.protocol()).await? {
                                report.started += 1;
                            }
                        }
                        readers.insert(stream.id, path.readers);
                    }
                    // Nobody may publish here: the stream is over, suspended or gone.
                    _ => match self.kick(path).await {
                        Ok(true) => report.kicked += 1,
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!(path = %path.name, %error, "could not disconnect a stray publisher")
                        }
                    },
                }
            }
            for stream in active
                .iter()
                .filter(|stream| stream.encoder_connected && !online.contains_key(stream.short_id.as_str()))
            {
                self.encoder_disconnected(stream).await?;
                report.disconnected += 1;
            }
            // A stream on air without an encoder and without a running
            // grace period (it was put on air by hand, or the note of its
            // disconnect was lost): start the grace now.
            let online_ids: Vec<String> = online.keys().map(|short_id| (*short_id).to_owned()).collect();
            sqlx::query(
                "UPDATE live_streams SET encoder_lost_at = now()
                 WHERE status = 2 AND NOT encoder_connected AND encoder_lost_at IS NULL AND NOT (short_id = ANY($1))",
            )
            .bind(&online_ids)
            .execute(&core.db)
            .await?;
        }

        let lost: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM live_streams
             WHERE status = 2 AND encoder_lost_at IS NOT NULL AND encoder_lost_at <= now() - make_interval(secs => $1)",
        )
        .bind(self.cfg.reconnect_grace_seconds as f64)
        .fetch_all(&core.db)
        .await?;
        for stream_id in lost {
            if self.finish_stream(stream_id, EndReason::END_REASON_ENCODER_GONE, None, None).await?.is_some() {
                report.ended += 1;
            }
        }
        let overdue: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM live_streams WHERE status = 2 AND started_at <= now() - make_interval(hours => $1)",
        )
        .bind(self.cfg.max_hours as i32)
        .fetch_all(&core.db)
        .await?;
        for stream_id in overdue {
            if self.finish_stream(stream_id, EndReason::END_REASON_TOO_LONG, None, None).await?.is_some() {
                report.ended += 1;
            }
        }

        self.sample_viewers(&readers).await?;
        Ok(report)
    }

    /// Counts the viewers of every stream on air, keeps the peak, stores
    /// the sample of the minute and pushes the count when it changed.
    /// `media_readers` are the readers the media server reported per stream.
    pub async fn sample_viewers(&self, media_readers: &HashMap<Uuid, i64>) -> AppResult<()> {
        let core = &self.core;
        let on_air: Vec<(Uuid, i64)> =
            sqlx::query_as("SELECT id, viewer_count FROM live_streams WHERE status = 2").fetch_all(&core.db).await?;
        for (stream_id, previous) in on_air {
            let readers = media_readers.get(&stream_id).copied().unwrap_or(0);
            let viewers = audience::viewers(core, stream_id, readers, self.cfg.report_interval_seconds).await?;
            // The ranking of "live now" only counts people who are signed in.
            let signed_in =
                audience::watching_users(core, stream_id, self.cfg.report_interval_seconds).await?.len() as i64;
            let updated: Option<(i64, i64)> = sqlx::query_as(
                "UPDATE live_streams SET viewer_count = $2, ranked_viewer_count = $3,
                        peak_viewer_count = GREATEST(peak_viewer_count, $2)
                 WHERE id = $1 AND status = 2 RETURNING peak_viewer_count, like_count",
            )
            .bind(stream_id)
            .bind(viewers)
            .bind(signed_in)
            .fetch_optional(&core.db)
            .await?;
            let Some((peak, likes)) = updated else { continue };
            sqlx::query(
                "INSERT INTO live_viewer_samples (stream_id, at, viewers) VALUES ($1, date_trunc('minute', now()), $2)
                 ON CONFLICT (stream_id, at) DO UPDATE SET viewers = GREATEST(live_viewer_samples.viewers, EXCLUDED.viewers)",
            )
            .bind(stream_id)
            .bind(viewers)
            .execute(&core.db)
            .await?;
            if viewers != previous {
                self.push_stats(stream_id, viewers, peak, likes).await;
            }
        }
        Ok(())
    }

    /// Finishes a deletion after its transaction committed: recordings and
    /// audiences of deleted streams, uploads of deleted clips.
    pub async fn clean_up(&self, aftermath: Aftermath) {
        let core = &self.core;
        for short_id in &aftermath.recordings {
            recordings::remove(&self.cfg, short_id).await;
        }
        for stream in &aftermath.streams {
            if let Err(error) = audience::clear(core, stream.id).await {
                tracing::warn!(stream_id = %stream.id, %error, "could not clear the audience of a deleted stream");
            }
            self.kick_quietly(stream).await;
        }
        for removal in &aftermath.assets {
            match tokuly_media::assets::delete(core, removal.asset_id, removal.owner_id).await {
                Ok(_) => {}
                // Something else (a short made from the clip) still uses it.
                Err(error) if error.reason == "ASSET_IN_USE" => {}
                Err(error) => {
                    tracing::warn!(asset_id = %removal.asset_id, %error, "could not delete the upload of a clip")
                }
            }
        }
    }

    /// Pushes a poll's state to its stream and the channel's poll widgets.
    pub(crate) async fn push_poll(&self, stream: &StreamRow, poll: &polls::PollRow) {
        let event = Event::PollUpdated(Box::new(poll.to_proto(0)));
        self.push_to_stream(stream.id, event.clone()).await;
        self.push_to_overlays(stream.channel_id, &[tokuly_proto::live::OverlayKind::OVERLAY_KIND_POLL], event).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_skip_the_time_the_encoder_was_away() {
        let t0 = chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let at = |seconds: i64| t0 + Duration::seconds(seconds);
        // On air for 100 s, away for 50 s, back for 100 s.
        let spans = [(at(0), at(100)), (at(150), at(250))];
        assert_eq!(locate(&spans, 0.0), Some(at(0)));
        assert_eq!(locate(&spans, 40.5), Some(at(40) + Duration::milliseconds(500)));
        assert_eq!(locate(&spans, 100.0), Some(at(100)));
        // 120 s into the timeline is 20 s into the second connection.
        assert_eq!(locate(&spans, 120.0), Some(at(170)));
        assert_eq!(locate(&spans, 200.0), Some(at(250)));
        assert_eq!(locate(&spans, 200.5), None);
        assert_eq!(locate(&spans, -1.0), None);
        assert_eq!(locate(&[], 0.0), None);
    }
}
