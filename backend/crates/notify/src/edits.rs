//! Comments edited after they were announced.
//!
//! A notification about a comment quotes it ("Bob commented: …"). When the
//! author edits it, engagement says so (`engagement.comment.edited`, with
//! the body notifications show for the new text and whom it mentions), and
//! every copy of the old text follows:
//!
//! - inbox entries quoting the comment show the new body;
//! - a mention the new text no longer contains is withdrawn from the inbox
//!   of the person it named — except the subject owner's of a top-level
//!   comment, which becomes their ordinary "new comment" entry (engagement
//!   told them once, with the mention, and would otherwise have told them
//!   about the comment);
//! - nobody the comment was hidden from (`notify_audiences`) is shown the
//!   new text;
//! - a request emitted before the edit but handled after it, and a push
//!   still queued from before it, carry the new body, or are dropped when
//!   they announce a mention that is gone.
//!
//! Every notification about a comment names the version it quotes
//! (`data.edited_at`, `null` before the first edit), and the latest edit of
//! each comment is remembered (`notify_edits`) for as long as events can
//! arrive late. So the order in which edits and requests are handled does
//! not matter: an older edit handled after a newer one changes nothing, and
//! nothing is updated to a version older than the one it shows.
use serde::Deserialize;
use sqlx::PgExecutor;
use tokuly_core::{events::Event, outbound::Localized, prelude::*};

use crate::{Notify, locks, sanitize::clean_body, withdraw::UNWITHDRAWABLE_CATEGORY};

pub const COMMENT_EDITED_TOPIC: &str = "engagement.comment.edited";
/// Kinds of the engagement domain, whose `data.comment_id` names the comment
/// the body quotes.
const COMMENT_KIND_PATTERN: &str = "engagement.%";
const MENTION_KIND: &str = "engagement.mention";

/// Our view of `engagement.comment.edited`.
#[derive(Debug, Clone, Deserialize)]
struct CommentEdited {
    comment_id: Uuid,
    author_id: Uuid,
    body: Localized,
    #[serde(default)]
    mentioned: Vec<Uuid>,
    #[serde(default)]
    owner_notice: Option<OwnerNotice>,
    /// The version the edit made; the event's time for producers that do not
    /// give one.
    #[serde(default)]
    edited_at: Option<Time>,
}

/// What the subject owner's mention of a top-level comment becomes when the
/// edit took the mention out.
#[derive(Debug, Clone, Deserialize)]
struct OwnerNotice {
    user_id: Uuid,
    kind: String,
    title: Localized,
}

/// The comment a notification quotes, and the version it quotes (`None`:
/// the text before any edit).
fn quoted(kind: &str, data: &serde_json::Value) -> Option<(Uuid, Option<Time>)> {
    if !kind.starts_with("engagement.") {
        return None;
    }
    let comment_id = data.get("comment_id")?.as_str().and_then(|id| Uuid::parse_str(id).ok())?;
    let version = data
        .get("edited_at")
        .filter(|value| !value.is_null())
        .and_then(|value| serde_json::from_value::<Time>(value.clone()).ok());
    Some((comment_id, version))
}

/// What a notification about a comment shows, given what is known of later
/// edits.
#[derive(Debug, Clone)]
pub(crate) enum Current {
    /// It quotes the latest known version.
    Unchanged,
    /// The comment was edited since: this is its body now, of this version,
    /// and — for the subject owner's mention the edit took out — the kind
    /// and title of the ordinary notification it becomes.
    Edited { body: Localized, edited_at: Time, becomes: Option<(String, Localized)> },
    /// It announces a mention the comment no longer contains.
    Unmentioned,
}

impl Current {
    /// Brings a notification's kind, title, body and data up to date.
    /// `false` when it is to be dropped.
    pub(crate) fn apply(
        self,
        kind: &mut String,
        title: &mut Localized,
        body: &mut Localized,
        data: &mut serde_json::Value,
    ) -> bool {
        match self {
            Current::Unchanged => true,
            Current::Unmentioned => false,
            Current::Edited { body: current, edited_at, becomes } => {
                *body = current;
                if let Some((new_kind, new_title)) = becomes {
                    *kind = new_kind;
                    *title = new_title;
                }
                if let Some(data) = data.as_object_mut() {
                    data.insert("edited_at".into(), serde_json::json!(edited_at));
                }
                true
            }
        }
    }
}

#[derive(sqlx::FromRow)]
struct EditRow {
    body_ja: String,
    body_en: String,
    mentioned: Vec<Uuid>,
    edited_at: Time,
    owner_id: Option<Uuid>,
    owner_kind: Option<String>,
    owner_title_ja: Option<String>,
    owner_title_en: Option<String>,
}

/// What a notification of `kind` to `user_id` with `data` should show now.
pub(crate) async fn current<'e, E: PgExecutor<'e>>(
    executor: E,
    kind: &str,
    user_id: Uuid,
    data: &serde_json::Value,
) -> AppResult<Current> {
    let Some((comment_id, version)) = quoted(kind, data) else {
        return Ok(Current::Unchanged);
    };
    let edit: Option<EditRow> = sqlx::query_as(
        "SELECT body_ja, body_en, mentioned, edited_at, owner_id, owner_kind, owner_title_ja, owner_title_en
         FROM notify_edits WHERE ref_id = $1 AND ($2::timestamptz IS NULL OR edited_at > $2)",
    )
    .bind(comment_id)
    .bind(version)
    .fetch_optional(executor)
    .await?;
    let Some(edit) = edit else { return Ok(Current::Unchanged) };
    let body = Localized::new(edit.body_ja, edit.body_en);
    if kind != MENTION_KIND || edit.mentioned.contains(&user_id) {
        return Ok(Current::Edited { body, edited_at: edit.edited_at, becomes: None });
    }
    Ok(match (edit.owner_id, edit.owner_kind, edit.owner_title_ja, edit.owner_title_en) {
        (Some(owner), Some(owner_kind), Some(ja), Some(en)) if owner == user_id => {
            Current::Edited { body, edited_at: edit.edited_at, becomes: Some((owner_kind, Localized::new(ja, en))) }
        }
        _ => Current::Unmentioned,
    })
}

impl Notify {
    /// `engagement.comment.edited`: entries quoting an older version of the
    /// comment show the new one, and mentions the new text dropped are
    /// withdrawn (the subject owner's becomes their "new comment" entry).
    /// Nobody the comment was hidden from is shown the new text. Serialized
    /// with the requests about the comment on its withdrawal lock.
    /// Idempotent, and an older edit than the latest one handled changes
    /// nothing.
    pub async fn handle_comment_edited(&self, event: &Event) -> AppResult<()> {
        let edit: CommentEdited = event.data()?;
        let edited_at = edit.edited_at.unwrap_or(event.occurred_at);
        let body = clean_body(&edit.body);
        // Only an engagement kind, and only for a top-level comment's owner.
        let owner = edit
            .owner_notice
            .as_ref()
            .filter(|notice| notice.kind.starts_with("engagement.") && notice.kind != MENTION_KIND)
            .map(|notice| (notice.user_id, notice.kind.clone(), clean_body(&notice.title)));
        let mut tx = self.core.db.begin().await?;
        locks::lock(&mut tx, locks::key(edit.comment_id)).await?;
        let latest: bool = sqlx::query_scalar(
            "INSERT INTO notify_edits AS e
                 (ref_id, author_id, body_ja, body_en, mentioned, edited_at, owner_id, owner_kind, owner_title_ja, owner_title_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (ref_id) DO UPDATE SET
                 author_id = EXCLUDED.author_id, body_ja = EXCLUDED.body_ja, body_en = EXCLUDED.body_en,
                 mentioned = EXCLUDED.mentioned, edited_at = EXCLUDED.edited_at,
                 owner_id = EXCLUDED.owner_id, owner_kind = EXCLUDED.owner_kind,
                 owner_title_ja = EXCLUDED.owner_title_ja, owner_title_en = EXCLUDED.owner_title_en,
                 recorded_at = now()
             WHERE e.edited_at <= EXCLUDED.edited_at
             RETURNING true",
        )
        .bind(edit.comment_id)
        .bind(edit.author_id)
        .bind(&body.ja)
        .bind(&body.en)
        .bind(&edit.mentioned)
        .bind(edited_at)
        .bind(owner.as_ref().map(|(user_id, _, _)| *user_id))
        .bind(owner.as_ref().map(|(_, kind, _)| kind.as_str()))
        .bind(owner.as_ref().map(|(_, _, title)| title.ja.as_str()))
        .bind(owner.as_ref().map(|(_, _, title)| title.en.as_str()))
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(false);
        if !latest {
            tx.commit().await?;
            return Ok(());
        }
        // Entries the comment was hidden from (by an audience change that
        // has not reached them yet, or that they were spared by being newer)
        // are left alone: the new text is not for them.
        const VISIBLE: &str = "NOT EXISTS (
                 SELECT 1 FROM notify_audiences a
                 WHERE a.ref_id = $1 AND a.visible_to IS NOT NULL AND n.occurred_at <= a.changed_at
                   AND a.owner_id <> n.user_id AND NOT n.user_id = ANY(a.visible_to)
             )";
        // The owner's mention the edit took out: their "new comment" entry now.
        if let Some((owner_id, owner_kind, title)) = &owner {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "UPDATE notifications n SET kind = $5, title_ja = $6, title_en = $7, body_ja = $2, body_en = $3,
                     data = jsonb_set(data, '{{edited_at}}', to_jsonb($4::timestamptz))
                 WHERE refs @> ARRAY[$1]::uuid[] AND jsonb_typeof(data) = 'object' AND data->>'comment_id' = $1::text
                   AND kind = $8 AND user_id = $9 AND NOT $9 = ANY($10)
                   AND (data->>'edited_at' IS NULL OR (data->>'edited_at')::timestamptz < $4)
                   AND {VISIBLE}"
            )))
            .bind(edit.comment_id)
            .bind(&body.ja)
            .bind(&body.en)
            .bind(edited_at)
            .bind(owner_kind)
            .bind(&title.ja)
            .bind(&title.en)
            .bind(MENTION_KIND)
            .bind(owner_id)
            .bind(&edit.mentioned)
            .execute(&mut *tx)
            .await?;
        }
        // A comment reaches a handful of people (the owner, the people it
        // answers and mentions), so this is one statement each.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE notifications n SET body_ja = $2, body_en = $3,
                 data = jsonb_set(data, '{{edited_at}}', to_jsonb($4::timestamptz))
             WHERE refs @> ARRAY[$1]::uuid[] AND jsonb_typeof(data) = 'object' AND data->>'comment_id' = $1::text
               AND kind LIKE $5 AND category <> $6
               AND (data->>'edited_at' IS NULL OR (data->>'edited_at')::timestamptz < $4)
               AND NOT (kind = $7 AND NOT user_id = ANY($8))
               AND {VISIBLE}"
        )))
        .bind(edit.comment_id)
        .bind(&body.ja)
        .bind(&body.en)
        .bind(edited_at)
        .bind(COMMENT_KIND_PATTERN)
        .bind(UNWITHDRAWABLE_CATEGORY)
        .bind(MENTION_KIND)
        .bind(&edit.mentioned)
        .execute(&mut *tx)
        .await?;
        let removed: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "DELETE FROM notifications
             WHERE refs @> ARRAY[$1]::uuid[] AND jsonb_typeof(data) = 'object' AND data->>'comment_id' = $1::text
               AND kind = $3 AND NOT user_id = ANY($4)
               AND (data->>'edited_at' IS NULL OR (data->>'edited_at')::timestamptz < $2)
             RETURNING user_id, id",
        )
        .bind(edit.comment_id)
        .bind(edited_at)
        .bind(MENTION_KIND)
        .bind(&edit.mentioned)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        self.announce_removed(removed).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_engagement_kinds_quote_a_comment() {
        let comment = Uuid::now_v7();
        let at = now();
        let data = serde_json::json!({ "comment_id": comment, "parent_id": null, "edited_at": at });
        assert_eq!(quoted("engagement.reply", &data), Some((comment, Some(at))));
        assert_eq!(quoted("community.reply", &data), None);
        let never_edited = serde_json::json!({ "comment_id": comment, "edited_at": null });
        assert_eq!(quoted("engagement.comment", &never_edited), Some((comment, None)));
        assert_eq!(quoted("engagement.comment", &serde_json::json!({ "comment_id": comment })), Some((comment, None)));
        assert_eq!(quoted("engagement.like", &serde_json::json!({})), None);
    }

    #[test]
    fn an_edit_replaces_the_body_and_the_version() {
        let at = now();
        let (mut kind, mut title) = ("engagement.mention".to_owned(), Localized::same("mentioned you"));
        let (mut body, mut data) =
            (Localized::same("old"), serde_json::json!({ "comment_id": "x", "edited_at": null }));
        let edited = Current::Edited { body: Localized::same("new"), edited_at: at, becomes: None };
        assert!(edited.apply(&mut kind, &mut title, &mut body, &mut data));
        assert_eq!((kind.as_str(), body.ja.as_str()), ("engagement.mention", "new"));
        assert_eq!(serde_json::from_value::<Time>(data["edited_at"].clone()).unwrap(), at);
        let becomes = Some(("engagement.comment".to_owned(), Localized::same("commented")));
        let converted = Current::Edited { body: Localized::same("new"), edited_at: at, becomes };
        assert!(converted.apply(&mut kind, &mut title, &mut body, &mut data));
        assert_eq!((kind.as_str(), title.ja.as_str()), ("engagement.comment", "commented"));
        assert!(!Current::Unmentioned.apply(&mut kind, &mut title, &mut body, &mut data));
    }
}
