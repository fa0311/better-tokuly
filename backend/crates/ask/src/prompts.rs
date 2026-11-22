//! Prompts (お題): the owner asks their friends something, friends respond
//! anonymously or signed, and the owner publishes the responses they like on
//! the prompt's page.
//!
//! A prompt is for the owner's friends: they are told when it opens, see it
//! among their friends' prompts, respond while it is open, and read what the
//! owner published. Nobody else sees it.
//!
//! Responses follow the same anonymity rules as questions (see
//! `senders.rs`): the responder's ID is selected only for signed responses,
//! a blocked responder's response is stored and dropped, and the responder
//! sees nothing of what became of it except whether it is on the page —
//! which they could read there anyway.
use chrono::{DateTime, Duration, Utc};
use tokuly_core::{events, prelude::*};
use tokuly_proto::{
    ask::{Prompt, PromptResponse, ResponseFilter},
    common::{PageInfo, UserSummary},
};
use tokuly_social::graph;

use crate::{
    Ask,
    events::{PROMPT_OPENED, PromptOpened, RESPONSE_STORED, Stored},
    questions::{REMOVED_BY_MODERATION, REMOVED_BY_OWNER},
    text,
};

/// How long a prompt stays open when the owner does not say.
const DEFAULT_DURATION_SECONDS: i64 = 24 * 3600;
const MIN_DURATION_SECONDS: i64 = 10 * 60;
const MAX_DURATION_SECONDS: i64 = 7 * 24 * 3600;
/// Prompts one owner may have open at once: each one notifies every friend.
const MAX_OPEN_PROMPTS: i64 = 3;
/// Prompts one owner may post per day.
const PROMPTS_PER_DAY: u32 = 10;

pub(crate) fn prompt_path(prompt_id: Uuid) -> String {
    format!("/ask/prompts/{prompt_id}")
}

/// SQL condition for "this response is in the owner's view": not deleted or
/// taken down, not dropped, and not sent by a banned account.
macro_rules! response_in_box {
    () => {
        "r.removed_at IS NULL AND NOT r.dropped \
         AND NOT EXISTS (SELECT 1 FROM users banned WHERE banned.id = r.sender_id AND banned.banned_at IS NOT NULL)"
    };
}
pub(crate) use response_in_box;

/// Columns of [`PromptRow`], from `ask_prompts p`. `$1` must be bound to the
/// viewer (NULL for guests). The owner-only counters are computed for
/// everyone and shown by [`PromptRow::to_proto`] to the owner alone.
macro_rules! prompt_columns {
    () => {
        concat!(
            "p.id, p.owner_id, p.text, p.emoji, p.created_at, p.closes_at, \
             (p.closed_at IS NULL AND p.closes_at > now()) AS open, \
             (SELECT count(*) FROM ask_prompt_responses r WHERE r.prompt_id = p.id AND r.published_at IS NOT NULL AND NOT r.held AND ",
            response_in_box!(),
            ") AS published_count, \
             (SELECT count(*) FROM ask_prompt_responses r WHERE r.prompt_id = p.id AND NOT r.held AND ",
            response_in_box!(),
            ") AS response_count, \
             (SELECT count(*) FROM ask_prompt_responses r WHERE r.prompt_id = p.id AND r.seen_at IS NULL AND NOT r.held AND ",
            response_in_box!(),
            ") AS unseen_count, \
             (SELECT count(*) FROM ask_prompt_responses r WHERE r.prompt_id = p.id AND r.held AND ",
            response_in_box!(),
            ") AS held_count, \
             EXISTS (SELECT 1 FROM ask_prompt_responses r WHERE r.prompt_id = p.id AND r.sender_id = $1) AS responded"
        )
    };
}

/// Columns of [`ResponseRow`], from `ask_prompt_responses r`. `$1` must be
/// bound to the viewer.
macro_rules! response_columns {
    () => {
        "r.id, r.prompt_id, r.owner_id, CASE WHEN r.signed THEN r.sender_id END AS responder_id, r.signed, r.text, \
         r.held, r.seen_at, r.published_at, r.created_at, (r.removed_at IS NULL AND NOT r.dropped) AS live, \
         COALESCE(r.sender_id = $1, false) AS sent_by_viewer"
    };
}
pub(crate) use response_columns;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct PromptRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub text: String,
    pub emoji: String,
    pub created_at: DateTime<Utc>,
    pub closes_at: DateTime<Utc>,
    pub open: bool,
    pub published_count: i64,
    pub response_count: i64,
    pub unseen_count: i64,
    pub held_count: i64,
    /// Whether the viewer the query was made for responded.
    pub responded: bool,
}

impl PromptRow {
    /// The prompt as `viewer` sees it. `viewer_is_friend` only matters for
    /// `can_respond`.
    pub(crate) fn to_proto(&self, viewer: Option<Uuid>, owner: UserSummary, viewer_is_friend: bool) -> Prompt {
        let is_owner = viewer == Some(self.owner_id);
        Prompt {
            id: self.id.to_string(),
            owner: owner.into(),
            text: self.text.clone(),
            emoji: self.emoji.clone(),
            created_at: ts(self.created_at),
            closes_at: ts(self.closes_at),
            open: self.open,
            published_count: self.published_count as i32,
            response_count: if is_owner { self.response_count as i32 } else { 0 },
            unseen_count: if is_owner { self.unseen_count as i32 } else { 0 },
            held_count: if is_owner { self.held_count as i32 } else { 0 },
            responded: !is_owner && self.responded,
            can_respond: !is_owner && viewer_is_friend && self.open && !self.responded,
            path: prompt_path(self.id),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ResponseRow {
    pub id: Uuid,
    pub prompt_id: Uuid,
    pub owner_id: Uuid,
    /// The responder, on signed responses only.
    pub responder_id: Option<Uuid>,
    pub signed: bool,
    pub text: String,
    pub held: bool,
    pub seen_at: Option<DateTime<Utc>>,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// In the owner's view: not removed and not dropped.
    pub live: bool,
    pub sent_by_viewer: bool,
}

impl ResponseRow {
    /// On the prompt's page.
    fn is_published(&self) -> bool {
        self.live && !self.held && self.published_at.is_some()
    }

    /// The response as `viewer` sees it. `responder` is the signed
    /// responder's summary, already withheld by the caller where a block
    /// between the viewer and the responder hides the name.
    pub(crate) fn to_proto(&self, viewer: Option<Uuid>, responder: Option<UserSummary>) -> PromptResponse {
        let is_owner = viewer == Some(self.owner_id);
        let published = self.is_published();
        PromptResponse {
            id: self.id.to_string(),
            prompt_id: self.prompt_id.to_string(),
            text: self.text.clone(),
            anonymous: !self.signed,
            responder: responder.filter(|_| self.signed).into(),
            created_at: ts(self.created_at),
            published,
            published_at: opt_ts(self.published_at.filter(|_| published)),
            // Whether it was held or seen is the owner's business alone.
            held: is_owner && self.held,
            seen: is_owner && self.seen_at.is_some(),
            sent_by_me: self.sent_by_viewer && !is_owner,
            ..Default::default()
        }
    }
}

/// Which of a prompt's responses its owner lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Filter {
    Received,
    Published,
    Held,
}

impl Filter {
    pub(crate) fn of(value: buffa::EnumValue<ResponseFilter>) -> AppResult<Self> {
        match value.as_known() {
            Some(ResponseFilter::RESPONSE_FILTER_UNSPECIFIED | ResponseFilter::RESPONSE_FILTER_RECEIVED) => {
                Ok(Self::Received)
            }
            Some(ResponseFilter::RESPONSE_FILTER_PUBLISHED) => Ok(Self::Published),
            Some(ResponseFilter::RESPONSE_FILTER_HELD) => Ok(Self::Held),
            None => Err(AppError::invalid("RESPONSE_FILTER_INVALID", "unknown response filter")),
        }
    }
}

/// How long a new prompt stays open, from the request's `duration_seconds`.
pub(crate) fn duration_of(seconds: i32) -> AppResult<Duration> {
    let seconds = if seconds == 0 { DEFAULT_DURATION_SECONDS } else { i64::from(seconds) };
    ensure!(
        (MIN_DURATION_SECONDS..=MAX_DURATION_SECONDS).contains(&seconds),
        invalid,
        "PROMPT_DURATION_INVALID",
        "a prompt stays open between 10 minutes and 7 days"
    );
    Ok(Duration::seconds(seconds))
}

fn prompt_not_found() -> AppError {
    AppError::not_found("PROMPT_NOT_FOUND", "prompt not found")
}

fn response_not_found() -> AppError {
    AppError::not_found("RESPONSE_NOT_FOUND", "response not found")
}

impl Ask {
    /// Posts a prompt to `owner`'s friends. Friends are told by the worker,
    /// from the `ask.prompt.opened` event.
    pub(crate) async fn open_prompt(
        &self,
        owner: Uuid,
        text: &str,
        emoji: &str,
        duration_seconds: i32,
    ) -> AppResult<PromptRow> {
        let text = text::single_line(&text::required(text, text::PROMPT_MAX, "PROMPT_TEXT_INVALID", "the prompt")?);
        let emoji = text::emoji(emoji, "PROMPT_TEXT_INVALID")?;
        let duration = duration_of(duration_seconds)?;
        self.core
            .cache
            .rate_limit(&format!("ask:prompt:{owner}"), PROMPTS_PER_DAY, std::time::Duration::from_secs(86_400))
            .await?;

        let mut tx = self.core.db.begin().await?;
        // Serializes one owner's prompts so the open-prompt cap holds.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("ask:prompts:{owner}"))
            .execute(&mut *tx)
            .await?;
        let open: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM ask_prompts WHERE owner_id = $1 AND removed_at IS NULL AND closed_at IS NULL AND closes_at > now()",
        )
        .bind(owner)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(
            open < MAX_OPEN_PROMPTS,
            precondition,
            "TOO_MANY_OPEN_PROMPTS",
            "close a prompt before posting another (at most {MAX_OPEN_PROMPTS} open)"
        );
        let closes_at = Utc::now() + duration;
        let prompt: PromptRow = sqlx::query_as(concat!(
            "WITH p AS (INSERT INTO ask_prompts (owner_id, text, emoji, closes_at) VALUES ($1, $2, $3, $4) RETURNING *)
             SELECT ",
            prompt_columns!(),
            " FROM p"
        ))
        .bind(owner)
        .bind(&text)
        .bind(&emoji)
        .bind(closes_at)
        .fetch_one(&mut *tx)
        .await?;
        events::emit(
            &mut *tx,
            PROMPT_OPENED,
            Some(owner),
            &PromptOpened { prompt_id: prompt.id, owner_id: owner, closes_at },
        )
        .await?;
        tx.commit().await?;
        Ok(prompt)
    }

    /// A prompt that exists (not deleted, not taken down), with counters for
    /// `viewer`. Says nothing about whether `viewer` may see it.
    async fn find_prompt(&self, prompt_id: Uuid, viewer: Option<Uuid>) -> AppResult<Option<PromptRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            prompt_columns!(),
            " FROM ask_prompts p WHERE p.id = $2 AND p.removed_at IS NULL"
        ))
        .bind(viewer)
        .bind(prompt_id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// A prompt of `owner`'s own, or `PROMPT_NOT_FOUND`.
    pub(crate) async fn owned_prompt(&self, owner: Uuid, prompt_id: Uuid) -> AppResult<PromptRow> {
        self.find_prompt(prompt_id, Some(owner))
            .await?
            .filter(|prompt| prompt.owner_id == owner)
            .ok_or_else(prompt_not_found)
    }

    /// A prompt `viewer` may see: their own or a friend's. Returns whether
    /// the viewer is a friend of the owner (false for the owner).
    pub(crate) async fn visible_prompt(&self, viewer: Uuid, prompt_id: Uuid) -> AppResult<(PromptRow, bool)> {
        let prompt = self.find_prompt(prompt_id, Some(viewer)).await?.ok_or_else(prompt_not_found)?;
        if prompt.owner_id == viewer {
            return Ok((prompt, false));
        }
        // Friends cannot be in a block with each other, so this covers blocks.
        ensure!(
            graph::are_friends(&self.core, prompt.owner_id, viewer).await?,
            not_found,
            "PROMPT_NOT_FOUND",
            "prompt not found"
        );
        Ok((prompt, true))
    }

    /// Closes a prompt early.
    pub(crate) async fn end_prompt(&self, owner: Uuid, prompt_id: Uuid) -> AppResult<PromptRow> {
        let closed = sqlx::query("UPDATE ask_prompts SET closed_at = COALESCE(closed_at, now()) WHERE id = $1 AND owner_id = $2 AND removed_at IS NULL")
            .bind(prompt_id)
            .bind(owner)
            .execute(&self.core.db)
            .await?;
        ensure!(closed.rows_affected() == 1, not_found, "PROMPT_NOT_FOUND", "prompt not found");
        self.owned_prompt(owner, prompt_id).await
    }

    /// Deletes a prompt with its responses. The rows stay for a while so a
    /// report filed before can still be acted on.
    pub(crate) async fn remove_prompt(&self, owner: Uuid, prompt_id: Uuid) -> AppResult<()> {
        let removed = sqlx::query("UPDATE ask_prompts SET removed_at = now(), removed_by = $3 WHERE id = $1 AND owner_id = $2 AND removed_at IS NULL")
            .bind(prompt_id)
            .bind(owner)
            .bind(REMOVED_BY_OWNER)
            .execute(&self.core.db)
            .await?;
        ensure!(removed.rows_affected() == 1, not_found, "PROMPT_NOT_FOUND", "prompt not found");
        Ok(())
    }

    /// Prompts of `owner` as `viewer` lists them, newest first.
    pub(crate) async fn prompts_page(
        &self,
        owner: Uuid,
        viewer: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<PromptRow>, PageInfo)> {
        let cursor: Option<Uuid> = page.decode()?;
        let mut rows: Vec<PromptRow> = sqlx::query_as(concat!(
            "SELECT ",
            prompt_columns!(),
            " FROM ask_prompts p
              WHERE p.owner_id = $2 AND p.removed_at IS NULL AND ($3::uuid IS NULL OR p.id < $3)
              ORDER BY p.id DESC LIMIT $4"
        ))
        .bind(viewer)
        .bind(owner)
        .bind(cursor)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }

    /// Open prompts of `viewer`'s friends, newest first. Friends the viewer
    /// muted are left out.
    pub(crate) async fn friend_prompts_page(&self, viewer: Uuid, page: &Page) -> AppResult<(Vec<PromptRow>, PageInfo)> {
        let cursor: Option<Uuid> = page.decode()?;
        let muted = self.muted_friends(viewer).await?;
        let friends: Vec<Uuid> =
            graph::friend_ids(&self.core, viewer).await?.into_iter().filter(|id| !muted.contains(id)).collect();
        let mut rows: Vec<PromptRow> = sqlx::query_as(concat!(
            "SELECT ",
            prompt_columns!(),
            " FROM ask_prompts p
              WHERE p.owner_id = ANY($2) AND p.removed_at IS NULL AND p.closed_at IS NULL AND p.closes_at > now()
                AND EXISTS (SELECT 1 FROM users u WHERE u.id = p.owner_id AND u.deleted_at IS NULL AND u.banned_at IS NULL)
                AND ($3::uuid IS NULL OR p.id < $3)
              ORDER BY p.id DESC LIMIT $4"
        ))
        .bind(viewer)
        .bind(friends)
        .bind(cursor)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }

    /// Stores `sender`'s response to a friend's open prompt and queues its
    /// delivery. As with questions, a blocked sender and a muted word change
    /// two column values and nothing else.
    pub(crate) async fn respond(
        &self,
        sender: Uuid,
        prompt_id: Uuid,
        text: &str,
        signed: bool,
    ) -> AppResult<ResponseRow> {
        let (prompt, is_friend) = self.visible_prompt(sender, prompt_id).await?;
        ensure!(prompt.owner_id != sender, invalid, "CANNOT_RESPOND_TO_OWN", "you cannot respond to your own prompt");
        debug_assert!(is_friend);
        ensure!(prompt.open, precondition, "PROMPT_CLOSED", "this prompt is closed");
        ensure!(!prompt.responded, already_exists, "ALREADY_RESPONDED", "you already responded to this prompt");
        let text = text::required(text, text::RESPONSE_MAX, "RESPONSE_TEXT_INVALID", "the response")?;
        self.limit_sends(sender, prompt.owner_id).await?;
        let muted_words = crate::settings::load(&self.core.db, prompt.owner_id).await?.muted_words;
        let intake = self.intake(sender, prompt.owner_id, &text, &muted_words).await?;

        let mut tx = self.core.db.begin().await?;
        let stored: Result<ResponseRow, sqlx::Error> = sqlx::query_as(concat!(
            "INSERT INTO ask_prompt_responses AS r (sender_id, prompt_id, owner_id, signed, text, held, dropped)
             SELECT $1, p.id, p.owner_id, $3, $4, $5, $6 FROM ask_prompts p
             WHERE p.id = $2 AND p.removed_at IS NULL AND p.closed_at IS NULL AND p.closes_at > now()
             RETURNING ",
            response_columns!()
        ))
        .bind(sender)
        .bind(prompt_id)
        .bind(signed)
        .bind(&text)
        .bind(intake.held)
        .bind(intake.dropped)
        .fetch_one(&mut *tx)
        .await;
        let response = match stored {
            Ok(response) => response,
            // A second request of the same person won the race.
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                bail!(already_exists, "ALREADY_RESPONDED", "you already responded to this prompt")
            }
            // The prompt closed between the check and the insert.
            Err(sqlx::Error::RowNotFound) => bail!(precondition, "PROMPT_CLOSED", "this prompt is closed"),
            Err(e) => return Err(e.into()),
        };
        events::emit(&mut *tx, RESPONSE_STORED, None, &Stored { id: response.id }).await?;
        tx.commit().await?;
        Ok(response)
    }

    /// The viewer's own response to a prompt, if they sent one.
    pub(crate) async fn own_response(&self, sender: Uuid, prompt_id: Uuid) -> AppResult<Option<ResponseRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            response_columns!(),
            " FROM ask_prompt_responses r WHERE r.sender_id = $1 AND r.prompt_id = $2 AND (r.removed_by IS NULL OR r.removed_by = 1)"
        ))
        .bind(sender)
        .bind(prompt_id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// Responses to a prompt of `owner`'s, newest first. Listing received
    /// responses marks the listed ones as seen.
    pub(crate) async fn responses_page(
        &self,
        owner: Uuid,
        prompt_id: Uuid,
        filter: Filter,
        page: &Page,
    ) -> AppResult<(Vec<ResponseRow>, PageInfo)> {
        self.owned_prompt(owner, prompt_id).await?;
        let cursor: Option<Uuid> = page.decode()?;
        let (held, published_only) = match filter {
            Filter::Received => (false, false),
            Filter::Published => (false, true),
            Filter::Held => (true, false),
        };
        let mut rows: Vec<ResponseRow> = sqlx::query_as(concat!(
            "SELECT ",
            response_columns!(),
            " FROM ask_prompt_responses r
              WHERE r.owner_id = $1 AND r.prompt_id = $2 AND r.held = $3 AND (NOT $4 OR r.published_at IS NOT NULL) AND ",
            response_in_box!(),
            " AND ($5::uuid IS NULL OR r.id < $5)
              ORDER BY r.id DESC LIMIT $6"
        ))
        .bind(owner)
        .bind(prompt_id)
        .bind(held)
        .bind(published_only)
        .bind(cursor)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        if filter == Filter::Received {
            let listed: Vec<Uuid> = rows.iter().filter(|row| row.seen_at.is_none()).map(|row| row.id).collect();
            if !listed.is_empty() {
                sqlx::query("UPDATE ask_prompt_responses SET seen_at = now() WHERE owner_id = $1 AND id = ANY($2) AND seen_at IS NULL")
                    .bind(owner)
                    .bind(listed)
                    .execute(&self.core.db)
                    .await?;
            }
        }
        Ok((rows, info))
    }

    /// The published responses of a prompt, newest publication first.
    pub(crate) async fn published_page(
        &self,
        prompt_id: Uuid,
        viewer: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<ResponseRow>, PageInfo)> {
        let cursor: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
        let (after_time, after_id) = cursor.unzip();
        let mut rows: Vec<ResponseRow> = sqlx::query_as(concat!(
            "SELECT ",
            response_columns!(),
            " FROM ask_prompt_responses r
              WHERE r.prompt_id = $2 AND r.published_at IS NOT NULL AND NOT r.held AND ",
            response_in_box!(),
            " AND ($3::timestamptz IS NULL OR (r.published_at, r.id) < ($3, $4))
              ORDER BY r.published_at DESC, r.id DESC LIMIT $5"
        ))
        .bind(viewer)
        .bind(prompt_id)
        .bind(after_time)
        .bind(after_id)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.published_at.unwrap_or(row.created_at), row.id));
        Ok((rows, info))
    }

    /// A response to a prompt of `owner`'s that is in their view.
    async fn owned_response(&self, owner: Uuid, response_id: Uuid) -> AppResult<ResponseRow> {
        let row: Option<ResponseRow> = sqlx::query_as(concat!(
            "SELECT ",
            response_columns!(),
            " FROM ask_prompt_responses r
              WHERE r.owner_id = $1 AND r.id = $2 AND ",
            response_in_box!(),
            " AND EXISTS (SELECT 1 FROM ask_prompts p WHERE p.id = r.prompt_id AND p.removed_at IS NULL)"
        ))
        .bind(owner)
        .bind(response_id)
        .fetch_optional(&self.core.db)
        .await?;
        row.ok_or_else(response_not_found)
    }

    /// Puts a response on the prompt's page, or takes it off.
    pub(crate) async fn set_published(
        &self,
        owner: Uuid,
        response_id: Uuid,
        published: bool,
    ) -> AppResult<ResponseRow> {
        let current = self.owned_response(owner, response_id).await?;
        // A held response has to be released first: publishing it by
        // accident would put on the page exactly what the filter caught.
        ensure!(!current.held, not_found, "RESPONSE_NOT_FOUND", "response not found");
        sqlx::query(
            "UPDATE ask_prompt_responses
             SET published_at = CASE WHEN $3 THEN COALESCE(published_at, now()) END, seen_at = COALESCE(seen_at, now())
             WHERE owner_id = $1 AND id = $2",
        )
        .bind(owner)
        .bind(response_id)
        .bind(published)
        .execute(&self.core.db)
        .await?;
        self.owned_response(owner, response_id).await
    }

    /// Releases a response the word filter held into the received ones.
    pub(crate) async fn unhold_response(&self, owner: Uuid, response_id: Uuid) -> AppResult<ResponseRow> {
        self.owned_response(owner, response_id).await?;
        sqlx::query("UPDATE ask_prompt_responses SET held = false WHERE owner_id = $1 AND id = $2")
            .bind(owner)
            .bind(response_id)
            .execute(&self.core.db)
            .await?;
        self.owned_response(owner, response_id).await
    }

    /// Deletes a response from the owner's view. Its sender keeps seeing it
    /// as sent, and cannot respond again.
    pub(crate) async fn remove_response(&self, owner: Uuid, response_id: Uuid) -> AppResult<()> {
        self.owned_response(owner, response_id).await?;
        sqlx::query("UPDATE ask_prompt_responses SET removed_at = now(), removed_by = $3, published_at = NULL WHERE owner_id = $1 AND id = $2 AND removed_at IS NULL")
            .bind(owner)
            .bind(response_id)
            .bind(REMOVED_BY_OWNER)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// `admin.content.removed` for a prompt or a response: gone for everyone.
    pub(crate) async fn take_down_prompt_content(&self, subject_id: Uuid) -> AppResult<()> {
        sqlx::query("UPDATE ask_prompts SET removed_at = now(), removed_by = $2 WHERE id = $1 AND removed_at IS NULL")
            .bind(subject_id)
            .bind(REMOVED_BY_MODERATION)
            .execute(&self.core.db)
            .await?;
        sqlx::query(
            "UPDATE ask_prompt_responses SET removed_at = COALESCE(removed_at, now()), removed_by = $2, published_at = NULL
             WHERE id = $1 AND removed_by IS DISTINCT FROM $2",
        )
        .bind(subject_id)
        .bind(REMOVED_BY_MODERATION)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(owner: Uuid) -> PromptRow {
        PromptRow {
            id: new_id(),
            owner_id: owner,
            text: "私の第一印象は?".into(),
            emoji: "👀".into(),
            created_at: now(),
            closes_at: now() + Duration::hours(1),
            open: true,
            published_count: 2,
            response_count: 5,
            unseen_count: 3,
            held_count: 1,
            responded: false,
        }
    }

    fn response(owner: Uuid) -> ResponseRow {
        ResponseRow {
            id: new_id(),
            prompt_id: new_id(),
            owner_id: owner,
            responder_id: None,
            signed: false,
            text: "やさしそう!".into(),
            held: false,
            seen_at: None,
            published_at: None,
            created_at: now(),
            live: true,
            sent_by_viewer: false,
        }
    }

    #[test]
    fn durations_default_to_a_day_and_stay_in_bounds() {
        assert_eq!(duration_of(0).unwrap(), Duration::hours(24));
        assert_eq!(duration_of(600).unwrap(), Duration::minutes(10));
        assert!(duration_of(7 * 24 * 3600).is_ok());
        for bad in [-5, 1, 599, 7 * 24 * 3600 + 1] {
            assert_eq!(duration_of(bad).unwrap_err().reason, "PROMPT_DURATION_INVALID", "{bad}");
        }
    }

    #[test]
    fn only_the_owner_sees_the_response_counters() {
        let (owner, friend) = (new_id(), new_id());
        let row = prompt(owner);
        let mine = row.to_proto(Some(owner), UserSummary::default(), false);
        assert_eq!((mine.response_count, mine.unseen_count, mine.held_count), (5, 3, 1));
        assert!(!mine.can_respond && !mine.responded);
        let theirs = row.to_proto(Some(friend), UserSummary::default(), true);
        assert_eq!((theirs.response_count, theirs.unseen_count, theirs.held_count), (0, 0, 0));
        assert_eq!(theirs.published_count, 2);
        assert!(theirs.can_respond);
    }

    #[test]
    fn responding_needs_an_open_prompt_a_friend_and_no_earlier_response() {
        let (owner, friend) = (new_id(), new_id());
        let mut row = prompt(owner);
        assert!(!row.to_proto(Some(friend), UserSummary::default(), false).can_respond, "not a friend");
        row.responded = true;
        let seen = row.to_proto(Some(friend), UserSummary::default(), true);
        assert!(seen.responded && !seen.can_respond);
        row.responded = false;
        row.open = false;
        assert!(!row.to_proto(Some(friend), UserSummary::default(), true).can_respond);
    }

    #[test]
    fn the_responder_never_learns_what_became_of_a_response() {
        let (owner, responder) = (new_id(), new_id());
        // Held, seen, dropped or deleted: the responder's copy looks the same.
        for (held, seen, live) in [(true, false, true), (false, true, true), (false, false, false), (true, true, false)]
        {
            let mut row = response(owner);
            row.sent_by_viewer = true;
            row.held = held;
            row.seen_at = seen.then(now);
            row.live = live;
            let mine = row.to_proto(Some(responder), None);
            assert!(
                mine.sent_by_me && !mine.held && !mine.seen && !mine.published,
                "held {held}, seen {seen}, live {live}"
            );
            assert!(mine.responder.is_unset() && mine.anonymous);
        }
    }

    #[test]
    fn an_anonymous_response_names_nobody_even_if_a_summary_is_passed() {
        let owner = new_id();
        let row = response(owner);
        let leaked = UserSummary { handle: "hana".into(), ..Default::default() };
        assert!(row.to_proto(Some(owner), Some(leaked)).responder.is_unset());
    }

    #[test]
    fn a_held_or_removed_response_is_not_published() {
        let owner = new_id();
        let mut row = response(owner);
        row.published_at = Some(now());
        assert!(row.to_proto(Some(owner), None).published);
        row.held = true;
        assert!(!row.to_proto(Some(owner), None).published);
        row.held = false;
        row.live = false;
        let gone = row.to_proto(Some(owner), None);
        assert!(!gone.published && gone.published_at.is_unset());
    }

    #[test]
    fn filters_map_and_reject_unknown_values() {
        assert_eq!(Filter::of(ResponseFilter::RESPONSE_FILTER_UNSPECIFIED.into()).unwrap(), Filter::Received);
        assert_eq!(Filter::of(ResponseFilter::RESPONSE_FILTER_HELD.into()).unwrap(), Filter::Held);
        assert_eq!(Filter::of(buffa::EnumValue::from(42)).unwrap_err().reason, "RESPONSE_FILTER_INVALID");
    }
}
