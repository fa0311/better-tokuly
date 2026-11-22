//! `ShortDiscoveryService`: the creator grid, hashtag pages and sound pages.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::shorts::*;
use tokuly_video::channels;

use crate::{
    Shorts,
    access::{self, Viewer},
    hashtags, hydrate,
    shorts::{ShortRow, short_columns},
    snapshot, sounds, text,
};

/// Largest page of the lists in this service.
const MAX_PAGE: i64 = 50;

/// A `limit` field: `default` when unset, never more than `max`.
fn bounded(limit: i32, default: i64, max: i64) -> i64 {
    if limit <= 0 { default } else { i64::from(limit).min(max) }
}

impl Shorts {
    /// Limits completion requests per caller (they arrive per keystroke).
    async fn limit_suggestions(&self, ctx: &RequestContext) -> AppResult<()> {
        let who = match ctx.maybe_user_id()? {
            Some(user_id) => user_id.to_string(),
            None => format!("ip:{}", ctx.client().ip),
        };
        self.core.cache.rate_limit(&format!("shorts-suggest:{who}"), 240, Duration::from_secs(60)).await
    }

    /// The pinned shorts of a channel that `viewer` sees on its grid, the
    /// newest pin first.
    async fn pinned(&self, viewer: &Viewer, channel_id: Uuid) -> AppResult<Vec<ShortRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            short_columns!(),
            " FROM shorts s WHERE s.channel_id = $4 AND s.pinned_at IS NOT NULL AND ",
            access::listed!(1, 2, 3),
            " ORDER BY s.pinned_at DESC, s.id DESC"
        ))
        .bind(viewer.user_id)
        .bind(&viewer.friends)
        .bind(&viewer.blocked)
        .bind(channel_id)
        .fetch_all(&self.core.db)
        .await?)
    }
}

impl ShortDiscoveryService for Shorts {
    async fn list_channel_shorts(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListChannelShortsRequest>,
    ) -> ServiceResult<ListChannelShortsResponse> {
        handle(async {
            let core = &self.core;
            let viewer = self.viewer(&ctx).await?;
            let channel = channels::find_visible(core, &viewer, request.channel_id).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let popular = request.sort == ChannelShortSort::CHANNEL_SHORT_SORT_POPULAR;
            let (rows, info) = if popular {
                // View counts move all the time: page on a snapshot of the order.
                let channel_id = channel.id;
                let build = || async move {
                    let ids: Vec<Uuid> = sqlx::query_scalar(
                        "SELECT id FROM shorts WHERE channel_id = $1 AND status = 2
                         ORDER BY view_count DESC, id DESC LIMIT $2",
                    )
                    .bind(channel_id)
                    .bind(snapshot::SNAPSHOT_SIZE)
                    .fetch_all(&core.db)
                    .await?;
                    Ok(ids)
                };
                let key = format!("shorts:top:channel:{}", channel_id.simple());
                snapshot::read(core, &key, &page, build, |row| access::sees_listed(&viewer, &row.facts())).await?
            } else {
                // The first page opens with the pins; the cursor then runs
                // through the unpinned shorts only.
                let before: Option<(Time, Uuid)> = page.decode()?;
                let pinned = if before.is_none() { self.pinned(&viewer, channel.id).await? } else { Vec::new() };
                let rest = Page { limit: (page.limit - pinned.len() as i64).max(1), cursor: page.cursor.clone() };
                let mut rows: Vec<ShortRow> = sqlx::query_as(concat!(
                    "SELECT ",
                    short_columns!(),
                    " FROM shorts s WHERE s.channel_id = $4 AND s.pinned_at IS NULL AND ",
                    access::listed!(1, 2, 3),
                    " AND ($5::timestamptz IS NULL OR (s.published_at, s.id) < ($5, $6))
                      ORDER BY s.published_at DESC, s.id DESC LIMIT $7"
                ))
                .bind(viewer.user_id)
                .bind(&viewer.friends)
                .bind(&viewer.blocked)
                .bind(channel.id)
                .bind(before.map(|(at, _)| at))
                .bind(before.map(|(_, id)| id))
                .bind(rest.fetch())
                .fetch_all(&core.db)
                .await?;
                let info = rest.finish(&mut rows, |row| (row.published_at.unwrap_or(row.created_at), row.id));
                (pinned.into_iter().chain(rows).collect(), info)
            };
            ok(ListChannelShortsResponse {
                shorts: hydrate::shorts(core, &viewer, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_hashtag(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetHashtagRequest>,
    ) -> ServiceResult<GetHashtagResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let tag = hashtags::normalize(request.tag)?;
            let found: Vec<_> = hashtags::find(&self.core, &tag).await?.into_iter().collect();
            // A hashtag of which the caller may see nothing is no hashtag to them.
            let hashtag = hashtags::to_protos(&self.core, &viewer, &found)
                .await?
                .pop()
                .or_not_found("HASHTAG_NOT_FOUND", "no public short carries this hashtag")?;
            ok(GetHashtagResponse { hashtag: hashtag.into(), ..Default::default() })
        })
        .await
    }

    async fn list_hashtag_shorts(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListHashtagShortsRequest>,
    ) -> ServiceResult<ListHashtagShortsResponse> {
        handle(async {
            let core = &self.core;
            let viewer = self.viewer(&ctx).await?;
            let tag = hashtags::normalize(request.tag)?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let sort = request.sort.as_known().unwrap_or(HashtagSort::HASHTAG_SORT_TOP);
            // A hashtag nobody uses is an empty page, not an error: the page
            // of a tag someone just typed exists as soon as it is used.
            let (rows, info) = match hashtags::find(core, &tag).await? {
                Some(hashtag) => hashtags::shorts_page(core, &viewer, hashtag.id, sort, &page).await?,
                None => {
                    page.decode::<serde_json::Value>()?;
                    (Vec::new(), Default::default())
                }
            };
            ok(ListHashtagShortsResponse {
                shorts: hydrate::shorts(core, &viewer, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_trending_hashtags(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListTrendingHashtagsRequest>,
    ) -> ServiceResult<ListTrendingHashtagsResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let limit = bounded(request.limit, 20, hashtags::TRENDING_SIZE);
            let trending = hashtags::trending(&self.core, limit).await?;
            ok(ListTrendingHashtagsResponse {
                hashtags: hashtags::to_protos(&self.core, &viewer, &trending).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn suggest_hashtags(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SuggestHashtagsRequest>,
    ) -> ServiceResult<SuggestHashtagsResponse> {
        handle(async {
            self.limit_suggestions(&ctx).await?;
            // What is typed so far need not be a whole hashtag yet ("#2" on
            // the way to "#2026年"): anything that normalizes to nothing
            // simply completes to nothing.
            let typed = request.prefix.trim().trim_start_matches(['#', '＃']);
            let prefix: String = {
                use unicode_normalization::UnicodeNormalization as _;
                typed.nfkc().collect::<String>().to_lowercase()
            };
            let usable = !prefix.is_empty()
                && text::char_count(&prefix) <= text::MAX_HASHTAG_CHARS
                && prefix.chars().all(|c| c.is_alphanumeric() || c == '_');
            let found = if usable {
                hashtags::suggest(&self.core, &prefix, bounded(request.limit, 10, 20)).await?
            } else {
                Vec::new()
            };
            let viewer = self.viewer(&ctx).await?;
            ok(SuggestHashtagsResponse {
                hashtags: hashtags::to_protos(&self.core, &viewer, &found).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn get_sound(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetSoundRequest>,
    ) -> ServiceResult<GetSoundResponse> {
        handle(async {
            let core = &self.core;
            let viewer = self.viewer(&ctx).await?;
            let sound = sounds::find_named(core, request.sound_id).await?;
            let channel_ids: Vec<Uuid> = sound.channel_id_for(&viewer).into_iter().collect();
            let channel_cards = channels::summaries(core, &channel_ids).await?;
            let hidden = sounds::hidden_uses(core, &viewer, &[sound.id]).await?.get(&sound.id).copied().unwrap_or(0);
            ok(GetSoundResponse { sound: sound.to_proto(&viewer, &channel_cards, hidden).into(), ..Default::default() })
        })
        .await
    }

    async fn list_sound_shorts(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListSoundShortsRequest>,
    ) -> ServiceResult<ListSoundShortsResponse> {
        handle(async {
            let core = &self.core;
            let viewer = self.viewer(&ctx).await?;
            let sound = sounds::find_named(core, request.sound_id).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, info) = sounds::shorts_page(core, &viewer, sound.id, &page).await?;
            ok(ListSoundShortsResponse {
                shorts: hydrate::shorts(core, &viewer, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn suggest_sounds(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SuggestSoundsRequest>,
    ) -> ServiceResult<SuggestSoundsResponse> {
        handle(async {
            let core = &self.core;
            self.limit_suggestions(&ctx).await?;
            let viewer = self.viewer(&ctx).await?;
            let found = sounds::suggest(core, &viewer, request.query, bounded(request.limit, 10, 20)).await?;
            let mut channel_ids: Vec<Uuid> = found.iter().filter_map(|sound| sound.channel_id_for(&viewer)).collect();
            channel_ids.sort_unstable();
            channel_ids.dedup();
            let channel_cards = channels::summaries(core, &channel_ids).await?;
            let sound_ids: Vec<Uuid> = found.iter().map(|sound| sound.id).collect();
            let hidden = sounds::hidden_uses(core, &viewer, &sound_ids).await?;
            ok(SuggestSoundsResponse {
                sounds: found
                    .iter()
                    .map(|sound| sound.to_proto(&viewer, &channel_cards, hidden.get(&sound.id).copied().unwrap_or(0)))
                    .collect(),
                ..Default::default()
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_default_and_are_capped() {
        assert_eq!(bounded(0, 10, 20), 10);
        assert_eq!(bounded(-4, 10, 20), 10);
        assert_eq!(bounded(15, 10, 20), 15);
        assert_eq!(bounded(500, 10, 20), 20);
    }
}
