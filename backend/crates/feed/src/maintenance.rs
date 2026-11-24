//! Background work of the feed domain: the event consumer, the news
//! scheduler and fetch job, the retention sweep, the monthly recap notice,
//! account deletion and data export.
use std::time::Duration;

use async_trait::async_trait;
use tokuly_core::{
    events::{self, ConsumeOptions, Event},
    export::DataExporter,
    jobs::JobOptions,
    prelude::*,
};
use tokuly_identity::maintenance::UserRef;
use tokuly_proto::common::SubjectType;
use tokuly_social::events::{BlockChanged, FriendRemoved};

use crate::{
    Feed,
    events::{
        ACTIVITY_TOPIC, CONSUMER_NAME, CONTENT_REMOVED, ContentRemoved, FETCH_JOB, FRIEND_REMOVED, FetchSourceJob,
        USER_BLOCKED, USER_CREATED, USER_DELETED, consumed_topics,
    },
    news::ranking::AFFINITY_EPSILON,
    withdrawals,
};

const SCHEDULER_INTERVAL: Duration = Duration::from_secs(30);
const RETENTION_INTERVAL: Duration = Duration::from_secs(3600);
const RECAP_NOTICE_INTERVAL: Duration = Duration::from_secs(3600);
/// Rows deleted per statement, so a large backlog never holds one long lock.
const SWEEP_BATCH: i64 = 5000;
/// Handled-event markers are kept a day longer than the event stream keeps
/// events (7 days): after that a redelivery is impossible.
const PROCESSED_EVENT_DAYS: i32 = 8;
/// Seen marks outlive the story window by this much, then they are swept.
const STORY_VIEW_GRACE_HOURS: i32 = 24;
/// Recap notice markers only matter while a notice could be sent again.
const RECAP_NOTICE_DAYS: i32 = 400;

/// What one retention sweep removed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RetentionReport {
    pub activities: u64,
    pub story_views: u64,
    pub processed_events: u64,
    pub subject_changes: u64,
    pub articles: u64,
    pub interests: u64,
    pub recap_notices: u64,
}

impl Feed {
    /// Starts the feed workers (worker processes only): the event consumer,
    /// the `feed.news.fetch` job runner, the news scheduler (which first
    /// creates the sources of `TOKULY_NEWS_FEEDS`), the hourly retention
    /// sweep and the hourly check for recap notices.
    pub fn spawn_workers(&self) {
        let feed = self.clone();
        tokio::spawn(async move {
            let handler = feed.clone();
            events::consume(
                feed.core.bus.clone(),
                CONSUMER_NAME,
                &consumed_topics(),
                ConsumeOptions::default(),
                feed.core.shutdown.clone(),
                move |event| {
                    let feed = handler.clone();
                    async move { feed.handle_event(event).await }
                },
            )
            .await;
        });

        let feed = self.clone();
        tokio::spawn(async move {
            let handler = feed.clone();
            // A fetch is bounded by the fetch timeout; the lease only has to outlast it.
            let options = JobOptions {
                concurrency: 4,
                lease: Duration::from_secs(feed.cfg.news_fetch_timeout_seconds + 60),
                max_deliver: 3,
            };
            feed.core
                .jobs
                .run(FETCH_JOB, options, feed.core.shutdown.clone(), move |job: FetchSourceJob| {
                    let feed = handler.clone();
                    async move { feed.refresh_source(job.source_id).await.map(|_| ()) }
                })
                .await;
        });

        let feed = self.clone();
        tokio::spawn(async move {
            match feed.seed_config_sources().await {
                Ok(0) => {}
                Ok(created) => tracing::info!(created, "news sources created from TOKULY_NEWS_FEEDS"),
                Err(error) => tracing::error!(%error, "could not create the news sources of TOKULY_NEWS_FEEDS"),
            }
            feed.run_periodically("news scheduler", SCHEDULER_INTERVAL, |feed| async move {
                feed.enqueue_due_fetches().await.map(|_| ())
            })
            .await;
        });

        let feed = self.clone();
        tokio::spawn(async move {
            feed.run_periodically("feed retention", RETENTION_INTERVAL, |feed| async move {
                let report = feed.run_retention().await?;
                if report != RetentionReport::default() {
                    tracing::info!(?report, "feed retention sweep");
                }
                Ok(())
            })
            .await;
        });

        let feed = self.clone();
        tokio::spawn(async move {
            feed.run_periodically("recap notices", RECAP_NOTICE_INTERVAL, |feed| async move {
                let sent = feed.send_recap_notices(now()).await?;
                if sent > 0 {
                    tracing::info!(sent, "recap notices sent");
                }
                Ok(())
            })
            .await;
        });
    }

    /// Runs `job` every `every` until shutdown, logging failures.
    async fn run_periodically<F, Fut>(&self, name: &'static str, every: Duration, job: F)
    where
        F: Fn(Feed) -> Fut,
        Fut: Future<Output = AppResult<()>>,
    {
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = self.core.shutdown.cancelled() => break,
                _ = tick.tick() => {
                    if let Err(error) = job(self.clone()).await {
                        tracing::error!(job = name, %error, "periodic job failed");
                    }
                }
            }
        }
    }

    /// Entry point of the durable consumer.
    pub async fn handle_event(&self, event: Event) -> AppResult<()> {
        match event.topic.as_str() {
            ACTIVITY_TOPIC => self.handle_activity_recorded(&event).await.map(|_| ()),
            USER_CREATED => self.handle_user_created(&event).await.map(|_| ()),
            USER_DELETED => {
                let deleted: UserRef = event.data()?;
                self.purge_user(deleted.user_id).await
            }
            FRIEND_REMOVED => {
                let removed: FriendRemoved = event.data()?;
                self.forget_story_views_between(removed.user_id, removed.friend_id).await
            }
            USER_BLOCKED => {
                let blocked: BlockChanged = event.data()?;
                self.forget_story_views_between(blocked.user_id, blocked.blocked_id).await
            }
            CONTENT_REMOVED => {
                let removed: ContentRemoved = event.data()?;
                self.handle_content_removed(&removed).await
            }
            topic if withdrawals::RULES.iter().any(|rule| rule.topic == topic) => {
                self.handle_subject_change(&event).await
            }
            _ => Ok(()),
        }
    }

    /// Two people stopped being friends (or one blocked the other): what one
    /// watched of the other's map stories is of no use anymore. Everything
    /// else is filtered when read, so nothing else needs fixing up.
    pub async fn forget_story_views_between(&self, a: Uuid, b: Uuid) -> AppResult<()> {
        sqlx::query("DELETE FROM map_story_views WHERE (viewer_id = $1 AND author_id = $2) OR (viewer_id = $2 AND author_id = $1)")
            .bind(a)
            .bind(b)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// Handles `admin.content.removed`: cards pointing at the removed
    /// content disappear from every log and feed; a removed news article is
    /// taken down.
    pub async fn handle_content_removed(&self, removed: &ContentRemoved) -> AppResult<()> {
        if removed.subject_type == SubjectType::SUBJECT_TYPE_NEWS_ARTICLE as i32 {
            self.take_article_down(removed.subject_id).await?;
            return Ok(());
        }
        // A profile takedown is not the end of a person: entries about them
        // (bumps, friendships) stay. Deleted accounts are handled separately.
        if removed.subject_type == SubjectType::SUBJECT_TYPE_USER as i32 {
            return Ok(());
        }
        let owners: Vec<Uuid> =
            sqlx::query_scalar("DELETE FROM activities WHERE subject_type = $1 AND subject_id = $2 RETURNING user_id")
                .bind(removed.subject_type)
                .bind(removed.subject_id)
                .fetch_all(&self.core.db)
                .await?;
        let mut owners = owners;
        owners.sort_unstable();
        owners.dedup();
        for owner in owners {
            self.invalidate_recaps(owner).await;
        }
        Ok(())
    }

    /// Handles `identity.user.deleted`: removes everything about the user,
    /// including other people's entries that name them (those carry their
    /// name in the summary).
    pub async fn purge_user(&self, user_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let affected: Vec<Uuid> = sqlx::query_scalar(
            "DELETE FROM activities
             WHERE user_id = $1 OR with_user_ids @> ARRAY[$1]::uuid[] OR (subject_type = $2 AND subject_id = $1)
             RETURNING user_id",
        )
        .bind(user_id)
        .bind(SubjectType::SUBJECT_TYPE_USER as i32)
        .fetch_all(&mut *tx)
        .await?;
        for statement in [
            "DELETE FROM activity_hidden_kinds WHERE user_id = $1",
            "DELETE FROM map_story_views WHERE viewer_id = $1 OR author_id = $1",
            "DELETE FROM news_clicks WHERE user_id = $1",
            "DELETE FROM news_feedback WHERE user_id = $1",
            "DELETE FROM news_tag_affinity WHERE user_id = $1",
            "DELETE FROM feed_recap_notices WHERE user_id = $1",
        ] {
            sqlx::query(statement).bind(user_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;

        // Other people's recaps counted the entries that named this user.
        let mut others: Vec<Uuid> = affected.into_iter().filter(|owner| *owner != user_id).collect();
        others.sort_unstable();
        others.dedup();
        for owner in others {
            self.invalidate_recaps(owner).await;
        }
        self.invalidate_recaps(user_id).await;
        Ok(())
    }

    /// Deletes rows matching `condition` from `table` in batches. Both are
    /// constants of this module, never user input.
    async fn sweep(
        &self,
        table: &'static str,
        key: &'static str,
        condition: &'static str,
        days_or_hours: i32,
    ) -> AppResult<u64> {
        let statement =
            format!("DELETE FROM {table} WHERE {key} IN (SELECT {key} FROM {table} WHERE {condition} LIMIT $2)");
        let mut total = 0;
        loop {
            let done = sqlx::query(sqlx::AssertSqlSafe(statement.clone()))
                .bind(days_or_hours)
                .bind(SWEEP_BATCH)
                .execute(&self.core.db)
                .await?;
            total += done.rows_affected();
            if done.rows_affected() < SWEEP_BATCH as u64 {
                return Ok(total);
            }
        }
    }

    /// Removes what is past its keep-by date: old activity entries, seen
    /// marks of stories that left the window, markers of long-handled
    /// events, news articles that are old and no longer in their feed,
    /// interests that faded to nothing and old recap notice markers.
    pub async fn run_retention(&self) -> AppResult<RetentionReport> {
        let mut report = RetentionReport::default();
        if self.cfg.activity_retention_days > 0 {
            report.activities = self
                .sweep(
                    "activities",
                    "id",
                    "occurred_at < now() - make_interval(days => $1)",
                    self.cfg.activity_retention_days as i32,
                )
                .await?;
        }
        // Seen marks are keyed by (viewer, activity): sweep by the row's physical ID.
        report.story_views = self
            .sweep(
                "map_story_views",
                "ctid",
                "seen_at < now() - make_interval(hours => $1)",
                self.cfg.map_story_hours as i32 + STORY_VIEW_GRACE_HOURS,
            )
            .await?;
        report.processed_events = self
            .sweep(
                "feed_processed_events",
                "event_id",
                "processed_at < now() - make_interval(days => $1)",
                PROCESSED_EVENT_DAYS,
            )
            .await?;
        // Remembered for as long as an event about the subject can still arrive.
        report.subject_changes = self
            .sweep(
                "feed_subject_changes",
                "ctid",
                "recorded_at < now() - make_interval(days => $1)",
                PROCESSED_EVENT_DAYS,
            )
            .await?;
        report.articles = self.prune_articles(SWEEP_BATCH).await?;
        report.interests = self.sweep_faded_interests().await?;
        report.recap_notices = self
            .sweep("feed_recap_notices", "ctid", "created_at < now() - make_interval(days => $1)", RECAP_NOTICE_DAYS)
            .await?;
        Ok(report)
    }

    /// Interests whose current (decayed) strength is below what is ever
    /// shown or ranked by.
    async fn sweep_faded_interests(&self) -> AppResult<u64> {
        let mut total = 0;
        loop {
            let done = sqlx::query(
                "DELETE FROM news_tag_affinity WHERE ctid IN (
                     SELECT ctid FROM news_tag_affinity
                     WHERE updated_at < now() - interval '1 day'
                       AND abs(score) * power(0.5::float8, extract(epoch FROM (now() - updated_at))::float8 / $1) < $2
                     LIMIT $3
                 )",
            )
            .bind(self.cfg.affinity_half_life_seconds())
            .bind(AFFINITY_EPSILON)
            .bind(SWEEP_BATCH)
            .execute(&self.core.db)
            .await?;
            total += done.rows_affected();
            if done.rows_affected() < SWEEP_BATCH as u64 {
                return Ok(total);
            }
        }
    }
}

/// The feed section of a personal data export.
pub(crate) struct FeedExporter {
    pub core: Core,
}

/// Runs a query that returns one JSON document.
async fn json(core: &Core, sql: &'static str, user_id: Uuid) -> AppResult<serde_json::Value> {
    Ok(sqlx::query_scalar(sql).bind(user_id).fetch_one(&core.db).await?)
}

#[async_trait]
impl DataExporter for FeedExporter {
    fn name(&self) -> &'static str {
        "feed"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let core = &self.core;
        let activities = json(
            core,
            "SELECT COALESCE(jsonb_agg(to_jsonb(a) ORDER BY a.occurred_at DESC), '[]'::jsonb) FROM (
                 SELECT id, kind, audience, subject_type, subject_id, with_user_ids, summary_ja, summary_en, path, image_url,
                        lat, lng, data, occurred_at
                 FROM activities WHERE user_id = $1
             ) a",
            user_id,
        )
        .await?;
        let hidden_kinds = json(
            core,
            "SELECT COALESCE(jsonb_agg(kind ORDER BY kind), '[]'::jsonb) FROM activity_hidden_kinds WHERE user_id = $1",
            user_id,
        )
        .await?;
        // What the user watched is theirs; the authors are not told.
        let map_story_views = json(
            core,
            "SELECT COALESCE(jsonb_agg(to_jsonb(v) ORDER BY v.seen_at DESC), '[]'::jsonb) FROM (
                 SELECT activity_id, author_id, seen_at FROM map_story_views WHERE viewer_id = $1
             ) v",
            user_id,
        )
        .await?;
        let news_clicks = json(
            core,
            "SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.last_clicked_at DESC), '[]'::jsonb) FROM (
                 SELECT c.article_id, n.title, n.url, c.click_count, c.first_clicked_at, c.last_clicked_at
                 FROM news_clicks c JOIN news_articles n ON n.id = c.article_id WHERE c.user_id = $1
             ) c",
            user_id,
        )
        .await?;
        let news_feedback = json(
            core,
            "SELECT COALESCE(jsonb_agg(to_jsonb(f) ORDER BY f.created_at DESC), '[]'::jsonb) FROM (
                 SELECT f.article_id, n.title, n.url, CASE f.feedback WHEN 1 THEN 'more' ELSE 'less' END AS feedback, f.created_at
                 FROM news_feedback f JOIN news_articles n ON n.id = f.article_id WHERE f.user_id = $1
             ) f",
            user_id,
        )
        .await?;
        let news_interests = json(
            core,
            "SELECT COALESCE(jsonb_agg(to_jsonb(i) ORDER BY i.tag), '[]'::jsonb) FROM (
                 SELECT tag, score, updated_at FROM news_tag_affinity WHERE user_id = $1
             ) i",
            user_id,
        )
        .await?;
        let recap_notices = json(core, "SELECT COALESCE(jsonb_agg(period_start ORDER BY period_start), '[]'::jsonb) FROM feed_recap_notices WHERE user_id = $1", user_id).await?;
        Ok(serde_json::json!({
            "activities": activities,
            "hidden_activity_kinds": hidden_kinds,
            "map_story_views": map_story_views,
            "news_clicks": news_clicks,
            "news_feedback": news_feedback,
            "news_interests": news_interests,
            "recap_notices": recap_notices,
        }))
    }
}
