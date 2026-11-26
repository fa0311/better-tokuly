//! News sources and `NewsAdminService`.
//!
//! A source is an RSS/Atom feed plus what admins say about it (name, site,
//! language, default tags). Every RPC here needs the `admin` role, except
//! taking a single article down, which moderators may do too.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use sqlx::{Postgres, Transaction};
use tokuly_core::{
    egress::{Policy, check_url},
    outbound,
    prelude::*,
};
use tokuly_media::link_preview::image_proxy::Kind;
use tokuly_proto::{common::SubjectType, feed::*};
use url::Url;

use super::{
    articles::{ArticleRow, SEARCH_KIND, article_columns, picture},
    parse::MAX_URL_LEN,
    tagging::normalize_tag,
};
use crate::Feed;

/// Sources that can exist at once.
pub const MAX_SOURCES: i64 = 500;
const MAX_NAME_CHARS: usize = 100;
const MAX_DEFAULT_TAGS: usize = 10;
const DEFAULT_LANGUAGE: &str = "ja";

/// Columns of [`SourceRow`], read from a table aliased `s`.
macro_rules! source_columns {
    () => {
        "s.id, s.name, s.feed_url, s.site_url, s.favicon_url, s.language, s.default_tags, s.enabled, s.from_config, s.name_from_feed, \
         s.site_from_feed, s.etag, s.last_modified, s.last_fetched_at, s.last_success_at, s.document_at, s.last_error, \
         s.consecutive_failures, s.next_fetch_at, s.created_at, s.updated_at"
    };
}
pub(crate) use source_columns;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SourceRow {
    pub id: Uuid,
    pub name: String,
    pub feed_url: String,
    pub site_url: String,
    pub favicon_url: String,
    pub language: String,
    pub default_tags: Vec<String>,
    pub enabled: bool,
    pub from_config: bool,
    pub name_from_feed: bool,
    pub site_from_feed: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub last_fetched_at: Option<Time>,
    pub last_success_at: Option<Time>,
    pub document_at: Option<Time>,
    pub last_error: String,
    pub consecutive_failures: i32,
    pub next_fetch_at: Time,
    pub created_at: Time,
    pub updated_at: Time,
}

impl SourceRow {
    pub fn to_proto(&self, core: &Core) -> NewsSource {
        NewsSource {
            id: self.id.to_string(),
            name: self.name.clone(),
            site_url: self.site_url.clone(),
            favicon_url: picture(core, Kind::Icon, &self.favicon_url),
            language: self.language.clone(),
            ..Default::default()
        }
    }

    fn to_managed(&self, core: &Core, article_count: i64, configured: &ConfiguredFeeds) -> ManagedNewsSource {
        ManagedNewsSource {
            configured: configured.lists(&self.feed_url),
            id: self.id.to_string(),
            name: self.name.clone(),
            feed_url: self.feed_url.clone(),
            site_url: self.site_url.clone(),
            favicon_url: picture(core, Kind::Icon, &self.favicon_url),
            language: self.language.clone(),
            default_tags: self.default_tags.clone(),
            enabled: self.enabled,
            from_config: self.from_config,
            last_fetched_at: opt_ts(self.last_fetched_at),
            last_success_at: opt_ts(self.last_success_at),
            last_error: self.last_error.clone(),
            consecutive_failures: self.consecutive_failures,
            next_fetch_at: ts(self.next_fetch_at),
            article_count: article_count.min(i64::from(i32::MAX)) as i32,
            created_at: ts(self.created_at),
            updated_at: ts(self.updated_at),
            ..Default::default()
        }
    }
}

/// The feeds `TOKULY_NEWS_FEEDS` lists now, as sources store their URLs.
/// A source of one of them is created again on every start
/// (`Feed::seed_config_sources`), so it cannot be deleted, only switched off.
pub(crate) struct ConfiguredFeeds(Vec<String>);

impl ConfiguredFeeds {
    /// The entries of the setting that are fetchable URLs; the others never
    /// become sources.
    pub(crate) fn of<'a>(entries: impl IntoIterator<Item = &'a str>, policy: Policy) -> Self {
        Self(entries.into_iter().filter_map(|raw| validate_feed_url(raw, policy).ok()).map(String::from).collect())
    }

    pub(crate) fn lists(&self, feed_url: &str) -> bool {
        self.0.iter().any(|configured| configured == feed_url)
    }
}

#[derive(sqlx::FromRow)]
struct ManagedRow {
    #[sqlx(flatten)]
    source: SourceRow,
    article_count: i64,
}

/// A feed URL the fetcher will accept, in its normalized spelling.
pub fn validate_feed_url(raw: &str, policy: Policy) -> AppResult<Url> {
    let raw = raw.trim();
    ensure!(
        !raw.is_empty() && raw.len() <= MAX_URL_LEN,
        invalid,
        "INVALID_FEED_URL",
        "the feed URL is empty or too long"
    );
    let mut url = match Url::parse(raw) {
        Ok(url) => url,
        Err(_) => bail!(invalid, "INVALID_FEED_URL", "the feed URL is not a valid URL"),
    };
    if let Err(refusal) = check_url(&url, policy) {
        bail!(invalid, "FEED_URL_NOT_ALLOWED", "this feed URL cannot be fetched: {refusal}");
    }
    url.set_fragment(None);
    Ok(url)
}

/// A source name; empty means "take it from the feed".
pub fn validate_name(raw: &str) -> AppResult<String> {
    let name = raw.trim();
    ensure!(
        name.chars().count() <= MAX_NAME_CHARS && !name.chars().any(char::is_control),
        invalid,
        "INVALID_NAME",
        "the name is too long or contains control characters"
    );
    Ok(name.to_owned())
}

/// A site URL; empty means "take it from the feed".
pub fn validate_site_url(raw: &str) -> AppResult<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(String::new());
    }
    let ok = raw.len() <= MAX_URL_LEN
        && Url::parse(raw).is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some());
    ensure!(ok, invalid, "INVALID_SITE_URL", "the site URL must be an http(s) URL");
    Ok(raw.to_owned())
}

/// A BCP 47 language tag, lowercased; empty means Japanese.
pub fn validate_language(raw: &str) -> AppResult<String> {
    let language = raw.trim().to_ascii_lowercase();
    if language.is_empty() {
        return Ok(DEFAULT_LANGUAGE.to_owned());
    }
    let mut subtags = language.split('-');
    let primary_ok = subtags
        .next()
        .is_some_and(|primary| (2..=3).contains(&primary.len()) && primary.bytes().all(|b| b.is_ascii_lowercase()));
    let rest_ok =
        subtags.all(|subtag| (1..=8).contains(&subtag.len()) && subtag.bytes().all(|b| b.is_ascii_alphanumeric()));
    ensure!(
        primary_ok && rest_ok && language.len() <= 16,
        invalid,
        "INVALID_LANGUAGE",
        "the language must be a BCP 47 tag such as `ja` or `en-us`"
    );
    Ok(language)
}

/// Default tags of a source, normalized and de-duplicated.
pub fn validate_tags(raw: &[String]) -> AppResult<Vec<String>> {
    let mut tags: Vec<String> = Vec::new();
    for tag in raw {
        let Some(tag) = normalize_tag(tag) else { bail!(invalid, "INVALID_TAG", "{tag:?} is not a usable tag") };
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    ensure!(tags.len() <= MAX_DEFAULT_TAGS, invalid, "TOO_MANY_TAGS", "at most {MAX_DEFAULT_TAGS} default tags");
    Ok(tags)
}

/// A source to create.
pub(crate) struct NewSource {
    pub feed_url: Url,
    /// Empty: the host name until the feed says what it is called.
    pub name: String,
    pub site_url: String,
    pub language: String,
    pub default_tags: Vec<String>,
    pub enabled: bool,
    pub from_config: bool,
}

fn not_found() -> AppError {
    AppError::not_found("NEWS_SOURCE_NOT_FOUND", "news source not found")
}

impl Feed {
    /// The feeds the settings list now (see [`ConfiguredFeeds`]).
    pub(crate) fn configured_feeds(&self) -> ConfiguredFeeds {
        ConfiguredFeeds::of(self.cfg.seed_feed_urls(), self.news_policy)
    }

    /// Creates a source; `None` when the feed URL is already a source.
    pub(crate) async fn insert_source(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        new: &NewSource,
    ) -> AppResult<Option<SourceRow>> {
        let name_from_feed = new.name.is_empty();
        let name = if name_from_feed { new.feed_url.host_str().unwrap_or("news").to_owned() } else { new.name.clone() };
        Ok(sqlx::query_as(concat!(
            "INSERT INTO news_sources AS s (name, feed_url, site_url, language, default_tags, enabled, from_config, name_from_feed, site_from_feed)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (feed_url) DO NOTHING
             RETURNING ",
            source_columns!()
        ))
        .bind(name)
        .bind(new.feed_url.as_str())
        .bind(&new.site_url)
        .bind(&new.language)
        .bind(&new.default_tags)
        .bind(new.enabled)
        .bind(new.from_config)
        .bind(name_from_feed)
        .bind(new.site_url.is_empty())
        .fetch_optional(&mut **tx)
        .await?)
    }

    pub(crate) async fn find_source(&self, id: Uuid) -> AppResult<Option<SourceRow>> {
        Ok(sqlx::query_as(concat!("SELECT ", source_columns!(), " FROM news_sources s WHERE s.id = $1"))
            .bind(id)
            .fetch_optional(&self.core.db)
            .await?)
    }

    async fn managed_source(&self, id: Uuid) -> AppResult<ManagedNewsSource> {
        let row: ManagedRow = sqlx::query_as(concat!(
            "SELECT ",
            source_columns!(),
            ", (SELECT count(*) FROM news_articles n WHERE n.source_id = s.id AND n.removed_at IS NULL) AS article_count
             FROM news_sources s WHERE s.id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.core.db)
        .await?
        .ok_or_else(not_found)?;
        Ok(row.source.to_managed(&self.core, row.article_count, &self.configured_feeds()))
    }

    /// Adds (`listed`) or removes every article of a source to or from the
    /// search index, following the source being switched on or off.
    async fn reindex_source(&self, tx: &mut Transaction<'_, Postgres>, source_id: Uuid, listed: bool) -> AppResult<()> {
        let rows: Vec<ArticleRow> = sqlx::query_as(concat!(
            "SELECT ",
            article_columns!(),
            " FROM news_articles n JOIN news_sources s ON s.id = n.source_id WHERE n.source_id = $1 AND n.removed_at IS NULL"
        ))
        .bind(source_id)
        .fetch_all(&mut **tx)
        .await?;
        for row in rows {
            if listed {
                outbound::index(&mut **tx, row.doc().search_doc(&self.core)).await?;
            } else {
                outbound::unindex(&mut **tx, SEARCH_KIND, row.id).await?;
            }
        }
        Ok(())
    }
}

impl NewsAdminService for Feed {
    async fn list_managed_news_sources(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListManagedNewsSourcesRequest>,
    ) -> ServiceResult<ListManagedNewsSourcesResponse> {
        handle(async {
            ctx.require_role("admin")?;
            let rows: Vec<ManagedRow> = sqlx::query_as(concat!(
                "SELECT ",
                source_columns!(),
                ", (SELECT count(*) FROM news_articles n WHERE n.source_id = s.id AND n.removed_at IS NULL) AS article_count
                 FROM news_sources s ORDER BY lower(s.name), s.id LIMIT $1"
            ))
            .bind(MAX_SOURCES)
            .fetch_all(&self.core.db)
            .await?;
            let configured = self.configured_feeds();
            let sources = rows.iter().map(|row| row.source.to_managed(&self.core, row.article_count, &configured)).collect();
            ok(ListManagedNewsSourcesResponse { sources, ..Default::default() })
        })
        .await
    }

    async fn create_news_source(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateNewsSourceRequest>,
    ) -> ServiceResult<CreateNewsSourceResponse> {
        handle(async {
            ctx.require_role("admin")?;
            let request = request.to_owned_message();
            let new = NewSource {
                feed_url: validate_feed_url(&request.feed_url, self.news_policy)?,
                name: validate_name(&request.name)?,
                site_url: validate_site_url(&request.site_url)?,
                language: validate_language(&request.language)?,
                default_tags: validate_tags(&request.default_tags)?,
                enabled: !request.disabled,
                from_config: false,
            };

            let mut tx = self.core.db.begin().await?;
            // Creations are serialized so the cap cannot be overshot.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('feed:news-sources', 0))")
                .execute(&mut *tx)
                .await?;
            let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM news_sources").fetch_one(&mut *tx).await?;
            ensure!(
                existing < MAX_SOURCES,
                precondition,
                "TOO_MANY_NEWS_SOURCES",
                "at most {MAX_SOURCES} news sources can exist"
            );
            let Some(source) = self.insert_source(&mut tx, &new).await? else {
                bail!(already_exists, "NEWS_SOURCE_EXISTS", "this feed is already a news source");
            };
            tx.commit().await?;

            if source.enabled {
                self.enqueue_fetch(source.id).await;
            }
            let created = source.to_managed(&self.core, 0, &self.configured_feeds());
            ok(CreateNewsSourceResponse { source: created.into(), ..Default::default() })
        })
        .await
    }

    async fn update_news_source(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateNewsSourceRequest>,
    ) -> ServiceResult<UpdateNewsSourceResponse> {
        handle(async {
            ctx.require_role("admin")?;
            let request = request.to_owned_message();
            let source_id = parse_id(&request.source_id)?;
            let feed_url =
                request.feed_url.as_deref().map(|raw| validate_feed_url(raw, self.news_policy)).transpose()?;
            let name = request.name.as_deref().map(validate_name).transpose()?;
            let site_url = request.site_url.as_deref().map(validate_site_url).transpose()?;
            let language = request.language.as_deref().map(validate_language).transpose()?;
            let default_tags = request.default_tags.as_option().map(|list| validate_tags(&list.tags)).transpose()?;

            let mut tx = self.core.db.begin().await?;
            let before: SourceRow = sqlx::query_as(concat!(
                "SELECT ",
                source_columns!(),
                " FROM news_sources s WHERE s.id = $1 FOR UPDATE"
            ))
            .bind(source_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(not_found)?;

            let feed_changed = feed_url.as_ref().is_some_and(|url| url.as_str() != before.feed_url);
            let enabled = request.enabled.unwrap_or(before.enabled);
            // An empty name or site hands the field back to the feed.
            let (name, name_from_feed) = match name {
                Some(name) if name.is_empty() => (before.name.clone(), true),
                Some(name) => (name, false),
                None => (before.name.clone(), before.name_from_feed),
            };
            let (site_url, site_from_feed) = match site_url {
                Some(site) if site.is_empty() => (before.site_url.clone(), true),
                Some(site) => (site, false),
                None => (before.site_url.clone(), before.site_from_feed),
            };
            // A new feed starts from scratch; a source switched on is fetched right away.
            let fetch_now = enabled && (feed_changed || !before.enabled);
            let updated = sqlx::query(
                "UPDATE news_sources SET
                     name = $2, name_from_feed = $3, site_url = $4, site_from_feed = $5,
                     language = COALESCE($6, language), default_tags = COALESCE($7, default_tags), enabled = $8,
                     feed_url = COALESCE($9, feed_url),
                     etag = CASE WHEN $10 THEN NULL ELSE etag END,
                     last_modified = CASE WHEN $10 THEN NULL ELSE last_modified END,
                     consecutive_failures = CASE WHEN $11 THEN 0 ELSE consecutive_failures END,
                     next_fetch_at = CASE WHEN $11 THEN now() ELSE next_fetch_at END
                 WHERE id = $1",
            )
            .bind(source_id)
            .bind(&name)
            .bind(name_from_feed)
            .bind(&site_url)
            .bind(site_from_feed)
            .bind(&language)
            .bind(&default_tags)
            .bind(enabled)
            .bind(feed_url.as_ref().map(Url::as_str))
            .bind(feed_changed)
            .bind(fetch_now)
            .execute(&mut *tx)
            .await;
            match updated {
                Ok(_) => {}
                Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                    bail!(already_exists, "NEWS_SOURCE_EXISTS", "this feed is already a news source")
                }
                Err(error) => return Err(error.into()),
            }
            if enabled != before.enabled {
                self.reindex_source(&mut tx, source_id, enabled).await?;
            }
            tx.commit().await?;

            if fetch_now {
                self.enqueue_fetch(source_id).await;
            }
            ok(UpdateNewsSourceResponse { source: self.managed_source(source_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_news_source(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteNewsSourceRequest>,
    ) -> ServiceResult<DeleteNewsSourceResponse> {
        handle(async {
            ctx.require_role("admin")?;
            let source_id = parse_id(request.source_id)?;
            let mut tx = self.core.db.begin().await?;
            let feed_url: Option<String> =
                sqlx::query_scalar("SELECT feed_url FROM news_sources WHERE id = $1 FOR UPDATE")
                    .bind(source_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            let feed_url = feed_url.ok_or_else(not_found)?;
            // The next start would create it again, without its articles.
            ensure!(
                !self.configured_feeds().lists(&feed_url),
                precondition,
                "NEWS_SOURCE_CONFIGURED",
                "this source is listed in TOKULY_NEWS_FEEDS and would be created again on the next start: \
                 disable it, or remove it from the setting first"
            );
            // The articles go with the source: out of search first.
            let article_ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM news_articles WHERE source_id = $1")
                .bind(source_id)
                .fetch_all(&mut *tx)
                .await?;
            for article_id in &article_ids {
                outbound::unindex(&mut *tx, SEARCH_KIND, *article_id).await?;
            }
            outbound::subjects_deleted(&mut *tx, SubjectType::SUBJECT_TYPE_NEWS_ARTICLE, &article_ids).await?;
            sqlx::query("DELETE FROM news_sources WHERE id = $1").bind(source_id).execute(&mut *tx).await?;
            tx.commit().await?;
            ok(DeleteNewsSourceResponse::default())
        })
        .await
    }

    async fn refresh_news_sources(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RefreshNewsSourcesRequest>,
    ) -> ServiceResult<RefreshNewsSourcesResponse> {
        handle(async {
            let admin = ctx.require_role("admin")?;
            let only = parse_opt_id(request.source_id)?;
            self.core
                .cache
                .rate_limit(&format!("feed:news-refresh:{}", admin.user_id), 10, Duration::from_secs(60))
                .await?;

            let source_ids: Vec<Uuid> = match only {
                Some(source_id) => {
                    let source = self.find_source(source_id).await?.ok_or_else(not_found)?;
                    ensure!(source.enabled, precondition, "NEWS_SOURCE_DISABLED", "this news source is switched off");
                    vec![source.id]
                }
                None => {
                    sqlx::query_scalar("SELECT id FROM news_sources WHERE enabled ORDER BY next_fetch_at")
                        .fetch_all(&self.core.db)
                        .await?
                }
            };
            // Due now, so the scheduler picks them up even if a queued job is lost.
            sqlx::query("UPDATE news_sources SET next_fetch_at = now() WHERE id = ANY($1)")
                .bind(&source_ids)
                .execute(&self.core.db)
                .await?;
            for source_id in &source_ids {
                self.enqueue_fetch(*source_id).await;
            }
            ok(RefreshNewsSourcesResponse { queued_count: source_ids.len() as i32, ..Default::default() })
        })
        .await
    }

    async fn remove_news_article(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveNewsArticleRequest>,
    ) -> ServiceResult<RemoveNewsArticleResponse> {
        handle(async {
            ctx.require_role("moderator")?;
            let article_id = parse_id(request.article_id)?;
            ensure!(
                self.take_article_down(article_id).await?,
                not_found,
                "NEWS_ARTICLE_NOT_FOUND",
                "news article not found"
            );
            ok(RemoveNewsArticleResponse::default())
        })
        .await
    }
}

impl Feed {
    /// Hides an article everywhere and takes it out of search. The row stays
    /// as a tombstone so the next fetch does not bring the article back.
    /// Returns whether the article exists.
    pub(crate) async fn take_article_down(&self, article_id: Uuid) -> AppResult<bool> {
        let mut tx = self.core.db.begin().await?;
        let removed_at: Option<Option<Time>> =
            sqlx::query_scalar("SELECT removed_at FROM news_articles WHERE id = $1 FOR UPDATE")
                .bind(article_id)
                .fetch_optional(&mut *tx)
                .await?;
        match removed_at {
            None => return Ok(false),
            // Already down: nothing to do.
            Some(Some(_)) => return Ok(true),
            Some(None) => {}
        }
        sqlx::query("UPDATE news_articles SET removed_at = now() WHERE id = $1")
            .bind(article_id)
            .execute(&mut *tx)
            .await?;
        outbound::unindex(&mut *tx, SEARCH_KIND, article_id).await?;
        tx.commit().await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_urls_must_be_fetchable() {
        let strict = Policy::default();
        assert_eq!(
            validate_feed_url(" https://News.Example/rss.xml#top ", strict).unwrap().as_str(),
            "https://news.example/rss.xml"
        );
        for (raw, reason) in [
            ("", "INVALID_FEED_URL"),
            ("not a url", "INVALID_FEED_URL"),
            ("/relative/feed.xml", "INVALID_FEED_URL"),
            ("ftp://news.example/rss.xml", "FEED_URL_NOT_ALLOWED"),
            ("https://user:pw@news.example/rss.xml", "FEED_URL_NOT_ALLOWED"),
            ("https://news.example:8443/rss.xml", "FEED_URL_NOT_ALLOWED"),
            ("http://127.0.0.1/rss.xml", "FEED_URL_NOT_ALLOWED"),
            ("http://169.254.169.254/latest/meta-data", "FEED_URL_NOT_ALLOWED"),
            ("http://[::1]/rss.xml", "FEED_URL_NOT_ALLOWED"),
        ] {
            assert_eq!(validate_feed_url(raw, strict).unwrap_err().reason, reason, "{raw:?}");
        }
        let too_long = format!("https://news.example/{}", "a".repeat(MAX_URL_LEN));
        assert_eq!(validate_feed_url(&too_long, strict).unwrap_err().reason, "INVALID_FEED_URL");
        // The development policy lets a local feed server through.
        assert!(validate_feed_url("http://127.0.0.1:4000/rss.xml", Policy { allow_loopback: true }).is_ok());
    }

    #[test]
    fn names_sites_and_languages_are_validated() {
        assert_eq!(validate_name("  ITmedia NEWS ").unwrap(), "ITmedia NEWS");
        assert_eq!(validate_name("").unwrap(), "");
        assert_eq!(validate_name(&"あ".repeat(MAX_NAME_CHARS + 1)).unwrap_err().reason, "INVALID_NAME");
        assert_eq!(validate_name("bad\u{0007}name").unwrap_err().reason, "INVALID_NAME");

        assert_eq!(validate_site_url(" https://news.example/ ").unwrap(), "https://news.example/");
        assert_eq!(validate_site_url("").unwrap(), "");
        for raw in ["news.example", "javascript:alert(1)", "ftp://news.example"] {
            assert_eq!(validate_site_url(raw).unwrap_err().reason, "INVALID_SITE_URL", "{raw}");
        }

        assert_eq!(validate_language("").unwrap(), "ja");
        assert_eq!(validate_language(" EN-US ").unwrap(), "en-us");
        assert_eq!(validate_language("zh-Hant-TW").unwrap(), "zh-hant-tw");
        for raw in ["j", "japanese", "ja_JP", "ja-", "日本語", "en-us-with-a-very-long-tail"] {
            assert_eq!(validate_language(raw).unwrap_err().reason, "INVALID_LANGUAGE", "{raw}");
        }
    }

    #[test]
    fn default_tags_are_normalized_and_capped() {
        let tags = validate_tags(&["Tech".to_owned(), " ゲーム ".to_owned(), "tech".to_owned()]).unwrap();
        assert_eq!(tags, vec!["tech", "ゲーム"]);
        assert_eq!(validate_tags(&["ok".to_owned(), "a,b".to_owned()]).unwrap_err().reason, "INVALID_TAG");
        assert_eq!(validate_tags(&[String::new()]).unwrap_err().reason, "INVALID_TAG");
        let many: Vec<String> = (0..=MAX_DEFAULT_TAGS).map(|i| format!("tag{i}")).collect();
        assert_eq!(validate_tags(&many).unwrap_err().reason, "TOO_MANY_TAGS");
        assert!(validate_tags(&[]).unwrap().is_empty());
    }
}
