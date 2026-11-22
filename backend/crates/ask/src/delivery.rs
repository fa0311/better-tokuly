//! Delivery: letting what was sent through to its recipient, and telling
//! them about it.
//!
//! Sending a question, a prompt response or a compliment vote only stores it
//! and one `ask.*.stored` event; so does a request from a blocked sender.
//! This worker picks the event up and deals with the item, once
//! (`delivered_at`). Until then the recipient does not have it: their lists
//! show only what was delivered.
//!
//! That makes this the one place where the sender's account matters. An
//! item whose sender is suspended when the worker gets to it is stopped
//! (stored as dropped, like one from a blocked sender). Everything that
//! was delivered stays with its recipient whatever becomes of the sender
//! afterwards, so nothing the recipient already has can appear or disappear
//! with someone's account.
//!
//! Whether the recipient is told (a notification and a realtime push) is
//! decided here too, after the sender's request is over, which is why the
//! sender's request cannot depend on it: nothing for an item that was
//! dropped or held by the word filter.
//!
//! Notifications about anonymous items carry no actor and no name. Nothing
//! about them may depend on who the sender is either: applying the
//! recipient's per-friend mute, for instance, would let the recipient find
//! the sender out by muting friends one at a time.
//!
//! Like every worker here, these read what they need through the pool
//! first and then use nothing but their transaction until it is committed.
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::ask::{
    FlameReceived, PromptResponseReceived, QuestionAnswered, QuestionReceived, ask_event::Event as Push,
};
use tokuly_social::graph;

use crate::{
    Ask,
    events::{AnswerPublished, PromptOpened},
    polls::flames::{FlameRow, flame_columns, in_inbox},
    prompts::{Access, ResponseRow, prompt_path, response_columns, response_in_box},
    questions::{QuestionRow, in_box, question_columns, question_path},
    senders::sender_suspended,
    text,
};

/// Notification kinds of this domain.
pub(crate) const QUESTION_KIND: &str = "ask.question";
pub(crate) const ANSWERED_KIND: &str = "ask.answered";
pub(crate) const PROMPT_KIND: &str = "ask.prompt";
pub(crate) const PROMPT_RESPONSE_KIND: &str = "ask.prompt_response";
pub(crate) const FLAME_KIND: &str = "ask.flame";

/// Where the owner reads new questions and flames.
const INBOX_PATH: &str = "/ask/inbox";
const FLAMES_PATH: &str = "/ask/flames";
/// Characters of someone's text quoted in a notification.
const QUOTE_CHARS: usize = 60;

#[derive(sqlx::FromRow)]
struct ArrivedFlame {
    receiver_id: Uuid,
    #[sqlx(flatten)]
    flame: FlameRow,
}

/// A user's name as notifications show it.
async fn name_of(core: &Core, user_id: Uuid) -> AppResult<String> {
    let user = tokuly_identity::summary(core, user_id).await?;
    Ok(if user.display_name.is_empty() { user.handle } else { user.display_name })
}

impl Ask {
    /// `ask.question.stored`: delivers the question, and tells the owner of
    /// the box if it reached their inbox.
    pub(crate) async fn deliver_question(&self, question_id: Uuid) -> AppResult<()> {
        let waiting: Option<Option<Uuid>> = sqlx::query_scalar(
            "SELECT CASE WHEN q.signed THEN q.sender_id END FROM ask_questions q WHERE q.id = $1 AND q.delivered_at IS NULL",
        )
        .bind(question_id)
        .fetch_optional(&self.core.db)
        .await?;
        let Some(asker) = waiting else { return Ok(()) };
        let asker_name = match asker {
            Some(asker) => Some(name_of(&self.core, asker).await?),
            None => None,
        };

        let mut tx = self.core.db.begin().await?;
        let delivered = sqlx::query(concat!(
            "UPDATE ask_questions q SET delivered_at = now(), dropped = q.dropped OR ",
            sender_suspended!("q"),
            " WHERE q.id = $1 AND q.delivered_at IS NULL"
        ))
        .bind(question_id)
        .execute(&mut *tx)
        .await?;
        if delivered.rows_affected() == 0 {
            return Ok(());
        }
        // The owner is told about a question in the inbox folder of the box:
        // not one that was filtered, dropped or withdrawn meanwhile.
        let arrived: Option<QuestionRow> = sqlx::query_as(concat!(
            "SELECT ",
            question_columns!(),
            " FROM ask_questions q WHERE q.id = $2 AND q.folder = 1 AND ",
            in_box!()
        ))
        .bind(None::<Uuid>)
        .bind(question_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(question) = arrived else {
            tx.commit().await?;
            return Ok(());
        };
        let owner = question.owner_id;
        let title = Localized::new("質問箱に新しい質問が届いたよ!", "A new question in your box!");
        // The text itself is not quoted: what an anonymous stranger wrote
        // does not belong on a lock screen.
        let notice = match (question.asker_id.filter(|_| question.signed), asker_name) {
            (Some(asker), Some(name)) => {
                let body = Localized::new(
                    format!("{name}さんからの質問だよ。ひらいて答えてみよう!"),
                    format!("{name} asked you something. Open it and answer!"),
                );
                NotificationRequest::new(owner, QUESTION_KIND, title, body, INBOX_PATH).actor(asker)
            }
            _ => {
                let body = Localized::new(
                    "匿名の質問だよ。ひらいて答えてみよう!",
                    "Someone asked you something anonymously. Open it and answer!",
                );
                NotificationRequest::new(owner, QUESTION_KIND, title, body, INBOX_PATH)
            }
        };
        let notice =
            notice.collapse(format!("{QUESTION_KIND}:{owner}")).data(serde_json::json!({ "question_id": question.id }));
        outbound::notify(&mut *tx, &notice).await?;
        tx.commit().await?;

        let rendered = self.render_one(&question, Some(owner)).await?;
        let counts = self.inbox_counts(owner).await?;
        let pushed = QuestionReceived { question: rendered.into(), counts: counts.into(), ..Default::default() };
        self.push(owner, Push::QuestionReceived(Box::new(pushed))).await;
        Ok(())
    }

    /// `ask.response.stored`: delivers the response, and tells the owner of
    /// the prompt if it reached them.
    pub(crate) async fn deliver_response(&self, response_id: Uuid) -> AppResult<()> {
        let waiting: Option<(Option<Uuid>, Option<String>)> = sqlx::query_as(
            "SELECT CASE WHEN r.signed THEN r.sender_id END,
                    (SELECT p.text FROM ask_prompts p WHERE p.id = r.prompt_id AND p.removed_at IS NULL)
             FROM ask_prompt_responses r WHERE r.id = $1 AND r.delivered_at IS NULL",
        )
        .bind(response_id)
        .fetch_optional(&self.core.db)
        .await?;
        let Some((responder, prompt_text)) = waiting else { return Ok(()) };
        let responder_name = match responder {
            Some(responder) => Some(name_of(&self.core, responder).await?),
            None => None,
        };

        let mut tx = self.core.db.begin().await?;
        let delivered = sqlx::query(concat!(
            "UPDATE ask_prompt_responses r SET delivered_at = now(), dropped = r.dropped OR ",
            sender_suspended!("r"),
            " WHERE r.id = $1 AND r.delivered_at IS NULL"
        ))
        .bind(response_id)
        .execute(&mut *tx)
        .await?;
        if delivered.rows_affected() == 0 {
            return Ok(());
        }
        let arrived: Option<ResponseRow> = sqlx::query_as(concat!(
            "SELECT ",
            response_columns!(),
            " FROM ask_prompt_responses r WHERE r.id = $2 AND NOT r.held AND ",
            response_in_box!(),
            " AND EXISTS (SELECT 1 FROM ask_prompts p WHERE p.id = r.prompt_id AND p.removed_at IS NULL)"
        ))
        .bind(None::<Uuid>)
        .bind(response_id)
        .fetch_optional(&mut *tx)
        .await?;
        let (Some(response), Some(prompt_text)) = (arrived, prompt_text) else {
            tx.commit().await?;
            return Ok(());
        };
        let owner = response.owner_id;
        let title = Localized::new("お題に回答が届いたよ!", "A new response to your prompt!");
        let quoted = text::excerpt(&prompt_text, QUOTE_CHARS);
        let path = prompt_path(response.prompt_id);
        let notice = match (response.responder_id.filter(|_| response.signed), responder_name) {
            (Some(responder), Some(name)) => {
                let body = Localized::new(
                    format!("{name}さんが「{quoted}」に答えたよ"),
                    format!("{name} responded to \u{201c}{quoted}\u{201d}"),
                );
                NotificationRequest::new(owner, PROMPT_RESPONSE_KIND, title, body, path).actor(responder)
            }
            _ => {
                let body = Localized::new(
                    format!("「{quoted}」に匿名の回答が届いたよ"),
                    format!("An anonymous response to \u{201c}{quoted}\u{201d}"),
                );
                NotificationRequest::new(owner, PROMPT_RESPONSE_KIND, title, body, path)
            }
        };
        let notice = notice
            .collapse(format!("{PROMPT_RESPONSE_KIND}:{}", response.prompt_id))
            .data(serde_json::json!({ "prompt_id": response.prompt_id, "response_id": response.id }));
        outbound::notify(&mut *tx, &notice).await?;
        tx.commit().await?;

        // The prompt, now counting this response.
        let prompt = self.owned_prompt(owner, response.prompt_id).await?;
        let pushed = PromptResponseReceived {
            response: self.render_response(&response, owner).await?.into(),
            prompt: self.render_prompt(&prompt, owner, Access::default()).await?.into(),
            ..Default::default()
        };
        self.push(owner, Push::PromptResponseReceived(Box::new(pushed))).await;
        Ok(())
    }

    /// `ask.flame.stored`: delivers the flame and tells whoever was picked,
    /// if it reached them. Never says who picked them.
    pub(crate) async fn deliver_flame(&self, flame_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let delivered = sqlx::query(concat!(
            "UPDATE ask_flames f SET delivered_at = now(), dropped = f.dropped OR ",
            sender_suspended!("f"),
            " WHERE f.id = $1 AND f.delivered_at IS NULL"
        ))
        .bind(flame_id)
        .execute(&mut *tx)
        .await?;
        if delivered.rows_affected() == 0 {
            return Ok(());
        }
        let arrived: Option<ArrivedFlame> = sqlx::query_as(concat!(
            "SELECT f.receiver_id, ",
            flame_columns!(),
            " FROM ask_flames f JOIN ask_poll_questions p ON p.id = f.question_id WHERE f.id = $1 AND ",
            in_inbox!()
        ))
        .bind(flame_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(ArrivedFlame { receiver_id: receiver, flame }) = arrived else {
            tx.commit().await?;
            return Ok(());
        };
        let notice = NotificationRequest::new(
            receiver,
            FLAME_KIND,
            Localized::new("🔥 フレイムが届いたよ!", "🔥 You got a flame!"),
            Localized::new(
                format!("「{}」にえらばれたよ!", flame.text_ja),
                format!("Someone picked you for \u{201c}{}\u{201d}", flame.text_en),
            ),
            FLAMES_PATH,
        )
        .collapse(format!("{FLAME_KIND}:{receiver}"))
        .data(serde_json::json!({ "flame_id": flame.id }));
        outbound::notify(&mut *tx, &notice).await?;
        tx.commit().await?;

        let mut rendered = self.render_flames(std::slice::from_ref(&flame)).await?;
        let pushed = FlameReceived {
            flame: rendered.pop().into(),
            unseen: self.unseen_flames(receiver).await?,
            ..Default::default()
        };
        self.push(receiver, Push::FlameReceived(Box::new(pushed))).await;
        Ok(())
    }

    /// `ask.answer.published`: tells the asker that their question was
    /// answered. Only the first publication of an answer counts; a later
    /// change of audience is not news.
    ///
    /// This runs after the owner's request is over and its result goes to
    /// the asker alone, so reading the sender here shows the owner nothing.
    pub(crate) async fn tell_asker(&self, event: AnswerPublished) -> AppResult<()> {
        if !event.first {
            return Ok(());
        }
        let asker: Option<Uuid> = sqlx::query_scalar(concat!(
            "SELECT q.sender_id FROM ask_questions q WHERE q.id = $1 AND q.folder = 2 AND ",
            in_box!()
        ))
        .bind(event.question_id)
        .fetch_optional(&self.core.db)
        .await?
        .flatten();
        let Some(asker) = asker else { return Ok(()) };
        // The question as its sender sees it; gone from their list (or the
        // two blocked each other meanwhile) means there is nobody to tell.
        let question = match self.sent_question(asker, event.question_id).await {
            Ok(question) if question.is_answered() => question,
            Ok(_) => return Ok(()),
            Err(e) if e.is_not_found() => return Ok(()),
            Err(e) => return Err(e),
        };
        if graph::is_blocked_either(&self.core, asker, question.owner_id).await? {
            return Ok(());
        }
        let owner_name = name_of(&self.core, question.owner_id).await?;
        let quoted = if question.answer_text.is_empty() {
            Localized::new("ひらいて見てみよう!", "Open it to have a look!")
        } else {
            Localized::same(text::excerpt(&question.answer_text, QUOTE_CHARS))
        };
        let notice = NotificationRequest::new(
            asker,
            ANSWERED_KIND,
            Localized::new(
                format!("{owner_name}さんが質問に答えたよ!"),
                format!("{owner_name} answered your question!"),
            ),
            quoted,
            question_path(question.id),
        )
        .actor(question.owner_id)
        .collapse(format!("{ANSWERED_KIND}:{}", question.id))
        .data(serde_json::json!({ "question_id": question.id }));
        outbound::notify(&self.core.db, &notice).await?;

        let rendered = self.render_one(&question, Some(asker)).await?;
        self.push(
            asker,
            Push::QuestionAnswered(Box::new(QuestionAnswered { question: rendered.into(), ..Default::default() })),
        )
        .await;
        Ok(())
    }

    /// `ask.prompt.opened`: tells the owner's friends, except those who
    /// muted the owner.
    pub(crate) async fn tell_friends(&self, event: PromptOpened) -> AppResult<()> {
        let owner = event.owner_id;
        let open: Option<String> = sqlx::query_scalar(
            "SELECT text FROM ask_prompts WHERE id = $1 AND owner_id = $2 AND removed_at IS NULL AND closed_at IS NULL AND closes_at > now()",
        )
        .bind(event.prompt_id)
        .bind(owner)
        .fetch_optional(&self.core.db)
        .await?;
        let Some(prompt_text) = open else { return Ok(()) };
        let friends = graph::friend_ids(&self.core, owner).await?;
        let muted_by = self.muted_by(owner, &friends).await?;
        let recipients: Vec<Uuid> = tokuly_identity::users::filter_active(&self.core, &friends)
            .await?
            .into_iter()
            .filter(|friend| !muted_by.contains(friend))
            .collect();
        if recipients.is_empty() {
            return Ok(());
        }
        let owner_name = name_of(&self.core, owner).await?;
        let title =
            Localized::new(format!("{owner_name}さんがお題を出したよ!"), format!("{owner_name} posted a prompt!"));
        let body = Localized::same(text::excerpt(&prompt_text, QUOTE_CHARS));
        let mut tx = self.core.db.begin().await?;
        for friend in recipients {
            let notice = NotificationRequest::new(
                friend,
                PROMPT_KIND,
                title.clone(),
                body.clone(),
                prompt_path(event.prompt_id),
            )
            .actor(owner)
            .collapse(format!("{PROMPT_KIND}:{owner}"))
            .data(serde_json::json!({ "prompt_id": event.prompt_id }));
            outbound::notify(&mut *tx, &notice).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
