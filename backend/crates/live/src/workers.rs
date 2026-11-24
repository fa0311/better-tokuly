//! Background work of the live domain: reacting to events, the jobs and
//! the timers.
//!
//! | Worker | Cadence | What |
//! |---|---|---|
//! | event consumer `live` | — | [`Live::handle_event`] |
//! | job `live.announce` | — | [`Live::announce`]: one batch of "went live" notifications |
//! | job `live.archive` | — | [`Live::archive`]: recording → asset → video |
//! | job `live.clip` | — | [`Live::cut_clip`]: cuts a clip |
//! | reconciler | `reconcile_interval_seconds` | [`Live::reconcile`]: follows the media server, ends abandoned streams, samples viewers |
//! | polls | 1 s | [`Live::tick_polls`]: closes polls whose time is up, pushes held-back vote counts |
//! | recordings | 5 min | [`Live::sweep_recordings`] |
//!
//! The reconciler holds a short lock in Valkey, so several worker processes
//! take turns instead of all asking the media server.
use std::time::Duration;

use tokuly_core::{
    events::{self as core_events, ConsumeOptions, Event},
    jobs::JobOptions,
    prelude::*,
};
use tokuly_media::events::AssetEvent;
use tokuly_proto::{
    common::SubjectType,
    live::{Alert, AlertKind, EndReason, OverlayKind, live_event},
};
use tokuly_video::events::{ChannelEvent, SubscriptionEvent, VideoEvent};

use crate::{
    Live, chat, clips,
    events::{
        ASSET_DELETED, ASSET_FAILED, ASSET_READY, AnnounceJob, ArchiveJob, CHANNEL_DELETED, CHANNEL_SUBSCRIBED,
        CIRCLE_DELETED, CIRCLE_MEMBERS_CHANGED, CONTENT_REMOVED, CircleChanged, ClipJob, ContentRemoved,
        FOLLOW_REMOVED, FOLLOW_SETTINGS_CHANGED, FRIEND_REMOVED, FollowRemoved, FriendRemoved, JOB_ANNOUNCE,
        JOB_ARCHIVE, JOB_CLIP, STREAM_ENDED, STREAM_STARTED, StreamEvent, USER_BANNED, USER_BLOCKED, USER_DELETED,
        USER_UNBANNED, UserBlocked, UserRef, VIDEO_DELETED, VIDEO_PUBLISHED, VIDEO_UNPUBLISHED,
        VIDEO_VISIBILITY_CHANGED,
    },
    overlays, search,
    streams::{self, ARCHIVE_FAILED, ARCHIVE_NONE, ARCHIVE_PENDING, Aftermath, StreamRow, stream_columns},
};

const POLL_TICK: Duration = Duration::from_secs(1);
const SWEEP_INTERVAL: Duration = Duration::from_secs(300);
const RECONCILE_LOCK: &str = "live:reconcile";
const RECONCILE_LOCK_TTL: Duration = Duration::from_secs(60);

impl Live {
    /// Starts the workers listed in the module documentation. Call once, in
    /// worker processes.
    pub fn spawn_workers(&self) {
        let live = self.clone();
        tokio::spawn(async move {
            let handler = live.clone();
            core_events::consume(
                live.core.bus.clone(),
                "live",
                &[
                    USER_DELETED,
                    USER_BANNED,
                    USER_UNBANNED,
                    USER_BLOCKED,
                    FOLLOW_SETTINGS_CHANGED,
                    FOLLOW_REMOVED,
                    CIRCLE_MEMBERS_CHANGED,
                    CIRCLE_DELETED,
                    FRIEND_REMOVED,
                    CONTENT_REMOVED,
                    ASSET_READY,
                    ASSET_FAILED,
                    ASSET_DELETED,
                    CHANNEL_DELETED,
                    CHANNEL_SUBSCRIBED,
                    VIDEO_PUBLISHED,
                    VIDEO_UNPUBLISHED,
                    VIDEO_VISIBILITY_CHANGED,
                    VIDEO_DELETED,
                    STREAM_STARTED,
                    STREAM_ENDED,
                ],
                ConsumeOptions { ack_wait: Duration::from_secs(120), ..Default::default() },
                live.core.shutdown.clone(),
                move |event| {
                    let live = handler.clone();
                    async move { live.handle_event(event).await }
                },
            )
            .await;
        });

        let live = self.clone();
        tokio::spawn(async move {
            let handler = live.clone();
            let options = JobOptions { concurrency: 4, lease: Duration::from_secs(120), max_deliver: 5 };
            live.core
                .jobs
                .run(JOB_ANNOUNCE, options, live.core.shutdown.clone(), move |job: AnnounceJob| {
                    let live = handler.clone();
                    async move { live.announce(job).await.map(|_| ()) }
                })
                .await;
        });

        let live = self.clone();
        tokio::spawn(async move {
            let handler = live.clone();
            // Joining hours of video takes a while; the lease is renewed while it runs.
            let options = JobOptions { concurrency: 2, lease: Duration::from_secs(300), max_deliver: 4 };
            live.core
                .jobs
                .run(JOB_ARCHIVE, options, live.core.shutdown.clone(), move |job: ArchiveJob| {
                    let live = handler.clone();
                    async move { live.archive(job).await }
                })
                .await;
        });

        let live = self.clone();
        tokio::spawn(async move {
            let handler = live.clone();
            let options = JobOptions { concurrency: 2, lease: Duration::from_secs(300), max_deliver: 3 };
            live.core
                .jobs
                .run(JOB_CLIP, options, live.core.shutdown.clone(), move |job: ClipJob| {
                    let live = handler.clone();
                    async move { live.cut_clip(job).await }
                })
                .await;
        });

        let live = self.clone();
        tokio::spawn(async move {
            let interval = Duration::from_secs(live.cfg.reconcile_interval_seconds.max(1));
            let mut tick = tokio::time::interval(interval);
            loop {
                tokio::select! {
                    _ = live.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        // One worker at a time: the lock is held until the
                        // run is over, so two looks at the media server are
                        // never applied in the wrong order. (It expires by
                        // itself if its holder dies.)
                        match live.core.cache.set_nx(RECONCILE_LOCK, "1", RECONCILE_LOCK_TTL.max(interval)).await {
                            Ok(true) => {}
                            Ok(false) => continue,
                            Err(error) => {
                                tracing::warn!(%error, "reconciler lock unavailable");
                                continue;
                            }
                        }
                        match live.reconcile().await {
                            Ok(report) if report == Default::default() => {}
                            Ok(report) => tracing::info!(?report, "live streams reconciled"),
                            Err(error) => tracing::error!(%error, "reconciling live streams failed"),
                        }
                        if let Err(error) = live.core.cache.del(RECONCILE_LOCK).await {
                            tracing::warn!(%error, "could not release the reconciler lock");
                        }
                    }
                }
            }
        });

        let live = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(POLL_TICK);
            loop {
                tokio::select! {
                    _ = live.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(error) = live.tick_polls().await {
                            tracing::error!(%error, "poll worker failed");
                        }
                    }
                }
            }
        });

        let live = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(SWEEP_INTERVAL);
            loop {
                tokio::select! {
                    _ = live.core.shutdown.cancelled() => break,
                    _ = tick.tick() => match live.sweep_recordings().await {
                        Ok(sweep) if sweep == Default::default() => {}
                        Ok(sweep) => tracing::info!(?sweep, "recordings swept"),
                        Err(error) => tracing::error!(%error, "sweeping recordings failed"),
                    },
                }
            }
        });
    }

    /// Dispatches a consumed event. Every reaction is idempotent. Public so
    /// tests can deliver events without a running relay.
    pub async fn handle_event(&self, event: Event) -> AppResult<()> {
        match event.topic.as_str() {
            ASSET_READY | ASSET_FAILED => self.sync_clip_asset(event.data::<AssetEvent>()?.asset_id).await,
            ASSET_DELETED => self.on_asset_deleted(event.data::<AssetEvent>()?.asset_id).await,
            USER_DELETED => self.on_user_deleted(event.data::<UserRef>()?.user_id).await,
            USER_BANNED | USER_UNBANNED => self.on_user_suspended(event.data::<UserRef>()?.user_id).await,
            USER_BLOCKED => {
                let blocked: UserBlocked = event.data()?;
                self.on_user_blocked(blocked.user_id, blocked.blocked_id).await
            }
            FRIEND_REMOVED => {
                let removed: FriendRemoved = event.data()?;
                self.cut_lost_viewers(&[removed.user_id, removed.friend_id]).await
            }
            FOLLOW_SETTINGS_CHANGED => self.sync_owner_lock(event.data::<UserRef>()?.user_id).await.map(drop),
            // A follow that ended, or a circle that lost members: followers,
            // close friends and circle streams may have lost viewers.
            FOLLOW_REMOVED => {
                let removed: FollowRemoved = event.data()?;
                self.cut_lost_viewers(&[removed.followee_id, removed.follower_id]).await
            }
            CIRCLE_MEMBERS_CHANGED | CIRCLE_DELETED => {
                self.cut_lost_viewers(&[event.data::<CircleChanged>()?.owner_id]).await
            }
            CONTENT_REMOVED => self.on_content_removed(event.data()?).await,
            CHANNEL_DELETED => self.on_channel_deleted(event.data::<ChannelEvent>()?.channel_id).await,
            CHANNEL_SUBSCRIBED => self.on_channel_subscribed(event.data()?).await,
            VIDEO_PUBLISHED | VIDEO_UNPUBLISHED | VIDEO_VISIBILITY_CHANGED => {
                self.reindex_clips_of_video(event.data::<VideoEvent>()?.video_id).await
            }
            VIDEO_DELETED => self.on_video_deleted(event.data::<VideoEvent>()?.video_id).await,
            STREAM_STARTED => self.start_announcing(event.data::<StreamEvent>()?.stream_id).await,
            STREAM_ENDED => self.on_stream_ended(event.data()?).await,
            _ => Ok(()),
        }
    }

    /// A stream is over: its archive job starts.
    async fn on_stream_ended(&self, ended: StreamEvent) -> AppResult<()> {
        let pending: bool =
            sqlx::query_scalar("SELECT COALESCE((SELECT archive_status = $2 FROM live_streams WHERE id = $1), false)")
                .bind(ended.stream_id)
                .bind(ARCHIVE_PENDING)
                .fetch_one(&self.core.db)
                .await?;
        if pending {
            let job = ArchiveJob { stream_id: ended.stream_id };
            self.core.jobs.enqueue_unique(JOB_ARCHIVE, &ended.stream_id.to_string(), &job).await?;
        }
        Ok(())
    }

    /// Ends a stream if it is on air and deletes it. With `erase` nothing
    /// is kept (an account is being erased); otherwise the row and its chat
    /// stay for the retention period, as for any deleted stream.
    async fn remove_stream(&self, stream: &StreamRow, erase: bool) -> AppResult<()> {
        self.finish_stream(stream.id, EndReason::END_REASON_REMOVED, None, None).await?;
        let mut tx = self.core.db.begin().await?;
        let aftermath = match streams::lock(&mut tx, stream.id).await? {
            Some(current) => streams::delete_in(&mut tx, &current, None).await?,
            // Deleted earlier: only the erasure is left to do.
            None => Aftermath::default(),
        };
        if erase {
            streams::erase_in(&mut tx, stream.id).await?;
        }
        tx.commit().await?;
        self.clean_up(aftermath).await;
        Ok(())
    }

    /// Streams by owner or channel, deleted ones included.
    async fn streams_where(&self, column: &str, id: Uuid) -> AppResult<Vec<StreamRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM live_streams s WHERE s.{column} = $1 ORDER BY s.id",
            stream_columns!()
        )))
        .bind(id)
        .fetch_all(&self.core.db)
        .await?)
    }

    async fn delete_clips_where(&self, condition: &str, id: Uuid, force: bool) -> AppResult<()> {
        let core = &self.core;
        let mut tx = core.db.begin().await?;
        let rows: Vec<clips::ClipRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM live_clips c WHERE {condition} FOR UPDATE",
            clips::clip_columns!()
        )))
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        let mut aftermath = Aftermath::default();
        for clip in &rows {
            aftermath.assets.extend(clips::delete_in(&mut tx, clip, None, force).await?);
        }
        tx.commit().await?;
        self.clean_up(aftermath).await;
        Ok(())
    }

    /// An upload was deleted under us: what showed it follows.
    async fn on_asset_deleted(&self, asset_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        sqlx::query(
            "UPDATE live_streams SET thumbnail = NULL, thumbnail_asset_id = NULL WHERE thumbnail_asset_id = $1",
        )
        .bind(asset_id)
        .execute(&core.db)
        .await?;
        // A recording's upload that is gone before its video exists cannot become one.
        sqlx::query(
            "UPDATE live_streams SET archive_asset_id = NULL,
                    archive_status = CASE WHEN archive_video_id IS NULL AND archive_status IN (2, 3) THEN $2 ELSE archive_status END,
                    archive_error = CASE WHEN archive_video_id IS NULL AND archive_status IN (2, 3)
                                         THEN 'the recording''s upload was deleted' ELSE archive_error END
             WHERE archive_asset_id = $1",
        )
        .bind(asset_id)
        .bind(ARCHIVE_FAILED)
        .execute(&core.db)
        .await?;
        self.delete_clips_where("c.asset_id = $1", asset_id, false).await
    }

    /// An account was erased: its streams with everything on them, the
    /// clips it cut, and every trace it left on other people's streams.
    /// Gifts stay as bookkeeping, without the name.
    async fn on_user_deleted(&self, user_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        for stream in self.streams_where("owner_id", user_id).await? {
            self.remove_stream(&stream, true).await?;
        }
        self.delete_clips_where("c.creator_id = $1 OR c.owner_id = $1", user_id, false).await?;

        // Their pinned messages go with their messages.
        sqlx::query(
            "UPDATE live_streams s SET pinned_message_id = NULL
             FROM live_chat_messages m WHERE m.id = s.pinned_message_id AND m.user_id = $1",
        )
        .bind(user_id)
        .execute(&core.db)
        .await?;
        let mut tx = core.db.begin().await?;
        for table in [
            "live_chat_messages",
            "live_reminders",
            "live_moderators",
            "live_chat_restrictions",
            "live_poll_votes",
            "live_stream_subscribers",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE user_id = $1")))
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        for table in ["live_overlays", "live_channel_settings"] {
            sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE owner_id = $1")))
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE live_chat_restrictions SET moderator_id = NULL WHERE moderator_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE live_polls SET created_by = NULL WHERE created_by = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        // A gift that was never paid for is nothing; paid ones stay as bookkeeping, without the name.
        sqlx::query("DELETE FROM live_gifts WHERE status IN (1, 3) AND (sender_id = $1 OR recipient_id = $1)")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE live_gifts SET sender_id = NULL WHERE sender_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE live_gifts SET recipient_id = NULL WHERE recipient_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// An account was suspended or reinstated: its streams disappear from
    /// (or return to) everyone else's view; one that is on air ends.
    ///
    /// The two events can arrive late or in the wrong order, so they only
    /// say "look again": what counts is whether the account is banned now.
    /// Applies whether `owner_id`'s account is locked to their streams: the
    /// copy in `live_streams.owner_locked`, search, one
    /// `live.stream.visibility_changed` per open stream whose reach changed,
    /// and — for a lock — the realtime subscriptions and media sessions of
    /// people who are no longer in the audience. The handler of
    /// `social.follow.settings_changed`. Idempotent; returns how many open
    /// streams changed reach.
    pub async fn sync_owner_lock(&self, owner_id: Uuid) -> AppResult<u64> {
        let mut changed = 0;
        // A lock and an unlock in quick succession can have their passes
        // cross: the last word is what the social domain says afterwards.
        loop {
            let locked = tokuly_video::access::is_locked_now(&self.core, owner_id).await?;
            changed += self.sync_lock_to(owner_id, locked).await?;
            if tokuly_video::access::is_locked_now(&self.core, owner_id).await? == locked {
                return Ok(changed);
            }
        }
    }

    /// One pass of [`Self::sync_owner_lock`]: brings every stream to `locked`.
    async fn sync_lock_to(&self, owner_id: Uuid, locked: bool) -> AppResult<u64> {
        let core = &self.core;
        // Everything from elsewhere is read before the transaction.
        let tied = streams::audience_members(core, owner_id, tokuly_social::graph::ContentAudience::Followers).await?;
        let owned: Vec<StreamRow> =
            self.streams_where("owner_id", owner_id).await?.into_iter().filter(|s| !s.is_deleted()).collect();
        let mut channels_of = std::collections::HashMap::new();
        for stream in owned.iter().filter(|stream| stream.owner_locked != locked) {
            if !channels_of.contains_key(&stream.channel_id) {
                channels_of
                    .insert(stream.channel_id, tokuly_video::channels::find_by_id(core, stream.channel_id).await?);
            }
        }
        let mut tx = core.db.begin().await?;
        let mut changed = Vec::new();
        for stream in &owned {
            let Some(current) = streams::lock(&mut tx, stream.id).await? else { continue };
            if current.owner_locked == locked {
                continue;
            }
            sqlx::query("UPDATE live_streams SET owner_locked = $2 WHERE id = $1")
                .bind(current.id)
                .bind(locked)
                .execute(&mut *tx)
                .await?;
            let updated = StreamRow { owner_locked: locked, ..current };
            let channel = channels_of.get(&updated.channel_id).and_then(Option::as_ref);
            search::sync_stream(&mut tx, &updated, channel).await?;
            if crate::access::is_open(updated.visibility) {
                let visible_to = if locked {
                    tied.clone().map(|members| tokuly_core::events::bounded_audience(members, None))
                } else {
                    None
                };
                streams::emit_visibility_changed_in(&mut tx, &updated, visible_to).await?;
                changed.push(updated);
            }
        }
        tx.commit().await?;
        // Clips of these streams leave (or rejoin) search with them.
        for stream in &changed {
            self.reindex_clips_of_stream(stream.id).await?;
        }
        if locked && !changed.is_empty() {
            // Strangers stop following and watching the streams at once.
            for stream in &changed {
                core.rt.recheck_topic(&stream.topic()).await;
            }
            self.access_changed(&[owner_id]).await?;
            for stream in changed.iter().filter(|stream| stream.is_live()) {
                if let Err(error) = self.enforce_access(stream, true).await {
                    tracing::warn!(stream_id = %stream.id, %error, "could not cut viewers after a lock");
                }
            }
        }
        Ok(changed.len() as u64)
    }

    async fn on_user_suspended(&self, user_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        let suspended: bool =
            sqlx::query_scalar("SELECT COALESCE((SELECT banned_at IS NOT NULL FROM users WHERE id = $1), false)")
                .bind(user_id)
                .fetch_one(&core.db)
                .await?;
        let owned: Vec<StreamRow> =
            self.streams_where("owner_id", user_id).await?.into_iter().filter(|s| !s.is_deleted()).collect();
        // Read before the transaction, which then needs no second connection.
        let mut channels_of = std::collections::HashMap::new();
        for stream in &owned {
            if !channels_of.contains_key(&stream.channel_id) {
                channels_of
                    .insert(stream.channel_id, tokuly_video::channels::find_by_id(core, stream.channel_id).await?);
            }
        }
        if suspended {
            for stream in owned.iter().filter(|stream| stream.is_live()) {
                self.finish_stream(stream.id, EndReason::END_REASON_REMOVED, None, None).await?;
            }
        }
        let mut tx = core.db.begin().await?;
        sqlx::query("UPDATE live_streams SET suspended = $2 WHERE owner_id = $1")
            .bind(user_id)
            .bind(suspended)
            .execute(&mut *tx)
            .await?;
        for stream in &owned {
            if let Some(current) = streams::lock(&mut tx, stream.id).await? {
                let channel = channels_of.get(&stream.channel_id).and_then(Option::as_ref);
                search::sync_stream(&mut tx, &current, channel).await?;
            }
        }
        tx.commit().await?;
        if suspended {
            // An encoder still connected to an upcoming stream is sent away
            // by the reconciler (the stream is suspended now).
            for stream in owned.iter().filter(|stream| stream.encoder_connected && !stream.is_live()) {
                self.kick_quietly(stream).await;
            }
            // Nobody but the owner may follow a suspended stream.
            for stream in owned.iter().filter(|stream| !stream.is_deleted()) {
                core.rt.recheck_topic(&stream.topic()).await;
            }
        }
        Ok(())
    }

    /// Two users blocked each other: neither moderates the other's chats
    /// nor waits for the other's streams. (That a blocked user cannot write
    /// in the blocker's chat is checked when they try.)
    async fn on_user_blocked(&self, user_id: Uuid, blocked_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        for (owner, other) in [(user_id, blocked_id), (blocked_id, user_id)] {
            sqlx::query(
                "DELETE FROM live_moderators m
                 WHERE m.user_id = $2 AND m.channel_id IN (SELECT DISTINCT channel_id FROM live_streams WHERE owner_id = $1
                                                           UNION SELECT channel_id FROM live_channel_settings WHERE owner_id = $1
                                                           UNION SELECT channel_id FROM live_overlays WHERE owner_id = $1)",
            )
            .bind(owner)
            .bind(other)
            .execute(&core.db)
            .await?;
            sqlx::query(
                "DELETE FROM live_reminders r USING live_streams s WHERE s.id = r.stream_id AND s.owner_id = $1 AND r.user_id = $2",
            )
            .bind(owner)
            .bind(other)
            .execute(&core.db)
            .await?;
        }
        self.cut_lost_viewers(&[user_id, blocked_id]).await
    }

    /// Two people stopped being friends or blocked each other: on the
    /// restricted streams either of them has on air, whoever may no longer
    /// watch is cut off now instead of when their playback token runs out.
    async fn cut_lost_viewers(&self, owners: &[Uuid]) -> AppResult<()> {
        // First of all the hook stops trusting what it remembers about these owners' streams.
        self.access_changed(owners).await?;
        // Each of them may follow streams of the other (a block hides them
        // from each other, a friends stream needs the friendship): the
        // gateway asks the guard again about their `live:` topics.
        for owner in owners {
            self.core.rt.revoke_prefix(*owner, "live").await;
        }
        if !self.cfg.media_server_configured() {
            return Ok(());
        }
        let on_air: Vec<StreamRow> = sqlx::query_as(concat!(
            "SELECT ",
            stream_columns!(),
            " FROM live_streams s WHERE s.owner_id = ANY($1) AND s.status = 2 AND ",
            crate::access::restricted!()
        ))
        .bind(owners)
        .fetch_all(&self.core.db)
        .await?;
        for stream in &on_air {
            // The hook already refuses them; a media server that does not answer right now is asked
            // again by the reconciler's regular pass.
            if let Err(error) = self.enforce_access(stream, true).await {
                tracing::warn!(stream_id = %stream.id, %error, "could not cut viewers who lost access");
            }
        }
        Ok(())
    }

    /// A moderator took content down. The admin domain notifies the owner.
    async fn on_content_removed(&self, removed: ContentRemoved) -> AppResult<()> {
        let core = &self.core;
        if removed.subject_type == SubjectType::SUBJECT_TYPE_LIVE_STREAM as i32 {
            if let Some(stream) = streams::find_by_id(core, removed.subject_id).await? {
                self.remove_stream(&stream, false).await?;
            }
        } else if removed.subject_type == SubjectType::SUBJECT_TYPE_CLIP as i32 {
            self.delete_clips_where("c.id = $1", removed.subject_id, true).await?;
        } else if removed.subject_type == SubjectType::SUBJECT_TYPE_LIVE_CHAT_MESSAGE as i32
            && let Some(message) = chat::find(core, removed.subject_id).await?
            && let Some(stream) = streams::find_by_id(core, message.stream_id).await?
        {
            self.remove_chat_message(&stream, &message).await?;
        }
        Ok(())
    }

    /// A channel is gone: its streams, clips, widgets and chat rules follow.
    async fn on_channel_deleted(&self, channel_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        for stream in self.streams_where("channel_id", channel_id).await?.iter().filter(|stream| !stream.is_deleted()) {
            self.remove_stream(stream, false).await?;
        }
        self.delete_clips_where("c.channel_id = $1", channel_id, false).await?;
        for table in ["live_overlays", "live_moderators", "live_chat_restrictions", "live_channel_settings"] {
            sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE channel_id = $1")))
                .bind(channel_id)
                .execute(&core.db)
                .await?;
        }
        Ok(())
    }

    /// Someone subscribed to a channel: its stream on air counts them, its
    /// alert boxes announce them and its subscriber goals move.
    async fn on_channel_subscribed(&self, subscribed: SubscriptionEvent) -> AppResult<()> {
        let core = &self.core;
        // Counted and announced once per person and stream: the event may
        // be delivered twice, and someone who subscribes, leaves and
        // subscribes again must not get an alert on air each time.
        let first =
            match streams::current_of_channel(core, subscribed.channel_id).await?.filter(|stream| stream.is_live()) {
                None => false,
                Some(stream) => {
                    let mut tx = core.db.begin().await?;
                    let first = sqlx::query(
                    "INSERT INTO live_stream_subscribers (stream_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                )
                .bind(stream.id)
                .bind(subscribed.user_id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                    == 1;
                    if first {
                        sqlx::query("UPDATE live_streams SET new_subscribers = new_subscribers + 1 WHERE id = $1")
                            .bind(stream.id)
                            .execute(&mut *tx)
                            .await?;
                    }
                    tx.commit().await?;
                    first
                }
            };
        // Subscriber goals follow the channel's count whether or not it is on air.
        overlays::push_goal_progress(self, subscribed.channel_id).await;
        let has_alert_box = overlays::topics(self, subscribed.channel_id)
            .await?
            .iter()
            .any(|(kind, _)| *kind == overlays::KIND_ALERT_BOX);
        if first && has_alert_box {
            let user = tokuly_identity::summary(core, subscribed.user_id).await?;
            let alert = Alert {
                kind: AlertKind::ALERT_KIND_SUBSCRIBER.into(),
                name: if user.display_name.is_empty() { user.handle } else { user.display_name },
                avatar: user.avatar,
                ..Default::default()
            };
            self.push_to_overlays(
                subscribed.channel_id,
                &[OverlayKind::OVERLAY_KIND_ALERT_BOX],
                live_event::Event::Alert(Box::new(alert)),
            )
            .await;
        }
        Ok(())
    }

    /// A video is gone: clips cut from it go with it, and a stream it was
    /// the archive of has no archive any more.
    async fn on_video_deleted(&self, video_id: Uuid) -> AppResult<()> {
        sqlx::query(
            "UPDATE live_streams SET archive_video_id = NULL, archive_asset_id = NULL, archive_status = $2, archive_changed_at = now()
             WHERE archive_video_id = $1",
        )
        .bind(video_id)
        .bind(ARCHIVE_NONE)
        .execute(&self.core.db)
        .await?;
        self.delete_clips_where("c.video_id = $1", video_id, false).await
    }
}
