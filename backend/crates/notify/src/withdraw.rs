//! Withdrawing notifications about content that is gone or hidden.
//!
//! An inbox entry quotes what it is about ("Bob commented: …"). When that
//! content is deleted, taken down or hidden, the entry must not keep showing
//! it. Each entry records the IDs it refers to ([`refs_of`]): its subject and
//! every `*_id` value at the top level of its data, for a grouped entry those
//! of every event folded in since it was last read. An event of another
//! domain ([`withdrawal_of`], rules in [`RULES`]) names one ID, and the
//! entries referring to it leave the inboxes. A grouped entry that still
//! stands for other events stays, one event smaller (and without its text
//! when that was the withdrawn one).
//!
//! Content that is gone for good is also remembered (`notify_withdrawn`),
//! because events are not ordered: a request about a message can be handled
//! after the message was unsent, and a push can still be waiting in the
//! queue. Both are dropped. Content that is only hidden (it can come back) is
//! not remembered.
//!
//! Notices of the `system` category are never withdrawn: the one telling an
//! owner that their content was taken down refers to that content.
use std::collections::HashMap;

use serde::Deserialize;
use tokuly_core::{events::Event, outbound::NotificationRequest, prelude::*};
use tokuly_proto::{
    common::{SubjectType, Visibility},
    notify::UnreadCount,
};

use crate::{Notify, locks};

/// The moderation topic: any subject type (except users) is withdrawn.
pub const CONTENT_REMOVED_TOPIC: &str = "admin.content.removed";

/// How an event of another domain withdraws notifications.
pub struct Rule {
    pub topic: &'static str,
    /// The payload field naming the content.
    pub id_field: &'static str,
    /// Only entries of these kinds; empty = every kind.
    pub kinds: &'static [&'static str],
    /// The content is hidden, not gone: the user named by this payload field
    /// (its owner) keeps their entries, and nothing is remembered.
    pub hidden_except: Option<&'static str>,
    /// Only when this payload field holds `VISIBILITY_PRIVATE`.
    pub when_private: Option<&'static str>,
}

const fn gone(topic: &'static str, id_field: &'static str) -> Rule {
    Rule { topic, id_field, kinds: &[], hidden_except: None, when_private: None }
}

/// The events that withdraw notifications. Each producer's entries carry the
/// ID as their subject or as a `*_id` field of their data.
pub const RULES: &[Rule] = &[
    gone("chat.message.deleted", "message_id"),
    gone("engagement.comment.deleted", "comment_id"),
    gone("notes.note.deleted", "note_id"),
    gone("notes.story.deleted", "story_id"),
    gone("community.thread.deleted", "thread_id"),
    gone("video.playlist.deleted", "playlist_id"),
    gone("video.video.deleted", "video_id"),
    gone("shorts.short.deleted", "id"),
    gone("live.stream.deleted", "stream_id"),
    // The answer went away, the question may stay (unanswered again): only
    // the asker's "answered" entry quotes the answer. Not remembered, since
    // the question can be answered again.
    Rule { kinds: &["ask.answered"], hidden_except: Some("owner_id"), ..gone("ask.answer.deleted", "question_id") },
    // Back to draft: only the owner can see it.
    Rule { hidden_except: Some("owner_id"), ..gone("video.video.unpublished", "video_id") },
    // Made private: only the author can see it. A note narrowed to friends
    // is not withdrawn: this domain does not know who is a friend.
    Rule {
        hidden_except: Some("author_id"),
        when_private: Some("visibility"),
        ..gone("notes.note.visibility_changed", "note_id")
    },
];

/// The category whose entries are never withdrawn.
pub(crate) const UNWITHDRAWABLE_CATEGORY: &str = "system";

/// Entries changed per statement, so that content with thousands of
/// recipients never holds one long transaction.
const BATCH: i64 = 1000;

/// What an event withdraws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Withdrawal {
    pub ref_id: Uuid,
    /// Only entries of these kinds; empty = every kind.
    pub kinds: &'static [&'static str],
    /// This user keeps their entries (the owner of hidden content).
    pub except_user: Option<Uuid>,
    /// Gone for good: remembered, so late requests and queued pushes about
    /// it are dropped.
    pub remember: bool,
}

#[derive(Deserialize)]
struct ContentRemoved {
    subject_type: i32,
    subject_id: Uuid,
}

fn uuid_field(event: &Event, field: &str) -> Option<Uuid> {
    event.data.get(field).and_then(|id| id.as_str()).and_then(|id| Uuid::parse_str(id).ok())
}

/// What a consumed event withdraws, if anything.
pub fn withdrawal_of(event: &Event) -> AppResult<Option<Withdrawal>> {
    if event.topic == CONTENT_REMOVED_TOPIC {
        let removed: ContentRemoved = event.data()?;
        // A profile takedown removes a picture or a bio, not the person:
        // what they did stays in other people's inboxes.
        if removed.subject_type == SubjectType::SUBJECT_TYPE_USER as i32 {
            return Ok(None);
        }
        return Ok(Some(Withdrawal { ref_id: removed.subject_id, kinds: &[], except_user: None, remember: true }));
    }
    let Some(rule) = RULES.iter().find(|rule| rule.topic == event.topic) else {
        return Ok(None);
    };
    if let Some(field) = rule.when_private
        && event.data.get(field).and_then(|v| v.as_i64()) != Some(i64::from(Visibility::VISIBILITY_PRIVATE as i32))
    {
        return Ok(None);
    }
    let Some(ref_id) = uuid_field(event, rule.id_field) else {
        return Ok(None);
    };
    Ok(Some(Withdrawal {
        ref_id,
        kinds: rule.kinds,
        except_user: rule.hidden_except.and_then(|field| uuid_field(event, field)),
        remember: rule.hidden_except.is_none() && rule.kinds.is_empty(),
    }))
}

/// The IDs a notification refers to: its subject and the `*_id` values at
/// the top level of its data. Sorted, without duplicates.
pub fn refs_of(request: &NotificationRequest) -> Vec<Uuid> {
    let mut refs: Vec<Uuid> = request.subject.map(|(_, id)| id).into_iter().collect();
    if let Some(data) = request.data.as_object() {
        refs.extend(
            data.iter()
                .filter(|(key, _)| key.ends_with("_id"))
                .filter_map(|(_, value)| value.as_str())
                .filter_map(|value| Uuid::parse_str(value).ok()),
        );
    }
    refs.sort_unstable();
    refs.dedup();
    refs
}

/// Whether any of `refs` was withdrawn.
pub(crate) async fn any_withdrawn<'e, E: sqlx::PgExecutor<'e>>(executor: E, refs: &[Uuid]) -> AppResult<bool> {
    if refs.is_empty() {
        return Ok(false);
    }
    Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM notify_withdrawn WHERE ref_id = ANY($1))")
        .bind(refs)
        .fetch_one(executor)
        .await?)
}

impl Notify {
    /// Applies a withdrawal: entries referring to the ID leave the inboxes
    /// (grouped ones that still stand for other events shrink instead), in
    /// batches, and content that is gone is remembered. Idempotent.
    pub async fn withdraw(&self, withdrawal: &Withdrawal) -> AppResult<()> {
        let mut first = true;
        loop {
            let mut tx = self.core.db.begin().await?;
            locks::lock(&mut tx, locks::key(withdrawal.ref_id)).await?;
            if first && withdrawal.remember {
                sqlx::query("INSERT INTO notify_withdrawn (ref_id) VALUES ($1) ON CONFLICT DO NOTHING")
                    .bind(withdrawal.ref_id)
                    .execute(&mut *tx)
                    .await?;
            }
            first = false;
            // A grouped entry folded in other events too: it loses this one
            // (and its text, when this was the one it showed), and stays.
            // Content that is the entry's subject takes the whole entry.
            let shrunk = sqlx::query(
                "UPDATE notifications n SET
                    refs = array_remove(n.refs, $1),
                    group_count = n.group_count - 1,
                    body_ja = CASE WHEN shows THEN '' ELSE n.body_ja END,
                    body_en = CASE WHEN shows THEN '' ELSE n.body_en END,
                    image_url = CASE WHEN shows THEN NULL ELSE n.image_url END
                 FROM (
                     SELECT id, EXISTS (
                         SELECT 1 FROM jsonb_each_text(CASE WHEN jsonb_typeof(data) = 'object' THEN data ELSE '{}' END) e
                         WHERE e.key LIKE '%\\_id' AND e.value = $1::text
                     ) AS shows
                     FROM notifications
                     WHERE refs @> ARRAY[$1]::uuid[] AND category <> $2
                       AND ($3::uuid IS NULL OR user_id <> $3)
                       AND (cardinality($4::text[]) = 0 OR kind = ANY($4))
                       AND group_count > 1 AND subject_id IS DISTINCT FROM $1
                     LIMIT $5
                 ) t
                 WHERE n.id = t.id",
            )
            .bind(withdrawal.ref_id)
            .bind(UNWITHDRAWABLE_CATEGORY)
            .bind(withdrawal.except_user)
            .bind(withdrawal.kinds)
            .bind(BATCH)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            let removed: Vec<(Uuid, Uuid)> = sqlx::query_as(
                "DELETE FROM notifications WHERE id IN (
                     SELECT id FROM notifications
                     WHERE refs @> ARRAY[$1]::uuid[] AND category <> $2
                       AND ($3::uuid IS NULL OR user_id <> $3)
                       AND (cardinality($4::text[]) = 0 OR kind = ANY($4))
                       AND (group_count <= 1 OR subject_id = $1)
                     LIMIT $5
                 )
                 RETURNING user_id, id",
            )
            .bind(withdrawal.ref_id)
            .bind(UNWITHDRAWABLE_CATEGORY)
            .bind(withdrawal.except_user)
            .bind(withdrawal.kinds)
            .bind(BATCH)
            .fetch_all(&mut *tx)
            .await?;
            tx.commit().await?;
            let done = shrunk < BATCH as u64 && (removed.len() as i64) < BATCH;
            self.announce_removed(removed).await?;
            if done {
                return Ok(());
            }
        }
    }

    /// Tells each affected user's clients which of their entries went away,
    /// with their new unread counts (read in one query for all of them).
    pub(crate) async fn announce_removed(&self, mut removed: Vec<(Uuid, Uuid)>) -> AppResult<()> {
        if removed.is_empty() {
            return Ok(());
        }
        removed.sort_unstable();
        let mut users: Vec<Uuid> = removed.iter().map(|(user_id, _)| *user_id).collect();
        users.dedup();
        let counts: Vec<(Uuid, String, i64)> = sqlx::query_as(
            "SELECT user_id, category, count(*) FROM notifications
             WHERE user_id = ANY($1) AND read_at IS NULL GROUP BY user_id, category",
        )
        .bind(&users)
        .fetch_all(&self.core.db)
        .await?;
        let mut unread: HashMap<Uuid, UnreadCount> = HashMap::new();
        for (user_id, category, count) in counts {
            let count = count.clamp(0, i32::MAX as i64) as i32;
            let entry = unread.entry(user_id).or_default();
            entry.total = entry.total.saturating_add(count);
            entry.by_category.insert(category, count);
        }
        for chunk in removed.chunk_by(|a, b| a.0 == b.0) {
            let user_id = chunk[0].0;
            let ids: Vec<Uuid> = chunk.iter().map(|(_, id)| *id).collect();
            self.publish_removed(user_id, &ids, &unread.remove(&user_id).unwrap_or_default()).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tokuly_core::outbound::Localized;

    use super::*;

    fn event(topic: &str, data: serde_json::Value) -> Event {
        Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id: None, occurred_at: now(), data }
    }

    #[test]
    fn refs_are_the_subject_and_the_ids_in_the_data() {
        let (subject, comment, conversation) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let request = NotificationRequest::new(
            Uuid::now_v7(),
            "engagement.comment",
            Localized::same("t"),
            Localized::same("b"),
            "/",
        )
        .subject(SubjectType::SUBJECT_TYPE_NOTE, subject)
        .data(serde_json::json!({
            "comment_id": comment,
            "parent_id": null,
            "conversation_id": conversation.to_string(),
            "seq": 4,
            "note": subject,
            "nested": { "message_id": Uuid::now_v7() },
        }));
        let mut expected = vec![subject, comment, conversation];
        expected.sort_unstable();
        assert_eq!(refs_of(&request), expected);
    }

    #[test]
    fn deletion_events_name_what_they_withdraw() {
        let id = Uuid::now_v7();
        let withdrawn = |topic: &str, data| withdrawal_of(&event(topic, data)).unwrap().map(|w| w.ref_id);
        assert_eq!(withdrawn("chat.message.deleted", serde_json::json!({ "message_id": id })), Some(id));
        assert_eq!(withdrawn("shorts.short.deleted", serde_json::json!({ "id": id, "short_id": "x" })), Some(id));
        assert_eq!(
            withdrawn(
                CONTENT_REMOVED_TOPIC,
                serde_json::json!({ "subject_type": 3, "subject_id": id, "reason": "", "moderator_id": null })
            ),
            Some(id)
        );
        assert_eq!(
            withdrawn(CONTENT_REMOVED_TOPIC, serde_json::json!({ "subject_type": 1, "subject_id": id })),
            None,
            "a profile takedown withdraws nothing"
        );
        assert_eq!(withdrawn("chat.message.deleted", serde_json::json!({})), None);
        assert_eq!(withdrawn("something.else", serde_json::json!({ "message_id": id })), None);
    }

    #[test]
    fn hidden_content_is_withdrawn_for_everyone_but_its_owner_and_not_remembered() {
        let (id, owner) = (Uuid::now_v7(), Uuid::now_v7());
        let private = withdrawal_of(&event(
            "notes.note.visibility_changed",
            serde_json::json!({ "note_id": id, "author_id": owner, "visibility": 5 }),
        ))
        .unwrap()
        .unwrap();
        assert_eq!((private.ref_id, private.except_user, private.remember), (id, Some(owner), false));
        let friends = withdrawal_of(&event(
            "notes.note.visibility_changed",
            serde_json::json!({ "note_id": id, "author_id": owner, "visibility": 4 }),
        ))
        .unwrap();
        assert_eq!(friends, None, "narrowed to friends: not known here who still sees it");
        let deleted =
            withdrawal_of(&event("video.video.deleted", serde_json::json!({ "video_id": id, "owner_id": owner })))
                .unwrap()
                .unwrap();
        assert_eq!((deleted.except_user, deleted.remember), (None, true));
        let answer =
            withdrawal_of(&event("ask.answer.deleted", serde_json::json!({ "question_id": id, "owner_id": owner })))
                .unwrap()
                .unwrap();
        assert_eq!((answer.kinds, answer.remember), (&["ask.answered"][..], false));
    }
}
