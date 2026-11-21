//! Fetching news sources: the scheduler, the fetch job and storing articles.
//!
//! Every enabled source has a `next_fetch_at`. The scheduler
//! ([`Feed::enqueue_due_fetches`]) claims the sources that are due and queues
//! one `feed.news.fetch` job each; the job ([`Feed::refresh_source`])
//! downloads the feed conditionally, parses it and stores what is new.
//!
//! A source that fails (network, HTTP status, oversized or malformed
//! document, refused address) is not retried by the job queue: the failure is
//! recorded on the source and its next fetch is pushed back exponentially,
//! up to a day.
//!
//! Articles are identified by their canonical URL: an article carried by two
//! feeds, or twice by one, is stored once and belongs to the source that
//! brought it first. Only that source updates its title, summary, image and
//! tags afterwards.
use std::{collections::HashMap, time::Duration};

use chrono::Duration as Span;
use sqlx::{Postgres, Transaction};
use tokuly_core::{outbound, prelude::*};
use url::Url;

use super::{
    articles::{ArticleDoc, SEARCH_KIND},
    fetch::{self, Document, Fetched, Limits, Validators},
    parse::{self, ParsedArticle, ParsedFeed},
    sources::{NewSource, SourceRow, validate_feed_url},
    tagging::{MAX_ARTICLE_TAGS, merge_tags},
};
use crate::{
    Feed,
    events::{FETCH_JOB, FetchSourceJob},
};

/// Sources claimed by one scheduler pass.
const SCHEDULER_BATCH: i64 = 50;
/// Longest pause between attempts on a failing source.
const MAX_BACKOFF: Duration = Duration::from_secs(24 * 3600);
/// `last_seen_at` of an article still in its feed is refreshed this often.
pub(crate) const SEEN_REFRESH_HOURS: i64 = 12;
const MAX_ERROR_CHARS: usize = 500;

/// What one fetch of a source came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// The source is gone, switched off, or being fetched by someone else.
    Skipped,
    /// The server answered "304 Not Modified".
    NotModified,
    /// A document was read: `new` articles stored, `updated` articles changed.
    Fetched { new: usize, updated: usize },
    /// The fetch failed; the reason is recorded on the source.
    Failed(String),
}

/// Seconds until the next attempt after `failures` failures in a row.
pub fn backoff_seconds(interval_seconds: u64, failures: i32) -> u64 {
    let doublings = failures.clamp(0, 6) as u32;
    interval_seconds.saturating_mul(1 << doublings).min(MAX_BACKOFF.as_secs())
}

#[derive(sqlx::FromRow)]
struct ExistingArticle {
    id: Uuid,
    source_id: Uuid,
    canonical_url: String,
    title: String,
    summary: String,
    image_url: String,
    tags: Vec<String>,
    click_count: i64,
    removed_at: Option<Time>,
}

impl Feed {
    /// Queues a fetch of one source. Best effort: a source that could not be
    /// queued is still picked up by the scheduler when it is due.
    pub(crate) async fn enqueue_fetch(&self, source_id: Uuid) {
        if let Err(error) = self.core.jobs.enqueue(FETCH_JOB, &FetchSourceJob { source_id }).await {
            tracing::warn!(source = %source_id, %error, "could not queue a news fetch");
        }
    }

    /// Claims the sources that are due and queues a fetch for each. Safe to
    /// run on several workers at once. Returns how many were queued.
    pub async fn enqueue_due_fetches(&self) -> AppResult<usize> {
        // The claim pushes `next_fetch_at` forward, so a lost job only costs one interval.
        let due: Vec<Uuid> = sqlx::query_scalar(
            "UPDATE news_sources SET next_fetch_at = now() + make_interval(secs => $1)
             WHERE id IN (
                 SELECT id FROM news_sources WHERE enabled AND next_fetch_at <= now()
                 ORDER BY next_fetch_at LIMIT $2 FOR UPDATE SKIP LOCKED
             )
             RETURNING id",
        )
        .bind(self.cfg.news_fetch_interval_seconds as f64)
        .bind(SCHEDULER_BATCH)
        .fetch_all(&self.core.db)
        .await?;
        for source_id in &due {
            self.enqueue_fetch(*source_id).await;
        }
        Ok(due.len())
    }

    /// Creates the sources listed in `TOKULY_NEWS_FEEDS` that do not exist
    /// yet. Entries that are not fetchable URLs are logged and skipped.
    /// Returns how many sources were created.
    pub async fn seed_config_sources(&self) -> AppResult<usize> {
        let mut created = 0;
        for raw in self.cfg.seed_feed_urls() {
            let feed_url = match validate_feed_url(raw, self.news_policy) {
                Ok(url) => url,
                Err(error) => {
                    tracing::warn!(url = raw, reason = %error.reason, "ignoring an entry of TOKULY_NEWS_FEEDS");
                    continue;
                }
            };
            let new = NewSource {
                feed_url,
                name: String::new(),
                site_url: String::new(),
                language: "ja".to_owned(),
                default_tags: Vec::new(),
                enabled: true,
                from_config: true,
            };
            let mut tx = self.core.db.begin().await?;
            let inserted = self.insert_source(&mut tx, &new).await?;
            tx.commit().await?;
            if let Some(source) = inserted {
                tracing::info!(source = %source.id, url = %source.feed_url, "news source created from TOKULY_NEWS_FEEDS");
                created += 1;
            }
        }
        Ok(created)
    }

    /// Fetches one source now and stores what it carries. Fetch failures are
    /// an outcome, not an error: they are recorded on the source.
    pub async fn refresh_source(&self, source_id: Uuid) -> AppResult<RefreshOutcome> {
        let Some(source) = self.find_source(source_id).await?.filter(|source| source.enabled) else {
            return Ok(RefreshOutcome::Skipped);
        };
        // One fetch per source at a time (the scheduler and a manual refresh may race).
        let lock = format!("feed:news:fetching:{source_id}");
        let lock_ttl = Duration::from_secs(self.cfg.news_fetch_timeout_seconds + 60);
        if !self.core.cache.set_nx(&lock, "1", lock_ttl).await? {
            return Ok(RefreshOutcome::Skipped);
        }
        let outcome = self.refresh_locked(&source).await;
        if let Err(error) = self.core.cache.del(&lock).await {
            tracing::warn!(source = %source_id, %error, "could not release a news fetch lock");
        }
        outcome
    }

    async fn refresh_locked(&self, source: &SourceRow) -> AppResult<RefreshOutcome> {
        let url = match Url::parse(&source.feed_url) {
            Ok(url) => url,
            Err(error) => return self.record_failure(source, format!("the feed URL is not valid: {error}")).await,
        };
        let limits = Limits {
            timeout: Duration::from_secs(self.cfg.news_fetch_timeout_seconds),
            max_bytes: self.cfg.news_max_feed_bytes,
        };
        let validators = Validators { etag: source.etag.clone(), last_modified: source.last_modified.clone() };
        let document = match fetch::fetch(&self.news_http, self.news_policy, limits, url, &validators).await {
            Ok(Fetched::NotModified) => {
                self.record_not_modified(source).await?;
                return Ok(RefreshOutcome::NotModified);
            }
            Ok(Fetched::Document(document)) => document,
            Err(error) => return self.record_failure(source, error.to_string()).await,
        };

        // Parsing megabytes of XML is CPU work: off the async threads.
        let fetched_at = now();
        let Document { url: document_url, body, validators } = document;
        let parsed = tokio::task::spawn_blocking(move || parse::parse_feed(&body, &document_url, fetched_at))
            .await
            .map_err(|e| AppError::internal(anyhow::anyhow!("feed parser task: {e}")))?;
        let parsed = match parsed {
            Ok(parsed) => parsed,
            Err(error) => return self.record_failure(source, format!("the document is not a feed: {error}")).await,
        };
        let (new, updated) = self.store_feed(source, parsed, &validators, fetched_at).await?;
        Ok(RefreshOutcome::Fetched { new, updated })
    }

    /// Stores the articles of a freshly read feed and marks the source as
    /// fetched, in one transaction.
    async fn store_feed(
        &self,
        source: &SourceRow,
        feed: ParsedFeed,
        validators: &Validators,
        fetched_at: Time,
    ) -> AppResult<(usize, usize)> {
        // What would be pruned right away is not imported.
        let oldest = fetched_at - Span::days(i64::from(self.cfg.news_retention_days));
        let articles: Vec<ParsedArticle> =
            feed.articles.into_iter().filter(|article| article.published_at.is_none_or(|at| at >= oldest)).collect();
        let canonical_urls: Vec<String> = articles.iter().map(|article| article.canonical_url.clone()).collect();
        // The feed may have told us what the source is called.
        let source_name = if source.name_from_feed && !feed.info.title.is_empty() {
            feed.info.title.as_str()
        } else {
            source.name.as_str()
        };

        let mut tx = self.core.db.begin().await?;
        let existing: Vec<ExistingArticle> = sqlx::query_as(
            "SELECT id, source_id, canonical_url, title, summary, image_url, tags, click_count, removed_at
             FROM news_articles WHERE canonical_url = ANY($1)",
        )
        .bind(&canonical_urls)
        .fetch_all(&mut *tx)
        .await?;
        let existing: HashMap<&str, &ExistingArticle> =
            existing.iter().map(|row| (row.canonical_url.as_str(), row)).collect();

        let (mut new, mut updated) = (0, 0);
        for article in &articles {
            let tags = self.article_tags(source, article);
            let published_at = article.published_at.unwrap_or(fetched_at);
            let doc = |id: Uuid, click_count: i64| ArticleDoc {
                id,
                url: &article.url,
                title: &article.title,
                summary: &article.summary,
                image_url: &article.image_url,
                tags: &tags,
                published_at,
                click_count,
                source_id: source.id,
                source_name,
                source_language: &source.language,
            };
            match existing.get(article.canonical_url.as_str()) {
                None => {
                    // Another source's fetch may be storing the same article right now.
                    let id: Option<Uuid> = sqlx::query_scalar(
                        "INSERT INTO news_articles (source_id, canonical_url, url, title, summary, image_url, tags, published_at, last_seen_at)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                         ON CONFLICT (canonical_url) DO NOTHING
                         RETURNING id",
                    )
                    .bind(source.id)
                    .bind(&article.canonical_url)
                    .bind(&article.url)
                    .bind(&article.title)
                    .bind(&article.summary)
                    .bind(&article.image_url)
                    .bind(&tags)
                    .bind(published_at)
                    .bind(fetched_at)
                    .fetch_optional(&mut *tx)
                    .await?;
                    if let Some(id) = id {
                        outbound::index(&mut *tx, doc(id, 0).search_doc()).await?;
                        new += 1;
                    }
                }
                Some(known) => {
                    let ours = known.source_id == source.id && known.removed_at.is_none();
                    let changed = known.title != article.title
                        || known.summary != article.summary
                        || known.image_url != article.image_url
                        || known.tags != tags;
                    if ours && changed {
                        sqlx::query("UPDATE news_articles SET url = $2, title = $3, summary = $4, image_url = $5, tags = $6 WHERE id = $1")
                            .bind(known.id)
                            .bind(&article.url)
                            .bind(&article.title)
                            .bind(&article.summary)
                            .bind(&article.image_url)
                            .bind(&tags)
                            .execute(&mut *tx)
                            .await?;
                        // The publication time stays what it was when first seen.
                        let mut search_doc = doc(known.id, known.click_count).search_doc();
                        search_doc.created_at = self.published_at_of(&mut tx, known.id).await?.timestamp();
                        outbound::index(&mut *tx, search_doc).await?;
                        updated += 1;
                    }
                }
            }
        }
        // Still in a feed: remembered at most twice a day per article.
        sqlx::query("UPDATE news_articles SET last_seen_at = $2 WHERE canonical_url = ANY($1) AND last_seen_at < $2 - make_interval(hours => $3)")
            .bind(&canonical_urls)
            .bind(fetched_at)
            .bind(SEEN_REFRESH_HOURS as i32)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE news_sources SET
                 etag = $2, last_modified = $3, last_fetched_at = $4, last_success_at = $4, document_at = $4,
                 last_error = '', consecutive_failures = 0, next_fetch_at = $4 + make_interval(secs => $5),
                 name = CASE WHEN name_from_feed AND $6 <> '' THEN $6 ELSE name END,
                 site_url = CASE WHEN site_from_feed AND $7 <> '' THEN $7 ELSE site_url END,
                 favicon_url = CASE WHEN $8 <> '' THEN $8 ELSE favicon_url END
             WHERE id = $1",
        )
        .bind(source.id)
        .bind(&validators.etag)
        .bind(&validators.last_modified)
        .bind(fetched_at)
        .bind(self.cfg.news_fetch_interval_seconds as f64)
        .bind(&feed.info.title)
        .bind(&feed.info.site_url)
        .bind(&feed.info.favicon_url)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((new, updated))
    }

    async fn published_at_of(&self, tx: &mut Transaction<'_, Postgres>, article_id: Uuid) -> AppResult<Time> {
        Ok(sqlx::query_scalar("SELECT published_at FROM news_articles WHERE id = $1")
            .bind(article_id)
            .fetch_one(&mut **tx)
            .await?)
    }

    /// The tags of an article: the source's, the feed's categories, and
    /// keyword matches in the title and summary.
    fn article_tags(&self, source: &SourceRow, article: &ParsedArticle) -> Vec<String> {
        let keyword_tags = self.dictionary.tags_for(&format!("{} {}", article.title, article.summary));
        merge_tags(
            source.default_tags.iter().chain(article.categories.iter()).chain(keyword_tags.iter()),
            MAX_ARTICLE_TAGS,
        )
    }

    /// "304 Not Modified": the source is fine and the feed unchanged, so the
    /// articles it carried last time are still in it.
    async fn record_not_modified(&self, source: &SourceRow) -> AppResult<()> {
        sqlx::query(
            "UPDATE news_sources SET last_fetched_at = now(), last_success_at = now(), last_error = '', consecutive_failures = 0,
                                     next_fetch_at = now() + make_interval(secs => $2)
             WHERE id = $1",
        )
        .bind(source.id)
        .bind(self.cfg.news_fetch_interval_seconds as f64)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// Records a failed fetch and schedules the next attempt with backoff.
    async fn record_failure(&self, source: &SourceRow, reason: String) -> AppResult<RefreshOutcome> {
        let reason: String = reason.chars().filter(|c| !c.is_control()).take(MAX_ERROR_CHARS).collect();
        let failures = source.consecutive_failures.saturating_add(1);
        tracing::warn!(source = %source.id, url = %source.feed_url, failures, reason = %reason, "news fetch failed");
        sqlx::query(
            "UPDATE news_sources SET last_fetched_at = now(), last_error = $2, consecutive_failures = $3,
                                     next_fetch_at = now() + make_interval(secs => $4)
             WHERE id = $1",
        )
        .bind(source.id)
        .bind(&reason)
        .bind(failures)
        .bind(backoff_seconds(self.cfg.news_fetch_interval_seconds, failures) as f64)
        .execute(&self.core.db)
        .await?;
        Ok(RefreshOutcome::Failed(reason))
    }

    /// Deletes articles that are older than the retention period and no
    /// longer in their feed, and takes them out of search. Returns how many.
    pub async fn prune_articles(&self, batch: i64) -> AppResult<u64> {
        let mut pruned = 0;
        loop {
            let mut tx = self.core.db.begin().await?;
            // "No longer in the feed": not seen for a while before the feed
            // was last read. Sources that are off, or that have not answered
            // for a week, cannot vouch for their articles anymore.
            let ids: Vec<Uuid> = sqlx::query_scalar(
                "DELETE FROM news_articles WHERE id IN (
                     SELECT n.id FROM news_articles n JOIN news_sources s ON s.id = n.source_id
                     WHERE n.published_at < now() - make_interval(days => $1)
                       AND (NOT s.enabled
                            OR s.last_success_at IS NULL OR s.last_success_at < now() - interval '7 days'
                            OR n.last_seen_at < s.document_at - make_interval(hours => $2))
                     LIMIT $3
                 )
                 RETURNING id",
            )
            .bind(self.cfg.news_retention_days as i32)
            .bind(SEEN_REFRESH_HOURS as i32)
            .bind(batch)
            .fetch_all(&mut *tx)
            .await?;
            for id in &ids {
                outbound::unindex(&mut *tx, SEARCH_KIND, *id).await?;
            }
            tx.commit().await?;
            pruned += ids.len() as u64;
            if (ids.len() as i64) < batch {
                return Ok(pruned);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failing_sources_back_off_exponentially_up_to_a_day() {
        let interval = 900;
        assert_eq!(backoff_seconds(interval, 1), 1800);
        assert_eq!(backoff_seconds(interval, 2), 3600);
        assert_eq!(backoff_seconds(interval, 3), 7200);
        assert_eq!(backoff_seconds(interval, 6), 57_600);
        assert_eq!(backoff_seconds(interval, 7), 57_600, "the doubling stops");
        assert_eq!(backoff_seconds(interval, 1000), 57_600);
        assert_eq!(backoff_seconds(interval, 0), 900);
        assert_eq!(backoff_seconds(interval, -3), 900);
        // Long intervals are capped at a day.
        assert_eq!(backoff_seconds(7200, 5), 86_400);
        assert_eq!(backoff_seconds(200_000, 1), 86_400);
        assert_eq!(backoff_seconds(u64::MAX, 3), 86_400);
    }
}
