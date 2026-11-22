//! Background work of the shorts domain: reacting to other domains'
//! events, the jobs, and the periodic workers.
//!
//! | Worker | Cadence | What |
//! |---|---|---|
//! | event consumer `shorts` | — | [`Shorts::handle_event`] |
//! | job `shorts.announce` | — | [`Shorts::announce`]: subscriber notifications for a new short |
//! | job `shorts.cover` | — | [`Shorts::cut_cover`]: a cover frame |
//! | job `shorts.reindex` | — | [`Shorts::reindex`]: republishes search documents |
//! | flush | `flush_interval_seconds` | buffered playback counters to PostgreSQL ([`Shorts::flush_all_signals`]) |
//! | ranking | `ranking_interval_seconds` | [`Shorts::run_ranking`]; every 5th run [`Shorts::reconcile`]; hourly [`Shorts::housekeeping`] |
//!
//! # Events arrive at least once, and not always in order
//!
//! Every handler gives the same result when its event comes twice, late,
//! or after the row it is about is gone. Three kinds of handler:
//!
//! - *State the truth.* Where the current state can be read, the event is
//!   only a hint that it changed: a ban or its lifting reads the account,
//!   a subscription or its end asks the video domain, an asset event reads
//!   the asset. Two events in the wrong order end in the right state.
//! - *Last change wins.* A like's state lives in another domain and cannot
//!   be read, so the copy keeps the time of the latest change it has seen
//!   and ignores older ones ([`Shorts::mirror_like`]).
//! - *Count once.* What adds to a number is claimed first: a marker per
//!   account for ranking signals ([`Shorts::count_signal`]), a durable
//!   cursor for the subscriber notifications ([`Shorts::announce`]).
use std::{collections::HashMap, time::Duration};

use tokuly_core::{
    events::{self as core_events, ConsumeOptions, Event},
    jobs::JobOptions,
    outbound,
    prelude::*,
};
use tokuly_media::events::AssetEvent;
use tokuly_proto::common::SubjectType;
use tokuly_social::graph;
use tokuly_video::channels::{self, ChannelRow, NotifyLevel};

use crate::{
    Shorts,
    access::{self, FRIENDS, PUBLIC},
    events::{
        ASSET_DELETED, ASSET_FAILED, ASSET_READY, AnnounceJob, CHANNEL_DELETED, CHANNEL_SUBSCRIBED,
        CHANNEL_UNSUBSCRIBED, COMMENT_PUBLISHED, CONTENT_REMOVED, ChannelDeleted, ChannelSubscribed, CommentPublished,
        ContentRemoved, CoverJob, JOB_ANNOUNCE, JOB_COVER, JOB_REINDEX, REACTION_CHANGED, REINDEX_REQUESTED,
        REPORT_CREATED, ReactionChanged, ReindexJob, ReindexRequested, ReportCreated, SHORT_PUBLISHED, ShortEvent,
        USER_BANNED, USER_BLOCKED, USER_DELETED, USER_UNBANNED, UserBlocked, UserRef,
    },
    hashtags, listing, ranking,
    shorts::{self, Removal, ShortRow, short_columns},
    signals::{self, Engagement},
};

/// Every this many ranking runs, missed processing results are caught up.
const RECONCILE_EVERY: u64 = 5;
/// "Not interested" marks stop mattering after 90 days and are deleted
/// after this many.
const HIDDEN_RETENTION_DAYS: i32 = 180;
/// How long a like that was taken back is remembered, so that the event of
/// the like itself cannot arrive late and bring it back.
const UNLIKE_MEMORY_DAYS: i32 = 7;
/// A subscriber "keeps watching" a channel's shorts if they liked or saved
/// one this recently.
const ENGAGED_DAYS: i32 = 30;
/// Batches of subscribers one `shorts.announce` job handles before it
/// queues its successor.
const ANNOUNCE_BATCHES_PER_JOB: usize = 20;
/// Search documents republished per `shorts.reindex` job.
const REINDEX_BATCH: i64 = 500;
/// The engagement domain's number for a like (`ReactionKind`).
const REACTION_LIKE: i32 = 1;
/// `announce_cursor` of a short whose subscribers are being told, before
/// the first of them has been, and of one that is finished with.
const ANNOUNCE_STARTED: Uuid = Uuid::nil();
const ANNOUNCE_FINISHED: Uuid = Uuid::max();

impl Shorts {
    /// Starts the workers listed in the module documentation. Call once, in
    /// worker processes.
    pub fn spawn_workers(&self) {
        let shorts = self.clone();
        tokio::spawn(async move {
            let handler = shorts.clone();
            core_events::consume(
                shorts.core.bus.clone(),
                "shorts",
                &[
                    ASSET_READY,
                    ASSET_FAILED,
                    ASSET_DELETED,
                    USER_DELETED,
                    USER_BANNED,
                    USER_UNBANNED,
                    USER_BLOCKED,
                    CONTENT_REMOVED,
                    CHANNEL_DELETED,
                    CHANNEL_SUBSCRIBED,
                    CHANNEL_UNSUBSCRIBED,
                    REACTION_CHANGED,
                    COMMENT_PUBLISHED,
                    REPORT_CREATED,
                    REINDEX_REQUESTED,
                    SHORT_PUBLISHED,
                ],
                // Deleting a creator's whole catalogue takes a while.
                ConsumeOptions { ack_wait: Duration::from_secs(300), ..Default::default() },
                shorts.core.shutdown.clone(),
                move |event| {
                    let shorts = handler.clone();
                    async move { shorts.handle_event(event).await }
                },
            )
            .await;
        });

        let shorts = self.clone();
        tokio::spawn(async move {
            let handler = shorts.clone();
            let options = JobOptions { concurrency: 4, lease: Duration::from_secs(120), max_deliver: 5 };
            shorts
                .core
                .jobs
                .run(JOB_ANNOUNCE, options, shorts.core.shutdown.clone(), move |job: AnnounceJob| {
                    let shorts = handler.clone();
                    async move { shorts.announce(job).await.map(|_| ()) }
                })
                .await;
        });

        let shorts = self.clone();
        tokio::spawn(async move {
            let handler = shorts.clone();
            // Cutting a frame downloads the upload and runs ffmpeg.
            let options = JobOptions { concurrency: 2, lease: Duration::from_secs(300), max_deliver: 4 };
            shorts
                .core
                .jobs
                .run(JOB_COVER, options, shorts.core.shutdown.clone(), move |job: CoverJob| {
                    let shorts = handler.clone();
                    async move { shorts.cut_cover(job).await }
                })
                .await;
        });

        let shorts = self.clone();
        tokio::spawn(async move {
            let handler = shorts.clone();
            let options = JobOptions { concurrency: 1, lease: Duration::from_secs(120), max_deliver: 5 };
            shorts
                .core
                .jobs
                .run(JOB_REINDEX, options, shorts.core.shutdown.clone(), move |job: ReindexJob| {
                    let shorts = handler.clone();
                    async move { shorts.reindex(job).await }
                })
                .await;
        });

        let shorts = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(signals::flush_interval(shorts.cfg.flush_interval_seconds));
            loop {
                tokio::select! {
                    _ = shorts.core.shutdown.cancelled() => {
                        // What is still buffered would otherwise wait for the next start.
                        if let Err(error) = shorts.flush_all_signals().await {
                            tracing::warn!(%error, "final flush of playback counters failed");
                        }
                        break;
                    }
                    _ = tick.tick() => {
                        if let Err(error) = shorts.flush_all_signals().await {
                            tracing::error!(%error, "flushing playback counters failed");
                        }
                    }
                }
            }
        });

        let shorts = self.clone();
        tokio::spawn(async move {
            let every = shorts.cfg.ranking_interval_seconds.max(5);
            let mut tick = tokio::time::interval(Duration::from_secs(every));
            let mut runs = 0u64;
            loop {
                tokio::select! {
                    _ = shorts.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        // One process per interval: the others skip the tick.
                        let lease = Duration::from_secs(every.saturating_sub(1).max(1));
                        match shorts.core.cache.set_nx("shorts:ranking:lease", "1", lease).await {
                            Ok(true) => {}
                            Ok(false) => continue,
                            Err(error) => {
                                tracing::warn!(%error, "ranking lease unavailable; skipping this run");
                                continue;
                            }
                        }
                        if let Err(error) = shorts.run_ranking().await {
                            tracing::error!(%error, "ranking run failed");
                        }
                        runs += 1;
                        if runs.is_multiple_of(RECONCILE_EVERY) {
                            match shorts.reconcile().await {
                                Ok(0) => {}
                                Ok(count) => tracing::info!(count, "caught up shorts with their processed media"),
                                Err(error) => tracing::error!(%error, "catching up processed media failed"),
                            }
                        }
                        if runs.is_multiple_of((3600 / every).max(1))
                            && let Err(error) = shorts.housekeeping().await
                        {
                            tracing::error!(%error, "shorts housekeeping failed");
                        }
                    }
                }
            }
        });
    }

    /// One run of the ranking worker: scores, trending hashtags, and the
    /// search documents of hashtags whose count moved. Public so tests and
    /// tooling can run it on demand.
    pub async fn run_ranking(&self) -> AppResult<()> {
        let core = &self.core;
        ranking::refresh_scores(core, self.cfg.ranking_half_life_hours).await?;
        hashtags::refresh_trending(core).await?;
        // Bounded: a reindex of every hashtag is spread over several runs.
        for _ in 0..10 {
            if hashtags::refresh_search(core).await? == 0 {
                break;
            }
        }
        Ok(())
    }

    /// The hourly clean-up: "not interested" marks and taken-back likes
    /// that are too old to matter, and withdrawn shorts whose retention
    /// period (counted from the withdrawal) is over. Returns how many
    /// withdrawn shorts were deleted for good.
    pub async fn housekeeping(&self) -> AppResult<u64> {
        let core = &self.core;
        sqlx::query("DELETE FROM short_hidden WHERE created_at < now() - make_interval(days => $1)")
            .bind(HIDDEN_RETENTION_DAYS)
            .execute(&core.db)
            .await?;
        sqlx::query("DELETE FROM short_likes WHERE NOT liked AND changed_at < now() - make_interval(days => $1)")
            .bind(UNLIKE_MEMORY_DAYS)
            .execute(&core.db)
            .await?;
        shorts::purge_withdrawn(core, self.cfg.reported_retention_days).await
    }

    /// Dispatches a consumed event. Every reaction is idempotent and
    /// indifferent to the order of events (see the module documentation).
    /// Public so tests can deliver events without a running relay.
    pub async fn handle_event(&self, event: Event) -> AppResult<()> {
        match event.topic.as_str() {
            ASSET_READY | ASSET_FAILED => self.on_asset_processed(event.data::<AssetEvent>()?.asset_id).await,
            ASSET_DELETED => self.on_asset_deleted(event.data::<AssetEvent>()?.asset_id).await,
            USER_DELETED => self.on_user_deleted(event.data::<UserRef>()?.user_id).await,
            USER_BANNED | USER_UNBANNED => self.on_user_standing_changed(event.data::<UserRef>()?.user_id).await,
            USER_BLOCKED => {
                let blocked: UserBlocked = event.data()?;
                self.on_user_blocked(blocked.user_id, blocked.blocked_id).await
            }
            CONTENT_REMOVED => self.on_content_removed(event.data()?).await,
            CHANNEL_DELETED => self.on_channel_deleted(event.data::<ChannelDeleted>()?.channel_id).await,
            CHANNEL_SUBSCRIBED | CHANNEL_UNSUBSCRIBED => {
                let change: ChannelSubscribed = event.data()?;
                self.on_subscription_changed(change.user_id, change.channel_id, event.topic == CHANNEL_SUBSCRIBED).await
            }
            REACTION_CHANGED => self.on_reaction_changed(event.data()?, event.occurred_at).await,
            COMMENT_PUBLISHED => self.on_comment_published(event.data()?).await,
            REPORT_CREATED => self.on_report_created(event.data()?, event.occurred_at).await,
            REINDEX_REQUESTED => {
                let requested: ReindexRequested = event.data()?;
                if matches!(requested.kind.as_str(), "short" | "hashtag") {
                    let job = ReindexJob { kind: requested.kind, after: None };
                    self.core.jobs.enqueue_unique(JOB_REINDEX, &format!("{}:{}", job.kind, event.id), &job).await?;
                }
                Ok(())
            }
            SHORT_PUBLISHED => {
                let published: ShortEvent = event.data()?;
                if published.first_publication {
                    let job = AnnounceJob { short_id: published.id, after: None };
                    self.core.jobs.enqueue_unique(JOB_ANNOUNCE, &format!("{}:start", published.id), &job).await?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// An asset finished processing (or failed): the short that plays it
    /// goes live (or fails), and a short waiting for it as its cover frame
    /// gets its cover. Both read the asset's current state.
    async fn on_asset_processed(&self, asset_id: Uuid) -> AppResult<()> {
        let short: Option<ShortRow> =
            sqlx::query_as(concat!("SELECT ", short_columns!(), " FROM shorts s WHERE s.asset_id = $1"))
                .bind(asset_id)
                .fetch_optional(&self.core.db)
                .await?;
        if let Some(short) = short {
            self.sync_processing(&short).await?;
        }
        self.sync_cover(asset_id).await?;
        Ok(())
    }

    /// An asset was deleted under us (a takedown of the file, or the media
    /// domain cleaning up): the short that played it goes with it; a cover
    /// image falls back to the poster frame.
    async fn on_asset_deleted(&self, asset_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        let short: Option<ShortRow> =
            sqlx::query_as(concat!("SELECT ", short_columns!(), " FROM shorts s WHERE s.asset_id = $1"))
                .bind(asset_id)
                .fetch_optional(&core.db)
                .await?;
        if let Some(short) = short {
            shorts::delete(core, &short, None, Removal::AssetDeleted).await?;
        }
        let covered: Vec<ShortRow> =
            sqlx::query_as(concat!("SELECT ", short_columns!(), " FROM shorts s WHERE s.cover_asset_id = $1"))
                .bind(asset_id)
                .fetch_all(&core.db)
                .await?;
        for short in covered {
            let context = listing::Context::load(core, &short).await?;
            let mut tx = core.db.begin().await?;
            sqlx::query(
                "UPDATE shorts SET cover_asset_id = NULL, cover_kind = 1, cover_offset_ms = NULL, cover = playback -> 'poster'
                 WHERE id = $1 AND cover_asset_id = $2",
            )
            .bind(short.id)
            .bind(asset_id)
            .execute(&mut *tx)
            .await?;
            if after_change_if_present(&mut tx, short.id, &context).await? {
                tx.commit().await?;
            }
        }
        // A frame that vanished while it was being processed.
        self.sync_cover(asset_id).await?;
        Ok(())
    }

    /// An account was erased: its shorts, and every trace it left on other
    /// people's.
    async fn on_user_deleted(&self, user_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        shorts::delete_of_owner(core, user_id, Removal::AccountDeleted).await?;
        let mut tx = core.db.begin().await?;
        // Other people's shorts lose this user's saves. (Their likes are the
        // engagement domain's to take back, which moves the counts.)
        sqlx::query(
            "WITH gone AS (DELETE FROM short_saves WHERE user_id = $1 RETURNING short_id)
             UPDATE shorts s SET save_count = GREATEST(s.save_count - 1, 0) FROM gone WHERE s.id = gone.short_id",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        for table in ["short_likes", "short_hidden", "short_follows"] {
            sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE user_id = $1")))
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        // Captions that mentioned them keep their text and lose the link.
        sqlx::query(
            "UPDATE shorts SET mentions = (
                 SELECT COALESCE(jsonb_agg(m), '[]'::jsonb) FROM jsonb_array_elements(mentions) m
                 WHERE m ->> 'user_id' <> $1::text
             )
             WHERE mentions @> jsonb_build_array(jsonb_build_object('user_id', $1::text))",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE shorts SET mentions_told = array_remove(mentions_told, $1) WHERE $1 = ANY(mentions_told)")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        signals::forget_viewer(core, user_id).await
    }

    /// An account was suspended or reinstated: its shorts disappear from
    /// (or return to) everyone else's view, the hashtag and sound pages and
    /// the search index. The event only says that something changed; what
    /// the account's standing is now is read from the account, so a ban and
    /// its lifting arriving in the wrong order still end right.
    async fn on_user_standing_changed(&self, user_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        let standing: Option<bool> = sqlx::query_scalar("SELECT banned_at IS NOT NULL FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&core.db)
            .await?;
        let Some(suspended) = standing else { return Ok(()) };
        let affected: Vec<ShortRow> = sqlx::query_as(concat!(
            "SELECT ",
            short_columns!(),
            " FROM shorts s WHERE s.owner_id = $1 AND s.suspended <> $2 ORDER BY s.id"
        ))
        .bind(user_id)
        .bind(suspended)
        .fetch_all(&core.db)
        .await?;
        for short in affected {
            let context = listing::Context::load(core, &short).await?;
            let mut tx = core.db.begin().await?;
            sqlx::query("UPDATE shorts SET suspended = $2 WHERE id = $1")
                .bind(short.id)
                .bind(suspended)
                .execute(&mut *tx)
                .await?;
            if after_change_if_present(&mut tx, short.id, &context).await? {
                tx.commit().await?;
            }
        }
        Ok(())
    }

    /// Two users blocked each other: neither keeps the other's shorts in
    /// their saved or liked list. (Feeds, pages and lookups hide the other's
    /// shorts immediately, without waiting for the event; the video domain
    /// ends the subscriptions between the two.) A block that was lifted
    /// again before its event arrived takes nothing away.
    async fn on_user_blocked(&self, user_id: Uuid, blocked_id: Uuid) -> AppResult<()> {
        if !graph::is_blocked_either(&self.core, user_id, blocked_id).await? {
            return Ok(());
        }
        let mut tx = self.core.db.begin().await?;
        sqlx::query(
            "WITH gone AS (
                 DELETE FROM short_saves v USING shorts s
                 WHERE s.id = v.short_id
                   AND ((v.user_id = $1 AND s.owner_id = $2) OR (v.user_id = $2 AND s.owner_id = $1))
                 RETURNING v.short_id
             )
             UPDATE shorts s SET save_count = GREATEST(s.save_count - 1, 0) FROM gone WHERE s.id = gone.short_id",
        )
        .bind(user_id)
        .bind(blocked_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "DELETE FROM short_likes l USING shorts s
             WHERE s.id = l.short_id
               AND ((l.user_id = $1 AND s.owner_id = $2) OR (l.user_id = $2 AND s.owner_id = $1))",
        )
        .bind(user_id)
        .bind(blocked_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// A moderator took a short down: it is deleted for good, withdrawn or
    /// not, and its upload with it. The admin domain notifies the owner.
    async fn on_content_removed(&self, removed: ContentRemoved) -> AppResult<()> {
        if removed.subject_type == SubjectType::SUBJECT_TYPE_SHORT as i32
            && let Some(short) = shorts::find_by_id(&self.core, removed.subject_id).await?
        {
            shorts::delete(&self.core, &short, None, Removal::Moderation).await?;
        }
        Ok(())
    }

    /// A channel was deleted: its shorts go with it, and nobody follows it
    /// or holds marks against it any more.
    async fn on_channel_deleted(&self, channel_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        shorts::delete_of_channel(core, channel_id, Removal::ChannelDeleted).await?;
        sqlx::query("DELETE FROM short_follows WHERE channel_id = $1").bind(channel_id).execute(&core.db).await?;
        sqlx::query("DELETE FROM short_hidden WHERE channel_id = $1").bind(channel_id).execute(&core.db).await?;
        Ok(())
    }

    /// A subscription began or ended. The copy in `short_follows` is set to
    /// what the video domain says *now*, not to what the event says: the
    /// two events of a quick subscribe-and-unsubscribe may arrive in either
    /// order. A new subscription also credits the short the user just
    /// watched, if any (once: the credit is claimed).
    async fn on_subscription_changed(&self, user_id: Uuid, channel_id: Uuid, began: bool) -> AppResult<()> {
        let core = &self.core;
        if channels::is_subscribed(core, user_id, channel_id).await? {
            sqlx::query(
                "INSERT INTO short_follows (user_id, channel_id)
                 SELECT $1, $2 WHERE EXISTS (SELECT 1 FROM users WHERE id = $1)
                 ON CONFLICT DO NOTHING",
            )
            .bind(user_id)
            .bind(channel_id)
            .execute(&core.db)
            .await?;
            if began {
                self.credit_follow(user_id, channel_id).await?;
            }
        } else {
            sqlx::query("DELETE FROM short_follows WHERE user_id = $1 AND channel_id = $2")
                .bind(user_id)
                .bind(channel_id)
                .execute(&core.db)
                .await?;
        }
        Ok(())
    }

    /// A reaction changed in the engagement domain at `at`: keep the copy
    /// of who likes which short, and count a new like towards the short's
    /// ranking (once per account).
    async fn on_reaction_changed(&self, changed: ReactionChanged, at: Time) -> AppResult<()> {
        if changed.subject_type != SubjectType::SUBJECT_TYPE_SHORT as i32 {
            return Ok(());
        }
        let (likes, liked) = (changed.kind == REACTION_LIKE, changed.previous_kind == REACTION_LIKE);
        if likes == liked {
            return Ok(());
        }
        let applied = self.mirror_like(changed.user_id, changed.subject_id, likes, at).await?;
        if applied && likes {
            self.count_engagement(changed.subject_id, Engagement::Like, changed.user_id).await?;
        }
        Ok(())
    }

    /// A comment entered public view: it counts towards the short's ranking
    /// once per commenter, however many comments they write.
    async fn on_comment_published(&self, comment: CommentPublished) -> AppResult<()> {
        if comment.subject_type != SubjectType::SUBJECT_TYPE_SHORT as i32 {
            return Ok(());
        }
        self.count_engagement(comment.subject_id, Engagement::Comment, comment.author_id).await
    }

    /// Adds a like or comment of `user_id` to the hourly statistics of a
    /// short, if it is the first of that account this week.
    async fn count_engagement(&self, short_id: Uuid, engagement: Engagement, user_id: Uuid) -> AppResult<()> {
        let Some(short) = shorts::find_by_id(&self.core, short_id).await? else { return Ok(()) };
        if self.count_signal(&short, engagement, user_id).await? {
            // The claim is given back if the write fails, so that the
            // event's redelivery (or the account's next like) counts.
            if let Err(error) = signals::bump_hourly(&self.core.db, short_id, engagement, 1).await {
                self.release_signal(&short, engagement, user_id).await;
                return Err(error);
            }
        }
        Ok(())
    }

    /// A short was reported: from now on its owner cannot delete it out of
    /// the moderators' reach (see [`shorts::delete_by_owner`]).
    async fn on_report_created(&self, report: ReportCreated, at: Time) -> AppResult<()> {
        if report.subject_type != SubjectType::SUBJECT_TYPE_SHORT as i32 {
            return Ok(());
        }
        sqlx::query("UPDATE shorts SET reported_at = GREATEST(COALESCE(reported_at, $2), $2) WHERE id = $1")
            .bind(report.subject_id)
            .bind(at)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// Decides, once per short, whether its subscribers are told, and
    /// returns the cursor to continue from: [`ANNOUNCE_STARTED`], or
    /// [`ANNOUNCE_FINISHED`] when the channel has used up its allowance for
    /// the day (subscribers still find the short in their following feed).
    /// A decision that was already made is returned as it stands.
    async fn start_announcement(&self, short: &ShortRow) -> AppResult<Uuid> {
        let mut tx = self.core.db.begin().await?;
        // One decision per channel at a time, so the allowance holds when
        // several shorts go out together. Everything runs on this transaction.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("shorts:announce:{}", short.channel_id.simple()))
            .execute(&mut *tx)
            .await?;
        let decided: Option<Option<Uuid>> =
            sqlx::query_scalar("SELECT announce_cursor FROM shorts WHERE id = $1 FOR UPDATE")
                .bind(short.id)
                .fetch_optional(&mut *tx)
                .await?;
        match decided {
            None => return Ok(ANNOUNCE_FINISHED), // deleted meanwhile
            Some(Some(cursor)) => return Ok(cursor),
            Some(None) => {}
        }
        let told_today: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM shorts
             WHERE channel_id = $1 AND subscribers_told AND announced_at > now() - interval '24 hours'",
        )
        .bind(short.channel_id)
        .fetch_one(&mut *tx)
        .await?;
        let tell = told_today < self.cfg.announcements_per_day;
        let cursor = if tell { ANNOUNCE_STARTED } else { ANNOUNCE_FINISHED };
        sqlx::query("UPDATE shorts SET announce_cursor = $2, subscribers_told = $3 WHERE id = $1")
            .bind(short.id)
            .bind(cursor)
            .bind(tell)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(cursor)
    }

    /// Handles one `shorts.announce` job: notifies subscribers of the
    /// channel about a newly published short, in batches by user ID, and
    /// queues its successor when subscribers are left. Returns how many
    /// notifications were requested.
    ///
    /// Who hears what: level ALL gets inbox and push; PERSONALIZED gets the
    /// inbox entry, and the push only if they liked or saved one of the
    /// channel's shorts recently; NONE gets nothing. A friends-only short is
    /// announced to subscribers who are friends of the owner. A channel
    /// announces a few shorts a day (`announcements_per_day`), not every one.
    ///
    /// How far the notifications got is stored with the short
    /// (`announce_cursor`) and moved in the transaction that requests a
    /// batch. The job's own `after` is only a hint: a job that is retried,
    /// or queued twice, continues from the stored cursor, so no subscriber
    /// is told twice.
    pub async fn announce(&self, job: AnnounceJob) -> AppResult<u64> {
        let core = &self.core;
        let Some(short) = shorts::find_by_id(core, job.short_id).await? else { return Ok(0) };
        let audience_exists = short.is_published() && !short.suspended && matches!(short.visibility, PUBLIC | FRIENDS);
        if !audience_exists || !short.notify_subscribers {
            return Ok(0);
        }
        let Some(channel) = channels::find_by_id(core, short.channel_id).await? else { return Ok(0) };
        let mut cursor = match short.announce_cursor {
            Some(cursor) => cursor,
            None => self.start_announcement(&short).await?,
        };
        if cursor == ANNOUNCE_FINISHED {
            return Ok(0);
        }
        // Both lists are sorted by user ID; everyone at ALL is also in the wider one.
        let listeners = channels::subscriber_ids(core, channel.id, NotifyLevel::Personalized).await?;
        let pushed = channels::subscriber_ids(core, channel.id, NotifyLevel::All).await?;
        let hidden = graph::blocked_ids_either(core, short.owner_id).await?;
        let friends =
            if short.visibility == FRIENDS { Some(graph::friend_ids(core, short.owner_id).await?) } else { None };
        let facts = short.facts();

        let batch_size = self.cfg.announce_batch_size.max(1) as usize;
        let mut notified = 0;
        for _ in 0..ANNOUNCE_BATCHES_PER_JOB {
            let start = listeners.partition_point(|user_id| *user_id <= cursor);
            let batch = &listeners[start..listeners.len().min(start + batch_size)];
            let Some(last) = batch.last().copied() else { break };
            let engaged: Vec<Uuid> = sqlx::query_scalar(
                "SELECT l.user_id FROM short_likes l JOIN shorts s ON s.id = l.short_id
                 WHERE s.channel_id = $1 AND l.user_id = ANY($2) AND l.liked
                   AND l.created_at > now() - make_interval(days => $3)
                 UNION
                 SELECT v.user_id FROM short_saves v JOIN shorts s ON s.id = v.short_id
                 WHERE s.channel_id = $1 AND v.user_id = ANY($2) AND v.created_at > now() - make_interval(days => $3)",
            )
            .bind(channel.id)
            .bind(batch)
            .bind(ENGAGED_DAYS)
            .fetch_all(&core.db)
            .await?;
            let mut tx = core.db.begin().await?;
            // Claim the batch: whoever moves the cursor past it tells it.
            let claimed = sqlx::query("UPDATE shorts SET announce_cursor = $2 WHERE id = $1 AND announce_cursor < $2")
                .bind(short.id)
                .bind(last)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                == 1;
            if !claimed {
                // Another run of this job is ahead of us (or the short is gone).
                return Ok(notified);
            }
            for user_id in batch {
                let is_friend = friends.as_ref().is_some_and(|friends| friends.contains(user_id));
                if hidden.contains(user_id) || !access::reaches(&facts, is_friend) {
                    continue;
                }
                let push = pushed.binary_search(user_id).is_ok() || engaged.contains(user_id);
                outbound::notify(&mut *tx, &published_notification(*user_id, &short, &channel, push)).await?;
                notified += 1;
            }
            tx.commit().await?;
            cursor = last;
        }
        if listeners.last().is_some_and(|final_listener| *final_listener > cursor) {
            let next = AnnounceJob { short_id: short.id, after: Some(cursor) };
            core.jobs.enqueue_unique(JOB_ANNOUNCE, &format!("{}:{cursor}", short.id), &next).await?;
        } else {
            sqlx::query("UPDATE shorts SET announce_cursor = $2 WHERE id = $1")
                .bind(short.id)
                .bind(ANNOUNCE_FINISHED)
                .execute(&core.db)
                .await?;
        }
        Ok(notified)
    }

    /// Handles one `shorts.reindex` job: republishes a batch of search
    /// documents of `job.kind` and queues its successor.
    pub async fn reindex(&self, job: ReindexJob) -> AppResult<()> {
        let core = &self.core;
        let last: Option<Uuid> = match job.kind.as_str() {
            "hashtag" => hashtags::mark_for_reindex(core, job.after).await?,
            "short" => {
                let rows: Vec<ShortRow> = sqlx::query_as(concat!(
                    "SELECT ",
                    short_columns!(),
                    " FROM shorts s WHERE s.listed AND ($1::uuid IS NULL OR s.id > $1) ORDER BY s.id LIMIT $2"
                ))
                .bind(job.after)
                .bind(REINDEX_BATCH)
                .fetch_all(&core.db)
                .await?;
                let mut channel_ids: Vec<Uuid> = rows.iter().map(|row| row.channel_id).collect();
                channel_ids.sort_unstable();
                channel_ids.dedup();
                let mut known: HashMap<Uuid, ChannelRow> = HashMap::new();
                for channel_id in channel_ids {
                    if let Some(channel) = channels::find_by_id(core, channel_id).await? {
                        known.insert(channel_id, channel);
                    }
                }
                // The documents are written from rows read again under a
                // lock: a short that stops being public meanwhile (an edit,
                // a suspension) writes its own change after ours, not before.
                let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
                let mut tx = core.db.begin().await?;
                let current: Vec<ShortRow> = sqlx::query_as(concat!(
                    "SELECT ",
                    short_columns!(),
                    " FROM shorts s WHERE s.id = ANY($1) AND s.listed ORDER BY s.id FOR SHARE"
                ))
                .bind(&ids)
                .fetch_all(&mut *tx)
                .await?;
                for row in &current {
                    if let Some(channel) = known.get(&row.channel_id) {
                        outbound::index(&mut *tx, listing::search_doc(row, channel)).await?;
                    }
                }
                tx.commit().await?;
                rows.last().map(|row| row.id).filter(|_| rows.len() as i64 == REINDEX_BATCH)
            }
            _ => None,
        };
        if let Some(last) = last {
            let next = ReindexJob { kind: job.kind.clone(), after: Some(last) };
            core.jobs.enqueue_unique(JOB_REINDEX, &format!("{}:{last}", job.kind), &next).await?;
        }
        Ok(())
    }
}

/// Brings what is derived from a short up to date inside `tx`. Returns
/// `false` (nothing to commit) when the short was deleted meanwhile.
async fn after_change_if_present(
    tx: &mut shorts::Tx<'_>,
    short_id: Uuid,
    context: &listing::Context,
) -> AppResult<bool> {
    match listing::after_change_in(tx, short_id, false, context).await {
        Ok(_) => Ok(true),
        Err(error) if error.is_not_found() => Ok(false),
        Err(error) => Err(error),
    }
}

/// The notification that tells a subscriber about a new short. It speaks
/// for the channel: there is no actor, because a channel does not reveal
/// the account behind it.
fn published_notification(
    user_id: Uuid,
    short: &ShortRow,
    channel: &ChannelRow,
    push: bool,
) -> outbound::NotificationRequest {
    let caption = crate::text::excerpt(&short.caption, 60);
    let body = if caption.is_empty() {
        outbound::Localized::new("新しいショートを投稿したよ!", "posted a new short")
    } else {
        outbound::Localized::new(format!("新しいショートを投稿したよ!「{caption}」"), format!("New short: {caption}"))
    };
    let mut request = outbound::NotificationRequest::new(
        user_id,
        "shorts.published",
        outbound::Localized::same(channel.name.clone()),
        body,
        short.path(),
    )
    .subject(SubjectType::SUBJECT_TYPE_SHORT, short.id)
    .collapse(format!("shorts.published:{}", short.id))
    .data(listing::short_data(short, channel));
    if let Some(url) = short.cover_url() {
        request = request.image(url);
    }
    if !push {
        request = request.inbox_only();
    }
    request
}
