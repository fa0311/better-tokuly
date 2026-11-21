//! `NewsService`: reading news, and the signals that personalize it.
//!
//! Lists come in two orders:
//!
//! - *latest*: newest first, a plain keyset page. Anonymous readers, and any
//!   request filtered by tag or source, always get this.
//! - *recommended* (signed-in readers): the recent articles ranked by the
//!   reader's interests with random picks mixed in (`ranking.rs`). The order
//!   is computed once per reading session and kept in Valkey for half an
//!   hour; the cursor carries the session token and an offset, so paging is
//!   stable while the reader keeps clicking. Should the snapshot expire, it
//!   is rebuilt from the same token (the random picks depend only on it).
use std::{collections::HashMap, time::Duration};

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder};
use tokuly_core::{analytics::AnalyticsEvent, prelude::*};
use tokuly_proto::feed::*;

use super::{
    articles::{ArticleRow, article_columns, visible_articles},
    ranking::{self, AFFINITY_EPSILON, CLICK_WEIGHT, Candidate, FEEDBACK_WEIGHT, MAX_AFFINITY, Ranked},
    sources::{SourceRow, source_columns, validate_language},
    tagging::normalize_tag,
};
use crate::Feed;

const MAX_PAGE_SIZE: i64 = 50;
const DEFAULT_PAGE_SIZE: i64 = 20;
/// Recent articles a recommended order is built from.
const POOL_SIZE: i64 = 500;
/// How long a recommended order is kept for paging.
const SNAPSHOT_TTL: Duration = Duration::from_secs(30 * 60);
const TOKEN_LEN: usize = 12;
/// Tags listed by `ListNewsTags` unless asked otherwise, and at most.
const DEFAULT_TAG_LIMIT: i64 = 20;
const MAX_TAG_LIMIT: i64 = 100;
/// "Recent" for the popular tags.
const POPULAR_DAYS: i32 = 7;
const POPULAR_CACHE_KEY: &str = "feed:news:popular-tags";
const POPULAR_CACHE_TTL: Duration = Duration::from_secs(300);

/// Position in a recommended order.
#[derive(Debug, Serialize, Deserialize)]
struct RecommendedCursor {
    /// Token of the reading session; the random picks are seeded by it.
    t: String,
    /// Articles already returned.
    o: usize,
}

#[derive(sqlx::FromRow)]
struct CandidateRow {
    id: Uuid,
    tags: Vec<String>,
    published_at: Time,
    clicked: bool,
}

/// An optional tag filter from a request.
fn tag_filter(raw: &str) -> AppResult<Option<String>> {
    if raw.trim().is_empty() {
        return Ok(None);
    }
    match normalize_tag(raw) {
        Some(tag) => Ok(Some(tag)),
        None => bail!(invalid, "INVALID_TAG", "{raw:?} is not a usable tag"),
    }
}

/// An optional language filter from a request.
fn language_filter(raw: &str) -> AppResult<Option<String>> {
    if raw.trim().is_empty() { Ok(None) } else { validate_language(raw).map(Some) }
}

fn push_language(query: &mut QueryBuilder<Postgres>, language: Option<&str>) {
    if let Some(language) = language {
        // `ja` also selects `ja-jp`.
        query.push(" AND (s.language = ").push_bind(language.to_owned());
        query.push(" OR starts_with(s.language, ").push_bind(format!("{language}-")).push("))");
    }
}

impl Feed {
    fn half_life_seconds(&self) -> f64 {
        self.cfg.affinity_half_life_seconds()
    }

    /// The reader's interests as they stand now, by tag.
    async fn affinity_of(&self, user_id: Uuid) -> AppResult<HashMap<String, f64>> {
        let rows: Vec<(String, f64)> = sqlx::query_as(
            "SELECT tag, score * power(0.5::float8, GREATEST(extract(epoch FROM (now() - updated_at)), 0)::float8 / $2)
             FROM news_tag_affinity WHERE user_id = $1",
        )
        .bind(user_id)
        .bind(self.half_life_seconds())
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows.into_iter().collect())
    }

    /// Adds `delta` to the reader's interest in each tag. The stored value
    /// is first brought up to date (it fades with time), then moved and kept
    /// within bounds.
    async fn bump_affinity<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E, user_id: Uuid, tags: &[String], delta: f64) -> AppResult<()> {
        if tags.is_empty() || delta == 0.0 {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO news_tag_affinity AS a (user_id, tag, score, updated_at)
             SELECT $1, tag, LEAST($5, GREATEST(-$5, $3::float8)), now() FROM unnest($2::text[]) AS tag
             ON CONFLICT (user_id, tag) DO UPDATE SET
                 score = LEAST($5, GREATEST(-$5,
                     a.score * power(0.5::float8, GREATEST(extract(epoch FROM (now() - a.updated_at)), 0)::float8 / $4) + $3::float8)),
                 updated_at = now()",
        )
        .bind(user_id)
        .bind(tags)
        .bind(delta)
        .bind(self.half_life_seconds())
        .bind(MAX_AFFINITY)
        .execute(executor)
        .await?;
        Ok(())
    }

    /// The recommended order of the recent articles for a reader.
    async fn build_recommendation(&self, user_id: Uuid, language: Option<&str>, token: &str) -> AppResult<Vec<Ranked>> {
        let mut query: QueryBuilder<Postgres> =
            QueryBuilder::new("SELECT n.id, n.tags, n.published_at, EXISTS(SELECT 1 FROM news_clicks c WHERE c.article_id = n.id AND c.user_id = ");
        query.push_bind(user_id).push(") AS clicked").push(visible_articles!());
        push_language(&mut query, language);
        query.push(" ORDER BY n.published_at DESC, n.id DESC LIMIT ").push_bind(POOL_SIZE);
        let rows: Vec<CandidateRow> = query.build_query_as().fetch_all(&self.core.db).await?;
        let candidates = rows.into_iter().map(|row| Candidate { id: row.id, tags: row.tags, published_at: row.published_at, clicked: row.clicked }).collect();
        let affinity = self.affinity_of(user_id).await?;
        Ok(ranking::recommend(candidates, &affinity, now(), self.cfg.news_random_share_percent, ranking::seed_of(token)))
    }

    async fn list_recommended(&self, user_id: Uuid, language: Option<&str>, page: &Page) -> AppResult<ListNewsResponse> {
        let cursor = match page.decode::<RecommendedCursor>()? {
            Some(cursor) => {
                let well_formed = cursor.t.len() == TOKEN_LEN && cursor.t.bytes().all(|b| b.is_ascii_alphanumeric()) && cursor.o <= POOL_SIZE as usize;
                ensure!(well_formed, invalid, "INVALID_CURSOR", "invalid page cursor");
                cursor
            }
            None => RecommendedCursor { t: tokuly_core::ids::random_base62(TOKEN_LEN), o: 0 },
        };
        // The language is part of the key: a cursor only continues the list it came from.
        let key = format!("feed:news:rec:{user_id}:{}:{}", cursor.t, language.unwrap_or("-"));
        let order: Vec<Ranked> = match self.core.cache.get_json(&key).await? {
            Some(order) if cursor.o > 0 => order,
            _ => {
                let order = self.build_recommendation(user_id, language, &cursor.t).await?;
                self.core.cache.set_json(&key, &order, Some(SNAPSHOT_TTL)).await?;
                order
            }
        };

        let limit = page.limit as usize;
        let slice: Vec<Ranked> = order.iter().skip(cursor.o).take(limit).copied().collect();
        let next = cursor.o + slice.len();
        let has_more = next < order.len();
        let ids: Vec<Uuid> = slice.iter().map(|ranked| ranked.id).collect();
        // Articles taken down since the order was built simply drop out.
        let rows = self.articles_in_order(&ids).await?;
        let marks = self.reader_marks(Some(user_id), &ids).await?;
        let random: HashMap<Uuid, bool> = slice.iter().map(|ranked| (ranked.id, ranked.random_pick)).collect();
        Ok(ListNewsResponse {
            articles: rows.iter().map(|row| row.to_proto(&marks, random.get(&row.id).copied().unwrap_or(false))).collect(),
            page: tokuly_proto::common::PageInfo {
                next_cursor: if has_more { tokuly_core::page::encode_cursor(&RecommendedCursor { t: cursor.t, o: next }) } else { String::new() },
                has_more,
                ..Default::default()
            }
            .into(),
            recommended: true,
            ..Default::default()
        })
    }

    async fn list_latest(&self, viewer: Option<Uuid>, tag: Option<&str>, source_id: Option<Uuid>, language: Option<&str>, page: &Page) -> AppResult<ListNewsResponse> {
        let after: Option<(Time, Uuid)> = page.decode()?;
        let mut query: QueryBuilder<Postgres> = QueryBuilder::new(concat!("SELECT ", article_columns!(), visible_articles!()));
        if let Some(tag) = tag {
            query.push(" AND n.tags @> ARRAY[").push_bind(tag.to_owned()).push("]::text[]");
        }
        if let Some(source_id) = source_id {
            query.push(" AND n.source_id = ").push_bind(source_id);
        }
        push_language(&mut query, language);
        if let Some((at, id)) = after {
            query.push(" AND (n.published_at, n.id) < (").push_bind(at).push(", ").push_bind(id).push(")");
        }
        query.push(" ORDER BY n.published_at DESC, n.id DESC LIMIT ").push_bind(page.fetch());
        let mut rows: Vec<ArticleRow> = query.build_query_as().fetch_all(&self.core.db).await?;
        let info = page.finish(&mut rows, |row| (row.published_at, row.id));
        let marks = self.reader_marks(viewer, &rows.iter().map(|row| row.id).collect::<Vec<_>>()).await?;
        Ok(ListNewsResponse { articles: rows.iter().map(|row| row.to_proto(&marks, false)).collect(), page: info.into(), recommended: false, ..Default::default() })
    }

    /// The tags most common among recent articles, most common first.
    async fn popular_tags(&self) -> AppResult<Vec<(String, i64)>> {
        if let Some(cached) = self.core.cache.get_json::<Vec<(String, i64)>>(POPULAR_CACHE_KEY).await? {
            return Ok(cached);
        }
        let tags: Vec<(String, i64)> = sqlx::query_as(concat!(
            "SELECT tag, count(*) AS articles FROM (SELECT unnest(n.tags) AS tag",
            visible_articles!(),
            " AND n.published_at > now() - make_interval(days => $1)) recent
             GROUP BY tag ORDER BY articles DESC, tag LIMIT $2"
        ))
        .bind(POPULAR_DAYS)
        .bind(MAX_TAG_LIMIT)
        .fetch_all(&self.core.db)
        .await?;
        self.core.cache.set_json(POPULAR_CACHE_KEY, &tags, Some(POPULAR_CACHE_TTL)).await?;
        Ok(tags)
    }
}

/// The client's address for rate limits on anonymous calls.
fn client_key(ctx: &RequestContext) -> String {
    let ip = ctx.client().ip;
    if ip.is_empty() { "unknown".to_owned() } else { ip }
}

impl NewsService for Feed {
    async fn list_news(&self, ctx: RequestContext, request: ServiceRequest<'_, ListNewsRequest>) -> ServiceResult<ListNewsResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let tag = tag_filter(request.tag)?;
            let source_id = parse_opt_id(request.source_id)?;
            let language = language_filter(request.language)?;
            let page = Page::new(request.page.as_option().map_or(0, |p| p.limit), request.page.as_option().map_or("", |p| p.cursor), DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE);
            let wants_latest = request.order.as_known() == Some(NewsOrder::NEWS_ORDER_LATEST);
            let response = match viewer {
                Some(user_id) if !wants_latest && tag.is_none() && source_id.is_none() => self.list_recommended(user_id, language.as_deref(), &page).await?,
                _ => self.list_latest(viewer, tag.as_deref(), source_id, language.as_deref(), &page).await?,
            };
            ok(response)
        })
        .await
    }

    async fn get_news_article(&self, ctx: RequestContext, request: ServiceRequest<'_, GetNewsArticleRequest>) -> ServiceResult<GetNewsArticleResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let row = self.visible_article(parse_id(request.article_id)?).await?;
            let marks = self.reader_marks(viewer, &[row.id]).await?;
            ok(GetNewsArticleResponse { article: row.to_proto(&marks, false).into(), ..Default::default() })
        })
        .await
    }

    async fn record_news_click(&self, ctx: RequestContext, request: ServiceRequest<'_, RecordNewsClickRequest>) -> ServiceResult<RecordNewsClickResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let article_id = parse_id(request.article_id)?;
            let limiter = match viewer {
                Some(user_id) => format!("feed:news-click:user:{user_id}"),
                None => format!("feed:news-click:ip:{}", client_key(&ctx)),
            };
            self.core.cache.rate_limit(&limiter, 120, Duration::from_secs(60)).await?;
            let article = self.visible_article(article_id).await?;

            // An article counts one click per reader: per account, or per
            // address and hour for anonymous readers.
            let first = match viewer {
                Some(user_id) => {
                    let mut tx = self.core.db.begin().await?;
                    let first: bool = sqlx::query_scalar(
                        "INSERT INTO news_clicks AS c (user_id, article_id) VALUES ($1, $2)
                         ON CONFLICT (user_id, article_id) DO UPDATE SET click_count = c.click_count + 1, last_clicked_at = now()
                         RETURNING (xmax = 0)",
                    )
                    .bind(user_id)
                    .bind(article_id)
                    .fetch_one(&mut *tx)
                    .await?;
                    if first {
                        // Opening an article is a (mild) sign of interest in its tags.
                        self.bump_affinity(&mut *tx, user_id, &article.tags, CLICK_WEIGHT).await?;
                        sqlx::query("UPDATE news_articles SET click_count = click_count + 1 WHERE id = $1").bind(article_id).execute(&mut *tx).await?;
                    }
                    tx.commit().await?;
                    first
                }
                None => {
                    let first = self.core.cache.set_nx(&format!("feed:news-click:seen:{article_id}:{}", client_key(&ctx)), "1", Duration::from_secs(3600)).await?;
                    if first {
                        sqlx::query("UPDATE news_articles SET click_count = click_count + 1 WHERE id = $1").bind(article_id).execute(&self.core.db).await?;
                    }
                    first
                }
            };
            self.core
                .analytics
                .track(AnalyticsEvent::new("news.click").user(viewer).subject("news", article_id).props(serde_json::json!({ "source_id": article.source_id, "first": first })))
                .await;
            ok(RecordNewsClickResponse::default())
        })
        .await
    }

    async fn send_news_feedback(&self, ctx: RequestContext, request: ServiceRequest<'_, SendNewsFeedbackRequest>) -> ServiceResult<SendNewsFeedbackResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let article_id = parse_id(request.article_id)?;
            let wanted: Option<i16> = match request.feedback.as_known() {
                Some(NewsFeedback::NEWS_FEEDBACK_MORE) => Some(1),
                Some(NewsFeedback::NEWS_FEEDBACK_LESS) => Some(2),
                Some(NewsFeedback::NEWS_FEEDBACK_UNSPECIFIED) => None,
                None => bail!(invalid, "INVALID_FEEDBACK", "unknown feedback value"),
            };
            self.core.cache.rate_limit(&format!("feed:news-feedback:{user_id}"), 60, Duration::from_secs(60)).await?;
            let article = self.visible_article(article_id).await?;
            let signed = |feedback: i16| if feedback == 1 { FEEDBACK_WEIGHT } else { -FEEDBACK_WEIGHT };

            let mut tx = self.core.db.begin().await?;
            // One reader's feedback changes are applied one after the other.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))").bind(format!("feed:news-feedback:{user_id}:{article_id}")).execute(&mut *tx).await?;
            let previous: Option<(i16, f64)> =
                sqlx::query_as("SELECT feedback, extract(epoch FROM (now() - created_at))::float8 FROM news_feedback WHERE user_id = $1 AND article_id = $2")
                    .bind(user_id)
                    .bind(article_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if previous.map(|(feedback, _)| feedback) != wanted {
                // Take back what is left of the earlier feedback (it faded
                // like every interest), then apply the new one.
                let undo = previous.map_or(0.0, |(feedback, age)| -ranking::decayed(signed(feedback), age, self.half_life_seconds()));
                let apply = wanted.map_or(0.0, signed);
                match wanted {
                    Some(feedback) => {
                        sqlx::query(
                            "INSERT INTO news_feedback (user_id, article_id, feedback) VALUES ($1, $2, $3)
                             ON CONFLICT (user_id, article_id) DO UPDATE SET feedback = EXCLUDED.feedback, created_at = now()",
                        )
                        .bind(user_id)
                        .bind(article_id)
                        .bind(feedback)
                        .execute(&mut *tx)
                        .await?;
                    }
                    None => {
                        sqlx::query("DELETE FROM news_feedback WHERE user_id = $1 AND article_id = $2").bind(user_id).bind(article_id).execute(&mut *tx).await?;
                    }
                }
                self.bump_affinity(&mut *tx, user_id, &article.tags, undo + apply).await?;
            }
            tx.commit().await?;
            ok(SendNewsFeedbackResponse::default())
        })
        .await
    }

    async fn list_news_tags(&self, ctx: RequestContext, request: ServiceRequest<'_, ListNewsTagsRequest>) -> ServiceResult<ListNewsTagsResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let limit = if request.limit <= 0 { DEFAULT_TAG_LIMIT } else { i64::from(request.limit).min(MAX_TAG_LIMIT) } as usize;
            let (mut interests, mut avoided) = (Vec::new(), Vec::new());
            if let Some(user_id) = viewer {
                let mut affinity: Vec<(String, f64)> = self.affinity_of(user_id).await?.into_iter().filter(|(_, score)| score.abs() >= AFFINITY_EPSILON).collect();
                // Strongest feelings first; ties in a stable order.
                affinity.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()).then_with(|| a.0.cmp(&b.0)));
                let interest = |(tag, score): &(String, f64)| NewsInterest { tag: tag.clone(), score: *score, ..Default::default() };
                interests = affinity.iter().filter(|(_, score)| *score > 0.0).take(limit).map(interest).collect();
                avoided = affinity.iter().filter(|(_, score)| *score < 0.0).take(limit).map(interest).collect();
            }
            let popular = self
                .popular_tags()
                .await?
                .into_iter()
                .take(limit)
                .map(|(tag, articles)| NewsTagCount { tag, article_count: articles.min(i64::from(i32::MAX)) as i32, ..Default::default() })
                .collect();
            ok(ListNewsTagsResponse { interests, avoided, popular, ..Default::default() })
        })
        .await
    }

    async fn reset_news_interests(&self, ctx: RequestContext, request: ServiceRequest<'_, ResetNewsInterestsRequest>) -> ServiceResult<ResetNewsInterestsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let mut tx = self.core.db.begin().await?;
            sqlx::query("DELETE FROM news_tag_affinity WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM news_feedback WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
            if request.clear_history {
                sqlx::query("DELETE FROM news_clicks WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
            }
            tx.commit().await?;
            ok(ResetNewsInterestsResponse::default())
        })
        .await
    }

    async fn list_news_sources(&self, _ctx: RequestContext, _request: ServiceRequest<'_, ListNewsSourcesRequest>) -> ServiceResult<ListNewsSourcesResponse> {
        handle(async {
            let rows: Vec<SourceRow> =
                sqlx::query_as(concat!("SELECT ", source_columns!(), " FROM news_sources s WHERE s.enabled ORDER BY lower(s.name), s.id")).fetch_all(&self.core.db).await?;
            ok(ListNewsSourcesResponse { sources: rows.iter().map(SourceRow::to_proto).collect(), ..Default::default() })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_are_optional_and_validated() {
        assert_eq!(tag_filter("").unwrap(), None);
        assert_eq!(tag_filter("  ").unwrap(), None);
        assert_eq!(tag_filter(" #Tech ").unwrap().as_deref(), Some("tech"));
        assert_eq!(tag_filter("a,b").unwrap_err().reason, "INVALID_TAG");

        assert_eq!(language_filter("").unwrap(), None);
        assert_eq!(language_filter("JA").unwrap().as_deref(), Some("ja"));
        assert_eq!(language_filter("japanese").unwrap_err().reason, "INVALID_LANGUAGE");
    }

    #[test]
    fn recommended_cursors_round_trip() {
        let encoded = tokuly_core::page::encode_cursor(&RecommendedCursor { t: "abcDEF123456".into(), o: 40 });
        let decoded: RecommendedCursor = tokuly_core::page::decode_cursor(&encoded).unwrap();
        assert_eq!((decoded.t.as_str(), decoded.o), ("abcDEF123456", 40));
        // A keyset cursor of the "latest" order is not a recommended cursor.
        let keyset = tokuly_core::page::encode_cursor(&(now(), Uuid::now_v7()));
        assert!(tokuly_core::page::decode_cursor::<RecommendedCursor>(&keyset).is_err());
    }
}
