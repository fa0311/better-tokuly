//! Background work of the video domain: reacting to other domains' events,
//! the subscriber notification fan-out, and the periodic workers.
//!
//! | Worker | Cadence | What |
//! |---|---|---|
//! | event consumer `video` | — | [`Video::handle_event`] |
//! | job `video.announce` | — | [`Video::announce`]: one batch of subscriber notifications |
//! | publishing | `publish_interval_seconds` | publishes scheduled videos that are due; every 20th run catches up videos whose processing result was missed |
//! | trending | `trending_interval_seconds` | recomputes trending scores |
//! | parties | 10 s | drops silent members, ends over-long parties, purges old ones |
use std::time::Duration;

use tokuly_core::{
    events::{self as core_events, ConsumeOptions, Event},
    jobs::JobOptions,
    outbound,
    prelude::*,
};
use tokuly_media::events::AssetEvent;
use tokuly_proto::common::SubjectType;
use tokuly_social::graph;

use crate::{
    Video, access,
    channels::{self, ChannelRow, channel_columns},
    events::{
        ASSET_DELETED, ASSET_FAILED, ASSET_READY, AnnounceJob, CONTENT_REMOVED, ContentRemoved, JOB_ANNOUNCE,
        USER_BANNED, USER_BLOCKED, USER_DELETED, USER_UNBANNED, UserBlocked, UserRef, VIDEO_PUBLISHED, VideoEvent,
    },
    feeds, party, playlists, search,
    videos::{self, VideoRow, video_columns},
};

/// How often the party worker runs.
const PARTY_SWEEP_INTERVAL: Duration = Duration::from_secs(10);
/// Every this many publishing runs, missed processing results are caught up.
const RECONCILE_EVERY: u64 = 20;
/// A subscriber "keeps watching" a channel if they watched it this recently.
const ENGAGED_WINDOW: &str = "30 days";

impl Video {
    /// Starts the workers listed in the module documentation. Call once, in
    /// worker processes.
    pub fn spawn_workers(&self) {
        let video = self.clone();
        tokio::spawn(async move {
            let handler = video.clone();
            core_events::consume(
                video.core.bus.clone(),
                "video",
                &[
                    ASSET_READY,
                    ASSET_FAILED,
                    ASSET_DELETED,
                    USER_DELETED,
                    USER_BANNED,
                    USER_UNBANNED,
                    USER_BLOCKED,
                    CONTENT_REMOVED,
                    VIDEO_PUBLISHED,
                ],
                // Deleting a creator's whole catalogue takes a while.
                ConsumeOptions { ack_wait: Duration::from_secs(300), ..Default::default() },
                video.core.shutdown.clone(),
                move |event| {
                    let video = handler.clone();
                    async move { video.handle_event(event).await }
                },
            )
            .await;
        });

        let video = self.clone();
        tokio::spawn(async move {
            let handler = video.clone();
            video
                .core
                .jobs
                .run(
                    JOB_ANNOUNCE,
                    JobOptions { concurrency: 4, lease: Duration::from_secs(120), max_deliver: 5 },
                    video.core.shutdown.clone(),
                    move |job: AnnounceJob| {
                        let video = handler.clone();
                        async move { video.announce(job).await.map(|_| ()) }
                    },
                )
                .await;
        });

        let video = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(video.cfg.publish_interval_seconds.max(1)));
            let mut runs = 0u64;
            loop {
                tokio::select! {
                    _ = video.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        match videos::publish_due(&video.core).await {
                            Ok(0) => {}
                            Ok(count) => tracing::info!(count, "published scheduled videos"),
                            Err(error) => tracing::error!(%error, "publishing scheduled videos failed"),
                        }
                        runs += 1;
                        if runs.is_multiple_of(RECONCILE_EVERY) {
                            match videos::reconcile_processing(&video.core).await {
                                Ok(0) => {}
                                Ok(count) => tracing::info!(count, "caught up videos with their processed uploads"),
                                Err(error) => tracing::error!(%error, "catching up processed uploads failed"),
                            }
                        }
                    }
                }
            }
        });

        let video = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(video.cfg.trending_interval_seconds.max(10)));
            loop {
                tokio::select! {
                    _ = video.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(error) = feeds::refresh_trending(&video.core, video.cfg.trending_half_life_hours).await {
                            tracing::error!(%error, "trending refresh failed");
                        }
                    }
                }
            }
        });

        let video = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(PARTY_SWEEP_INTERVAL);
            loop {
                tokio::select! {
                    _ = video.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        match party::sweep(&video.core, video.cfg.party_member_timeout_seconds, video.cfg.party_max_hours).await {
                            Ok(sweep) if sweep == party::Sweep::default() => {}
                            Ok(sweep) => tracing::debug!(?sweep, "watch party sweep"),
                            Err(error) => tracing::error!(%error, "watch party sweep failed"),
                        }
                    }
                }
            }
        });
    }

    /// Dispatches a consumed event. Every reaction is idempotent. Public so
    /// tests can deliver events without a running relay.
    pub async fn handle_event(&self, event: Event) -> AppResult<()> {
        match event.topic.as_str() {
            ASSET_READY | ASSET_FAILED => self.on_asset_processed(event.data::<AssetEvent>()?.asset_id).await,
            ASSET_DELETED => self.on_asset_deleted(event.data::<AssetEvent>()?.asset_id).await,
            USER_DELETED => self.on_user_deleted(event.data::<UserRef>()?.user_id).await,
            USER_BANNED => self.on_user_suspended(event.data::<UserRef>()?.user_id, true).await,
            USER_UNBANNED => self.on_user_suspended(event.data::<UserRef>()?.user_id, false).await,
            USER_BLOCKED => {
                let blocked: UserBlocked = event.data()?;
                self.on_user_blocked(blocked.user_id, blocked.blocked_id).await
            }
            CONTENT_REMOVED => self.on_content_removed(event.data()?).await,
            VIDEO_PUBLISHED => self.on_video_published(event.data()?).await,
            _ => Ok(()),
        }
    }

    /// An upload finished processing (or failed): move the video that plays it.
    async fn on_asset_processed(&self, asset_id: Uuid) -> AppResult<()> {
        let video: Option<VideoRow> =
            sqlx::query_as(concat!("SELECT ", video_columns!(), " FROM videos v WHERE v.asset_id = $1"))
                .bind(asset_id)
                .fetch_optional(&self.core.db)
                .await?;
        match video {
            Some(video) => videos::sync_processing(&self.core, &video).await,
            None => Ok(()),
        }
    }

    /// An upload was deleted under us (a takedown of the file, or the media
    /// domain cleaning up): what showed it goes with it.
    async fn on_asset_deleted(&self, asset_id: Uuid) -> AppResult<()> {
        let video: Option<VideoRow> =
            sqlx::query_as(concat!("SELECT ", video_columns!(), " FROM videos v WHERE v.asset_id = $1"))
                .bind(asset_id)
                .fetch_optional(&self.core.db)
                .await?;
        if let Some(video) = video {
            videos::delete(&self.core, &video, None, false).await?;
        }
        // A custom thumbnail falls back to the poster frame; channel art is
        // cleared. Search documents that showed the picture show the new one.
        let mut tx = self.core.db.begin().await?;
        let videos: Vec<Uuid> = sqlx::query_scalar(
            "UPDATE videos SET thumbnail_asset_id = NULL, thumbnail = playback -> 'poster' WHERE thumbnail_asset_id = $1
             RETURNING id",
        )
        .bind(asset_id)
        .fetch_all(&mut *tx)
        .await?;
        for video_id in videos {
            let Some(video) = videos::find_in(&mut tx, video_id).await? else { continue };
            if !video.is_public() {
                continue;
            }
            if let Some(channel) = channels::find_in(&mut tx, video.channel_id).await? {
                search::sync_video(&mut tx, true, &video, &channel).await?;
            }
            playlists::sync_search_of_video_in(&mut tx, video.id).await?;
        }
        let iconless: Vec<Uuid> = sqlx::query_scalar(
            "UPDATE channels SET icon = NULL, icon_asset_id = NULL WHERE icon_asset_id = $1 RETURNING id",
        )
        .bind(asset_id)
        .fetch_all(&mut *tx)
        .await?;
        for channel_id in iconless {
            if let Some(channel) = channels::find_in(&mut tx, channel_id).await? {
                search::index_channel(&mut tx, &channel).await?;
            }
        }
        sqlx::query("UPDATE channels SET banner = NULL, banner_asset_id = NULL WHERE banner_asset_id = $1")
            .bind(asset_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// An account was erased: its channels with everything on them, its
    /// playlists, and every trace it left on other people's content.
    async fn on_user_deleted(&self, user_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        for channel in channels::owned_by(core, user_id).await? {
            channels::delete(core, &channel, None, false).await?;
        }

        // Seats in parties: leaving hands over hosting or ends the party.
        let seats: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT m.party_id, m.id FROM watch_party_members m JOIN watch_parties p ON p.id = m.party_id
             WHERE m.user_id = $1 AND p.ended_at IS NULL",
        )
        .bind(user_id)
        .fetch_all(&core.db)
        .await?;
        for (party_id, member_id) in seats {
            let mut tx = core.db.begin().await?;
            let Ok(open) = party::lock_open(&mut tx, party_id).await else { continue };
            let Some(member) = party::member_in(&mut tx, party_id, member_id).await? else { continue };
            let departure = party::remove_member_in(&mut tx, &open, &member).await?;
            tx.commit().await?;
            party::announce_departure(core, &departure, tokuly_proto::video::PartyLeaveReason::PARTY_LEAVE_REASON_LEFT)
                .await;
        }

        let mut tx = core.db.begin().await?;
        let playlist_ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM playlists WHERE owner_id = $1")
            .bind(user_id)
            .fetch_all(&mut *tx)
            .await?;
        for playlist_id in playlist_ids {
            playlists::delete_in(&mut tx, playlist_id, user_id, None).await?;
        }
        // Counters on other people's channels and videos lose this user's share.
        sqlx::query(
            "UPDATE channels c SET subscriber_count = GREATEST(c.subscriber_count - 1, 0)
             FROM channel_subscriptions s WHERE s.channel_id = c.id AND s.user_id = $1",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE videos v SET like_count = GREATEST(v.like_count - (r.rating = 2)::int, 0),
                                 dislike_count = GREATEST(v.dislike_count - (r.rating = 3)::int, 0)
             FROM video_reactions r WHERE r.video_id = v.id AND r.user_id = $1",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        for table in [
            "channel_subscriptions",
            "video_reactions",
            "watch_history",
            "video_user_settings",
            "video_feed_hidden",
            "watch_party_bans",
            "watch_party_messages",
            "watch_party_members",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE user_id = $1")))
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// An account was suspended or reinstated: its channels and everything
    /// on them disappear from (or return to) everyone else's view and search.
    async fn on_user_suspended(&self, user_id: Uuid, suspended: bool) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        sqlx::query("UPDATE channels SET suspended = $2 WHERE owner_id = $1")
            .bind(user_id)
            .bind(suspended)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE videos SET suspended = $2 WHERE owner_id = $1")
            .bind(user_id)
            .bind(suspended)
            .execute(&mut *tx)
            .await?;
        let owned: Vec<ChannelRow> =
            sqlx::query_as(concat!("SELECT ", channel_columns!(), " FROM channels c WHERE c.owner_id = $1"))
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?;
        for channel in &owned {
            search::index_channel(&mut tx, channel).await?;
            let public: Vec<VideoRow> = sqlx::query_as(concat!(
                "SELECT ",
                video_columns!(),
                " FROM videos v WHERE v.channel_id = $1 AND v.status = 3 AND v.visibility = 1"
            ))
            .bind(channel.id)
            .fetch_all(&mut *tx)
            .await?;
            for video in &public {
                // `was_public` forces the write: the document is rewritten or removed.
                search::sync_video(&mut tx, true, video, channel).await?;
                // Other people's playlists show it as their picture, or stop to.
                playlists::sync_search_of_video_in(&mut tx, video.id).await?;
            }
        }
        let listed: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM playlists WHERE owner_id = $1 AND kind = 1 AND visibility = 1")
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?;
        for playlist_id in listed {
            playlists::sync_search_in(&mut tx, true, playlist_id).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Two users blocked each other: neither stays subscribed to the other's channels.
    async fn on_user_blocked(&self, user_id: Uuid, blocked_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let subscriptions: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as(
            "SELECT s.user_id, s.channel_id, c.owner_id FROM channel_subscriptions s JOIN channels c ON c.id = s.channel_id
             WHERE (s.user_id = $1 AND c.owner_id = $2) OR (s.user_id = $2 AND c.owner_id = $1)",
        )
        .bind(user_id)
        .bind(blocked_id)
        .fetch_all(&mut *tx)
        .await?;
        for (subscriber, channel_id, owner_id) in subscriptions {
            channels::remove_subscription_in(&mut tx, subscriber, channel_id, owner_id).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// A moderator took content down. The admin domain notifies the owner.
    async fn on_content_removed(&self, removed: ContentRemoved) -> AppResult<()> {
        let core = &self.core;
        if removed.subject_type == SubjectType::SUBJECT_TYPE_VIDEO as i32 {
            if let Some(video) = videos::find_by_id(core, removed.subject_id).await? {
                videos::delete(core, &video, None, true).await?;
            }
        } else if removed.subject_type == SubjectType::SUBJECT_TYPE_CHANNEL as i32 {
            if let Some(channel) = channels::find_by_id(core, removed.subject_id).await? {
                channels::delete(core, &channel, None, true).await?;
            }
        } else if removed.subject_type == SubjectType::SUBJECT_TYPE_PLAYLIST as i32
            && let Some(playlist) = playlists::find_by_id(core, removed.subject_id).await?
        {
            let mut tx = core.db.begin().await?;
            playlists::delete_in(&mut tx, playlist.id, playlist.owner_id, None).await?;
            tx.commit().await?;
        }
        Ok(())
    }

    /// A video reached its audience for the first time: start telling the
    /// channel's subscribers.
    async fn on_video_published(&self, published: VideoEvent) -> AppResult<()> {
        if !published.first_publication {
            return Ok(());
        }
        let job = AnnounceJob { video_id: published.video_id, after: None };
        self.core.jobs.enqueue_unique(JOB_ANNOUNCE, &format!("{}:start", published.video_id), &job).await
    }

    /// Handles one `video.announce` job: notifies the next batch of the
    /// channel's subscribers about a newly published video and queues the
    /// batch after it. Returns how many notifications were requested.
    ///
    /// Who hears what: level ALL gets inbox and push; PERSONALIZED gets the
    /// inbox entry, and the push only if they watched the channel recently;
    /// NONE gets nothing. A friends-only video is announced to subscribers
    /// who are friends of the owner. Notifications of one video share a
    /// collapse key, so a batch that is retried does not double up.
    pub async fn announce(&self, job: AnnounceJob) -> AppResult<u64> {
        let core = &self.core;
        let Some(video) = videos::find_by_id(core, job.video_id).await? else { return Ok(0) };
        let audience_exists =
            video.is_published() && !video.suspended && matches!(video.visibility, access::PUBLIC | access::FRIENDS);
        if !audience_exists || !video.notify_subscribers {
            return Ok(0);
        }
        let Some(channel) = channels::find_by_id(core, video.channel_id).await? else { return Ok(0) };
        let batch_size = self.cfg.announce_batch_size;
        let batch: Vec<(Uuid, i32, bool)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT s.user_id, s.notify_level,
                    EXISTS (SELECT 1 FROM watch_history h JOIN videos w ON w.id = h.video_id
                            WHERE h.user_id = s.user_id AND w.channel_id = s.channel_id
                              AND h.last_watched_at > now() - interval '{ENGAGED_WINDOW}')
             FROM channel_subscriptions s
             WHERE s.channel_id = $1 AND s.notify_level >= 2 AND ($2::uuid IS NULL OR s.user_id > $2)
             ORDER BY s.user_id
             LIMIT $3"
        )))
        .bind(channel.id)
        .bind(job.after)
        .bind(batch_size)
        .fetch_all(&core.db)
        .await?;
        let Some(last) = batch.last().map(|(user_id, _, _)| *user_id) else { return Ok(0) };

        let hidden = graph::blocked_ids_either(core, video.owner_id).await?;
        let friends = if video.visibility == access::FRIENDS {
            Some(graph::friend_ids(core, video.owner_id).await?)
        } else {
            None
        };
        let mut tx = core.db.begin().await?;
        let mut notified = 0;
        for (user_id, level, engaged) in &batch {
            if hidden.contains(user_id) || friends.as_ref().is_some_and(|friends| !friends.contains(user_id)) {
                continue;
            }
            // No actor: the notification is from the channel, which does not
            // reveal the account behind it.
            let mut request = outbound::NotificationRequest::new(
                *user_id,
                "video.published",
                outbound::Localized::same(channel.name.clone()),
                outbound::Localized::new(
                    format!("新しい動画を公開したよ!「{}」", video.title),
                    format!("New video: {}", video.title),
                ),
                video.path(),
            )
            .subject(SubjectType::SUBJECT_TYPE_VIDEO, video.id)
            .collapse(format!("video.published:{}", video.id))
            .data(serde_json::json!({
                "video_id": video.id,
                "short_id": video.short_id,
                "channel_id": channel.id,
                "channel_handle": channel.handle,
                "channel_name": channel.name,
                "channel_icon_url": channel.icon_url(),
            }));
            if let Some(url) = video.thumbnail_url() {
                request = request.image(url);
            }
            let push = *level == channels::NotifyLevel::All as i32 || *engaged;
            if !push {
                request = request.inbox_only();
            }
            outbound::notify(&mut *tx, &request).await?;
            notified += 1;
        }
        tx.commit().await?;
        if batch.len() as i64 == batch_size {
            let next = AnnounceJob { video_id: video.id, after: Some(last) };
            core.jobs.enqueue_unique(JOB_ANNOUNCE, &format!("{}:{last}", video.id), &next).await?;
        }
        Ok(notified)
    }
}
