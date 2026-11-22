//! `AskModerationService`: who sent what, for staff acting on a report, and
//! the poll question bank.
//!
//! This is the only RPC surface that returns the sender of something sent
//! anonymously. Two rules hold for every lookup here:
//!
//! - the role check is the first thing the handler does;
//! - the lookup is written to `ask_inspections` in the same transaction as
//!   the read, before anything is returned. If the trace cannot be written,
//!   nothing is revealed.
//!
//! The row types below carry the raw sender ID and are private to this
//! module, so no other code can come by them.
use chrono::{DateTime, Utc};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_media::assets::image_from_json;
use tokuly_proto::{ask::*, common::UserSummary};

use crate::{Ask, polls::bank::poll_question, questions::MAX_PAGE};

const MODERATOR: &str = "moderator";
const ADMIN: &str = "admin";

/// What every anonymous item has in common, as staff see it.
struct Parties {
    sender_id: Option<Uuid>,
    recipient_id: Uuid,
    signed: bool,
    dropped: bool,
    sender_banned: bool,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct InspectedQuestion {
    sender_id: Option<Uuid>,
    owner_id: Uuid,
    signed: bool,
    dropped: bool,
    sender_banned: bool,
    created_at: DateTime<Utc>,
    text: String,
    image: Option<serde_json::Value>,
    folder: i16,
    removed_at: Option<DateTime<Utc>>,
    answer_text: String,
    answer_image: Option<serde_json::Value>,
    answer_stamp_id: Option<Uuid>,
    answer_audience: i16,
    answered_at: Option<DateTime<Utc>>,
    answer_edited_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct InspectedResponse {
    sender_id: Option<Uuid>,
    owner_id: Uuid,
    signed: bool,
    dropped: bool,
    sender_banned: bool,
    created_at: DateTime<Utc>,
    text: String,
    prompt_id: Uuid,
    prompt_text: String,
    published: bool,
    held: bool,
    removed_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct InspectedFlame {
    sender_id: Option<Uuid>,
    receiver_id: Uuid,
    dropped: bool,
    sender_banned: bool,
    created_at: DateTime<Utc>,
    question_id: Uuid,
    text_ja: String,
    text_en: String,
    emoji: String,
    hints_bought: i64,
}

/// `EXISTS` clause telling whether the sender's account is banned; `x` is
/// the table alias of the inspected row.
macro_rules! sender_banned {
    ($alias:literal) => {
        concat!(
            "EXISTS (SELECT 1 FROM users banned WHERE banned.id = ",
            $alias,
            ".sender_id AND banned.banned_at IS NOT NULL) AS sender_banned"
        )
    };
}

impl Ask {
    /// Leaves the trace of a lookup. Runs on the transaction of the read.
    async fn record_inspection(
        tx: &mut Transaction<'_, Postgres>,
        staff: Uuid,
        kind: &str,
        target: Uuid,
    ) -> AppResult<()> {
        sqlx::query("INSERT INTO ask_inspections (staff_id, kind, target_id) VALUES ($1, $2, $3)")
            .bind(staff)
            .bind(kind)
            .bind(target)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    async fn sender_record(&self, parties: Parties) -> AppResult<SenderRecord> {
        let ids: Vec<Uuid> = parties.sender_id.into_iter().chain([parties.recipient_id]).collect();
        let mut users = tokuly_identity::summaries(&self.core, &ids).await?;
        let sender: Option<UserSummary> = parties.sender_id.and_then(|id| users.remove(&id));
        Ok(SenderRecord {
            sender: sender.into(),
            recipient: users.remove(&parties.recipient_id).into(),
            signed: parties.signed,
            dropped: parties.dropped,
            sender_banned: parties.sender_banned,
            created_at: ts(parties.created_at),
            ..Default::default()
        })
    }
}

impl AskModerationService for Ask {
    async fn inspect_question(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, InspectQuestionRequest>,
    ) -> ServiceResult<InspectQuestionResponse> {
        handle(async {
            let staff = ctx.require_role(MODERATOR)?.user_id;
            let question_id = parse_id(request.question_id)?;
            let mut tx = self.core.db.begin().await?;
            // Removed and dropped questions are found too: a report may be
            // about something its recipient has since deleted.
            let row: Option<InspectedQuestion> = sqlx::query_as(concat!(
                "SELECT q.sender_id, q.owner_id, q.signed, q.dropped, ",
                sender_banned!("q"),
                ", q.created_at, q.text, q.image, q.folder, q.removed_at, q.answer_text, q.answer_image, q.answer_stamp_id,
                   q.answer_audience, q.answered_at, q.answer_edited_at
                 FROM ask_questions q WHERE q.id = $1"
            ))
            .bind(question_id)
            .fetch_optional(&mut *tx)
            .await?;
            let row = row.or_not_found("QUESTION_NOT_FOUND", "question not found")?;
            Self::record_inspection(&mut tx, staff, "question", question_id).await?;
            tx.commit().await?;

            let answer = match row.answered_at {
                Some(answered_at) => {
                    let stamp = match row.answer_stamp_id {
                        Some(stamp_id) => tokuly_stamp::stamp_refs(&self.core, &[stamp_id]).await?.remove(&stamp_id),
                        None => None,
                    };
                    Some(Answer {
                        text: row.answer_text.clone(),
                        image: image_from_json(row.answer_image.as_ref()).into(),
                        stamp: stamp.into(),
                        audience: i32::from(row.answer_audience).into(),
                        answered_at: ts(answered_at),
                        edited_at: opt_ts(row.answer_edited_at),
                        ..Default::default()
                    })
                }
                None => None,
            };
            let parties = Parties {
                sender_id: row.sender_id,
                recipient_id: row.owner_id,
                signed: row.signed,
                dropped: row.dropped,
                sender_banned: row.sender_banned,
                created_at: row.created_at,
            };
            ok(InspectQuestionResponse {
                record: self.sender_record(parties).await?.into(),
                text: row.text,
                image: image_from_json(row.image.as_ref()).into(),
                folder: i32::from(row.folder).into(),
                answer: answer.into(),
                removed_at: opt_ts(row.removed_at),
                ..Default::default()
            })
        })
        .await
    }

    async fn inspect_prompt_response(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, InspectPromptResponseRequest>,
    ) -> ServiceResult<InspectPromptResponseResponse> {
        handle(async {
            let staff = ctx.require_role(MODERATOR)?.user_id;
            let response_id = parse_id(request.response_id)?;
            let mut tx = self.core.db.begin().await?;
            let row: Option<InspectedResponse> = sqlx::query_as(concat!(
                "SELECT r.sender_id, r.owner_id, r.signed, r.dropped, ",
                sender_banned!("r"),
                ", r.created_at, r.text, r.prompt_id, p.text AS prompt_text, (r.published_at IS NOT NULL) AS published, r.held, r.removed_at
                 FROM ask_prompt_responses r JOIN ask_prompts p ON p.id = r.prompt_id WHERE r.id = $1"
            ))
            .bind(response_id)
            .fetch_optional(&mut *tx)
            .await?;
            let row = row.or_not_found("RESPONSE_NOT_FOUND", "response not found")?;
            Self::record_inspection(&mut tx, staff, "prompt_response", response_id).await?;
            tx.commit().await?;

            let parties = Parties {
                sender_id: row.sender_id,
                recipient_id: row.owner_id,
                signed: row.signed,
                dropped: row.dropped,
                sender_banned: row.sender_banned,
                created_at: row.created_at,
            };
            ok(InspectPromptResponseResponse {
                record: self.sender_record(parties).await?.into(),
                text: row.text,
                prompt_id: row.prompt_id.to_string(),
                prompt_text: row.prompt_text,
                published: row.published,
                held: row.held,
                removed_at: opt_ts(row.removed_at),
                ..Default::default()
            })
        })
        .await
    }

    async fn inspect_flame(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, InspectFlameRequest>,
    ) -> ServiceResult<InspectFlameResponse> {
        handle(async {
            let staff = ctx.require_role(MODERATOR)?.user_id;
            let flame_id = parse_id(request.flame_id)?;
            let mut tx = self.core.db.begin().await?;
            let row: Option<InspectedFlame> = sqlx::query_as(concat!(
                "SELECT f.sender_id, f.receiver_id, f.dropped, ",
                sender_banned!("f"),
                ", f.created_at, f.question_id, p.text_ja, p.text_en, p.emoji,
                   (SELECT count(*) FROM ask_flame_hints h WHERE h.flame_id = f.id AND h.entry_id IS NOT NULL) AS hints_bought
                 FROM ask_flames f JOIN ask_poll_questions p ON p.id = f.question_id WHERE f.id = $1"
            ))
            .bind(flame_id)
            .fetch_optional(&mut *tx)
            .await?;
            let row = row.or_not_found("FLAME_NOT_FOUND", "flame not found")?;
            Self::record_inspection(&mut tx, staff, "flame", flame_id).await?;
            tx.commit().await?;

            let parties = Parties {
                sender_id: row.sender_id,
                recipient_id: row.receiver_id,
                // A vote is never signed.
                signed: false,
                dropped: row.dropped,
                sender_banned: row.sender_banned,
                created_at: row.created_at,
            };
            ok(InspectFlameResponse {
                record: self.sender_record(parties).await?.into(),
                question: poll_question(row.question_id, &row.text_ja, &row.text_en, &row.emoji).into(),
                hints_bought: row.hints_bought as i32,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_poll_questions(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListPollQuestionsRequest>,
    ) -> ServiceResult<ListPollQuestionsResponse> {
        handle(async {
            ctx.require_role(MODERATOR)?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, info) = self.bank_page(&page).await?;
            ok(ListPollQuestionsResponse {
                questions: rows.iter().map(|row| row.to_entry()).collect(),
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn add_poll_question(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddPollQuestionRequest>,
    ) -> ServiceResult<AddPollQuestionResponse> {
        handle(async {
            let staff = ctx.require_role(ADMIN)?.user_id;
            let request = request.to_owned_message();
            let text = request.text.as_option().cloned().unwrap_or_default();
            let question = self.add_bank_question(staff, &text.ja, &text.en, &request.emoji).await?;
            tracing::info!(%staff, question_id = %question.id, "poll question added");
            ok(AddPollQuestionResponse { question: question.to_entry().into(), ..Default::default() })
        })
        .await
    }

    async fn set_poll_question_enabled(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetPollQuestionEnabledRequest>,
    ) -> ServiceResult<SetPollQuestionEnabledResponse> {
        handle(async {
            let staff = ctx.require_role(ADMIN)?.user_id;
            let question = self.set_bank_question_enabled(parse_id(request.question_id)?, request.enabled).await?;
            tracing::info!(%staff, question_id = %question.id, enabled = question.enabled, "poll question switched");
            ok(SetPollQuestionEnabledResponse { question: question.to_entry().into(), ..Default::default() })
        })
        .await
    }
}
