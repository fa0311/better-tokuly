//! `EmojiService`: the catalogue as clients read it, and what each user
//! keeps on top of it (deck, recently used emoji).
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::emoji::*;

use crate::{
    Emoji, catalog, deck, emojis,
    model::{EmojiRow, Viewer, emoji_columns},
    usage::{self, Item},
    validate::{self, MAX_USES_PER_CALL, RECENTS_KEPT},
};

/// Role that may see and change every emoji (admins hold every role).
pub(crate) const MODERATOR: &str = "moderator";

const MINUTE: Duration = Duration::from_secs(60);

/// Who is calling, for the visibility rules.
pub(crate) fn viewer(ctx: &RequestContext) -> AppResult<Viewer> {
    Ok(match ctx.maybe_auth()? {
        Some(auth) => Viewer { user_id: Some(auth.user_id), moderator: auth.has_role(MODERATOR) },
        None => Viewer::default(),
    })
}

/// Rate-limit identity of a caller who may be anonymous.
fn caller(ctx: &RequestContext) -> AppResult<String> {
    Ok(match ctx.maybe_user_id()? {
        Some(user_id) => user_id.to_string(),
        None => format!("ip:{}", ctx.client().ip),
    })
}

/// A list size a client asked for, within what the RPC allows.
fn list_limit(requested: i32, default: usize, max: usize) -> usize {
    if requested <= 0 { default } else { (requested as usize).min(max) }
}

impl EmojiService for Emoji {
    async fn get_catalog(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetCatalogRequest>,
    ) -> ServiceResult<GetCatalogResponse> {
        handle(async {
            self.core.cache.rate_limit(&format!("emoji-catalog:{}", caller(&ctx)?), 60, MINUTE).await?;
            let snapshot = catalog::snapshot(&self.core).await?;
            ok(GetCatalogResponse {
                version: snapshot.version,
                emojis: snapshot.emojis(),
                categories: snapshot.categories(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_catalog_changes(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetCatalogChangesRequest>,
    ) -> ServiceResult<GetCatalogChangesResponse> {
        handle(async {
            let since = request.since_version;
            ensure!(since >= 0, invalid, "VERSION_INVALID", "a catalogue version is never negative");
            self.core.cache.rate_limit(&format!("emoji-changes:{}", caller(&ctx)?), 240, MINUTE).await?;
            let snapshot = catalog::snapshot(&self.core).await?;
            let delta = snapshot.changes_since(since);
            let categories = if delta.version == since { Vec::new() } else { snapshot.categories() };
            ok(GetCatalogChangesResponse {
                version: delta.version,
                full: delta.full,
                upserted: delta.upserted,
                removed_ids: delta.removed_ids,
                categories,
                ..Default::default()
            })
        })
        .await
    }

    async fn search_emoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SearchEmojiRequest>,
    ) -> ServiceResult<SearchEmojiResponse> {
        handle(async {
            let query = validate::query(request.query)?;
            self.core.cache.rate_limit(&format!("emoji-search:{}", caller(&ctx)?), 300, MINUTE).await?;
            let snapshot = catalog::snapshot(&self.core).await?;
            let popularity = catalog::popularity(&self.core).await?;
            let emojis = snapshot.search(&query, request.category, list_limit(request.limit, 30, 100), &popularity);
            ok(SearchEmojiResponse { emojis, ..Default::default() })
        })
        .await
    }

    async fn list_popular_emoji(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListPopularEmojiRequest>,
    ) -> ServiceResult<ListPopularEmojiResponse> {
        handle(async {
            let snapshot = catalog::snapshot(&self.core).await?;
            let popularity = catalog::popularity(&self.core).await?;
            let emojis = snapshot.popular(list_limit(request.limit, 30, 100), &popularity);
            ok(ListPopularEmojiResponse { emojis, ..Default::default() })
        })
        .await
    }

    async fn list_newest_emoji(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListNewestEmojiRequest>,
    ) -> ServiceResult<ListNewestEmojiResponse> {
        handle(async {
            let page = Page::from_view(request.page.as_option(), 100);
            let after: Option<(i64, Uuid)> = page.decode()?;
            let snapshot = catalog::snapshot(&self.core).await?;
            let mut rows = snapshot.newest(after, page.fetch() as usize);
            let page = page.finish(&mut rows, |(key, _)| *key);
            ok(ListNewestEmojiResponse {
                emojis: rows.into_iter().map(|(_, emoji)| emoji).collect(),
                page: page.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_emoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetEmojiRequest>,
    ) -> ServiceResult<GetEmojiResponse> {
        handle(async {
            let viewer = viewer(&ctx)?;
            let emoji = emojis::require(&self.core, request.emoji).await?;
            // An emoji the caller may not see does not exist for them.
            ensure!(viewer.can_see(&emoji), not_found, "EMOJI_NOT_FOUND", "emoji not found");
            ok(GetEmojiResponse { emoji: self.detail(&emoji, viewer).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn list_categories(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, ListCategoriesRequest>,
    ) -> ServiceResult<ListCategoriesResponse> {
        handle(async {
            let snapshot = catalog::snapshot(&self.core).await?;
            ok(ListCategoriesResponse {
                categories: snapshot.categories(),
                uncategorized_count: snapshot.uncategorized_count(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_author_emoji(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListAuthorEmojiRequest>,
    ) -> ServiceResult<ListAuthorEmojiResponse> {
        handle(async {
            let author_id = parse_id(request.user_id)?;
            let page = Page::from_view(request.page.as_option(), 100);
            let cursor: Option<(Time, Uuid)> = page.decode()?;
            let (before_at, before_id) = cursor.unzip();
            let mut rows: Vec<EmojiRow> = sqlx::query_as(concat!(
                "SELECT ",
                emoji_columns!(),
                " FROM emojis e
                  WHERE e.author_id = $1 AND e.state = 2
                    AND ($2::timestamptz IS NULL OR (e.approved_at, e.id) < ($2, $3))
                  ORDER BY e.approved_at DESC, e.id DESC
                  LIMIT $4"
            ))
            .bind(author_id)
            .bind(before_at)
            .bind(before_id)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let page = page.finish(&mut rows, |row| (row.approved_at.unwrap_or(row.created_at), row.id));
            ok(ListAuthorEmojiResponse {
                emojis: rows.iter().map(EmojiRow::to_proto).collect(),
                page: page.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_deck(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetDeckRequest>,
    ) -> ServiceResult<GetDeckResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let saved = deck::load(&self.core, user_id).await?;
            let is_default = saved.is_none();
            let items = saved.unwrap_or_else(|| deck::default_items(&self.cfg));
            let snapshot = catalog::snapshot(&self.core).await?;
            ok(GetDeckResponse { items: deck::picker_items(&snapshot, &items), is_default, ..Default::default() })
        })
        .await
    }

    async fn set_deck(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetDeckRequest>,
    ) -> ServiceResult<SetDeckResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let items = deck::parse(&request.emojis)?;
            self.core.cache.rate_limit(&format!("emoji-deck:{user_id}"), 60, MINUTE).await?;

            // A custom emoji can be pinned when it is in the catalogue. One
            // that was pinned before stays, even if it has left since: the
            // deck must remain saveable, and the emoji may come back.
            let snapshot = catalog::snapshot(&self.core).await?;
            let pinned = deck::load(&self.core, user_id).await?.unwrap_or_default();
            let stored: Vec<String> = items.iter().map(Item::text).collect();
            for (item, text) in items.iter().zip(&stored) {
                if let Item::Custom(name) = item {
                    ensure!(
                        snapshot.get(name).is_some() || pinned.contains(text),
                        not_found,
                        "EMOJI_NOT_FOUND",
                        "there is no emoji {text}"
                    );
                }
            }
            deck::save(&self.core, user_id, &stored).await?;
            ok(SetDeckResponse { items: deck::picker_items(&snapshot, &stored), ..Default::default() })
        })
        .await
    }

    async fn reset_deck(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ResetDeckRequest>,
    ) -> ServiceResult<ResetDeckResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            deck::reset(&self.core, user_id).await?;
            let snapshot = catalog::snapshot(&self.core).await?;
            ok(ResetDeckResponse {
                items: deck::picker_items(&snapshot, &deck::default_items(&self.cfg)),
                ..Default::default()
            })
        })
        .await
    }

    async fn record_use(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RecordUseRequest>,
    ) -> ServiceResult<RecordUseResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            ensure!(
                request.emojis.len() <= MAX_USES_PER_CALL,
                invalid,
                "TOO_MANY_ITEMS",
                "report at most {MAX_USES_PER_CALL} emoji per call"
            );
            self.core.cache.rate_limit(&format!("emoji-use:{user_id}"), 120, MINUTE).await?;
            let items: Vec<Item> = request.emojis.iter().filter_map(|text| Item::parse(text)).collect();
            let snapshot = catalog::snapshot(&self.core).await?;
            usage::record(&self.core, &snapshot, user_id, &items).await?;
            ok(RecordUseResponse::default())
        })
        .await
    }

    async fn list_recent_emoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListRecentEmojiRequest>,
    ) -> ServiceResult<ListRecentEmojiResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let limit = list_limit(request.limit, RECENTS_KEPT as usize, RECENTS_KEPT as usize);
            let recent = usage::recent(&self.core, user_id, limit as i64).await?;
            let snapshot = catalog::snapshot(&self.core).await?;
            ok(ListRecentEmojiResponse { items: deck::picker_items(&snapshot, &recent), ..Default::default() })
        })
        .await
    }

    async fn clear_recent_emoji(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ClearRecentEmojiRequest>,
    ) -> ServiceResult<ClearRecentEmojiResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            usage::clear_recent(&self.core, user_id).await?;
            ok(ClearRecentEmojiResponse::default())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_sizes_stay_within_what_the_rpc_allows() {
        assert_eq!(list_limit(0, 30, 100), 30);
        assert_eq!(list_limit(-5, 30, 100), 30);
        assert_eq!(list_limit(7, 30, 100), 7);
        assert_eq!(list_limit(5000, 30, 100), 100);
    }
}
