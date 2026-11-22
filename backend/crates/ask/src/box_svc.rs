//! `QuestionBoxService`: thin handlers over `questions`, `answers`,
//! `senders` and `settings`.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{events, prelude::*};
use tokuly_identity::UserRow;
use tokuly_media::assets;
use tokuly_proto::ask::{block_sender_request::Target, *};
use tokuly_social::graph;

use crate::{
    Ask,
    answers::{IMAGE_PURPOSES, question_holder},
    events::{QUESTION_STORED, Stored},
    questions::{ANSWERED, ARCHIVED, FILTERED, INBOX, MAX_PAGE, QuestionRow, question_columns},
    senders::BlockTarget,
    settings, text,
};

/// IDs accepted by one "mark as read / seen" call.
pub(crate) const MAX_IDS: usize = 100;
/// Sender blocks one user may make per hour.
const BLOCKS_PER_HOUR: u32 = 120;

impl Ask {
    /// The owner of the box named by `reference` (a user ID or a handle), as
    /// far as `viewer` may know of them: an active account with no block
    /// between the two. Anything else is `BOX_NOT_FOUND`.
    pub(crate) async fn box_owner(&self, reference: &str, viewer: Option<Uuid>) -> AppResult<UserRow> {
        self.visible_user(reference, viewer).await?.or_not_found("BOX_NOT_FOUND", "question box not found")
    }

    /// Stores a question and queues its delivery. The statements are the
    /// same for every sender; what differs for a blocked sender or a muted
    /// word is two column values.
    async fn store_question(
        &self,
        sender: Uuid,
        owner: Uuid,
        signed: bool,
        text: &str,
        image: Option<(Uuid, serde_json::Value)>,
        folder: i16,
        dropped: bool,
    ) -> AppResult<QuestionRow> {
        let mut tx = self.core.db.begin().await?;
        // `$1` is the sender: the columns compare it with the stored sender,
        // so the row comes back as "sent by the viewer".
        let question: QuestionRow = sqlx::query_as(concat!(
            "INSERT INTO ask_questions AS q (sender_id, owner_id, signed, text, image, image_asset_id, folder, dropped)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             RETURNING ",
            question_columns!()
        ))
        .bind(sender)
        .bind(owner)
        .bind(signed)
        .bind(text)
        .bind(image.as_ref().map(|(_, image)| image))
        .bind(image.as_ref().map(|(asset_id, _)| *asset_id))
        .bind(folder)
        .bind(dropped)
        .fetch_one(&mut *tx)
        .await?;
        if let Some((asset_id, _)) = &image {
            assets::attach(&mut *tx, *asset_id, &question_holder(question.id)).await?;
        }
        // No actor: the event stream must not say who asked.
        events::emit(&mut *tx, QUESTION_STORED, None, &Stored { id: question.id }).await?;
        tx.commit().await?;
        Ok(question)
    }
}

impl QuestionBoxService for Ask {
    async fn get_settings(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetSettingsRequest>,
    ) -> ServiceResult<GetSettingsResponse> {
        handle(async {
            let settings = settings::load(&self.core.db, ctx.user_id()?).await?;
            ok(GetSettingsResponse { settings: settings.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn update_settings(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateSettingsRequest>,
    ) -> ServiceResult<UpdateSettingsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let change = request.to_owned_message();
            let mut tx = self.core.db.begin().await?;
            // Serializes two devices changing different settings at once.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(format!("ask:settings:{user_id}"))
                .execute(&mut *tx)
                .await?;
            let mut settings = settings::load(&mut *tx, user_id).await?;
            settings.apply(&change)?;
            settings::save(&mut *tx, user_id, &settings).await?;
            tx.commit().await?;
            ok(UpdateSettingsResponse { settings: settings.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn get_box(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetBoxRequest>,
    ) -> ServiceResult<GetBoxResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let owner = self.box_owner(request.user, viewer).await?;
            let settings = settings::load(&self.core.db, owner.id).await?;
            let can_ask = match viewer {
                Some(viewer) if viewer != owner.id && settings.box_open => {
                    !settings.friends_only() || graph::are_friends(&self.core, owner.id, viewer).await?
                }
                _ => false,
            };
            let audiences = self.page_audiences(owner.id, viewer).await?;
            let prompt = if settings.box_prompt.is_empty() {
                settings::DEFAULT_BOX_PROMPT.0.to_owned()
            } else {
                settings.box_prompt.clone()
            };
            let question_box = QuestionBox {
                owner: owner.summary().into(),
                open: settings.box_open,
                prompt,
                audience: i32::from(settings.audience).into(),
                allow_signed: settings.allow_signed,
                allow_images: settings.allow_images,
                can_ask,
                share_url: self.core.web_url(&format!("/u/{}/ask", owner.handle)),
                answer_count: self.answer_count(owner.id, audiences).await? as i32,
                ..Default::default()
            };
            ok(GetBoxResponse { r#box: question_box.into(), ..Default::default() })
        })
        .await
    }

    async fn ask_question(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AskQuestionRequest>,
    ) -> ServiceResult<AskQuestionResponse> {
        handle(async {
            let sender = ctx.user_id()?;
            let owner = self.box_owner(request.user, Some(sender)).await?;
            ensure!(owner.id != sender, invalid, "CANNOT_ASK_SELF", "you cannot put a question into your own box");
            let settings = settings::load(&self.core.db, owner.id).await?;
            ensure!(settings.box_open, precondition, "BOX_CLOSED", "this question box is closed");
            if settings.friends_only() {
                ensure!(
                    graph::are_friends(&self.core, owner.id, sender).await?,
                    denied,
                    "FRIENDS_ONLY",
                    "only friends can ask here"
                );
            }
            let signed = request.signed;
            ensure!(
                !signed || settings.allow_signed,
                precondition,
                "SIGNED_NOT_ALLOWED",
                "this box takes anonymous questions only"
            );
            let text = text::required(request.text, text::QUESTION_MAX, "QUESTION_TEXT_INVALID", "the question")?;
            let image = match parse_opt_id(request.image_asset_id)? {
                Some(asset_id) => {
                    // An upload names its uploader to anyone who asks the
                    // media API for it, so an image on an anonymous question
                    // would undo the anonymity. Signed questions only.
                    ensure!(signed, invalid, "IMAGE_REQUIRES_SIGNATURE", "only signed questions can carry an image");
                    ensure!(settings.allow_images, precondition, "IMAGES_NOT_ALLOWED", "this box does not take images");
                    Some((asset_id, assets::require_image_json(&self.core, asset_id, sender, IMAGE_PURPOSES).await?))
                }
                None => None,
            };
            // From here on nothing may depend on whether the owner blocked
            // the sender: the limits count first, and both outcomes store
            // the same row and event.
            self.limit_sends(sender, owner.id).await?;
            let intake = self.intake(sender, owner.id, &text, &settings.muted_words).await?;
            let folder = if intake.held { FILTERED } else { INBOX };
            let question = self.store_question(sender, owner.id, signed, &text, image, folder, intake.dropped).await?;
            ok(AskQuestionResponse {
                question: self.render_one(&question, Some(sender)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_asked_questions(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListAskedQuestionsRequest>,
    ) -> ServiceResult<ListAskedQuestionsResponse> {
        handle(async {
            let sender = ctx.user_id()?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, info) = self.sent_page(sender, &page).await?;
            let questions = self.renderer(&rows, Some(sender)).await?.render_all(&rows);
            ok(ListAskedQuestionsResponse { questions, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn withdraw_question(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, WithdrawQuestionRequest>,
    ) -> ServiceResult<WithdrawQuestionResponse> {
        handle(async {
            self.take_back_question(ctx.user_id()?, parse_id(request.question_id)?).await?;
            ok(WithdrawQuestionResponse::default())
        })
        .await
    }

    async fn list_inbox(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListInboxRequest>,
    ) -> ServiceResult<ListInboxResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            let folder = match request.folder.as_known() {
                Some(QuestionFolder::QUESTION_FOLDER_UNSPECIFIED | QuestionFolder::QUESTION_FOLDER_INBOX) => INBOX,
                Some(QuestionFolder::QUESTION_FOLDER_ANSWERED) => ANSWERED,
                Some(QuestionFolder::QUESTION_FOLDER_ARCHIVED) => ARCHIVED,
                Some(QuestionFolder::QUESTION_FOLDER_FILTERED) => FILTERED,
                None => bail!(invalid, "FOLDER_INVALID", "unknown folder"),
            };
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, info) = self.inbox_page(owner, folder, &page).await?;
            let questions = self.renderer(&rows, Some(owner)).await?.render_all(&rows);
            ok(ListInboxResponse { questions, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_inbox_counts(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetInboxCountsRequest>,
    ) -> ServiceResult<GetInboxCountsResponse> {
        handle(async {
            ok(GetInboxCountsResponse { counts: self.inbox_counts(ctx.user_id()?).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn mark_questions_read(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, MarkQuestionsReadRequest>,
    ) -> ServiceResult<MarkQuestionsReadResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            ensure!(
                request.question_ids.len() <= MAX_IDS,
                invalid,
                "TOO_MANY_IDS",
                "at most {MAX_IDS} questions per call"
            );
            let ids = request.question_ids.iter().map(|id| parse_id(id)).collect::<AppResult<Vec<Uuid>>>()?;
            self.mark_read(owner, &ids, request.all).await?;
            ok(MarkQuestionsReadResponse { counts: self.inbox_counts(owner).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn move_question(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, MoveQuestionRequest>,
    ) -> ServiceResult<MoveQuestionResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            let folder = match request.folder.as_known() {
                Some(QuestionFolder::QUESTION_FOLDER_INBOX) => INBOX,
                Some(QuestionFolder::QUESTION_FOLDER_ARCHIVED) => ARCHIVED,
                _ => bail!(invalid, "FOLDER_INVALID", "a question can be moved to the inbox or the archive"),
            };
            let question = self.refile_question(owner, parse_id(request.question_id)?, folder).await?;
            ok(MoveQuestionResponse {
                question: self.render_one(&question, Some(owner)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_question(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteQuestionRequest>,
    ) -> ServiceResult<DeleteQuestionResponse> {
        handle(async {
            self.remove_question(ctx.user_id()?, parse_id(request.question_id)?).await?;
            ok(DeleteQuestionResponse::default())
        })
        .await
    }

    async fn answer_question(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AnswerQuestionRequest>,
    ) -> ServiceResult<AnswerQuestionResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            let question = self
                .write_answer(
                    owner,
                    parse_id(request.question_id)?,
                    request.text,
                    request.image_asset_id,
                    request.stamp_id,
                    request.audience,
                )
                .await?;
            ok(AnswerQuestionResponse {
                question: self.render_one(&question, Some(owner)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_answer(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateAnswerRequest>,
    ) -> ServiceResult<UpdateAnswerResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            let question = self.edit_answer(owner, &request.to_owned_message()).await?;
            ok(UpdateAnswerResponse {
                question: self.render_one(&question, Some(owner)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_answer(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteAnswerRequest>,
    ) -> ServiceResult<DeleteAnswerResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            let question = self.clear_answer(owner, parse_id(request.question_id)?).await?;
            ok(DeleteAnswerResponse {
                question: self.render_one(&question, Some(owner)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_answers(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListAnswersRequest>,
    ) -> ServiceResult<ListAnswersResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let owner = self.box_owner(request.user, viewer).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let audiences = self.page_audiences(owner.id, viewer).await?;
            let (rows, info) = self.answers_where_owner(owner.id, viewer, audiences, &page).await?;
            let questions = self.renderer(&rows, viewer).await?.render_all(&rows);
            ok(ListAnswersResponse { questions, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_question(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetQuestionRequest>,
    ) -> ServiceResult<GetQuestionResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let question = self.readable_question(parse_id(request.question_id)?, viewer).await?;
            ok(GetQuestionResponse { question: self.render_one(&question, viewer).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn block_sender(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BlockSenderRequest>,
    ) -> ServiceResult<BlockSenderResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            let target = match request.to_owned_message().target {
                Some(Target::QuestionId(id)) => BlockTarget::Question(parse_id(&id)?),
                Some(Target::PromptResponseId(id)) => BlockTarget::PromptResponse(parse_id(&id)?),
                Some(Target::FlameId(id)) => BlockTarget::Flame(parse_id(&id)?),
                None => bail!(
                    invalid,
                    "BLOCK_TARGET_REQUIRED",
                    "name the question, response or flame whose sender to block"
                ),
            };
            self.core
                .cache
                .rate_limit(&format!("ask:block:{owner}"), BLOCKS_PER_HOUR, Duration::from_secs(3600))
                .await?;
            let block = self.block_sender_of(owner, target).await?;
            ok(BlockSenderResponse { block: block.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn list_sender_blocks(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListSenderBlocksRequest>,
    ) -> ServiceResult<ListSenderBlocksResponse> {
        handle(async {
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, info) = self.sender_blocks_page(ctx.user_id()?, &page).await?;
            ok(ListSenderBlocksResponse {
                blocks: rows.iter().map(|block| block.to_proto()).collect(),
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn unblock_sender(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnblockSenderRequest>,
    ) -> ServiceResult<UnblockSenderResponse> {
        handle(async {
            self.lift_block(ctx.user_id()?, parse_id(request.block_id)?).await?;
            ok(UnblockSenderResponse::default())
        })
        .await
    }
}
