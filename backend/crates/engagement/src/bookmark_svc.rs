//! `BookmarkService`: saved subjects and the collections they are kept in.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::engagement::*;

use crate::{
    Engagement,
    bookmarks::BookmarkRow,
    stats,
    subjects::{SubjectKey, preview_of},
};

const PAGE: i64 = 30;
const PAGE_MAX: i64 = 100;

impl Engagement {
    async fn bookmark_count(&self, key: SubjectKey) -> AppResult<i64> {
        Ok(stats::load(&self.core, &[key]).await?.of(key).bookmark_count)
    }
}

impl BookmarkService for Engagement {
    async fn add_bookmark(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddBookmarkRequest>,
    ) -> ServiceResult<AddBookmarkResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let collection_id = parse_opt_id(&request.collection_id)?;
            let subject = self.requested_subject(request.subject.as_option(), Some(user_id)).await?;
            Engagement::add_bookmark(self, user_id, &subject, collection_id).await?;
            ok(AddBookmarkResponse { bookmark_count: self.bookmark_count(subject.key).await?, ..Default::default() })
        })
        .await
    }

    async fn remove_bookmark(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveBookmarkRequest>,
    ) -> ServiceResult<RemoveBookmarkResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let collection_id = parse_opt_id(&request.collection_id)?;
            let subject = self.requested_subject_loose(request.subject.as_option(), user_id).await?;
            let bookmarked = Engagement::remove_bookmark(self, user_id, &subject, collection_id).await?;
            // The numbers of a subject the caller can no longer see are not theirs to read.
            let bookmark_count = match subject.subject {
                Some(_) => self.bookmark_count(subject.key).await?,
                None => 0,
            };
            ok(RemoveBookmarkResponse { bookmarked, bookmark_count, ..Default::default() })
        })
        .await
    }

    async fn list_bookmarks(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListBookmarksRequest>,
    ) -> ServiceResult<ListBookmarksResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let collection_id = parse_opt_id(request.collection_id)?;
            let page = match request.page.as_option() {
                Some(page) => Page::new(page.limit, page.cursor, PAGE, PAGE_MAX),
                None => Page::new(0, "", PAGE, PAGE_MAX),
            };
            let (rows, info) = self.bookmarks(user_id, collection_id, request.subject_type.to_i32(), &page).await?;
            let keys: Vec<SubjectKey> = rows.iter().map(BookmarkRow::key).collect();
            let resolved = self.subjects(&keys, Some(user_id)).await?;
            let bookmarks = rows
                .into_iter()
                .map(|row| Bookmark {
                    subject: preview_of(row.key(), resolved.get(&row.key())).into(),
                    saved_at: ts(row.created_at),
                    ..Default::default()
                })
                .collect();
            ok(ListBookmarksResponse { bookmarks, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn list_collections(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListCollectionsRequest>,
    ) -> ServiceResult<ListCollectionsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let subject = match request.subject.as_option() {
                Some(subject) => Some(self.requested_subject_loose(Some(subject), user_id).await?.key),
                None => None,
            };
            let collections = self.collections(user_id, subject).await?;
            ok(ListCollectionsResponse {
                collections: collections.iter().map(|(collection, holds)| collection.to_proto(*holds)).collect(),
                ..Default::default()
            })
        })
        .await
    }

    async fn create_collection(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateCollectionRequest>,
    ) -> ServiceResult<CreateCollectionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let collection = Engagement::create_collection(self, user_id, request.name).await?;
            ok(CreateCollectionResponse { collection: collection.to_proto(false).into(), ..Default::default() })
        })
        .await
    }

    async fn update_collection(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateCollectionRequest>,
    ) -> ServiceResult<UpdateCollectionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let collection = self.rename_collection(user_id, parse_id(request.collection_id)?, request.name).await?;
            ok(UpdateCollectionResponse { collection: collection.to_proto(false).into(), ..Default::default() })
        })
        .await
    }

    async fn delete_collection(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteCollectionRequest>,
    ) -> ServiceResult<DeleteCollectionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            Engagement::delete_collection(self, user_id, parse_id(request.collection_id)?).await?;
            ok(DeleteCollectionResponse::default())
        })
        .await
    }
}
