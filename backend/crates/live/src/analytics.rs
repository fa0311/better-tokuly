//! Numbers for the streamer: the dashboard while on air and the report
//! afterwards.
//!
//! Where they come from:
//!
//! - the stream row: peak viewers, chat messages, gifts, new subscribers,
//!   likes (counters kept as things happen);
//! - `live_viewer_samples`: one viewer count a minute (timeline, average);
//! - the ClickHouse `events` table: who watched and for how long, who
//!   chatted.
//!
//! Events this domain tracks, all with subject `("live_stream", id)` and
//! `owner_id` = the channel's owner:
//!
//! | Name | Value | Props |
//! |---|---|---|
//! | `live.view` | 1 | `channel_id`, `viewer`: a viewer's first heartbeat on the stream |
//! | `live.watch` | seconds since the previous heartbeat | `channel_id`, `viewer` |
//! | `live.chat` | 1 | `channel_id`, `kind` |
//! | `live.gift` | coins | `channel_id`, `tier`, `net` |
//!
//! `viewer` is the account ID, or `guest:<keyed hash>` for anonymous
//! clients. The streamer's own heartbeats are not tracked.
use tokuly_core::prelude::*;
use tokuly_proto::live::{RealtimeStats, StreamAnalytics, StreamReport, ViewerSample};

use crate::{
    gifts,
    streams::{self, StreamRow},
};

fn unavailable(error: clickhouse::error::Error) -> AppError {
    AppError::unavailable("ANALYTICS_UNAVAILABLE", format!("analytics query failed: {error}"))
}

/// Seconds a stream was (or has been) on air.
pub fn seconds_on_air(stream: &StreamRow) -> i64 {
    match stream.started_at {
        Some(started) => (stream.ended_at.unwrap_or_else(now) - started).num_seconds().max(0),
        None => 0,
    }
}

async fn samples(core: &Core, stream_id: Uuid, last_minutes: Option<i32>) -> AppResult<Vec<ViewerSample>> {
    let rows: Vec<(Time, i64)> = sqlx::query_as(
        "SELECT at, viewers FROM live_viewer_samples
         WHERE stream_id = $1 AND ($2::int IS NULL OR at > now() - make_interval(mins => $2)) ORDER BY at",
    )
    .bind(stream_id)
    .bind(last_minutes)
    .fetch_all(&core.db)
    .await?;
    Ok(rows.into_iter().map(|(at, viewers)| ViewerSample { at: ts(at), viewers, ..Default::default() }).collect())
}

/// The dashboard of a stream.
pub async fn realtime(core: &Core, stream: &StreamRow) -> AppResult<RealtimeStats> {
    let per_minute: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM live_chat_messages WHERE stream_id = $1 AND kind <> 4 AND created_at > now() - interval '1 minute'",
    )
    .bind(stream.id)
    .fetch_one(&core.db)
    .await?;
    Ok(RealtimeStats {
        status: stream.status.into(),
        encoder_connected: stream.encoder_connected,
        viewers: if stream.is_live() { stream.viewer_count } else { 0 },
        peak_viewers: stream.peak_viewer_count,
        uptime_seconds: seconds_on_air(stream),
        like_count: stream.like_count,
        chat_message_count: stream.chat_message_count,
        chat_messages_per_minute: per_minute,
        gift_coins: stream.gift_coins,
        gift_count: stream.gift_count,
        new_subscribers: stream.new_subscribers,
        viewer_samples: samples(core, stream.id, Some(30)).await?,
        ..Default::default()
    })
}

/// The headline numbers of streams, in order.
pub async fn reports(core: &Core, rows: &[StreamRow]) -> AppResult<Vec<StreamReport>> {
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let averages: Vec<(Uuid, f64)> = sqlx::query_as(
        "SELECT stream_id, avg(viewers)::float8 FROM live_viewer_samples WHERE stream_id = ANY($1) GROUP BY stream_id",
    )
    .bind(&ids)
    .fetch_all(&core.db)
    .await?;
    let averages: std::collections::HashMap<Uuid, f64> = averages.into_iter().collect();
    let summaries = streams::summaries(core, rows).await?;
    Ok(rows
        .iter()
        .zip(summaries)
        .map(|(row, summary)| StreamReport {
            stream: Some(summary).into(),
            duration_seconds: seconds_on_air(row),
            peak_viewers: row.peak_viewer_count,
            average_viewers: averages.get(&row.id).copied().unwrap_or(0.0),
            chat_message_count: row.chat_message_count,
            new_subscribers: row.new_subscribers,
            gift_coins: row.gift_coins,
            gift_net_coins: row.gift_net_coins,
            gift_count: row.gift_count,
            like_count: row.like_count,
            ..Default::default()
        })
        .collect())
}

/// The full report of a stream.
pub async fn stream_analytics(core: &Core, stream: &StreamRow) -> AppResult<StreamAnalytics> {
    let report = reports(core, std::slice::from_ref(stream)).await?.pop().unwrap_or_default();
    let (unique_viewers, watch_seconds, unique_chatters): (u64, f64, u64) = core
        .analytics
        .ch
        .query(
            "SELECT toUInt64(uniqExactIf(JSONExtractString(props, 'viewer'), name IN ('live.view', 'live.watch'))),
                    toFloat64(sumIf(value, name = 'live.watch')),
                    toUInt64(uniqExactIf(user_id, name = 'live.chat'))
             FROM events
             WHERE owner_id = toUUID(?) AND name IN ('live.view', 'live.watch', 'live.chat')
               AND subject_type = 'live_stream' AND subject_id = toUUID(?)",
        )
        .bind(stream.owner_id.to_string())
        .bind(stream.id.to_string())
        .fetch_one()
        .await
        .map_err(unavailable)?;
    Ok(StreamAnalytics {
        report: Some(report).into(),
        unique_viewers: unique_viewers as i64,
        watch_seconds: watch_seconds.round() as i64,
        average_watch_seconds: if unique_viewers > 0 { watch_seconds / unique_viewers as f64 } else { 0.0 },
        unique_chatters: unique_chatters as i64,
        viewer_samples: samples(core, stream.id, None).await?,
        top_supporters: gifts::top_supporters(core, stream.id, 5).await?,
        ..Default::default()
    })
}
