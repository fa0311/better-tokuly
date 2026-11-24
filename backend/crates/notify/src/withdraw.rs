//! Withdrawing notifications about content that is gone.
//!
//! An inbox entry quotes what it is about ("Bob commented: …"). When that
//! content is deleted or taken down, the entry must not keep showing it. Each
//! entry records the IDs it refers to ([`refs_of`]): its subject and every
//! `*_id` value at the top level of its data. A deletion event of another
//! domain ([`withdrawn_by`]) names one ID, and every entry referring to it
//! leaves every inbox.
//!
//! The ID is also remembered (`notify_withdrawn`), because events are not
//! ordered: a request about a message can be handled after the message was
//! unsent, and a push can still be waiting in the queue. Both are dropped.
//!
//! Notices of the `system` category are never withdrawn: the one telling an
//! owner that their content was taken down refers to that content.
use serde::Deserialize;
use tokuly_core::{events::Event, outbound::NotificationRequest, prelude::*};
use tokuly_proto::common::SubjectType;

use crate::{Notify, locks};

/// The moderation topic: any subject type (except users) is withdrawn.
pub const CONTENT_REMOVED_TOPIC: &str = "admin.content.removed";

/// Deletion events of other domains and the field naming what was deleted.
pub const DELETION_TOPICS: &[(&str, &str)] = &[
    ("chat.message.deleted", "message_id"),
    ("engagement.comment.deleted", "comment_id"),
    ("notes.note.deleted", "note_id"),
    ("notes.story.deleted", "story_id"),
    ("community.thread.deleted", "thread_id"),
    ("video.playlist.deleted", "playlist_id"),
];

/// The category whose entries are never withdrawn.
pub(crate) const UNWITHDRAWABLE_CATEGORY: &str = "system";

#[derive(Deserialize)]
struct ContentRemoved {
    subject_type: i32,
    subject_id: Uuid,
}

/// The ID a consumed event withdraws, if it is one of the deletion topics.
pub fn withdrawn_by(event: &Event) -> AppResult<Option<Uuid>> {
    if event.topic == CONTENT_REMOVED_TOPIC {
        let removed: ContentRemoved = event.data()?;
        // A profile takedown removes a picture or a bio, not the person:
        // what they did stays in other people's inboxes.
        if removed.subject_type == SubjectType::SUBJECT_TYPE_USER as i32 {
            return Ok(None);
        }
        return Ok(Some(removed.subject_id));
    }
    let Some((_, field)) = DELETION_TOPICS.iter().find(|(topic, _)| *topic == event.topic) else {
        return Ok(None);
    };
    Ok(event.data.get(*field).and_then(|id| id.as_str()).and_then(|id| Uuid::parse_str(id).ok()))
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
    /// Handles a deletion event: removes every entry referring to the ID
    /// from every inbox, and remembers the ID. Idempotent.
    pub async fn withdraw(&self, ref_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        locks::lock(&mut tx, locks::key(ref_id)).await?;
        sqlx::query("INSERT INTO notify_withdrawn (ref_id) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(ref_id)
            .execute(&mut *tx)
            .await?;
        let removed: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "DELETE FROM notifications WHERE refs @> ARRAY[$1]::uuid[] AND category <> $2
             RETURNING user_id, id",
        )
        .bind(ref_id)
        .bind(UNWITHDRAWABLE_CATEGORY)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        self.announce_removed(removed).await
    }

    /// Tells each affected user's clients which of their entries went away.
    pub(crate) async fn announce_removed(&self, mut removed: Vec<(Uuid, Uuid)>) -> AppResult<()> {
        removed.sort_unstable();
        for chunk in removed.chunk_by(|a, b| a.0 == b.0) {
            let user_id = chunk[0].0;
            let ids: Vec<Uuid> = chunk.iter().map(|(_, id)| *id).collect();
            let unread = crate::inbox::unread_count(&self.core.db, user_id).await?;
            self.publish_removed(user_id, &ids, &unread).await;
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
        let withdrawn = |topic: &str, data| withdrawn_by(&event(topic, data)).unwrap();
        assert_eq!(withdrawn("chat.message.deleted", serde_json::json!({ "message_id": id })), Some(id));
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
}
