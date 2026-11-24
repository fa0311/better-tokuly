//! `EngagementService`: every number of a subject at once, and shares.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use futures::StreamExt as _;
use tokuly_core::prelude::*;
use tokuly_proto::engagement::*;

use crate::{
    Engagement,
    reactions::{self, Pictures},
    shares, stats,
    subjects::{Subject, SubjectKey, requested_type},
};

/// Subjects one `BatchGetEngagement` call may ask about.
const MAX_BATCH: usize = 100;
/// Subjects of a batch resolved at once.
const RESOLVE_CONCURRENCY: usize = 8;

impl Engagement {
    /// The engagement of subjects the viewer may see, in the given order.
    /// Each comes with the ID the request named it by.
    async fn engagement_of(
        &self,
        subjects: Vec<(String, Subject)>,
        viewer: Option<Uuid>,
    ) -> AppResult<Vec<SubjectEngagement>> {
        let keys: Vec<SubjectKey> = subjects.iter().map(|(_, subject)| subject.key).collect();
        let numbers = stats::load(&self.core, &keys, self.cfg.reaction_emoji_per_subject).await?;
        let (mut own_reactions, saved) = match viewer {
            Some(viewer) => {
                (reactions::of_user(&self.core, viewer, &keys).await?, self.saved_among(viewer, &keys).await?)
            }
            None => Default::default(),
        };
        let resolved: Vec<&Subject> = subjects.iter().map(|(_, subject)| subject).collect();
        let rules = self.rules_of(&resolved).await?;
        let mut summaries: Vec<ReactionSummary> = subjects
            .iter()
            .map(|(_, subject)| {
                let key = subject.key;
                let mut summary = numbers.reactions(key, &own_reactions.remove(&key).unwrap_or_default());
                summary.rules = rules.get(&key).map(|rules| rules.to_proto()).into();
                summary
            })
            .collect();
        let pictures = Pictures::for_summaries(&self.core, &summaries.iter().collect::<Vec<_>>()).await?;
        summaries.iter_mut().for_each(|summary| pictures.dress_summary(summary));
        Ok(subjects
            .into_iter()
            .zip(summaries)
            .map(|((requested_id, subject), reactions)| {
                let key = subject.key;
                let row = numbers.of(key);
                SubjectEngagement {
                    subject: key.to_ref().into(),
                    requested_id,
                    reactions: reactions.into(),
                    comment_count: row.comment_count,
                    bookmark_count: row.bookmark_count,
                    share_count: row.share_count,
                    bookmarked: saved.contains(&key),
                    interactions_enabled: subject.info.interactions_enabled,
                    ..Default::default()
                }
            })
            .collect())
    }
}

impl EngagementService for Engagement {
    async fn get_engagement(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetEngagementRequest>,
    ) -> ServiceResult<GetEngagementResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let request = request.to_owned_message();
            let subject = self.requested_subject(request.subject.as_option(), viewer).await?;
            let requested_id = request.subject.as_option().map(|subject| subject.id.clone()).unwrap_or_default();
            let engagement = self.engagement_of(vec![(requested_id, subject)], viewer).await?.pop();
            ok(GetEngagementResponse { engagement: engagement.into(), ..Default::default() })
        })
        .await
    }

    async fn batch_get_engagement(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BatchGetEngagementRequest>,
    ) -> ServiceResult<BatchGetEngagementResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let request = request.to_owned_message();
            ensure!(
                request.subjects.len() <= MAX_BATCH,
                invalid,
                "TOO_MANY_SUBJECTS",
                "at most {MAX_BATCH} subjects per call"
            );
            let mut wanted = Vec::with_capacity(request.subjects.len());
            for subject in request.subjects {
                wanted.push((requested_type(subject.r#type)?, subject.id));
            }
            // One cached lookup of the viewer's blocks instead of one query per subject.
            let hidden = self.hidden_from(viewer).await?;
            let mut resolving = futures::stream::iter(wanted)
                .map(|(kind, id)| async move {
                    let resolved = self.subject_unguarded(kind, &id, viewer).await;
                    (id, resolved)
                })
                .buffered(RESOLVE_CONCURRENCY);
            let mut visible = Vec::new();
            while let Some((id, resolved)) = resolving.next().await {
                match resolved {
                    Ok(subject) if !hidden.contains(&subject.info.owner_id) => visible.push((id, subject)),
                    Ok(_) => {}
                    // Not there, or not for this viewer: left out. A type no
                    // domain serves is the caller's mistake and fails the call.
                    Err(error) if error.is_not_found() => {}
                    Err(error) => return Err(error),
                }
            }
            drop(resolving);
            ok(BatchGetEngagementResponse {
                engagements: self.engagement_of(visible, viewer).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn record_share(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RecordShareRequest>,
    ) -> ServiceResult<RecordShareResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let request = request.to_owned_message();
            let target = shares::requested_target(request.target)?;
            let subject = self.requested_subject(request.subject.as_option(), viewer).await?;
            let share_count = Engagement::record_share(self, viewer, &ctx.client().ip, &subject, target).await?;
            ok(RecordShareResponse { share_count, url: self.core.web_url(&subject.info.path), ..Default::default() })
        })
        .await
    }

    async fn get_share_stats(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetShareStatsRequest>,
    ) -> ServiceResult<GetShareStatsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let subject = self.requested_subject(request.subject.as_option(), Some(user_id)).await?;
            ensure!(
                subject.is_owned_by(user_id),
                denied,
                "NOT_SUBJECT_OWNER",
                "only the owner of the content can see this"
            );
            let (share_count, targets) = self.share_stats(&subject).await?;
            ok(GetShareStatsResponse { share_count, targets, ..Default::default() })
        })
        .await
    }
}
