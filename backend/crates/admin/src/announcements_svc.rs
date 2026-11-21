//! `AnnouncementService`: the public update log and its editor.
//! `ListAnnouncements` and `GetAnnouncement` are public; the rest needs the
//! `admin` role.
use std::time::Duration;

use chrono::{DateTime, Utc};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::admin::*;

use crate::{
    Admin,
    announcements::{
        AnnouncementRow, BODY_MAX_CHARS, SUMMARY_MAX_CHARS, TITLE_MAX_CHARS, announcement_columns, localized_text, validate_category, validate_image,
    },
    audit::Audit,
    staff::Staff,
};

/// How far ahead an announcement may be scheduled.
const MAX_SCHEDULE_DAYS: i64 = 366;

/// Keyset of the public feed: pinned first, then newest.
type FeedCursor = (bool, DateTime<Utc>, Uuid);

impl Admin {
    /// The language to answer in: the request's, else the client's
    /// `Accept-Language`, else Japanese.
    fn announcement_locale(ctx: &RequestContext, requested: &str) -> String {
        let requested = requested.trim();
        if requested.is_empty() { ctx.client().language } else { requested.to_owned() }
    }

    /// The update log is public, so reads are limited per client address.
    async fn limit_public_reads(&self, ctx: &RequestContext) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("announcements:{}", ctx.client().ip), 240, Duration::from_secs(60)).await
    }
}

impl AnnouncementService for Admin {
    async fn list_announcements(&self, ctx: RequestContext, request: ServiceRequest<'_, ListAnnouncementsRequest>) -> ServiceResult<ListAnnouncementsResponse> {
        handle(async {
            ctx.maybe_auth()?;
            self.limit_public_reads(&ctx).await?;
            let locale = Self::announcement_locale(&ctx, request.locale);
            let page = Page::new(request.page.as_option().map_or(0, |p| p.limit), request.page.as_option().map_or("", |p| p.cursor), 20, 50);
            let cursor = page.decode::<FeedCursor>()?;
            let mut rows: Vec<AnnouncementRow> = sqlx::query_as(concat!(
                "SELECT ",
                announcement_columns!(),
                " FROM announcements
                 WHERE published_at <= now()
                   AND ($1 = 0 OR category = $1)
                   AND ($2::boolean IS NULL OR (pinned, published_at, id) < ($2, $3, $4))
                 ORDER BY pinned DESC, published_at DESC, id DESC LIMIT $5"
            ))
            .bind(request.category.to_i32())
            .bind(cursor.map(|c| c.0))
            .bind(cursor.map(|c| c.1))
            .bind(cursor.map(|c| c.2))
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |r| (r.pinned, r.published_at.unwrap_or_default(), r.id));
            ok(ListAnnouncementsResponse { announcements: rows.iter().map(|row| row.to_public(&locale)).collect(), page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_announcement(&self, ctx: RequestContext, request: ServiceRequest<'_, GetAnnouncementRequest>) -> ServiceResult<GetAnnouncementResponse> {
        handle(async {
            ctx.maybe_auth()?;
            self.limit_public_reads(&ctx).await?;
            let row = self.announcement(parse_id(request.id)?).await?;
            // Drafts and scheduled announcements do not exist as far as the public is concerned.
            ensure!(row.is_published(Utc::now()), not_found, "ANNOUNCEMENT_NOT_FOUND", "announcement not found");
            let locale = Self::announcement_locale(&ctx, request.locale);
            ok(GetAnnouncementResponse { announcement: row.to_public(&locale).into(), ..Default::default() })
        })
        .await
    }

    async fn list_managed_announcements(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListManagedAnnouncementsRequest>,
    ) -> ServiceResult<ListManagedAnnouncementsResponse> {
        handle(async {
            Staff::admin(&ctx)?;
            let page = Page::from_view(request.page.as_option(), 100);
            // 0 = any, 1 = draft, 2 = scheduled, 3 = published.
            let status = request.status.as_known().unwrap_or(AnnouncementStatus::ANNOUNCEMENT_STATUS_UNSPECIFIED) as i32;
            let mut rows: Vec<AnnouncementRow> = sqlx::query_as(concat!(
                "SELECT ",
                announcement_columns!(),
                " FROM announcements
                 WHERE ($1::uuid IS NULL OR id < $1)
                   AND CASE $2 WHEN 1 THEN published_at IS NULL
                               WHEN 2 THEN published_at > now()
                               WHEN 3 THEN published_at <= now()
                               ELSE true END
                 ORDER BY id DESC LIMIT $3"
            ))
            .bind(page.decode::<Uuid>()?)
            .bind(status)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |r| r.id);
            let authors: Vec<Uuid> = rows.iter().filter_map(|r| r.author_id).collect();
            let people = tokuly_identity::summaries(&self.core, &authors).await?;
            let announcements = rows.iter().map(|row| row.to_managed(row.author_id.and_then(|id| people.get(&id).cloned()))).collect();
            ok(ListManagedAnnouncementsResponse { announcements, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_managed_announcement(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetManagedAnnouncementRequest>,
    ) -> ServiceResult<GetManagedAnnouncementResponse> {
        handle(async {
            Staff::admin(&ctx)?;
            let row = self.announcement(parse_id(request.id)?).await?;
            ok(GetManagedAnnouncementResponse { announcement: self.managed_announcement(&row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn create_announcement(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateAnnouncementRequest>,
    ) -> ServiceResult<CreateAnnouncementResponse> {
        handle(async {
            let staff = Staff::admin(&ctx)?;
            let request = request.to_owned_message();
            let category = match request.category.as_known() {
                Some(AnnouncementCategory::ANNOUNCEMENT_CATEGORY_UNSPECIFIED) => AnnouncementCategory::ANNOUNCEMENT_CATEGORY_UPDATE as i32,
                _ => validate_category(request.category)?,
            };
            let title = localized_text(request.title.as_option(), true, TITLE_MAX_CHARS, "TITLE_INVALID")?;
            let summary = localized_text(request.summary.as_option(), false, SUMMARY_MAX_CHARS, "SUMMARY_TOO_LONG")?;
            let body = localized_text(request.body.as_option(), true, BODY_MAX_CHARS, "BODY_INVALID")?;
            let image = request.image.as_option().map(validate_image).transpose()?;

            let mut tx = self.core.db.begin().await?;
            let row: AnnouncementRow = sqlx::query_as(concat!(
                "INSERT INTO announcements (category, title_ja, title_en, summary_ja, summary_en, body_ja, body_en, image, pinned, author_id)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING ",
                announcement_columns!()
            ))
            .bind(category)
            .bind(&title.ja)
            .bind(&title.en)
            .bind(&summary.ja)
            .bind(&summary.en)
            .bind(&body.ja)
            .bind(&body.en)
            .bind(image)
            .bind(request.pinned)
            .bind(staff.id)
            .fetch_one(&mut *tx)
            .await?;
            Audit::by(&staff, "announcement.created").announcement(row.id).after(row.snapshot()).record(&mut *tx).await?;
            tx.commit().await?;
            ok(CreateAnnouncementResponse { announcement: self.managed_announcement(&row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn update_announcement(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateAnnouncementRequest>,
    ) -> ServiceResult<UpdateAnnouncementResponse> {
        handle(async {
            let staff = Staff::admin(&ctx)?;
            let request = request.to_owned_message();
            let id = parse_id(&request.id)?;
            let category = request.category.map(validate_category).transpose()?;
            let title = request.title.as_option().map(|t| localized_text(Some(t), true, TITLE_MAX_CHARS, "TITLE_INVALID")).transpose()?;
            let summary = request.summary.as_option().map(|t| localized_text(Some(t), false, SUMMARY_MAX_CHARS, "SUMMARY_TOO_LONG")).transpose()?;
            let body = request.body.as_option().map(|t| localized_text(Some(t), true, BODY_MAX_CHARS, "BODY_INVALID")).transpose()?;
            // `Some(None)` clears the image, `Some(Some(json))` replaces it, `None` keeps it.
            let image: Option<Option<serde_json::Value>> =
                if request.clear_image { Some(None) } else { request.image.as_option().map(validate_image).transpose()?.map(Some) };

            let mut tx = self.core.db.begin().await?;
            let before: AnnouncementRow = sqlx::query_as(concat!("SELECT ", announcement_columns!(), " FROM announcements WHERE id = $1 FOR UPDATE"))
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
                .or_not_found("ANNOUNCEMENT_NOT_FOUND", "announcement not found")?;
            let after: AnnouncementRow = sqlx::query_as(concat!(
                "UPDATE announcements SET
                     category   = COALESCE($2, category),
                     title_ja   = COALESCE($3, title_ja),     title_en   = COALESCE($4, title_en),
                     summary_ja = COALESCE($5, summary_ja),   summary_en = COALESCE($6, summary_en),
                     body_ja    = COALESCE($7, body_ja),      body_en    = COALESCE($8, body_en),
                     image      = CASE WHEN $9 THEN $10 ELSE image END,
                     pinned     = COALESCE($11, pinned)
                 WHERE id = $1 RETURNING ",
                announcement_columns!()
            ))
            .bind(id)
            .bind(category)
            .bind(title.as_ref().map(|t| &t.ja))
            .bind(title.as_ref().map(|t| &t.en))
            .bind(summary.as_ref().map(|t| &t.ja))
            .bind(summary.as_ref().map(|t| &t.en))
            .bind(body.as_ref().map(|t| &t.ja))
            .bind(body.as_ref().map(|t| &t.en))
            .bind(image.is_some())
            .bind(image.flatten())
            .bind(request.pinned)
            .fetch_one(&mut *tx)
            .await?;
            Audit::by(&staff, "announcement.updated").announcement(id).before(before.snapshot()).after(after.snapshot()).record(&mut *tx).await?;
            tx.commit().await?;
            ok(UpdateAnnouncementResponse { announcement: self.managed_announcement(&after).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn publish_announcement(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PublishAnnouncementRequest>,
    ) -> ServiceResult<PublishAnnouncementResponse> {
        handle(async {
            let staff = Staff::admin(&ctx)?;
            let request = request.to_owned_message();
            let id = parse_id(&request.id)?;
            let now = Utc::now();
            // A time in the past means "now": the feed is ordered by publish
            // time, and backdating would bury the announcement.
            let publish_at = request.publish_at.as_option().map(from_ts).filter(|at| *at > now).unwrap_or(now);
            ensure!(
                publish_at <= now + chrono::Duration::days(MAX_SCHEDULE_DAYS),
                invalid,
                "PUBLISH_TIME_INVALID",
                "announcements can be scheduled at most a year ahead"
            );

            let mut tx = self.core.db.begin().await?;
            let before: AnnouncementRow = sqlx::query_as(concat!("SELECT ", announcement_columns!(), " FROM announcements WHERE id = $1 FOR UPDATE"))
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
                .or_not_found("ANNOUNCEMENT_NOT_FOUND", "announcement not found")?;
            ensure!(!before.is_published(now), precondition, "ANNOUNCEMENT_ALREADY_PUBLISHED", "this announcement is already published");
            // `broadcast_queued_at` is reset so the worker queues the
            // broadcast when the (new) publish time arrives.
            let after: AnnouncementRow = sqlx::query_as(concat!(
                "UPDATE announcements SET published_at = $2, notify_users = $3, broadcast_queued_at = NULL WHERE id = $1 RETURNING ",
                announcement_columns!()
            ))
            .bind(id)
            .bind(publish_at)
            .bind(request.notify_users)
            .fetch_one(&mut *tx)
            .await?;
            Audit::by(&staff, "announcement.published").announcement(id).before(before.snapshot()).after(after.snapshot()).record(&mut *tx).await?;
            tx.commit().await?;

            if after.notify_users && after.is_published(Utc::now()) {
                // Best effort: the worker queues it within a minute otherwise.
                if let Err(e) = self.queue_due_broadcasts().await {
                    tracing::warn!(announcement_id = %id, error = %e, "queueing the announcement broadcast failed; the worker will retry");
                }
            }
            ok(PublishAnnouncementResponse { announcement: self.managed_announcement(&after).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn unpublish_announcement(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnpublishAnnouncementRequest>,
    ) -> ServiceResult<UnpublishAnnouncementResponse> {
        handle(async {
            let staff = Staff::admin(&ctx)?;
            let id = parse_id(request.id)?;
            let mut tx = self.core.db.begin().await?;
            let before: AnnouncementRow = sqlx::query_as(concat!("SELECT ", announcement_columns!(), " FROM announcements WHERE id = $1 FOR UPDATE"))
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
                .or_not_found("ANNOUNCEMENT_NOT_FOUND", "announcement not found")?;
            ensure!(before.published_at.is_some(), precondition, "ANNOUNCEMENT_NOT_PUBLISHED", "this announcement is a draft");
            let after: AnnouncementRow = sqlx::query_as(concat!(
                "UPDATE announcements SET published_at = NULL, broadcast_queued_at = NULL WHERE id = $1 RETURNING ",
                announcement_columns!()
            ))
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
            Audit::by(&staff, "announcement.unpublished").announcement(id).before(before.snapshot()).after(after.snapshot()).record(&mut *tx).await?;
            tx.commit().await?;
            ok(UnpublishAnnouncementResponse { announcement: self.managed_announcement(&after).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_announcement(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteAnnouncementRequest>,
    ) -> ServiceResult<DeleteAnnouncementResponse> {
        handle(async {
            let staff = Staff::admin(&ctx)?;
            let id = parse_id(request.id)?;
            let mut tx = self.core.db.begin().await?;
            let deleted: AnnouncementRow = sqlx::query_as(concat!("DELETE FROM announcements WHERE id = $1 RETURNING ", announcement_columns!()))
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
                .or_not_found("ANNOUNCEMENT_NOT_FOUND", "announcement not found")?;
            Audit::by(&staff, "announcement.deleted").announcement(id).before(deleted.snapshot()).record(&mut *tx).await?;
            tx.commit().await?;
            ok(DeleteAnnouncementResponse::default())
        })
        .await
    }
}
