//! Background work: the event consumer, two jobs and four timers.
//!
//! | what | name | does |
//! |---|---|---|
//! | consumer | durable `community` | [`Community::handle_event`] for the topics in `cleanup` |
//! | job | `community.announce` | notifies the members about an announcement, in chunks |
//! | job | `community.sweep` | after a community or channel was deleted or changed visibility: the search documents of its threads |
//! | timer | hot scores | recomputes the "hot" order of threads |
//! | timer | trend scores | recomputes which communities are trending |
//! | timer | reminders | sends event reminders that are due (every minute) |
//! | timer | maintenance | prunes expired bans and old declined requests, purges removed content whose retention is over, restarts stuck announcements |
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokuly_core::{
    events::{self as core_events, ConsumeOptions},
    jobs::JobOptions,
    outbound,
    prelude::*,
};
use tokuly_social::graph;

use crate::{
    Community,
    cleanup::CONSUMED_TOPICS,
    model::{CommunityRow, ThreadRow, community_columns, level, thread_columns},
    notices, threads,
};

pub const JOB_ANNOUNCE: &str = "community.announce";
pub const JOB_SWEEP: &str = "community.sweep";

/// Members notified per transaction while an announcement goes out.
const ANNOUNCE_CHUNK: i64 = 500;
/// Threads handled per transaction by a sweep.
const SWEEP_CHUNK: i64 = 200;
/// How long threads keep a hot score after their last activity.
const HOT_DAYS: i32 = 30;
/// The window the trending score looks at.
const TREND_DAYS: i32 = 7;
const REMINDER_INTERVAL: Duration = Duration::from_secs(60);
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(600);
/// Days a declined join request is kept (and silently stands in the way of a new one).
const DECLINED_REQUEST_DAYS: i32 = 7;

/// Notify the members of a community about an announcement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnounceJob {
    pub thread_id: Uuid,
}

/// Bring the search documents in line with a community (or one channel of
/// it) that was deleted or changed visibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepJob {
    pub community_id: Uuid,
    /// Only the threads of this channel.
    pub channel_id: Option<Uuid>,
}

impl Community {
    /// Starts the background work. Call once, in worker processes.
    pub fn spawn_workers(&self) {
        let community = self.clone();
        tokio::spawn(async move {
            let handler = community.clone();
            core_events::consume(
                community.core.bus.clone(),
                "community",
                &CONSUMED_TOPICS,
                // Erasing a busy member takes a while.
                ConsumeOptions { ack_wait: Duration::from_secs(120), ..Default::default() },
                community.core.shutdown.clone(),
                move |event| {
                    let community = handler.clone();
                    async move { community.handle_event(event).await }
                },
            )
            .await;
        });

        let community = self.clone();
        tokio::spawn(async move {
            let handler = community.clone();
            let options = JobOptions { concurrency: 2, lease: Duration::from_secs(120), max_deliver: 5 };
            community
                .core
                .jobs
                .run(JOB_ANNOUNCE, options, community.core.shutdown.clone(), move |job: AnnounceJob| {
                    let community = handler.clone();
                    async move { community.run_announcement(job.thread_id).await }
                })
                .await;
        });

        let community = self.clone();
        tokio::spawn(async move {
            let handler = community.clone();
            let options = JobOptions { concurrency: 2, lease: Duration::from_secs(300), max_deliver: 5 };
            community
                .core
                .jobs
                .run(JOB_SWEEP, options, community.core.shutdown.clone(), move |job: SweepJob| {
                    let community = handler.clone();
                    async move { community.run_sweep(job).await }
                })
                .await;
        });

        self.spawn_timer(
            "community hot scores",
            Duration::from_secs(self.cfg.hot_interval_seconds),
            |community| async move { community.refresh_hot_scores().await.map(|_| ()) },
        );
        self.spawn_timer(
            "community trend scores",
            Duration::from_secs(self.cfg.trend_interval_seconds),
            |community| async move { community.refresh_trend_scores().await.map(|_| ()) },
        );
        self.spawn_timer("community event reminders", REMINDER_INTERVAL, |community| async move {
            community.send_due_reminders().await.map(|_| ())
        });
        self.spawn_timer("community maintenance", MAINTENANCE_INTERVAL, |community| async move {
            community.run_maintenance().await
        });
    }

    /// Runs `job` every `every` until shutdown, logging failures.
    fn spawn_timer<F, Fut>(&self, name: &'static str, every: Duration, job: F)
    where
        F: Fn(Community) -> Fut + Send + 'static,
        Fut: Future<Output = AppResult<()>> + Send + 'static,
    {
        let community = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = community.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(error) = job(community.clone()).await {
                            tracing::error!(job = name, %error, "periodic job failed");
                        }
                    }
                }
            }
        });
    }

    /// Queues the notification of the members about an announcement. If
    /// queueing fails the maintenance timer picks the announcement up.
    pub(crate) async fn queue_announcement(&self, job: AnnounceJob) {
        if let Err(error) = self.core.jobs.enqueue_unique(JOB_ANNOUNCE, &job.thread_id.to_string(), &job).await {
            tracing::warn!(%error, thread_id = %job.thread_id, "queueing an announcement failed; maintenance will retry");
        }
    }

    /// Queues a sweep; if queueing fails the sweep runs in this process.
    pub(crate) async fn queue_sweep(&self, job: SweepJob) {
        if let Err(error) = self.core.jobs.enqueue(JOB_SWEEP, &job).await {
            tracing::warn!(%error, community_id = %job.community_id, "queueing a sweep failed; running it here");
            let community = self.clone();
            tokio::spawn(async move {
                if let Err(error) = community.run_sweep(job).await {
                    tracing::error!(%error, "community sweep failed");
                }
            });
        }
    }

    /// Notifies the members about an announcement: everyone whose level
    /// lets announcements through, the author aside. Works in chunks with
    /// the progress stored on the thread, so a retry or a second worker
    /// continues instead of starting over.
    pub async fn run_announcement(&self, thread_id: Uuid) -> AppResult<()> {
        loop {
            // What a chunk needs is read before its transaction opens: a
            // worker holding the thread's lock never asks the pool for a
            // second connection.
            let thread =
                self.thread_by_id(thread_id).await?.filter(|thread| thread.is_visible() && thread.announcement);
            let community = match &thread {
                Some(thread) => self.community_by_id(thread.community_id).await?,
                None => None,
            };
            // Nobody hears from someone they have a block with — unless the
            // thread is anonymous, where the silence would name the author.
            let skip = match &thread {
                Some(thread) if !thread.anonymous => graph::blocked_ids_either(&self.core, thread.author_id).await?,
                _ => Vec::new(),
            };

            let mut tx = self.core.db.begin().await?;
            // The lock makes two workers on one announcement take turns.
            let pending: Option<Option<Uuid>> = sqlx::query_scalar(
                "SELECT announce_cursor FROM community_threads
                 WHERE id = $1 AND announced_at IS NOT NULL AND announce_done_at IS NULL FOR UPDATE",
            )
            .bind(thread_id)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(cursor) = pending else { return Ok(()) };
            let (Some(thread), Some(community)) = (thread, community) else {
                // Deleted, hidden or no longer an announcement: nothing to tell.
                sqlx::query("UPDATE community_threads SET announce_done_at = now() WHERE id = $1")
                    .bind(thread_id)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                return Ok(());
            };
            let recipients: Vec<Uuid> = sqlx::query_scalar(
                "SELECT user_id FROM community_members
                 WHERE community_id = $1 AND notification_level IN ($2, $3) AND user_id <> $4
                   AND ($5::uuid IS NULL OR user_id > $5) AND NOT (user_id = ANY($6))
                 ORDER BY user_id LIMIT $7",
            )
            .bind(community.id)
            .bind(level::ALL)
            .bind(level::MENTIONS)
            .bind(thread.author_id)
            .bind(cursor)
            .bind(&skip)
            .bind(ANNOUNCE_CHUNK)
            .fetch_all(&mut *tx)
            .await?;
            for recipient in &recipients {
                let notice = notices::announcement(*recipient, &community, &thread);
                let notice = if thread.anonymous { notice } else { notice.actor(thread.author_id) };
                outbound::notify(&mut *tx, &notice).await?;
            }
            let done = (recipients.len() as i64) < ANNOUNCE_CHUNK;
            sqlx::query(
                "UPDATE community_threads SET announce_cursor = coalesce($2, announce_cursor),
                        announce_done_at = CASE WHEN $3 THEN now() END
                 WHERE id = $1",
            )
            .bind(thread_id)
            .bind(recipients.last().copied())
            .bind(done)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            if done {
                return Ok(());
            }
        }
    }

    /// Walks the threads of a community (or one channel) and puts search
    /// in line with what the threads now are: threads that are gone —
    /// deleted, or of a closed community — leave the index; the others are
    /// indexed again, which adds or removes them as the community's
    /// visibility says. Uploads are not touched: removed content keeps them
    /// until its retention is over (see `retention`).
    pub async fn run_sweep(&self, job: SweepJob) -> AppResult<()> {
        let db = &self.core.db;
        // Closed communities included: closing is what most sweeps are about.
        let community: Option<CommunityRow> =
            sqlx::query_as(concat!("SELECT ", community_columns!(), " FROM communities c WHERE c.id = $1"))
                .bind(job.community_id)
                .fetch_optional(db)
                .await?;
        let Some(community) = community else { return Ok(()) };
        let closed = community.deleted_at.is_some();
        let mut after: Option<Uuid> = None;
        loop {
            let threads: Vec<ThreadRow> = sqlx::query_as(concat!(
                "SELECT ",
                thread_columns!(),
                " FROM community_threads t
                  WHERE t.community_id = $1 AND ($2::uuid IS NULL OR t.channel_id = $2) AND ($3::uuid IS NULL OR t.id > $3)
                  ORDER BY t.id LIMIT $4"
            ))
            .bind(community.id)
            .bind(job.channel_id)
            .bind(after)
            .bind(SWEEP_CHUNK)
            .fetch_all(db)
            .await?;
            let Some(last) = threads.last() else { break };
            after = Some(last.id);
            let mut tx = db.begin().await?;
            for thread in &threads {
                if closed || thread.deleted_at.is_some() {
                    outbound::unindex(&mut *tx, "thread", thread.id).await?;
                } else {
                    outbound::index(&mut *tx, threads::search_doc(&community, thread)).await?;
                }
            }
            tx.commit().await?;
            if (threads.len() as i64) < SWEEP_CHUNK {
                break;
            }
        }
        Ok(())
    }

    /// Recomputes the hot score of threads active in the last weeks and
    /// lets older ones drop to zero. Returns how many threads were scored.
    pub async fn refresh_hot_scores(&self) -> AppResult<u64> {
        let db = &self.core.db;
        let scored = sqlx::query(
            "UPDATE community_threads SET hot_score = community_hot_score(
                 reply_count, like_count, comment_count, created_at, last_activity_at, now())
             WHERE deleted_at IS NULL AND last_activity_at > now() - make_interval(days => $1)",
        )
        .bind(HOT_DAYS)
        .execute(db)
        .await?;
        sqlx::query(
            "UPDATE community_threads SET hot_score = 0
             WHERE hot_score > 0 AND last_activity_at <= now() - make_interval(days => $1)",
        )
        .bind(HOT_DAYS)
        .execute(db)
        .await?;
        Ok(scored.rows_affected())
    }

    /// Recomputes the trend score of every community that has (or had)
    /// something going on: new members count three, new threads two, new
    /// replies one, over the last week — divided by a gentle function of
    /// the size, so a small community that doubles outranks a big one that
    /// merely stays busy. Returns how many scores changed.
    pub async fn refresh_trend_scores(&self) -> AppResult<u64> {
        let updated = sqlx::query(
            "WITH joins AS (
                 SELECT community_id, count(*) AS n FROM community_members
                 WHERE joined_at > now() - make_interval(days => $1) GROUP BY community_id
             ), threads AS (
                 SELECT community_id, count(*) AS n FROM community_threads
                 WHERE created_at > now() - make_interval(days => $1) AND deleted_at IS NULL AND state = 1
                 GROUP BY community_id
             ), replies AS (
                 SELECT community_id, count(*) AS n FROM community_replies
                 WHERE created_at > now() - make_interval(days => $1) AND deleted_at IS NULL AND state = 1
                 GROUP BY community_id
             ), scored AS (
                 SELECT c.id,
                        (3 * coalesce(j.n, 0) + 2 * coalesce(t.n, 0) + coalesce(r.n, 0))::double precision
                            / power(c.member_count + 10, 0.3) AS score
                 FROM communities c
                 LEFT JOIN joins j ON j.community_id = c.id
                 LEFT JOIN threads t ON t.community_id = c.id
                 LEFT JOIN replies r ON r.community_id = c.id
                 WHERE c.deleted_at IS NULL
                   AND (j.n IS NOT NULL OR t.n IS NOT NULL OR r.n IS NOT NULL OR c.trend_score > 0)
             )
             UPDATE communities c SET trend_score = scored.score
             FROM scored WHERE c.id = scored.id AND c.trend_score IS DISTINCT FROM scored.score",
        )
        .bind(TREND_DAYS)
        .execute(&self.core.db)
        .await?;
        Ok(updated.rows_affected())
    }

    /// Housekeeping: bans that ran out and declined requests that are old
    /// enough to be asked again are deleted; removed content whose
    /// retention is over is purged; announcements whose notification job
    /// got lost are queued again.
    pub async fn run_maintenance(&self) -> AppResult<()> {
        let db = &self.core.db;
        self.purge_removed().await?;
        sqlx::query("DELETE FROM community_bans WHERE expires_at <= now()").execute(db).await?;
        sqlx::query("DELETE FROM community_join_requests WHERE declined_at < now() - make_interval(days => $1)")
            .bind(DECLINED_REQUEST_DAYS)
            .execute(db)
            .await?;
        let stuck: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM community_threads
             WHERE announced_at IS NOT NULL AND announce_done_at IS NULL AND announced_at < now() - interval '10 minutes'
             ORDER BY announced_at LIMIT 100",
        )
        .fetch_all(db)
        .await?;
        for thread_id in stuck {
            self.queue_announcement(AnnounceJob { thread_id }).await;
        }
        Ok(())
    }
}
