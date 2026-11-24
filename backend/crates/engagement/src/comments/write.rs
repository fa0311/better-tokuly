//! Creating, editing and deleting comments.
use std::time::Duration;

use tokuly_core::{analytics::AnalyticsEvent, events, prelude::*};
use tokuly_media::assets;
use tokuly_proto::{common::SubjectType, engagement::CreateCommentRequest, media::AssetPurpose};
use tokuly_stamp::SendContext;

use super::{
    CommentRow,
    announce::{self, Concerned, Reason, Sender},
    comment_columns, comment_not_found, find, image_holder, kept, lifecycle, lock_thread,
};
use crate::{
    Engagement,
    effects::{Effects, Push},
    events::{COMMENT_DELETED, COMMENT_EDITED, COMMENT_PUBLISHED, CommentEdited, CommentEvent},
    subjects::{Subject, requested_type},
    text,
};

/// The latest moment of a video a comment may point at.
const MAX_TIMESTAMP_SECONDS: i32 = 7 * 24 * 3600;
/// How long the same comment cannot be posted on the same subject again.
const DUPLICATE_WINDOW: Duration = Duration::from_secs(60);
/// Edits one user may make per minute.
const EDITS_PER_MINUTE: u32 = 30;
/// Uploads a comment image may come from. There is no purpose of its own
/// for comment images; both of these are processed as plain pictures.
const IMAGE_PURPOSES: [AssetPurpose; 2] =
    [AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE];

/// The domain event payload about a comment.
pub(crate) fn comment_event(row: &CommentRow) -> CommentEvent {
    CommentEvent {
        comment_id: row.id,
        subject_type: row.subject_type,
        subject_id: row.subject_id,
        subject_owner_id: row.subject_owner_id,
        author_id: row.author_id,
        channel_id: row.channel_id,
        parent_id: row.parent_id,
    }
}

/// Everything about a new comment that was decided before its transaction.
struct Draft {
    id: Uuid,
    author_id: Uuid,
    channel_id: Option<Uuid>,
    text: String,
    stamp_id: Option<Uuid>,
    image: Option<serde_json::Value>,
    image_asset_id: Option<Uuid>,
    timestamp_seconds: Option<i32>,
    status: i32,
    hold_reason: i32,
    /// The top-level comment, for a reply.
    thread_id: Option<Uuid>,
    /// The reply it answers, when that is not the top-level comment.
    reply_to_id: Option<Uuid>,
    mentioned: Vec<Uuid>,
    /// Who is told if it enters public view at once.
    audience: Vec<(Uuid, Reason)>,
    sender: Sender,
}

impl Engagement {
    /// The users a text mentions, as far as the author may mention them:
    /// active accounts, not the author, nobody in a block with the author.
    /// `TOO_MANY_MENTIONS` beyond [`text::MAX_MENTIONS`].
    pub(crate) async fn mentioned_users(&self, author_id: Uuid, text: &str) -> AppResult<Vec<Uuid>> {
        let handles = text::mention_handles(text);
        if handles.is_empty() {
            return Ok(Vec::new());
        }
        let found: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM users WHERE handle = ANY($1::text[]::citext[]) AND deleted_at IS NULL AND banned_at IS NULL ORDER BY id",
        )
        .bind(&handles)
        .fetch_all(&self.core.db)
        .await?;
        ensure!(
            found.len() <= text::MAX_MENTIONS,
            invalid,
            "TOO_MANY_MENTIONS",
            "a comment mentions at most {} people",
            text::MAX_MENTIONS
        );
        let hidden = self.hidden_from(Some(author_id)).await?;
        Ok(found.into_iter().filter(|id| *id != author_id && !hidden.contains(id)).collect())
    }

    /// Writes a comment or a reply (`CommentService.CreateComment`).
    pub(crate) async fn create_comment(
        &self,
        user_id: Uuid,
        request: &CreateCommentRequest,
    ) -> AppResult<(CommentRow, Subject)> {
        let parent_id = parse_opt_id(&request.parent_id)?;
        let stamp_id = parse_opt_id(&request.stamp_id)?;
        let image_asset_id = parse_opt_id(&request.image_asset_id)?;
        let channel_id = parse_opt_id(&request.channel_id)?;
        ensure!(
            stamp_id.is_none() || image_asset_id.is_none(),
            invalid,
            "ATTACHMENT_CONFLICT",
            "a comment carries a stamp or an image, not both"
        );
        let text = text::comment_text(&request.text, stamp_id.is_some() || image_asset_id.is_some())?;
        if let Some(seconds) = request.timestamp_seconds {
            ensure!(
                (0..=MAX_TIMESTAMP_SECONDS).contains(&seconds),
                invalid,
                "TIMESTAMP_INVALID",
                "the timestamp is out of range"
            );
        }

        // Where it goes: below a comment, or onto a subject.
        let (subject, thread, reply_to) = match parent_id {
            Some(parent_id) => {
                let found = self.comment_for(parent_id, Some(user_id)).await?.ok_or_else(comment_not_found)?;
                // Replying to a reply adds to the same thread.
                let (thread, reply_to) = match (found.row.is_reply(), found.parent) {
                    (false, _) => (found.row, None),
                    (true, Some(top)) => (top, Some(found.row)),
                    (true, None) => return Err(comment_not_found()),
                };
                ensure!(
                    thread.reads_published_to(user_id)
                        && reply_to.as_ref().is_none_or(|reply| reply.reads_published_to(user_id)),
                    precondition,
                    "COMMENT_NOT_PUBLISHED",
                    "this comment cannot be replied to while it is not published"
                );
                (found.subject, Some(thread), reply_to)
            }
            None => {
                let kind = request.subject.as_option().map(|subject| subject.r#type).unwrap_or_default();
                ensure!(
                    requested_type(kind)? != SubjectType::SUBJECT_TYPE_COMMENT,
                    invalid,
                    "UNSUPPORTED_SUBJECT",
                    "reply to a comment with parent_id"
                );
                (self.requested_subject(request.subject.as_option(), Some(user_id)).await?, None, None)
            }
        };
        ensure!(subject.info.interactions_enabled, precondition, "COMMENTS_DISABLED", "comments are turned off here");

        if let Some(channel_id) = channel_id {
            self.owned_channel(user_id, channel_id).await?;
        }
        let cache = &self.core.cache;
        cache
            .rate_limit(
                &format!("engagement:comment:minute:{user_id}"),
                self.cfg.comments_per_minute,
                Duration::from_secs(60),
            )
            .await?;
        cache
            .rate_limit(
                &format!("engagement:comment:hour:{user_id}"),
                self.cfg.comments_per_hour,
                Duration::from_secs(3600),
            )
            .await?;

        let mentioned = self.mentioned_users(user_id, &text).await?;
        if let Some(stamp_id) = stamp_id {
            tokuly_stamp::ensure_usable(&self.core, user_id, stamp_id).await?;
        }
        let image = match image_asset_id {
            Some(asset_id) => Some(assets::require_image_json(&self.core, asset_id, user_id, &IMAGE_PURPOSES).await?),
            None => None,
        };
        let verdict = self.screen_comment(subject.info.owner_id, user_id, &text).await?;

        // Decided now, so that the transaction below only writes.
        let enters_public_view = verdict.is_published() && thread.as_ref().is_none_or(CommentRow::is_published);
        let audience = if enters_public_view {
            let concerned = Concerned {
                thread_author: thread.as_ref().map(|thread| thread.author_id),
                replied_author: reply_to.as_ref().map(|reply| reply.author_id),
                mentioned: mentioned.clone(),
                subject_owner: subject.owner().filter(|_| thread.is_none()),
            };
            self.comment_audience(user_id, &concerned, &subject).await?
        } else {
            Vec::new()
        };
        let draft = Draft {
            id: new_id(),
            author_id: user_id,
            channel_id,
            text,
            stamp_id,
            image,
            image_asset_id,
            timestamp_seconds: request.timestamp_seconds,
            status: verdict.status,
            hold_reason: verdict.reason as i32,
            thread_id: thread.as_ref().map(|thread| thread.id),
            reply_to_id: reply_to.as_ref().map(|reply| reply.id),
            mentioned,
            audience,
            sender: self.sender_of(user_id, channel_id).await?,
        };

        // The same comment twice in a row is a double tap or spam.
        let duplicate_key = (!draft.text.is_empty() || stamp_id.is_some()).then(|| {
            let thread = draft.thread_id.map(|id| id.to_string()).unwrap_or_default();
            let stamp = stamp_id.map(|id| id.to_string()).unwrap_or_default();
            let fingerprint = text::fingerprint(&format!("{thread}|{stamp}|{}", draft.text));
            format!("engagement:comment:dup:{user_id}:{}:{}:{fingerprint:x}", subject.key.kind, subject.key.id)
        });
        if let Some(key) = &duplicate_key {
            ensure!(
                cache.set_nx(key, "1", DUPLICATE_WINDOW).await?,
                already_exists,
                "DUPLICATE_COMMENT",
                "you just posted the same comment here"
            );
        }
        let stored = self.store_comment(&draft, &subject).await;
        if stored.is_err()
            && let Some(key) = &duplicate_key
        {
            // The comment was not posted after all; do not hold the retry against them.
            let _ = cache.del(key).await;
        }
        let (row, effects) = stored?;

        if let Some(stamp_id) = stamp_id
            && let Err(error) = tokuly_stamp::record_send(&self.core, user_id, stamp_id, SendContext::Comment).await
        {
            tracing::warn!(%stamp_id, %error, "stamp send of a comment was not recorded");
        }
        self.apply(effects).await;
        let mut tracked = AnalyticsEvent::new("engagement.comment")
            .user(Some(user_id))
            .subject("comment", row.id)
            .props(serde_json::json!({ "subject_type": row.subject_type, "subject_id": row.subject_id, "reply": row.is_reply(), "status": row.status }));
        if let Some(owner_id) = subject.owner() {
            tracked = tracked.owner(owner_id);
        }
        self.core.analytics.track(tracked).await;
        Ok((row, subject))
    }

    /// The transaction of [`Self::create_comment`].
    async fn store_comment(&self, draft: &Draft, subject: &Subject) -> AppResult<(CommentRow, Effects)> {
        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        // Locking the thread keeps it from being hidden or deleted between
        // this check and the insert.
        let mut thread = match draft.thread_id {
            Some(thread_id) => {
                let (thread, _) = lock_thread(&mut tx, thread_id).await?.ok_or_else(comment_not_found)?;
                ensure!(
                    thread.reads_published_to(draft.author_id),
                    precondition,
                    "COMMENT_NOT_PUBLISHED",
                    "this comment cannot be replied to while it is not published"
                );
                Some(thread)
            }
            None => None,
        };
        let row: CommentRow = sqlx::query_as(concat!(
            "INSERT INTO comments (id, subject_type, subject_id, subject_owner_id, parent_id, reply_to_id, author_id, channel_id,
                                   text, stamp_id, image, image_asset_id, timestamp_seconds, status, hold_reason)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
             RETURNING ",
            comment_columns!()
        ))
        .bind(draft.id)
        .bind(subject.key.kind)
        .bind(subject.key.id)
        .bind(subject.info.owner_id)
        .bind(draft.thread_id)
        .bind(draft.reply_to_id)
        .bind(draft.author_id)
        .bind(draft.channel_id)
        .bind(&draft.text)
        .bind(draft.stamp_id)
        .bind(&draft.image)
        .bind(draft.image_asset_id)
        .bind(draft.timestamp_seconds)
        .bind(draft.status)
        .bind(draft.hold_reason)
        .fetch_one(&mut *tx)
        .await?;
        save_mentions(&mut tx, row.id, &draft.mentioned).await?;
        if let Some(asset_id) = draft.image_asset_id {
            assets::attach(&mut *tx, asset_id, &image_holder(row.id)).await?;
        }
        lifecycle::inserted(&mut tx, &mut effects, &row, thread.as_mut()).await?;
        if row.in_public_view(thread.as_ref()) {
            self.announce_published(&mut tx, &row, subject, &draft.audience, &draft.sender).await?;
            events::emit(&mut *tx, COMMENT_PUBLISHED, Some(row.author_id), &comment_event(&row)).await?;
        } else if row.is_held() {
            self.announce_held(&mut tx, &row, subject).await?;
        }
        tx.commit().await?;
        Ok((row, effects))
    }

    /// Changes the text of the caller's own comment (`CommentService.UpdateComment`).
    pub(crate) async fn update_comment(
        &self,
        user_id: Uuid,
        comment_id: &str,
        new_text: &str,
    ) -> AppResult<(CommentRow, Subject)> {
        let found = self.require_comment(comment_id, Some(user_id)).await?;
        ensure!(found.row.author_id == user_id, denied, "NOT_COMMENT_AUTHOR", "only the author can edit a comment");
        // Nobody sees the comments while they are turned off; an edit would
        // still reach the people it newly mentions.
        ensure!(
            found.subject.info.interactions_enabled,
            precondition,
            "COMMENTS_DISABLED",
            "comments are turned off here"
        );
        self.core
            .cache
            .rate_limit(&format!("engagement:comment:edit:{user_id}"), EDITS_PER_MINUTE, Duration::from_secs(60))
            .await?;
        let has_attachment = found.row.stamp_id.is_some() || found.row.image_asset_id.is_some();
        let text = text::comment_text(new_text, has_attachment)?;
        if text == found.row.text {
            return Ok((found.row, found.subject));
        }
        let mentioned = self.mentioned_users(user_id, &text).await?;
        let verdict = self.screen_comment(found.owner_id(), user_id, &text).await?;

        // People the comment did not mention before hear about it now —
        // unless it was never announced, in which case its approval will
        // announce it with the mentions it has by then.
        let previously: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM comment_mentions WHERE comment_id = $1")
            .bind(found.row.id)
            .fetch_all(&self.core.db)
            .await?;
        let newly: Vec<Uuid> = mentioned.iter().copied().filter(|id| !previously.contains(id)).collect();
        let audience = if found.row.notified_at.is_some() && !newly.is_empty() {
            let concerned = Concerned { mentioned: newly, ..Default::default() };
            self.comment_audience(user_id, &concerned, &found.subject).await?
        } else {
            Vec::new()
        };
        let sender = self.sender_of(user_id, found.row.channel_id).await?;

        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        let (mut row, mut parent) = lock_thread(&mut tx, found.row.id).await?.ok_or_else(comment_not_found)?;
        // What the people who read it read stays reportable.
        kept::keep_edited(&mut tx, &row).await?;
        // The clock, not the transaction's start: taken under the lock, it
        // orders two edits of one comment the way they were applied.
        let edited_at: Time = sqlx::query_scalar(
            "UPDATE comments SET text = $2, edited_at = clock_timestamp() WHERE id = $1 RETURNING edited_at",
        )
        .bind(row.id)
        .bind(&text)
        .fetch_one(&mut *tx)
        .await?;
        row.text = text;
        row.edited_at = Some(edited_at);
        sqlx::query("DELETE FROM comment_mentions WHERE comment_id = $1").bind(row.id).execute(&mut *tx).await?;
        save_mentions(&mut tx, row.id, &mentioned).await?;
        let edited = CommentEdited {
            comment: comment_event(&row),
            body: announce::body_of(&row),
            mentioned: mentioned.clone(),
            edited_at,
        };
        events::emit(&mut *tx, COMMENT_EDITED, Some(user_id), &edited).await?;
        if row.is_published() && !verdict.is_published() {
            // The new text trips one of the owner's rules. The other way
            // round never happens: an edit does not publish a comment.
            lifecycle::set_status(&mut tx, &mut effects, &mut row, parent.as_mut(), verdict.status, verdict.reason)
                .await?;
            if row.is_held() {
                self.announce_held(&mut tx, &row, &found.subject).await?;
            }
        } else if row.in_public_view(parent.as_ref()) {
            effects.push(Push::Updated { key: row.key(), comment_id: row.id });
            self.announce(&mut tx, &row, &found.subject, &audience, &sender).await?;
        }
        tx.commit().await?;
        self.apply(effects).await;
        Ok((row, found.subject))
    }

    /// Deletes a comment with its replies (`CommentService.DeleteComment`):
    /// its author always may, anyone else must own the subject. Returns how
    /// many comments are gone.
    pub(crate) async fn delete_comment(&self, user_id: Uuid, comment_id: &str) -> AppResult<i32> {
        let comment_id = parse_id(comment_id)?;
        let row = find(&self.core.db, comment_id).await?.ok_or_else(comment_not_found)?;
        // An author can take their words back even where they can no longer
        // see the subject; everyone else has to see the comment first.
        if row.author_id != user_id {
            let found = self.comment_for(comment_id, Some(user_id)).await?.ok_or_else(comment_not_found)?;
            ensure!(
                found.subject.is_owned_by(user_id),
                denied,
                "COMMENT_DELETE_FORBIDDEN",
                "only its author and the owner of the content can delete a comment"
            );
        }
        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        let (row, mut parent) = lock_thread(&mut tx, comment_id).await?.ok_or_else(comment_not_found)?;
        // What the people who read it read stays reportable.
        kept::keep_deleted(&mut tx, &row).await?;
        let removed = lifecycle::delete(&mut tx, &mut effects, &row, parent.as_mut()).await?;
        events::emit(&mut *tx, COMMENT_DELETED, Some(user_id), &comment_event(&row)).await?;
        tx.commit().await?;
        self.apply(effects).await;
        Ok(removed)
    }
}

async fn save_mentions(conn: &mut sqlx::PgConnection, comment_id: Uuid, user_ids: &[Uuid]) -> AppResult<()> {
    if user_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO comment_mentions (comment_id, user_id) SELECT $1, unnest($2::uuid[]) ON CONFLICT DO NOTHING",
    )
    .bind(comment_id)
    .bind(user_ids)
    .execute(conn)
    .await?;
    Ok(())
}
