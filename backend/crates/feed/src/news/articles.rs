//! News articles: rows, API messages, search documents and the subject
//! resolver that lets the engagement domain attach comments and likes.
//!
//! Article pictures and source icons are stored as the feeds give them and
//! handed out only as URLs of media's image proxy ([`picture`]), like the
//! pictures of link previews: readers' apps never ask the news sites.
use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use tokuly_core::{
    outbound::SearchDoc,
    prelude::*,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_media::link_preview::image_proxy::{self, Kind};
use tokuly_proto::feed::{NewsArticle, NewsFeedback, NewsSource};

use crate::Feed;

/// Search index kind of news articles.
pub const SEARCH_KIND: &str = "news";

/// Columns of [`ArticleRow`]: an article `n` joined with its source `s`.
macro_rules! article_columns {
    () => {
        "n.id, n.source_id, n.url, n.title, n.summary, n.image_url, n.tags, n.published_at, n.click_count, n.like_count, \
         n.comment_count, s.name AS source_name, s.site_url AS source_site_url, s.favicon_url AS source_favicon_url, \
         s.language AS source_language"
    };
}
pub(crate) use article_columns;

/// `FROM ... WHERE` of the articles readers may see: not taken down, and
/// from a source that is switched on.
macro_rules! visible_articles {
    () => {
        " FROM news_articles n JOIN news_sources s ON s.id = n.source_id WHERE n.removed_at IS NULL AND s.enabled"
    };
}
pub(crate) use visible_articles;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ArticleRow {
    pub id: Uuid,
    pub source_id: Uuid,
    pub url: String,
    pub title: String,
    pub summary: String,
    pub image_url: String,
    pub tags: Vec<String>,
    pub published_at: Time,
    pub click_count: i64,
    pub like_count: i64,
    pub comment_count: i64,
    pub source_name: String,
    pub source_site_url: String,
    pub source_favicon_url: String,
    pub source_language: String,
}

/// The URL clients load a news picture from: the image proxy's URL for
/// `url` (empty when there is nothing to show).
pub fn picture(core: &Core, kind: Kind, url: &str) -> String {
    if url.is_empty() { String::new() } else { image_proxy::proxy_url(core, kind, url) }
}

/// Path of an article's page in the app.
pub fn article_path(id: Uuid) -> String {
    format!("/news/articles/{id}")
}

/// What the search index holds for an article.
pub struct ArticleDoc<'a> {
    pub id: Uuid,
    pub url: &'a str,
    pub title: &'a str,
    pub summary: &'a str,
    pub image_url: &'a str,
    pub tags: &'a [String],
    pub published_at: Time,
    pub click_count: i64,
    pub source_id: Uuid,
    pub source_name: &'a str,
    pub source_language: &'a str,
}

impl ArticleDoc<'_> {
    pub fn search_doc(&self, core: &Core) -> SearchDoc {
        let image_url = picture(core, Kind::Image, self.image_url);
        SearchDoc {
            kind: SEARCH_KIND.to_owned(),
            id: self.id,
            title: self.title.to_owned(),
            text: self.summary.to_owned(),
            tags: self.tags.to_vec(),
            // Articles belong to nobody: official content.
            owner_id: None,
            public: true,
            image_url: (!image_url.is_empty()).then_some(image_url),
            path: article_path(self.id),
            created_at: self.published_at.timestamp(),
            score: self.click_count as f64,
            extra: serde_json::json!({
                "url": self.url,
                "source_id": self.source_id,
                "source_name": self.source_name,
                "language": self.source_language,
            }),
        }
    }
}

impl ArticleRow {
    pub fn source(&self, core: &Core) -> NewsSource {
        NewsSource {
            id: self.source_id.to_string(),
            name: self.source_name.clone(),
            site_url: self.source_site_url.clone(),
            favicon_url: picture(core, Kind::Icon, &self.source_favicon_url),
            language: self.source_language.clone(),
            ..Default::default()
        }
    }

    pub fn doc(&self) -> ArticleDoc<'_> {
        ArticleDoc {
            id: self.id,
            url: &self.url,
            title: &self.title,
            summary: &self.summary,
            image_url: &self.image_url,
            tags: &self.tags,
            published_at: self.published_at,
            click_count: self.click_count,
            source_id: self.source_id,
            source_name: &self.source_name,
            source_language: &self.source_language,
        }
    }

    /// The article as a reader sees it.
    pub fn to_proto(&self, core: &Core, marks: &ReaderMarks, random_pick: bool) -> NewsArticle {
        let feedback = match marks.feedback.get(&self.id) {
            Some(1) => NewsFeedback::NEWS_FEEDBACK_MORE,
            Some(2) => NewsFeedback::NEWS_FEEDBACK_LESS,
            _ => NewsFeedback::NEWS_FEEDBACK_UNSPECIFIED,
        };
        NewsArticle {
            id: self.id.to_string(),
            title: self.title.clone(),
            url: self.url.clone(),
            summary: self.summary.clone(),
            image_url: picture(core, Kind::Image, &self.image_url),
            source: self.source(core).into(),
            tags: self.tags.clone(),
            published_at: ts(self.published_at),
            random_pick,
            clicked: marks.clicked.contains(&self.id),
            feedback: feedback.into(),
            path: article_path(self.id),
            click_count: self.click_count,
            like_count: self.like_count,
            comment_count: self.comment_count,
            ..Default::default()
        }
    }
}

/// What a reader did with a set of articles.
#[derive(Debug, Default)]
pub struct ReaderMarks {
    pub clicked: HashSet<Uuid>,
    /// `tokuly.feed.v1.NewsFeedback` numbers by article.
    pub feedback: HashMap<Uuid, i16>,
}

impl Feed {
    /// An article readers may see, or `NEWS_ARTICLE_NOT_FOUND`.
    pub(crate) async fn visible_article(&self, id: Uuid) -> AppResult<ArticleRow> {
        sqlx::query_as(concat!("SELECT ", article_columns!(), visible_articles!(), " AND n.id = $1"))
            .bind(id)
            .fetch_optional(&self.core.db)
            .await?
            .or_not_found("NEWS_ARTICLE_NOT_FOUND", "news article not found")
    }

    /// The visible articles among `ids`, in the order of `ids`.
    pub(crate) async fn articles_in_order(&self, ids: &[Uuid]) -> AppResult<Vec<ArticleRow>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<ArticleRow> =
            sqlx::query_as(concat!("SELECT ", article_columns!(), visible_articles!(), " AND n.id = ANY($1)"))
                .bind(ids)
                .fetch_all(&self.core.db)
                .await?;
        let mut by_id: HashMap<Uuid, ArticleRow> = rows.into_iter().map(|row| (row.id, row)).collect();
        Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
    }

    /// The reader's clicks and feedback on `ids` (nothing for anonymous readers).
    pub(crate) async fn reader_marks(&self, viewer: Option<Uuid>, ids: &[Uuid]) -> AppResult<ReaderMarks> {
        let Some(viewer) = viewer else { return Ok(ReaderMarks::default()) };
        if ids.is_empty() {
            return Ok(ReaderMarks::default());
        }
        let clicked: Vec<Uuid> =
            sqlx::query_scalar("SELECT article_id FROM news_clicks WHERE user_id = $1 AND article_id = ANY($2)")
                .bind(viewer)
                .bind(ids)
                .fetch_all(&self.core.db)
                .await?;
        let feedback: Vec<(Uuid, i16)> = sqlx::query_as(
            "SELECT article_id, feedback FROM news_feedback WHERE user_id = $1 AND article_id = ANY($2)",
        )
        .bind(viewer)
        .bind(ids)
        .fetch_all(&self.core.db)
        .await?;
        Ok(ReaderMarks { clicked: clicked.into_iter().collect(), feedback: feedback.into_iter().collect() })
    }
}

/// Lets generic domains (comments, likes, reports, bookmarks) work on news
/// articles. Articles are official content: their owner is the nil UUID,
/// which nobody notifies.
pub(crate) struct NewsSubjects {
    pub core: Core,
}

#[async_trait]
impl SubjectResolver for NewsSubjects {
    async fn resolve(&self, id: &str, _viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let Ok(id) = Uuid::parse_str(id) else { return Ok(None) };
        let row: Option<(String, String)> =
            sqlx::query_as(concat!("SELECT n.title, n.image_url", visible_articles!(), " AND n.id = $1"))
                .bind(id)
                .fetch_optional(&self.core.db)
                .await?;
        Ok(row.map(|(title, image_url)| SubjectInfo {
            id,
            owner_id: Uuid::nil(),
            title,
            path: article_path(id),
            image_url: Some(picture(&self.core, Kind::Image, &image_url)).filter(|url| !url.is_empty()),
            interactions_enabled: true,
        }))
    }

    async fn counter_changed(&self, id: Uuid, counter: &str, delta: i64) -> AppResult<()> {
        let sql = match counter {
            "like_count" => "UPDATE news_articles SET like_count = GREATEST(like_count + $2, 0) WHERE id = $1",
            "comment_count" => "UPDATE news_articles SET comment_count = GREATEST(comment_count + $2, 0) WHERE id = $1",
            _ => return Ok(()),
        };
        sqlx::query(sql).bind(id).bind(delta).execute(&self.core.db).await?;
        Ok(())
    }
}
