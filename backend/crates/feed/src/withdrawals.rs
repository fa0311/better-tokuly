//! Entries about content that went away, or that fewer people may see now.
//!
//! An entry quotes what it is about: a note's text and picture, where it was
//! posted, a story's title and cover. Reading `activities` decides *who* may
//! see an entry, not whether the content behind it still exists or who may
//! see that content now, so other domains' events tell this domain
//! ([`RULES`]):
//!
//! - content that is gone (a deleted note, story, video, short, stream or
//!   stamp pack): every entry pointing at it is deleted, everyone's;
//! - a note whose audience changed: its author's entries about it get the
//!   note's new audience (narrower or wider);
//! - a story whose day is over: its owner's entry becomes the owner's own
//!   memory (friends no longer see the story, so not its title and cover);
//! - a video that went back to draft or became private: its owner's entry
//!   is the owner's own until the video is published to an audience again;
//! - a deleted answer to a question: the "answered" entries go, but the
//!   question can be answered again under the same ID, so nothing is
//!   remembered about it.
//!
//! Events are not ordered and can arrive twice. The latest change per
//! subject is remembered (`feed_subject_changes`) for as long as events can
//! still arrive late: an entry stored afterwards gets the same treatment,
//! and an older change delivered after a newer one is ignored (ordered by
//! the producer's own `changed_at` where it gives one, taken after it
//! locked the content, otherwise by the event's time). A subject that is
//! gone stays gone.
use tokuly_core::{events::Event, prelude::*};
use tokuly_proto::common::SubjectType;

use crate::{
    Feed,
    activity::{AUDIENCE_FRIENDS, AUDIENCE_PRIVATE, AUDIENCE_PUBLIC},
};

/// What an event says about the content entries point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The content is gone for good.
    Gone,
    /// The owner's entries about it are shown to this audience from now on.
    Audience(i16),
    /// The entries of this kind about it are deleted; nothing is remembered
    /// (the content can come back under the same ID).
    Withdrawn(&'static str),
}

/// How an event of another domain changes the entries about its content.
pub struct Rule {
    pub topic: &'static str,
    /// The payload field naming the content.
    pub id_field: &'static str,
    pub subject_type: SubjectType,
    /// The payload field naming the content's owner (for audience changes).
    pub owner_field: Option<&'static str>,
    /// The payload field holding when the change was made (RFC 3339), where
    /// the event's own time could put two racing changes in the wrong order.
    pub time_field: Option<&'static str>,
    pub change: fn(&Event) -> Option<Change>,
}

fn gone(_: &Event) -> Option<Change> {
    Some(Change::Gone)
}

/// A note's `tokuly.common.v1.Visibility` as the audience of its entries.
fn note_audience(event: &Event) -> Option<Change> {
    match event.data.get("visibility").and_then(serde_json::Value::as_i64)? {
        1 => Some(Change::Audience(AUDIENCE_PUBLIC)),
        // Friends, or followers (and friends): the feed's friends audience
        // is inside both.
        4 | 6 => Some(Change::Audience(AUDIENCE_FRIENDS)),
        // Private, or anything this build does not know: the narrowest.
        _ => Some(Change::Audience(AUDIENCE_PRIVATE)),
    }
}

fn owner_only(_: &Event) -> Option<Change> {
    Some(Change::Audience(AUDIENCE_PRIVATE))
}

/// A published video's `tokuly.common.v1.Visibility` as the audience of
/// its announcement, which the video domain always gives to friends (a
/// channel does not reveal its owner to everyone): friends while the video
/// is public or for friends, the owner alone otherwise (private, unlisted).
fn video_audience(event: &Event) -> Option<Change> {
    match event.data.get("visibility").and_then(serde_json::Value::as_i64)? {
        1 | 4 => Some(Change::Audience(AUDIENCE_FRIENDS)),
        _ => Some(Change::Audience(AUDIENCE_PRIVATE)),
    }
}

fn answer_withdrawn(_: &Event) -> Option<Change> {
    Some(Change::Withdrawn("ask.answered"))
}

const fn deleted(topic: &'static str, id_field: &'static str, subject_type: SubjectType) -> Rule {
    Rule { topic, id_field, subject_type, owner_field: None, time_field: None, change: gone }
}

/// The owner's entries about a video follow its publication state.
const fn video(topic: &'static str, change: fn(&Event) -> Option<Change>) -> Rule {
    Rule {
        topic,
        id_field: "video_id",
        subject_type: SubjectType::SUBJECT_TYPE_VIDEO,
        owner_field: Some("owner_id"),
        time_field: None,
        change,
    }
}

/// The events this domain follows, with the subject their producers put on
/// activity entries.
pub const RULES: &[Rule] = &[
    deleted("notes.note.deleted", "note_id", SubjectType::SUBJECT_TYPE_NOTE),
    deleted("notes.story.deleted", "story_id", SubjectType::SUBJECT_TYPE_STORY),
    deleted("video.video.deleted", "video_id", SubjectType::SUBJECT_TYPE_VIDEO),
    deleted("shorts.short.deleted", "id", SubjectType::SUBJECT_TYPE_SHORT),
    deleted("live.stream.deleted", "stream_id", SubjectType::SUBJECT_TYPE_LIVE_STREAM),
    deleted("stamp.pack.deleted", "pack_id", SubjectType::SUBJECT_TYPE_STAMP_PACK),
    Rule {
        topic: "notes.note.visibility_changed",
        id_field: "note_id",
        subject_type: SubjectType::SUBJECT_TYPE_NOTE,
        owner_field: Some("author_id"),
        time_field: Some("changed_at"),
        change: note_audience,
    },
    Rule {
        topic: "notes.story.expired",
        id_field: "story_id",
        subject_type: SubjectType::SUBJECT_TYPE_STORY,
        owner_field: Some("owner_id"),
        time_field: None,
        change: owner_only,
    },
    // Back to draft: the owner's own; published (again, or with another
    // visibility): as the visibility allows.
    video("video.video.unpublished", owner_only),
    video("video.video.published", video_audience),
    video("video.video.visibility_changed", video_audience),
    Rule {
        topic: "ask.answer.deleted",
        id_field: "question_id",
        subject_type: SubjectType::SUBJECT_TYPE_QUESTION,
        owner_field: None,
        time_field: None,
        change: answer_withdrawn,
    },
];

/// The topics of [`RULES`], for the consumer's subscription.
pub fn topics() -> impl Iterator<Item = &'static str> {
    RULES.iter().map(|rule| rule.topic)
}

/// What one consumed event changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectChange {
    pub subject_type: i32,
    pub subject_id: Uuid,
    pub owner_id: Option<Uuid>,
    pub change: Change,
    /// When the change was made: the producer's `time_field`, or the event's time.
    pub changed_at: Time,
}

fn uuid_field(event: &Event, field: &str) -> Option<Uuid> {
    event.data.get(field).and_then(|id| id.as_str()).and_then(|id| Uuid::parse_str(id).ok())
}

/// What `event` changes, if it is one of [`RULES`] and well formed.
pub fn subject_change(event: &Event) -> Option<SubjectChange> {
    let rule = RULES.iter().find(|rule| rule.topic == event.topic)?;
    let owner_id = match rule.owner_field {
        // An audience change without its owner cannot be applied to the right entries.
        Some(field) => Some(uuid_field(event, field)?),
        None => None,
    };
    Some(SubjectChange {
        subject_type: rule.subject_type as i32,
        subject_id: uuid_field(event, rule.id_field)?,
        owner_id,
        change: (rule.change)(event)?,
        changed_at: rule
            .time_field
            .and_then(|field| event.data.get(field))
            .and_then(|value| serde_json::from_value::<Time>(value.clone()).ok())
            .unwrap_or(event.occurred_at),
    })
}

impl Feed {
    /// Handles an event of [`RULES`]. Idempotent.
    pub async fn handle_subject_change(&self, event: &Event) -> AppResult<()> {
        let Some(change) = subject_change(event) else {
            tracing::warn!(event = %event.id, topic = %event.topic, "ignoring a malformed content event");
            return Ok(());
        };
        let audience = match change.change {
            Change::Gone => None,
            Change::Audience(audience) => Some(audience),
            Change::Withdrawn(kind) => {
                let owners: Vec<Uuid> = sqlx::query_scalar(
                    "DELETE FROM activities WHERE subject_type = $1 AND subject_id = $2 AND kind = $3 RETURNING user_id",
                )
                .bind(change.subject_type)
                .bind(change.subject_id)
                .bind(kind)
                .fetch_all(&self.core.db)
                .await?;
                self.invalidate_recaps_of(owners).await;
                return Ok(());
            }
        };
        let mut tx = self.core.db.begin().await?;
        lock_subject(&mut tx, change.subject_type, change.subject_id).await?;
        // Remember it, unless something newer (or the end of the content) is known already.
        let applies: bool = sqlx::query_scalar(
            "INSERT INTO feed_subject_changes AS c (subject_type, subject_id, audience, owner_id, changed_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (subject_type, subject_id) DO UPDATE
                 SET audience = EXCLUDED.audience, owner_id = EXCLUDED.owner_id,
                     changed_at = GREATEST(c.changed_at, EXCLUDED.changed_at), recorded_at = now()
                 WHERE c.audience IS NOT NULL AND (EXCLUDED.audience IS NULL OR c.changed_at <= EXCLUDED.changed_at)
             RETURNING true",
        )
        .bind(change.subject_type)
        .bind(change.subject_id)
        .bind(audience)
        .bind(change.owner_id)
        .bind(change.changed_at)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(false);
        if !applies {
            return Ok(());
        }
        let owners: Vec<Uuid> = match audience {
            None => {
                sqlx::query_scalar(
                    "DELETE FROM activities WHERE subject_type = $1 AND subject_id = $2 RETURNING user_id",
                )
                .bind(change.subject_type)
                .bind(change.subject_id)
                .fetch_all(&mut *tx)
                .await?
            }
            Some(audience) => {
                sqlx::query(
                    "UPDATE activities SET audience = $4
                     WHERE subject_type = $1 AND subject_id = $2 AND user_id = $3 AND audience <> $4",
                )
                .bind(change.subject_type)
                .bind(change.subject_id)
                .bind(change.owner_id)
                .bind(audience)
                .execute(&mut *tx)
                .await?;
                // Recaps count the owner's own log, whatever its audience.
                Vec::new()
            }
        };
        tx.commit().await?;
        self.invalidate_recaps_of(owners).await;
        Ok(())
    }

    async fn invalidate_recaps_of(&self, mut owners: Vec<Uuid>) {
        owners.sort_unstable();
        owners.dedup();
        for owner in owners {
            self.invalidate_recaps(owner).await;
        }
    }
}

/// Serializes the changes of one subject with the entries stored about it,
/// so an entry and a change arriving together cannot miss each other.
async fn lock_subject(tx: &mut sqlx::PgConnection, subject_type: i32, subject_id: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("feed:subject:{subject_type}:{subject_id}"))
        .execute(tx)
        .await?;
    Ok(())
}

/// What is known about the subject of a new entry by `user_id`, read (and
/// locked) on the ingesting transaction: `None` when nothing applies to it.
pub(crate) async fn remembered(
    tx: &mut sqlx::PgConnection,
    subject: Option<(i32, Uuid)>,
    user_id: Uuid,
) -> AppResult<Option<Change>> {
    let Some((subject_type, subject_id)) = subject else { return Ok(None) };
    lock_subject(&mut *tx, subject_type, subject_id).await?;
    let row: Option<(Option<i16>, Option<Uuid>)> = sqlx::query_as(
        "SELECT audience, owner_id FROM feed_subject_changes WHERE subject_type = $1 AND subject_id = $2",
    )
    .bind(subject_type)
    .bind(subject_id)
    .fetch_optional(tx)
    .await?;
    Ok(match row {
        None => None,
        Some((None, _)) => Some(Change::Gone),
        // An audience change concerns its owner's entries only.
        Some((Some(audience), owner)) if owner == Some(user_id) => Some(Change::Audience(audience)),
        Some(_) => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(topic: &str, data: serde_json::Value) -> Event {
        Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id: None, occurred_at: now(), data }
    }

    #[test]
    fn deletions_name_their_subject() {
        let id = Uuid::now_v7();
        let change = subject_change(&event("shorts.short.deleted", serde_json::json!({ "id": id }))).unwrap();
        assert_eq!(
            change,
            SubjectChange {
                subject_type: SubjectType::SUBJECT_TYPE_SHORT as i32,
                subject_id: id,
                owner_id: None,
                change: Change::Gone,
                changed_at: change.changed_at,
            }
        );
        assert_eq!(subject_change(&event("shorts.short.deleted", serde_json::json!({ "id": "nope" }))), None);
        assert_eq!(subject_change(&event("something.else", serde_json::json!({ "id": id }))), None);
    }

    #[test]
    fn a_note_visibility_becomes_the_audience_of_its_authors_entries() {
        let (note, author) = (Uuid::now_v7(), Uuid::now_v7());
        let audience = |visibility: i64| {
            subject_change(&event(
                "notes.note.visibility_changed",
                serde_json::json!({ "note_id": note, "author_id": author, "visibility": visibility }),
            ))
            .map(|change| (change.owner_id, change.change))
        };
        assert_eq!(audience(1), Some((Some(author), Change::Audience(AUDIENCE_PUBLIC))));
        assert_eq!(audience(4), Some((Some(author), Change::Audience(AUDIENCE_FRIENDS))));
        assert_eq!(audience(5), Some((Some(author), Change::Audience(AUDIENCE_PRIVATE))));
        assert_eq!(audience(99), Some((Some(author), Change::Audience(AUDIENCE_PRIVATE))), "unknown: narrowest");
        let ownerless = event("notes.note.visibility_changed", serde_json::json!({ "note_id": note, "visibility": 1 }));
        assert_eq!(subject_change(&ownerless), None);
    }

    #[test]
    fn a_note_change_is_ordered_by_when_it_was_made_not_by_its_event() {
        let (note, author) = (Uuid::now_v7(), Uuid::now_v7());
        let made = now() - chrono::Duration::seconds(3);
        let change = subject_change(&event(
            "notes.note.visibility_changed",
            serde_json::json!({ "note_id": note, "author_id": author, "visibility": 4, "changed_at": made }),
        ))
        .unwrap();
        assert_eq!(change.changed_at, made);
        let video = subject_change(&event(
            "video.video.published",
            serde_json::json!({ "video_id": note, "owner_id": author, "visibility": 5 }),
        ))
        .unwrap();
        assert_eq!(video.change, Change::Audience(AUDIENCE_PRIVATE), "published, but private");
    }

    #[test]
    fn every_rule_has_a_distinct_topic() {
        let mut topics: Vec<&str> = topics().collect();
        topics.sort_unstable();
        topics.dedup();
        assert_eq!(topics.len(), RULES.len());
    }
}
