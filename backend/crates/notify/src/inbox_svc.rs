//! `InboxService`: the signed-in user reading and tidying their inbox.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::notify::{mark_read_request::Target, notify_event::Event as NotifyEventKind, *};

use crate::{
    Notify, categories,
    inbox::{self, NotificationRow, notification_columns},
};

const MAX_PAGE_SIZE: i64 = 100;
/// IDs accepted by one request, and listed in one realtime event.
const MAX_IDS: usize = 200;

/// An optional category filter: empty means "all".
fn category_filter(category: &str) -> AppResult<Option<&'static str>> {
    if category.is_empty() {
        return Ok(None);
    }
    match categories::find(category) {
        Some(category) => Ok(Some(category.id)),
        None => bail!(
            invalid,
            "CATEGORY_UNKNOWN",
            "unknown notification category {category:?}"
        ),
    }
}

fn check_id_count(count: usize) -> AppResult<()> {
    ensure!(
        count <= MAX_IDS,
        invalid,
        "TOO_MANY_IDS",
        "at most {MAX_IDS} notifications per request"
    );
    Ok(())
}

/// The IDs to list in a realtime event, or "too many, refetch".
fn ids_or_bulk(ids: &[Uuid]) -> (Vec<String>, bool) {
    if ids.len() > MAX_IDS {
        (Vec::new(), true)
    } else {
        (ids.iter().map(Uuid::to_string).collect(), false)
    }
}

fn count_of(ids: &[Uuid]) -> i32 {
    ids.len().min(i32::MAX as usize) as i32
}

impl Notify {
    /// Tells the user's other devices that entries were read.
    async fn publish_read(&self, user_id: Uuid, ids: &[Uuid], unread: &UnreadCount) {
        if ids.is_empty() {
            return;
        }
        let (notification_ids, bulk) = ids_or_bulk(ids);
        let event = NotificationsRead {
            notification_ids,
            bulk,
            unread: unread.clone().into(),
            ..Default::default()
        };
        self.publish(user_id, NotifyEventKind::from(event)).await;
    }

    /// Tells the user's other devices that entries were deleted.
    async fn publish_removed(&self, user_id: Uuid, ids: &[Uuid], unread: &UnreadCount) {
        if ids.is_empty() {
            return;
        }
        let (notification_ids, bulk) = ids_or_bulk(ids);
        let event = NotificationsRemoved {
            notification_ids,
            bulk,
            unread: unread.clone().into(),
            ..Default::default()
        };
        self.publish(user_id, NotifyEventKind::from(event)).await;
    }

    pub(crate) async fn publish(&self, user_id: Uuid, event: NotifyEventKind) {
        let event = NotifyEvent {
            event: Some(event),
            ..Default::default()
        };
        self.core.rt.notify_user(user_id, &event).await;
    }
}

impl InboxService for Notify {
    async fn list_notifications(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListNotificationsRequest>,
    ) -> ServiceResult<ListNotificationsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let category = category_filter(request.category)?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE_SIZE);
            // Keyset: the sort position of the last entry of the previous page.
            let after: Option<(Time, Uuid)> = page.decode()?;
            let mut rows: Vec<NotificationRow> = sqlx::query_as(concat!(
                "SELECT ",
                notification_columns!(),
                " FROM notifications
                 WHERE user_id = $1
                   AND ($2::text IS NULL OR category = $2)
                   AND (NOT $3 OR read_at IS NULL)
                   AND ($4::timestamptz IS NULL OR (occurred_at, id) < ($4, $5))
                 ORDER BY occurred_at DESC, id DESC
                 LIMIT $6"
            ))
            .bind(user_id)
            .bind(category)
            .bind(request.unread_only)
            .bind(after.map(|(occurred_at, _)| occurred_at))
            .bind(after.map(|(_, id)| id))
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |r| (r.occurred_at, r.id));
            let locale = inbox::locale_of(&self.core, user_id).await?;
            ok(ListNotificationsResponse {
                notifications: inbox::to_protos(&self.core, &rows, &locale).await?,
                page: info.into(),
                unread: inbox::unread_count(&self.core.db, user_id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_unread_count(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetUnreadCountRequest>,
    ) -> ServiceResult<GetUnreadCountResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            ok(GetUnreadCountResponse {
                unread: inbox::unread_count(&self.core.db, user_id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn mark_read(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, MarkReadRequest>,
    ) -> ServiceResult<MarkReadResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let category = category_filter(&request.category)?;
            let db = &self.core.db;
            let marked: Vec<Uuid> = match request.target {
                Some(Target::Ids(ids)) => {
                    ensure!(!ids.ids.is_empty(), invalid, "TARGET_REQUIRED", "say which notifications to mark as read");
                    check_id_count(ids.ids.len())?;
                    sqlx::query_scalar("UPDATE notifications SET read_at = now() WHERE user_id = $1 AND id = ANY($2) AND read_at IS NULL RETURNING id")
                        .bind(user_id)
                        .bind(parse_ids(&ids.ids)?)
                        .fetch_all(db)
                        .await?
                }
                Some(Target::UpToId(id)) => {
                    let pivot_id = parse_id(&id)?;
                    let pivot: Option<Time> =
                        sqlx::query_scalar("SELECT occurred_at FROM notifications WHERE id = $1 AND user_id = $2").bind(pivot_id).bind(user_id).fetch_optional(db).await?;
                    let pivot = pivot.or_not_found("NOTIFICATION_NOT_FOUND", "notification not found")?;
                    sqlx::query_scalar(
                        "UPDATE notifications SET read_at = now()
                         WHERE user_id = $1 AND read_at IS NULL AND (occurred_at, id) <= ($2, $3) AND ($4::text IS NULL OR category = $4)
                         RETURNING id",
                    )
                    .bind(user_id)
                    .bind(pivot)
                    .bind(pivot_id)
                    .bind(category)
                    .fetch_all(db)
                    .await?
                }
                Some(Target::All(true)) => {
                    sqlx::query_scalar("UPDATE notifications SET read_at = now() WHERE user_id = $1 AND read_at IS NULL AND ($2::text IS NULL OR category = $2) RETURNING id")
                        .bind(user_id)
                        .bind(category)
                        .fetch_all(db)
                        .await?
                }
                Some(Target::All(false)) | None => bail!(invalid, "TARGET_REQUIRED", "say which notifications to mark as read"),
            };
            let unread = inbox::unread_count(db, user_id).await?;
            self.publish_read(user_id, &marked, &unread).await;
            ok(MarkReadResponse { marked_count: count_of(&marked), unread: unread.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_notifications(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteNotificationsRequest>,
    ) -> ServiceResult<DeleteNotificationsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            ensure!(
                !request.notification_ids.is_empty(),
                invalid,
                "IDS_REQUIRED",
                "say which notifications to delete"
            );
            check_id_count(request.notification_ids.len())?;
            // Scoped to the caller: someone else's IDs simply match nothing.
            let deleted: Vec<Uuid> = sqlx::query_scalar(
                "DELETE FROM notifications WHERE user_id = $1 AND id = ANY($2) RETURNING id",
            )
            .bind(user_id)
            .bind(parse_ids(&request.notification_ids)?)
            .fetch_all(&self.core.db)
            .await?;
            let unread = inbox::unread_count(&self.core.db, user_id).await?;
            self.publish_removed(user_id, &deleted, &unread).await;
            ok(DeleteNotificationsResponse {
                deleted_count: count_of(&deleted),
                unread: unread.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn clear_notifications(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ClearNotificationsRequest>,
    ) -> ServiceResult<ClearNotificationsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let category = category_filter(request.category)?;
            let deleted: Vec<Uuid> = sqlx::query_scalar(
                "DELETE FROM notifications
                 WHERE user_id = $1 AND ($2::text IS NULL OR category = $2) AND (NOT $3 OR read_at IS NOT NULL)
                 RETURNING id",
            )
            .bind(user_id)
            .bind(category)
            .bind(request.read_only)
            .fetch_all(&self.core.db)
            .await?;
            let unread = inbox::unread_count(&self.core.db, user_id).await?;
            self.publish_removed(user_id, &deleted, &unread).await;
            ok(ClearNotificationsResponse { deleted_count: count_of(&deleted), unread: unread.into(), ..Default::default() })
        })
        .await
    }
}
