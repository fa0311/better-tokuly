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
//! Content whose audience narrowed (a comment hidden, a note narrowed to
//! friends) is withdrawn from everyone but its owner and those who may still
//! see it ([`AUDIENCE_RULES`]). Who that is, and when it changed, is
//! remembered (`notify_audiences`) for as long as events can arrive late:
//! a request about it from before the change that is handled after it, and
//! a push still queued from before it, are dropped for everyone else. Two
//! changes of one subject are ordered by the producer's `changed_at`: an
//! older change handled after a newer one does nothing, and a change only
//! removes entries about what happened before it.
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
}

const fn gone(topic: &'static str, id_field: &'static str) -> Rule {
    Rule { topic, id_field, kinds: &[], hidden_except: None }
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
];

/// How an event that changes who may see content withdraws notifications.
/// The payload names the content, its owner, `visible_to` (who besides the
/// owner may still see it, `null` = everyone) and `changed_at` (when the
/// change was made, taken while the content was locked).
pub struct AudienceRule {
    pub topic: &'static str,
    /// The payload field naming the content.
    pub id_field: &'static str,
    /// The payload field naming its owner, who keeps their entries.
    pub owner_field: &'static str,
}

/// The events that narrow (or widen) who may see content.
pub const AUDIENCE_RULES: &[AudienceRule] = &[
    // Hidden by the subject owner, by a block, or held for review again.
    AudienceRule { topic: "engagement.comment.hidden", id_field: "comment_id", owner_field: "author_id" },
    // Narrowed to friends or made private (or opened up again): who may
    // still see it is the author's friends, the author of the note a reply
    // answers, or nobody else.
    AudienceRule { topic: "notes.note.visibility_changed", id_field: "note_id", owner_field: "author_id" },
    // Shared with another audience, or the owner locked (or unlocked) their
    // account: who may still watch it is in `visible_to`.
    AudienceRule { topic: "video.video.visibility_changed", id_field: "video_id", owner_field: "owner_id" },
    AudienceRule { topic: "shorts.short.visibility_changed", id_field: "id", owner_field: "owner_id" },
    AudienceRule { topic: "live.stream.visibility_changed", id_field: "stream_id", owner_field: "owner_id" },
];

/// The category whose entries are never withdrawn.
pub(crate) const UNWITHDRAWABLE_CATEGORY: &str = "system";

/// Entries changed per statement, so that content with thousands of
/// recipients never holds one long transaction.
const BATCH: i64 = 1000;

/// Whether [`Notify::withdraw_batch`] left work for another batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Batch {
    More,
    Done,
}

/// What an event withdraws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Withdrawal {
    pub ref_id: Uuid,
    /// Only entries of these kinds; empty = every kind.
    pub kinds: &'static [&'static str],
    /// These users keep their entries: the owner of hidden content, and
    /// whoever may still see it.
    pub except_users: Vec<Uuid>,
    /// Gone for good: remembered, so late requests and queued pushes about
    /// it are dropped.
    pub remember: bool,
    /// The content's audience changed: remembered and ordered by its time.
    pub audience: Option<Audience>,
}

/// Who may see content from a moment on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Audience {
    pub owner_id: Uuid,
    /// Who besides the owner; `None` = everyone.
    pub visible_to: Option<Vec<Uuid>>,
    pub changed_at: Time,
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
        return Ok(Some(Withdrawal {
            ref_id: removed.subject_id,
            kinds: &[],
            except_users: Vec::new(),
            remember: true,
            audience: None,
        }));
    }
    if let Some(rule) = AUDIENCE_RULES.iter().find(|rule| rule.topic == event.topic) {
        return Ok(audience_change(event, rule));
    }
    let Some(rule) = RULES.iter().find(|rule| rule.topic == event.topic) else {
        return Ok(None);
    };
    let Some(ref_id) = uuid_field(event, rule.id_field) else {
        return Ok(None);
    };
    Ok(Some(Withdrawal {
        ref_id,
        kinds: rule.kinds,
        except_users: rule.hidden_except.and_then(|field| uuid_field(event, field)).into_iter().collect(),
        remember: rule.hidden_except.is_none() && rule.kinds.is_empty(),
        audience: None,
    }))
}

/// The withdrawal of an audience change: everyone but the owner and
/// `visible_to` loses what happened up to `changed_at`.
fn audience_change(event: &Event, rule: &AudienceRule) -> Option<Withdrawal> {
    let (Some(ref_id), Some(owner_id)) = (uuid_field(event, rule.id_field), uuid_field(event, rule.owner_field)) else {
        return None;
    };
    let visible_to: Option<Vec<Uuid>> = match event.data.get("visible_to") {
        // An event from before producers named the audience: only "nobody
        // else" is known for certain.
        None if is_private(event) => Some(Vec::new()),
        None => return None,
        Some(value) => serde_json::from_value(value.clone()).ok()?,
    };
    let changed_at = event
        .data
        .get("changed_at")
        .and_then(|value| serde_json::from_value::<Time>(value.clone()).ok())
        .unwrap_or(event.occurred_at);
    let mut except_users = visible_to.clone().unwrap_or_default();
    except_users.push(owner_id);
    except_users.sort_unstable();
    except_users.dedup();
    Some(Withdrawal {
        ref_id,
        kinds: &[],
        except_users,
        remember: false,
        audience: Some(Audience { owner_id, visible_to, changed_at }),
    })
}

/// Whether a note event says the note is private.
fn is_private(event: &Event) -> bool {
    event.data.get("visibility").and_then(|v| v.as_i64()) == Some(i64::from(Visibility::VISIBILITY_PRIVATE as i32))
}

/// Whether, for `user_id`, any of `refs` was hidden after `occurred_at`: a
/// request or push from before an audience change that left them out.
pub(crate) async fn hidden_since<'e, E: sqlx::PgExecutor<'e>>(
    executor: E,
    refs: &[Uuid],
    user_id: Uuid,
    occurred_at: Time,
) -> AppResult<bool> {
    if refs.is_empty() {
        return Ok(false);
    }
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM notify_audiences
             WHERE ref_id = ANY($1) AND visible_to IS NOT NULL AND changed_at >= $3
               AND owner_id <> $2 AND NOT $2 = ANY(visible_to)
         )",
    )
    .bind(refs)
    .bind(user_id)
    .bind(occurred_at)
    .fetch_one(executor)
    .await?)
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
        while self.withdraw_batch(withdrawal, first).await? == Batch::More {
            first = false;
        }
        Ok(())
    }

    /// One batch of [`Self::withdraw`], in a transaction of its own; the
    /// first one also records what is remembered. A later batch of an
    /// audience change stops when a newer change of the same content was
    /// recorded in the meantime: from then on, that one decides.
    #[doc(hidden)]
    pub async fn withdraw_batch(&self, withdrawal: &Withdrawal, first: bool) -> AppResult<Batch> {
        let until = withdrawal.audience.as_ref().map(|audience| audience.changed_at);
        {
            let mut tx = self.core.db.begin().await?;
            locks::lock(&mut tx, locks::key(withdrawal.ref_id)).await?;
            if first && withdrawal.remember {
                sqlx::query("INSERT INTO notify_withdrawn (ref_id) VALUES ($1) ON CONFLICT DO NOTHING")
                    .bind(withdrawal.ref_id)
                    .execute(&mut *tx)
                    .await?;
            }
            if first && let Some(audience) = &withdrawal.audience {
                // Only the latest change counts. The same one again (a
                // redelivery, or a retry after a batch failed) carries on.
                let latest: bool = sqlx::query_scalar(
                    "INSERT INTO notify_audiences AS a (ref_id, owner_id, visible_to, changed_at)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT (ref_id) DO UPDATE SET
                         owner_id = EXCLUDED.owner_id, visible_to = EXCLUDED.visible_to,
                         changed_at = EXCLUDED.changed_at, recorded_at = now()
                     WHERE a.changed_at <= EXCLUDED.changed_at
                     RETURNING true",
                )
                .bind(withdrawal.ref_id)
                .bind(audience.owner_id)
                .bind(&audience.visible_to)
                .bind(audience.changed_at)
                .fetch_optional(&mut *tx)
                .await?
                .unwrap_or(false);
                if !latest || audience.visible_to.is_none() {
                    // Older than what is known, or open to everyone: nothing to take away.
                    tx.commit().await?;
                    return Ok(Batch::Done);
                }
            }
            if !first && let Some(audience) = &withdrawal.audience {
                let recorded: Option<Time> =
                    sqlx::query_scalar("SELECT changed_at FROM notify_audiences WHERE ref_id = $1")
                        .bind(withdrawal.ref_id)
                        .fetch_optional(&mut *tx)
                        .await?;
                if recorded != Some(audience.changed_at) {
                    tx.commit().await?;
                    return Ok(Batch::Done);
                }
            }
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
                       AND NOT user_id = ANY($3)
                       AND (cardinality($4::text[]) = 0 OR kind = ANY($4))
                       AND ($6::timestamptz IS NULL OR occurred_at <= $6)
                       AND group_count > 1 AND subject_id IS DISTINCT FROM $1
                     LIMIT $5
                 ) t
                 WHERE n.id = t.id",
            )
            .bind(withdrawal.ref_id)
            .bind(UNWITHDRAWABLE_CATEGORY)
            .bind(&withdrawal.except_users)
            .bind(withdrawal.kinds)
            .bind(BATCH)
            .bind(until)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            let removed: Vec<(Uuid, Uuid)> = sqlx::query_as(
                "DELETE FROM notifications WHERE id IN (
                     SELECT id FROM notifications
                     WHERE refs @> ARRAY[$1]::uuid[] AND category <> $2
                       AND NOT user_id = ANY($3)
                       AND (cardinality($4::text[]) = 0 OR kind = ANY($4))
                       AND ($6::timestamptz IS NULL OR occurred_at <= $6)
                       AND (group_count <= 1 OR subject_id = $1)
                     LIMIT $5
                 )
                 RETURNING user_id, id",
            )
            .bind(withdrawal.ref_id)
            .bind(UNWITHDRAWABLE_CATEGORY)
            .bind(&withdrawal.except_users)
            .bind(withdrawal.kinds)
            .bind(BATCH)
            .bind(until)
            .fetch_all(&mut *tx)
            .await?;
            tx.commit().await?;
            let done = shrunk < BATCH as u64 && (removed.len() as i64) < BATCH;
            self.announce_removed(removed).await?;
            Ok(if done { Batch::Done } else { Batch::More })
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
        assert_eq!((private.ref_id, private.except_users, private.remember), (id, vec![owner], false));
        let old_friends = withdrawal_of(&event(
            "notes.note.visibility_changed",
            serde_json::json!({ "note_id": id, "author_id": owner, "visibility": 4 }),
        ))
        .unwrap();
        assert_eq!(old_friends, None, "narrowed to friends without saying who: not known here who still sees it");
        let friend = Uuid::now_v7();
        let friends = withdrawal_of(&event(
            "notes.note.visibility_changed",
            serde_json::json!({ "note_id": id, "author_id": owner, "visibility": 4, "visible_to": [friend] }),
        ))
        .unwrap()
        .unwrap();
        assert!(friends.except_users.contains(&friend) && friends.except_users.contains(&owner));
        let public = withdrawal_of(&event(
            "notes.note.visibility_changed",
            serde_json::json!({ "note_id": id, "author_id": owner, "visibility": 1, "visible_to": null }),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(public.audience.unwrap().visible_to, None, "open to everyone again: remembered, nothing taken");
        let deleted =
            withdrawal_of(&event("video.video.deleted", serde_json::json!({ "video_id": id, "owner_id": owner })))
                .unwrap()
                .unwrap();
        assert_eq!((deleted.except_users, deleted.remember), (vec![], true));
        let answer =
            withdrawal_of(&event("ask.answer.deleted", serde_json::json!({ "question_id": id, "owner_id": owner })))
                .unwrap()
                .unwrap();
        assert_eq!((answer.kinds, answer.remember), (&["ask.answered"][..], false));
    }

    #[test]
    fn videos_shorts_and_streams_narrowed_to_an_audience_stay_with_it() {
        let (id, owner, follower) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let at = now() - chrono::Duration::seconds(2);
        for (topic, id_field) in [
            ("video.video.visibility_changed", "video_id"),
            ("shorts.short.visibility_changed", "id"),
            ("live.stream.visibility_changed", "stream_id"),
        ] {
            let narrowed = withdrawal_of(&event(
                topic,
                serde_json::json!({ id_field: id, "owner_id": owner, "visibility": 6, "visible_to": [follower], "changed_at": at }),
            ))
            .unwrap()
            .unwrap_or_else(|| panic!("{topic} withdraws"));
            let mut keep = vec![owner, follower];
            keep.sort_unstable();
            assert_eq!((narrowed.ref_id, narrowed.except_users), (id, keep), "{topic}");
            assert_eq!(narrowed.audience.unwrap().changed_at, at, "{topic} is ordered by its own time");
            let opened = withdrawal_of(&event(
                topic,
                serde_json::json!({ id_field: id, "owner_id": owner, "visibility": 1, "visible_to": null, "changed_at": at }),
            ))
            .unwrap()
            .unwrap();
            assert_eq!(opened.audience.unwrap().visible_to, None, "{topic}: open again");
            assert!(crate::consumer::TOPICS.contains(&topic), "{topic} is consumed");
        }
    }

    #[test]
    fn a_hidden_comment_stays_with_its_author_and_whoever_still_sees_it() {
        let (comment, author, owner) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let at = now() - chrono::Duration::seconds(3);
        let hidden = withdrawal_of(&event(
            "engagement.comment.hidden",
            serde_json::json!({ "comment_id": comment, "author_id": author, "visible_to": [owner], "changed_at": at }),
        ))
        .unwrap()
        .unwrap();
        let mut keep = vec![author, owner];
        keep.sort_unstable();
        assert_eq!((hidden.ref_id, hidden.except_users, hidden.remember), (comment, keep, false));
        assert_eq!(hidden.audience, Some(Audience { owner_id: author, visible_to: Some(vec![owner]), changed_at: at }));
        // Without its author it cannot be applied to the right entries.
        let ownerless = withdrawal_of(&event(
            "engagement.comment.hidden",
            serde_json::json!({ "comment_id": comment, "visible_to": [] }),
        ))
        .unwrap();
        assert_eq!(ownerless, None);
    }
}
