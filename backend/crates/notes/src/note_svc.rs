//! `NoteService`: thin handlers over [`crate::posting`], [`crate::reactions`],
//! [`crate::timelines`], [`crate::hashtags`] and [`crate::rules`].
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::notes::*;
use unicode_normalization::UnicodeNormalization as _;

use crate::{
    Notes, hashtags,
    posting::{NewNote, requested_audience},
    reactions::{MAX_NOTES_PER_CALL, Reaction},
    rules::{self, RULES_VERSION},
    timelines::MAX_PAGE,
    visibility::Viewer,
};

/// Hashtag completions one caller may ask for per minute.
const HASHTAG_SEARCHES_PER_MINUTE: u32 = 120;

/// `(default, maximum)` sizes of the hashtag lists.
const TRENDING_LIMITS: (i32, i32) = (10, 50);
const COMPLETION_LIMITS: (i32, i32) = (10, 30);

fn clamp_limit(requested: i32, (default, max): (i32, i32)) -> i64 {
    i64::from(if requested <= 0 { default } else { requested.min(max) })
}

/// Parses the note IDs of a batch request (at most 100 of them).
fn batch_ids<'a>(ids: impl ExactSizeIterator<Item = &'a str>) -> AppResult<Vec<Uuid>> {
    ensure!(ids.len() <= MAX_NOTES_PER_CALL, invalid, "TOO_MANY_NOTES", "at most {MAX_NOTES_PER_CALL} notes per call");
    ids.map(parse_id).collect()
}

impl Notes {
    /// The caller as the visibility rules see them (`None` = anonymous).
    pub(crate) async fn viewer(&self, ctx: &RequestContext) -> AppResult<Viewer> {
        Viewer::load(&self.core, ctx.maybe_user_id()?).await
    }

    /// The caller, who must be signed in.
    pub(crate) async fn signed_in(&self, ctx: &RequestContext) -> AppResult<(Viewer, Uuid)> {
        let user_id = ctx.user_id()?;
        Ok((Viewer::load(&self.core, Some(user_id)).await?, user_id))
    }
}

impl NoteService for Notes {
    async fn get_note_rules(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetNoteRulesRequest>,
    ) -> ServiceResult<GetNoteRulesResponse> {
        handle(async {
            let agreement = match ctx.maybe_user_id()? {
                Some(user_id) => rules::agreement(&self.core, user_id).await?,
                None => None,
            };
            ok(GetNoteRulesResponse {
                version: RULES_VERSION,
                rules: rules::rules(&ctx.client().language),
                agreed: agreement.is_some_and(|(version, _)| version == RULES_VERSION),
                agreed_version: agreement.map_or(0, |(version, _)| version),
                agreed_at: opt_ts(agreement.map(|(_, at)| at)),
                ..Default::default()
            })
        })
        .await
    }

    async fn agree_to_note_rules(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AgreeToNoteRulesRequest>,
    ) -> ServiceResult<AgreeToNoteRulesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            // Nobody agrees to a text they were not shown.
            ensure!(
                request.version == RULES_VERSION,
                precondition,
                "RULES_VERSION_MISMATCH",
                "the rules changed; read version {RULES_VERSION}"
            );
            ok(AgreeToNoteRulesResponse {
                agreed_at: ts(rules::agree(&self.core, user_id).await?),
                ..Default::default()
            })
        })
        .await
    }

    async fn create_note(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateNoteRequest>,
    ) -> ServiceResult<CreateNoteResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let request = request.to_owned_message();
            let input = NewNote {
                text: request.text,
                asset_ids: parse_ids(&request.asset_ids)?,
                stamp_id: parse_opt_id(&request.stamp_id)?,
                place: request.place.as_option().cloned(),
                audience: requested_audience(request.visibility, &request.circle_id, &request.direct_user_ids)?,
                reply_to_id: parse_opt_id(&request.reply_to_id)?,
                quote_of_id: parse_opt_id(&request.quote_of_id)?,
                content_warning: request.content_warning,
                channel_id: parse_opt_id(&request.channel_id)?,
            };
            let note = self.create_note_once(user_id, &request.idempotency_key, input).await?;
            ok(CreateNoteResponse { note: self.hydrate_one(&viewer, &note).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_note(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteNoteRequest>,
    ) -> ServiceResult<DeleteNoteResponse> {
        handle(async {
            self.delete_own_note(ctx.user_id()?, parse_id(request.note_id)?).await?;
            ok(DeleteNoteResponse::default())
        })
        .await
    }

    async fn set_note_visibility(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetNoteVisibilityRequest>,
    ) -> ServiceResult<SetNoteVisibilityResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let request = request.to_owned_message();
            let wanted = requested_audience(request.visibility, &request.circle_id, &request.direct_user_ids)?
                .ok_or_else(|| AppError::invalid("INVALID_VISIBILITY", "say who may see the note"))?;
            let note = self.set_visibility(user_id, parse_id(&request.note_id)?, wanted).await?;
            ok(SetNoteVisibilityResponse { note: self.hydrate_one(&viewer, &note).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn pin_note(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PinNoteRequest>,
    ) -> ServiceResult<PinNoteResponse> {
        handle(async {
            self.pin_note(ctx.user_id()?, parse_id(request.note_id)?).await?;
            ok(PinNoteResponse::default())
        })
        .await
    }

    async fn unpin_note(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnpinNoteRequest>,
    ) -> ServiceResult<UnpinNoteResponse> {
        handle(async {
            self.unpin_note(ctx.user_id()?, parse_id(request.note_id)?).await?;
            ok(UnpinNoteResponse::default())
        })
        .await
    }

    async fn get_note(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetNoteRequest>,
    ) -> ServiceResult<GetNoteResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let note = self.require_note(&viewer, parse_id(request.note_id)?).await?;
            ok(GetNoteResponse { note: self.hydrate_one(&viewer, &note).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn batch_get_notes(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BatchGetNotesRequest>,
    ) -> ServiceResult<BatchGetNotesResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let note_ids = batch_ids(request.note_ids.iter().copied())?;
            ok(BatchGetNotesResponse { notes: self.batch(&viewer, &note_ids).await?, ..Default::default() })
        })
        .await
    }

    async fn get_note_thread(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetNoteThreadRequest>,
    ) -> ServiceResult<GetNoteThreadResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let note_id = parse_id(request.note_id)?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (ancestors, note, replies, info) = self.thread(&viewer, note_id, &page).await?;
            // Opening a thread is a view of its note (once, not per page of replies).
            if page.cursor.is_none()
                && let Err(e) = self.record_views(&viewer, &ctx.client().ip, &[note_id]).await
            {
                tracing::warn!(error = %e, "counting a thread view failed");
            }
            ok(GetNoteThreadResponse { ancestors, note: note.into(), replies, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn list_note_quotes(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListNoteQuotesRequest>,
    ) -> ServiceResult<ListNoteQuotesResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (notes, info) = self.quotes(&viewer, parse_id(request.note_id)?, &page).await?;
            ok(ListNoteQuotesResponse { notes, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn record_note_views(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RecordNoteViewsRequest>,
    ) -> ServiceResult<RecordNoteViewsResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let note_ids = batch_ids(request.note_ids.iter().copied())?;
            self.record_views(&viewer, &ctx.client().ip, &note_ids).await?;
            ok(RecordNoteViewsResponse::default())
        })
        .await
    }

    async fn like_note(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, LikeNoteRequest>,
    ) -> ServiceResult<LikeNoteResponse> {
        handle(async {
            let counters = self.react(ctx.user_id()?, parse_id(request.note_id)?, Reaction::Like).await?;
            ok(LikeNoteResponse { counters: counters.into(), ..Default::default() })
        })
        .await
    }

    async fn unlike_note(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnlikeNoteRequest>,
    ) -> ServiceResult<UnlikeNoteResponse> {
        handle(async {
            let counters = self.unreact(ctx.user_id()?, parse_id(request.note_id)?, Reaction::Like).await?;
            ok(UnlikeNoteResponse { counters: counters.into(), ..Default::default() })
        })
        .await
    }

    async fn renote_note(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RenoteNoteRequest>,
    ) -> ServiceResult<RenoteNoteResponse> {
        handle(async {
            let counters = self.react(ctx.user_id()?, parse_id(request.note_id)?, Reaction::Renote).await?;
            ok(RenoteNoteResponse { counters: counters.into(), ..Default::default() })
        })
        .await
    }

    async fn undo_renote(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UndoRenoteRequest>,
    ) -> ServiceResult<UndoRenoteResponse> {
        handle(async {
            let counters = self.unreact(ctx.user_id()?, parse_id(request.note_id)?, Reaction::Renote).await?;
            ok(UndoRenoteResponse { counters: counters.into(), ..Default::default() })
        })
        .await
    }

    async fn list_note_likers(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListNoteLikersRequest>,
    ) -> ServiceResult<ListNoteLikersResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (users, info) = self.reactors(&viewer, parse_id(request.note_id)?, Reaction::Like, &page).await?;
            ok(ListNoteLikersResponse { users, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn list_note_renoters(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListNoteRenotersRequest>,
    ) -> ServiceResult<ListNoteRenotersResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (users, info) = self.reactors(&viewer, parse_id(request.note_id)?, Reaction::Renote, &page).await?;
            ok(ListNoteRenotersResponse { users, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_home_timeline(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetHomeTimelineRequest>,
    ) -> ServiceResult<GetHomeTimelineResponse> {
        handle(async {
            let (viewer, _) = self.signed_in(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (entries, info) = self.home_timeline(&viewer, &page).await?;
            ok(GetHomeTimelineResponse { entries, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_following_timeline(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetFollowingTimelineRequest>,
    ) -> ServiceResult<GetFollowingTimelineResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (entries, info) = self.following_timeline(&viewer, user_id, &page).await?;
            ok(GetFollowingTimelineResponse { entries, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_global_timeline(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetGlobalTimelineRequest>,
    ) -> ServiceResult<GetGlobalTimelineResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (entries, info) = self.global_timeline(&viewer, &page).await?;
            ok(GetGlobalTimelineResponse { entries, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_user_timeline(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetUserTimelineRequest>,
    ) -> ServiceResult<GetUserTimelineResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let filter = request.filter.as_known().unwrap_or_default();
            let (entries, info, pinned) =
                self.user_timeline(&viewer, parse_id(request.user_id)?, filter, &page).await?;
            ok(GetUserTimelineResponse { entries, page: info.into(), pinned_notes: pinned, ..Default::default() })
        })
        .await
    }

    async fn get_hashtag_timeline(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetHashtagTimelineRequest>,
    ) -> ServiceResult<GetHashtagTimelineResponse> {
        handle(async {
            let viewer = self.viewer(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (hashtag, entries, info) = self.hashtag_timeline(&viewer, request.tag, &page).await?;
            ok(GetHashtagTimelineResponse { hashtag: hashtag.into(), entries, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_mentions_timeline(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetMentionsTimelineRequest>,
    ) -> ServiceResult<GetMentionsTimelineResponse> {
        handle(async {
            let (viewer, user_id) = self.signed_in(&ctx).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (entries, info) = self.mentions_timeline(&viewer, user_id, &page).await?;
            ok(GetMentionsTimelineResponse { entries, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn list_trending_hashtags(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListTrendingHashtagsRequest>,
    ) -> ServiceResult<ListTrendingHashtagsResponse> {
        handle(async {
            ctx.maybe_user_id()?;
            let (hashtags, computed_at) =
                hashtags::trending(&self.core, clamp_limit(request.limit, TRENDING_LIMITS)).await?;
            ok(ListTrendingHashtagsResponse { hashtags, computed_at: opt_ts(computed_at), ..Default::default() })
        })
        .await
    }

    async fn search_hashtags(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SearchHashtagsRequest>,
    ) -> ServiceResult<SearchHashtagsResponse> {
        handle(async {
            let caller = match ctx.maybe_user_id()? {
                Some(user_id) => user_id.to_string(),
                None => format!("ip:{}", ctx.client().ip),
            };
            self.core
                .cache
                .rate_limit(
                    &format!("notes:hashtag-search:{caller}"),
                    HASHTAG_SEARCHES_PER_MINUTE,
                    Duration::from_secs(60),
                )
                .await?;
            // Folded the way stored tags are, so `ＡＢ` completes to `abc`.
            let typed = request.prefix.trim();
            let typed = typed.strip_prefix('#').or_else(|| typed.strip_prefix('＃')).unwrap_or(typed);
            let prefix: String = typed.nfkc().collect::<String>().to_lowercase();
            let usable = prefix.chars().count() <= crate::text::MAX_HASHTAG_CHARS
                && prefix.chars().all(|c| c.is_alphanumeric() || c == '_');
            let hashtags = if usable {
                hashtags::complete(&self.core, &prefix, clamp_limit(request.limit, COMPLETION_LIMITS)).await?
            } else {
                Vec::new()
            };
            ok(SearchHashtagsResponse { hashtags, ..Default::default() })
        })
        .await
    }

    // ── Being built: replaced by the poll, edit, redraft, analytics and trend handlers ──

    async fn edit_note(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, EditNoteRequest>,
    ) -> ServiceResult<EditNoteResponse> {
        handle(async {
            Err::<connectrpc::Response<EditNoteResponse>, _>(AppError::unavailable("NOT_READY", "not ready"))
        })
        .await
    }

    async fn list_note_edits(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, ListNoteEditsRequest>,
    ) -> ServiceResult<ListNoteEditsResponse> {
        handle(async {
            Err::<connectrpc::Response<ListNoteEditsResponse>, _>(AppError::unavailable("NOT_READY", "not ready"))
        })
        .await
    }

    async fn redraft_note(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, RedraftNoteRequest>,
    ) -> ServiceResult<RedraftNoteResponse> {
        handle(async {
            Err::<connectrpc::Response<RedraftNoteResponse>, _>(AppError::unavailable("NOT_READY", "not ready"))
        })
        .await
    }

    async fn vote_poll(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, VotePollRequest>,
    ) -> ServiceResult<VotePollResponse> {
        handle(async {
            Err::<connectrpc::Response<VotePollResponse>, _>(AppError::unavailable("NOT_READY", "not ready"))
        })
        .await
    }

    async fn get_note_analytics(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, GetNoteAnalyticsRequest>,
    ) -> ServiceResult<GetNoteAnalyticsResponse> {
        handle(async {
            Err::<connectrpc::Response<GetNoteAnalyticsResponse>, _>(AppError::unavailable("NOT_READY", "not ready"))
        })
        .await
    }

    async fn list_trending_notes(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, ListTrendingNotesRequest>,
    ) -> ServiceResult<ListTrendingNotesResponse> {
        handle(async {
            Err::<connectrpc::Response<ListTrendingNotesResponse>, _>(AppError::unavailable("NOT_READY", "not ready"))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_sizes_have_defaults_and_ceilings() {
        assert_eq!(clamp_limit(0, TRENDING_LIMITS), 10);
        assert_eq!(clamp_limit(-3, TRENDING_LIMITS), 10);
        assert_eq!(clamp_limit(25, TRENDING_LIMITS), 25);
        assert_eq!(clamp_limit(500, TRENDING_LIMITS), 50);
        assert_eq!(clamp_limit(500, COMPLETION_LIMITS), 30);
    }

    #[test]
    fn batches_are_capped_and_ids_checked() {
        let id = Uuid::now_v7().to_string();
        assert_eq!(batch_ids([id.as_str()].into_iter()).unwrap().len(), 1);
        let many = vec![id.as_str(); MAX_NOTES_PER_CALL + 1];
        assert_eq!(batch_ids(many.into_iter()).unwrap_err().reason, "TOO_MANY_NOTES");
        assert_eq!(batch_ids(["nope"].into_iter()).unwrap_err().reason, "INVALID_ID");
    }
}
