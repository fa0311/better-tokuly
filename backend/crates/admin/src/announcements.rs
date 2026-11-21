//! Announcements (the product update log): rows, input rules, language
//! selection and the "notify everyone" broadcast.
//!
//! State is derived from `published_at`: `NULL` is a draft, a future time is
//! scheduled, anything else is published. Only published announcements are
//! visible to users.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokuly_core::{
    outbound::{self, Localized},
    prelude::*,
};
use tokuly_proto::{
    admin::{Announcement, AnnouncementCategory, AnnouncementStatus, LocalizedText, ManagedAnnouncement},
    common::{Image, UserSummary},
};

use crate::{Admin, notices};

pub(crate) const TITLE_MAX_CHARS: usize = 120;
pub(crate) const SUMMARY_MAX_CHARS: usize = 300;
pub(crate) const BODY_MAX_CHARS: usize = 20_000;
/// Length of the summary derived from the body when none was written.
const EXCERPT_CHARS: usize = 140;
const IMAGE_URL_MAX_LEN: usize = 2048;

/// Background job that notifies every active user about one announcement.
pub const BROADCAST_JOB: &str = "admin.announcement.broadcast";

/// A broadcast that has not finished after this long is queued again (its
/// job was lost or failed permanently); it resumes where it stopped.
const BROADCAST_REQUEUE_AFTER_MINUTES: i32 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastJob {
    pub announcement_id: Uuid,
}

macro_rules! announcement_columns {
    () => {
        "id, category, title_ja, title_en, summary_ja, summary_en, body_ja, body_en, image, pinned, published_at, \
         notify_users, broadcast_cursor, notified_count, notified_at, author_id, created_at, updated_at"
    };
}
pub(crate) use announcement_columns;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct AnnouncementRow {
    pub id: Uuid,
    pub category: i32,
    pub title_ja: String,
    pub title_en: String,
    pub summary_ja: String,
    pub summary_en: String,
    pub body_ja: String,
    pub body_en: String,
    pub image: Option<serde_json::Value>,
    pub pinned: bool,
    pub published_at: Option<DateTime<Utc>>,
    pub notify_users: bool,
    pub broadcast_cursor: Option<Uuid>,
    pub notified_count: i32,
    pub notified_at: Option<DateTime<Utc>>,
    pub author_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl AnnouncementRow {
    pub fn status(&self, now: DateTime<Utc>) -> AnnouncementStatus {
        match self.published_at {
            None => AnnouncementStatus::ANNOUNCEMENT_STATUS_DRAFT,
            Some(at) if at > now => AnnouncementStatus::ANNOUNCEMENT_STATUS_SCHEDULED,
            Some(_) => AnnouncementStatus::ANNOUNCEMENT_STATUS_PUBLISHED,
        }
    }

    pub fn is_published(&self, now: DateTime<Utc>) -> bool {
        self.status(now) == AnnouncementStatus::ANNOUNCEMENT_STATUS_PUBLISHED
    }

    fn image(&self) -> Option<Image> {
        self.image.as_ref().and_then(|json| serde_json::from_value(json.clone()).ok())
    }

    /// English is used only when the announcement is fully translated;
    /// mixing an English title with a Japanese body would read worse than
    /// plain Japanese.
    fn has_english(&self) -> bool {
        !self.title_en.is_empty() && !self.body_en.is_empty()
    }

    /// The summary in one language: the written one, or the start of the body.
    fn summary(&self, english: bool) -> String {
        let (summary, body) = if english { (&self.summary_en, &self.body_en) } else { (&self.summary_ja, &self.body_ja) };
        if summary.is_empty() { plain_excerpt(body, EXCERPT_CHARS) } else { summary.clone() }
    }

    /// The announcement in the language closest to `locale`.
    pub fn to_public(&self, locale: &str) -> Announcement {
        let english = !prefers_japanese(locale) && self.has_english();
        Announcement {
            id: self.id.to_string(),
            category: self.category.into(),
            title: if english { self.title_en.clone() } else { self.title_ja.clone() },
            summary: self.summary(english),
            body: if english { self.body_en.clone() } else { self.body_ja.clone() },
            image: self.image().into(),
            pinned: self.pinned,
            published_at: opt_ts(self.published_at),
            locale: if english { "en" } else { "ja" }.to_owned(),
            ..Default::default()
        }
    }

    pub fn to_managed(&self, author: Option<UserSummary>) -> ManagedAnnouncement {
        let text = |ja: &str, en: &str| LocalizedText { ja: ja.to_owned(), en: en.to_owned(), ..Default::default() };
        ManagedAnnouncement {
            id: self.id.to_string(),
            category: self.category.into(),
            title: text(&self.title_ja, &self.title_en).into(),
            summary: text(&self.summary_ja, &self.summary_en).into(),
            body: text(&self.body_ja, &self.body_en).into(),
            image: self.image().into(),
            pinned: self.pinned,
            status: self.status(Utc::now()).into(),
            published_at: opt_ts(self.published_at),
            notify_users: self.notify_users,
            notified_at: opt_ts(self.notified_at),
            notified_count: self.notified_count,
            author: author.into(),
            created_at: ts(self.created_at),
            updated_at: ts(self.updated_at),
            ..Default::default()
        }
    }

    /// Title and summary for the broadcast notification; English falls back
    /// to Japanese inside `Localized::pick` when it is empty.
    fn notification_texts(&self) -> (Localized, Localized) {
        if self.has_english() {
            (Localized::new(&self.title_ja, &self.title_en), Localized::new(self.summary(false), self.summary(true)))
        } else {
            (Localized::new(&self.title_ja, ""), Localized::new(self.summary(false), ""))
        }
    }

    /// The state recorded in the audit log (texts are left out: they can be
    /// long, and the titles identify the announcement well enough).
    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "title_ja": self.title_ja,
            "title_en": self.title_en,
            "category": self.category,
            "pinned": self.pinned,
            "published_at": self.published_at,
            "notify_users": self.notify_users,
        })
    }
}

/// Whether a BCP 47 tag asks for Japanese. Anything that is not Japanese
/// gets English when a translation exists.
pub(crate) fn prefers_japanese(locale: &str) -> bool {
    let locale = locale.trim();
    locale.is_empty() || locale.to_ascii_lowercase().starts_with("ja")
}

/// A two-language text field after validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Texts {
    pub ja: String,
    pub en: String,
}

/// Validates a localized text: Japanese is required when `required`, both
/// languages are limited to `max_chars`.
pub(crate) fn localized_text(text: Option<&LocalizedText>, required: bool, max_chars: usize, reason: &'static str) -> AppResult<Texts> {
    let (ja, en) = text.map(|t| (t.ja.trim(), t.en.trim())).unwrap_or_default();
    ensure!(!(required && ja.is_empty()), invalid, reason, "the Japanese text is required");
    ensure!(ja.chars().count() <= max_chars && en.chars().count() <= max_chars, invalid, reason, "texts are at most {max_chars} characters per language");
    Ok(Texts { ja: ja.to_owned(), en: en.to_owned() })
}

/// A category that can be stored: known and not UNSPECIFIED.
pub(crate) fn validate_category(category: buffa::EnumValue<AnnouncementCategory>) -> AppResult<i32> {
    match category.as_known() {
        Some(AnnouncementCategory::ANNOUNCEMENT_CATEGORY_UNSPECIFIED) | None => Err(AppError::invalid("CATEGORY_INVALID", "unknown announcement category")),
        Some(known) => Ok(known as i32),
    }
}

/// Validates a header image and returns it as stored JSON. Staff upload the
/// picture through the media service and pass the resulting `Image`; since
/// its URLs end up in every client, only http(s) URLs are accepted.
pub(crate) fn validate_image(image: &Image) -> AppResult<serde_json::Value> {
    let is_web_url =
        |url: &str| url.len() <= IMAGE_URL_MAX_LEN && url::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
    ensure!(is_web_url(&image.url), invalid, "IMAGE_INVALID", "the image needs an http(s) URL");
    ensure!(image.variants.iter().all(|v| is_web_url(&v.url)), invalid, "IMAGE_INVALID", "image variants need http(s) URLs");
    ensure!(image.width >= 0 && image.height >= 0, invalid, "IMAGE_INVALID", "image dimensions cannot be negative");
    serde_json::to_value(image).map_err(|e| AppError::internal(anyhow::anyhow!("image encode: {e}")))
}

/// The first `max_chars` characters of a Markdown text as plain prose:
/// headings, emphasis, links, images, quotes, list bullets and code fences
/// are reduced to their text.
pub(crate) fn plain_excerpt(markdown: &str, max_chars: usize) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut in_code_block = false;
    for line in markdown.lines() {
        let line = line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }
        // Block markers: headings, quotes, bullets, rules.
        let line = line.trim_start_matches(['#', '>', ' ']);
        let line = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).or_else(|| line.strip_prefix("+ ")).unwrap_or(line);
        if line.chars().all(|c| matches!(c, '-' | '*' | '_' | '=' | ' ')) {
            continue;
        }
        let text = strip_inline_markdown(line);
        if !text.is_empty() {
            words.push(text);
        }
    }
    notices::truncate_chars(&words.join(" "), max_chars)
}

/// Removes inline Markdown: `![alt](url)` disappears, `[text](url)` keeps
/// its text, emphasis and code markers are dropped.
fn strip_inline_markdown(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find('[') {
        let (before, link) = rest.split_at(start);
        let is_image = before.ends_with('!');
        out.push_str(if is_image { &before[..before.len() - 1] } else { before });
        // `[text](target)`; anything else is literal text.
        let parsed = link[1..].find("](").and_then(|text_end| {
            let after = &link[1 + text_end + 2..];
            after.find(')').map(|target_end| (&link[1..1 + text_end], &after[target_end + 1..]))
        });
        match parsed {
            Some((text, remainder)) => {
                if !is_image {
                    out.push_str(text);
                }
                rest = remainder;
            }
            None => {
                out.push('[');
                rest = &link[1..];
            }
        }
    }
    out.push_str(rest);
    out.replace(['*', '`'], "").replace("__", "").replace("~~", "").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a broadcast pass has more users to notify.
#[derive(Debug, PartialEq, Eq)]
enum Progress {
    More,
    Done,
}

impl Admin {
    /// An announcement in any state, or `ANNOUNCEMENT_NOT_FOUND`.
    pub(crate) async fn announcement(&self, id: Uuid) -> AppResult<AnnouncementRow> {
        sqlx::query_as(concat!("SELECT ", announcement_columns!(), " FROM announcements WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.core.db)
            .await?
            .or_not_found("ANNOUNCEMENT_NOT_FOUND", "announcement not found")
    }

    pub(crate) async fn managed_announcement(&self, row: &AnnouncementRow) -> AppResult<ManagedAnnouncement> {
        let author = match row.author_id {
            Some(author_id) => tokuly_identity::summaries(&self.core, &[author_id]).await?.remove(&author_id),
            None => None,
        };
        Ok(row.to_managed(author))
    }

    /// Queues the broadcast job of every published announcement that asked
    /// to notify users and has not finished doing so. Called when an
    /// announcement is published and periodically by the worker (which is
    /// what picks up scheduled announcements when their time comes).
    pub async fn queue_due_broadcasts(&self) -> AppResult<u64> {
        let due: Vec<(Uuid, DateTime<Utc>)> = sqlx::query_as(
            "SELECT id, published_at FROM announcements
             WHERE notify_users AND notified_at IS NULL AND published_at <= now()
               AND (broadcast_queued_at IS NULL OR broadcast_queued_at < now() - make_interval(mins => $1))
             ORDER BY published_at LIMIT 20",
        )
        .bind(BROADCAST_REQUEUE_AFTER_MINUTES)
        .fetch_all(&self.core.db)
        .await?;
        let mut queued = 0;
        for (announcement_id, published_at) in due {
            // Processes racing to queue the same publication collapse into
            // one job; publishing again (a new publish time) queues a new one.
            let publication = format!("{announcement_id}:{}", published_at.timestamp_millis());
            // Enqueue first: a job without the mark is merely repeated (the
            // job is idempotent), a mark without a job would never be sent.
            self.core.jobs.enqueue_unique(BROADCAST_JOB, &publication, &BroadcastJob { announcement_id }).await?;
            sqlx::query("UPDATE announcements SET broadcast_queued_at = now() WHERE id = $1").bind(announcement_id).execute(&self.core.db).await?;
            queued += 1;
        }
        Ok(queued)
    }

    /// Notifies every active user about an announcement, a chunk at a time.
    /// Progress is stored with the announcement in the same transaction as
    /// each chunk's notification requests, so a retried or duplicated job
    /// continues after the last notified user instead of starting over.
    pub async fn run_broadcast(&self, job: BroadcastJob) -> AppResult<()> {
        while self.broadcast_chunk(job.announcement_id).await? == Progress::More {
            if self.core.shutdown.is_cancelled() {
                // Stop between chunks; the job is redelivered and resumes.
                return Err(AppError::unavailable("SHUTTING_DOWN", "broadcast interrupted by shutdown"));
            }
        }
        Ok(())
    }

    async fn broadcast_chunk(&self, announcement_id: Uuid) -> AppResult<Progress> {
        let mut tx = self.core.db.begin().await?;
        // The row lock makes concurrent copies of the job take turns.
        let row: Option<AnnouncementRow> = sqlx::query_as(concat!("SELECT ", announcement_columns!(), " FROM announcements WHERE id = $1 FOR UPDATE"))
            .bind(announcement_id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(row) = row else {
            return Ok(Progress::Done);
        };
        // Unpublished or switched off since it was queued: stop quietly. A
        // later publish queues it again and it continues from the cursor.
        if row.notified_at.is_some() || !row.notify_users || !row.is_published(Utc::now()) {
            return Ok(Progress::Done);
        }
        let recipients: Vec<Uuid> = sqlx::query_scalar(
            "SELECT u.id FROM users u
             WHERE ($1::uuid IS NULL OR u.id > $1) AND u.deleted_at IS NULL AND u.banned_at IS NULL
               AND EXISTS (
                   SELECT 1 FROM sessions s
                   WHERE s.user_id = u.id AND s.revoked_at IS NULL AND s.expires_at > now()
                     AND s.last_active_at > now() - make_interval(days => $2)
               )
             ORDER BY u.id LIMIT $3",
        )
        .bind(row.broadcast_cursor)
        .bind(self.cfg.announcement_notify_active_days)
        .bind(self.cfg.announcement_notify_chunk)
        .fetch_all(&mut *tx)
        .await?;
        let Some(last) = recipients.last().copied() else {
            sqlx::query("UPDATE announcements SET notified_at = now() WHERE id = $1").bind(row.id).execute(&mut *tx).await?;
            tx.commit().await?;
            tracing::info!(announcement_id = %row.id, notified = row.notified_count, "announcement broadcast finished");
            return Ok(Progress::Done);
        };
        let (title, summary) = row.notification_texts();
        let image = row.image().map(|i| i.url).filter(|url| !url.is_empty());
        for user_id in &recipients {
            let request = notices::announcement(*user_id, row.id, title.clone(), summary.clone(), image.as_deref());
            outbound::notify(&mut *tx, &request).await?;
        }
        sqlx::query("UPDATE announcements SET broadcast_cursor = $2, notified_count = notified_count + $3 WHERE id = $1")
            .bind(row.id)
            .bind(last)
            .bind(recipients.len() as i32)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Progress::More)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> AnnouncementRow {
        AnnouncementRow {
            id: Uuid::now_v7(),
            category: AnnouncementCategory::ANNOUNCEMENT_CATEGORY_UPDATE as i32,
            title_ja: "スタンプが作れるようになりました!".into(),
            title_en: String::new(),
            summary_ja: String::new(),
            summary_en: String::new(),
            body_ja: "# 新機能\n\n**スタンプ**を[スタジオ](https://tokuly.com/studio)で作れます。".into(),
            body_en: String::new(),
            image: None,
            pinned: false,
            published_at: None,
            notify_users: false,
            broadcast_cursor: None,
            notified_count: 0,
            notified_at: None,
            author_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn status_follows_the_publish_time() {
        let now = Utc::now();
        assert_eq!(row().status(now), AnnouncementStatus::ANNOUNCEMENT_STATUS_DRAFT);
        let scheduled = AnnouncementRow { published_at: Some(now + chrono::Duration::hours(1)), ..row() };
        assert_eq!(scheduled.status(now), AnnouncementStatus::ANNOUNCEMENT_STATUS_SCHEDULED);
        let published = AnnouncementRow { published_at: Some(now - chrono::Duration::seconds(1)), ..row() };
        assert!(published.is_published(now) && !scheduled.is_published(now));
    }

    #[test]
    fn language_falls_back_to_japanese_unless_fully_translated() {
        let japanese_only = row();
        assert_eq!(japanese_only.to_public("en-US").locale, "ja");
        let title_only = AnnouncementRow { title_en: "Stamps!".into(), ..row() };
        assert_eq!(title_only.to_public("en").title, japanese_only.title_ja, "a half translation is not used");

        let translated = AnnouncementRow { title_en: "Make your own stamps!".into(), body_en: "You can now **create** stamps.".into(), ..row() };
        let english = translated.to_public("en-GB");
        assert_eq!((english.locale.as_str(), english.title.as_str()), ("en", "Make your own stamps!"));
        assert_eq!(english.summary, "You can now create stamps.");
        assert_eq!(translated.to_public("ja-JP").locale, "ja");
        assert_eq!(translated.to_public("").locale, "ja", "no preference means Japanese");
        assert_eq!(translated.to_public("fr").locale, "en", "other languages get English");
    }

    #[test]
    fn summary_is_the_written_one_or_an_excerpt_of_the_body() {
        assert_eq!(row().to_public("ja").summary, "新機能 スタンプをスタジオで作れます。");
        let written = AnnouncementRow { summary_ja: "スタンプ機能を追加!".into(), ..row() };
        assert_eq!(written.to_public("ja").summary, "スタンプ機能を追加!");
    }

    #[test]
    fn excerpt_strips_markdown() {
        let markdown = "## Title\n\n> quote with *emphasis* and `code`\n\n- item [link](https://example.com/a_b)\n* ![alt](https://example.com/i.png) after image\n\n---\n```\nlet hidden = 1;\n```\n~~gone~~ __bold__ [not a link] end";
        assert_eq!(plain_excerpt(markdown, 500), "Title quote with emphasis and code item link after image gone bold [not a link] end");
        assert_eq!(plain_excerpt("あいうえおかきくけこ", 5), "あいうえ…");
        assert_eq!(plain_excerpt("", 10), "");
    }

    #[test]
    fn localized_text_requires_japanese_and_respects_the_limit() {
        let text = |ja: &str, en: &str| LocalizedText { ja: ja.into(), en: en.into(), ..Default::default() };
        assert_eq!(localized_text(Some(&text(" こん ", " Hi ")), true, 10, "TITLE_INVALID").unwrap(), Texts { ja: "こん".into(), en: "Hi".into() });
        assert_eq!(localized_text(None, true, 10, "TITLE_INVALID").unwrap_err().reason, "TITLE_INVALID");
        assert_eq!(localized_text(Some(&text("", "English only")), true, 20, "BODY_INVALID").unwrap_err().reason, "BODY_INVALID");
        assert_eq!(localized_text(Some(&text("あ", "toolongtext")), true, 10, "TITLE_INVALID").unwrap_err().reason, "TITLE_INVALID");
        assert_eq!(localized_text(None, false, 10, "SUMMARY_TOO_LONG").unwrap(), Texts { ja: String::new(), en: String::new() });
    }

    #[test]
    fn images_need_web_urls() {
        let image = |url: &str| Image { url: url.into(), width: 1200, height: 630, ..Default::default() };
        assert!(validate_image(&image("https://cdn.tokuly.com/media/a.webp")).is_ok());
        for bad in ["", "javascript:alert(1)", "data:image/png;base64,AAAA", "/relative.png", "ftp://example.com/a.png"] {
            assert_eq!(validate_image(&image(bad)).unwrap_err().reason, "IMAGE_INVALID", "{bad:?}");
        }
        let mut with_bad_variant = image("https://cdn.tokuly.com/a.webp");
        with_bad_variant.variants.push(tokuly_proto::common::ImageVariant { size: 320, url: "javascript:x".into(), ..Default::default() });
        assert_eq!(validate_image(&with_bad_variant).unwrap_err().reason, "IMAGE_INVALID");
    }

    #[test]
    fn categories_must_be_known() {
        assert_eq!(validate_category(AnnouncementCategory::ANNOUNCEMENT_CATEGORY_EVENT.into()).unwrap(), 3);
        assert!(validate_category(AnnouncementCategory::ANNOUNCEMENT_CATEGORY_UNSPECIFIED.into()).is_err());
        assert_eq!(validate_category(buffa::EnumValue::from(42)).unwrap_err().reason, "CATEGORY_INVALID");
    }

    #[test]
    fn notification_texts_leave_english_empty_without_a_translation() {
        let (title, summary) = row().notification_texts();
        assert!(title.en.is_empty() && summary.en.is_empty());
        assert_eq!(title.pick("en"), "スタンプが作れるようになりました!", "recipients fall back to Japanese");
        let translated = AnnouncementRow { title_en: "Stamps!".into(), body_en: "Create stamps.".into(), ..row() };
        let (title, summary) = translated.notification_texts();
        assert_eq!((title.pick("en"), summary.pick("en")), ("Stamps!", "Create stamps."));
    }
}
