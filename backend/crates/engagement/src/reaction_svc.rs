//! `ReactionService`: likes, dislikes and emoji (Unicode and custom) on any
//! subject, and owners' settings for reactions to their content.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_identity::users;
use tokuly_proto::engagement::*;

use crate::{
    Engagement, page_of,
    reactions::{self, Pictures, ReactedRow, rules::Settings},
    subjects::{SubjectKey, preview_of},
};

const PAGE: i64 = 30;
const PAGE_MAX: i64 = 100;

impl ReactionService for Engagement {
    async fn set_reaction(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetReactionRequest>,
    ) -> ServiceResult<SetReactionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let reaction = reactions::requested(request.reaction.as_option())?;
            let subject = self.requested_subject(request.subject.as_option(), Some(user_id)).await?;
            Engagement::set_reaction(self, user_id, &subject, &reaction).await?;
            ok(SetReactionResponse {
                reactions: self.reaction_summary(&subject, Some(user_id)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn remove_reaction(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveReactionRequest>,
    ) -> ServiceResult<RemoveReactionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let which = match request.reaction.as_option() {
                Some(reaction) => Some(reactions::requested(Some(reaction))?),
                None => None,
            };
            let subject = self.requested_subject_loose(request.subject.as_option(), user_id).await?;
            Engagement::remove_reaction(self, user_id, &subject, which.as_ref()).await?;
            // The numbers of a subject the caller can no longer see are not theirs to read.
            let reactions = match &subject.subject {
                Some(resolved) => self.reaction_summary(resolved, Some(user_id)).await?,
                None => ReactionSummary::default(),
            };
            ok(RemoveReactionResponse { reactions: reactions.into(), ..Default::default() })
        })
        .await
    }

    async fn list_reactions(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListReactionsRequest>,
    ) -> ServiceResult<ListReactionsResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let request = request.to_owned_message();
            let subject = self.requested_subject(request.subject.as_option(), viewer).await?;
            let page = page_of(request.page.as_option(), PAGE, PAGE_MAX);
            let kind = request.kind.as_known().unwrap_or_default();
            let (rows, info) = self.reactors(&subject, viewer, kind, &request.emoji, &page).await?;
            let user_ids: Vec<Uuid> = rows.iter().map(|row| row.user_id).collect();
            let summaries = tokuly_identity::summaries(&self.core, &user_ids).await?;
            let pictures = Pictures::load(&self.core, rows.iter().map(|row| row.emoji.as_str())).await?;
            let reactors = rows
                .into_iter()
                .map(|row| Reactor {
                    user: users::pick(&summaries, row.user_id).into(),
                    reaction: pictures.reaction(&row.reaction()).into(),
                    reacted_at: ts(row.created_at),
                    ..Default::default()
                })
                .collect();
            ok(ListReactionsResponse { reactors, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn list_my_reactions(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListMyReactionsRequest>,
    ) -> ServiceResult<ListMyReactionsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let page = match request.page.as_option() {
                Some(page) => Page::new(page.limit, page.cursor, PAGE, PAGE_MAX),
                None => Page::new(0, "", PAGE, PAGE_MAX),
            };
            let (rows, info) =
                self.reacted_subjects(user_id, request.subject_type.to_i32(), request.kind.to_i32(), &page).await?;
            let keys: Vec<SubjectKey> = rows.iter().map(ReactedRow::key).collect();
            let resolved = self.subjects(&keys, Some(user_id)).await?;
            let pictures = Pictures::load(&self.core, rows.iter().map(|row| row.emoji.as_str())).await?;
            let subjects = rows
                .into_iter()
                .map(|row| ReactedSubject {
                    subject: preview_of(row.key(), resolved.get(&row.key())).into(),
                    reaction: pictures.reaction(&row.reaction()).into(),
                    reacted_at: ts(row.created_at),
                    ..Default::default()
                })
                .collect();
            ok(ListMyReactionsResponse { subjects, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_reaction_settings(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetReactionSettingsRequest>,
    ) -> ServiceResult<GetReactionSettingsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let settings = self.reaction_settings(user_id).await?;
            ok(GetReactionSettingsResponse { settings: settings.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn update_reaction_settings(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateReactionSettingsRequest>,
    ) -> ServiceResult<UpdateReactionSettingsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let wanted = request.settings.as_option().cloned().unwrap_or_default();
            let settings = Settings { custom_emoji: wanted.custom_emoji, sensitive_emoji: wanted.sensitive_emoji };
            let stored = self.update_reaction_settings(user_id, settings).await?;
            ok(UpdateReactionSettingsResponse { settings: stored.to_proto().into(), ..Default::default() })
        })
        .await
    }
}
