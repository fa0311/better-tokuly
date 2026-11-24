//! What a new reaction tells others: the owner's notification, grouped per
//! subject, and the "pop" on the screens showing the subject.
use sqlx::PgConnection;
use tokuly_core::{
    outbound::{Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::engagement::{EngagementEvent, Reaction, ReactionAdded};

use super::{DISLIKE, EMOJI, KIND_LIKE, LIKE, Stored};
use crate::{Engagement, subjects::Subject, text};

/// Pops one subject's topic carries per second at most. A burst beyond
/// that is still counted; only the animation is skipped.
const POPS_PER_SECOND: i64 = 10;

/// How many other people (than `actor_id` and `owner_id`) hold an applause
/// reaction — a like, or for anything but a comment an emoji — on the
/// subject: the "他N人" of the owner's notification. Read in the
/// reaction's transaction, after its own row was written.
pub(super) async fn others_applauding(
    conn: &mut PgConnection,
    subject: &Subject,
    actor_id: Uuid,
    owner_id: Uuid,
) -> AppResult<i64> {
    let kinds: &[i32] = if subject.key.is_comment() { &[LIKE] } else { &[LIKE, EMOJI] };
    let others: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT user_id) FROM reactions
         WHERE subject_type = $1 AND subject_id = $2 AND kind = ANY($3) AND user_id <> $4 AND user_id <> $5",
    )
    .bind(subject.key.kind)
    .bind(subject.key.id)
    .bind(kinds)
    .bind(actor_id)
    .bind(owner_id)
    .fetch_one(conn)
    .await?;
    Ok(others)
}

/// The notification a subject's owner gets about a like or an emoji. One
/// entry per subject (collapse key): each new one replaces the previous and
/// names the newest person with the number of the others ("Aと他2人が…").
/// `picture` is the image of a custom emoji, for clients that draw it.
pub(super) fn applause(
    owner_id: Uuid,
    actor_id: Uuid,
    actor_name: &str,
    subject: &Subject,
    reaction: &Stored,
    others: i64,
    picture: Option<&str>,
) -> NotificationRequest {
    debug_assert_ne!(reaction.kind, DISLIKE);
    let title = match (reaction.kind == EMOJI, subject.key.is_comment(), others) {
        (true, _, 0) => Localized::new(
            format!("{actor_name}が{}でリアクションしたよ!", reaction.emoji),
            format!("{actor_name} reacted {}", reaction.emoji),
        ),
        (true, _, others) => Localized::new(
            format!("{actor_name}と他{others}人がリアクションしたよ! {}", reaction.emoji),
            format!("{actor_name} and {others} {} reacted {}", people(others), reaction.emoji),
        ),
        (false, true, 0) => {
            Localized::new(format!("{actor_name}がコメントにいいねしたよ!"), format!("{actor_name} liked your comment"))
        }
        (false, true, others) => Localized::new(
            format!("{actor_name}と他{others}人がコメントにいいねしたよ!"),
            format!("{actor_name} and {others} {} liked your comment", people(others)),
        ),
        (false, false, 0) => {
            Localized::new(format!("{actor_name}がいいねしたよ!"), format!("{actor_name} liked your post"))
        }
        (false, false, others) => Localized::new(
            format!("{actor_name}と他{others}人がリアクションしたよ!"),
            format!("{actor_name} and {others} {} reacted to your post", people(others)),
        ),
    };
    let mut request = NotificationRequest::new(
        owner_id,
        KIND_LIKE,
        title,
        Localized::same(text::excerpt(&subject.info.title, 100)),
        &subject.info.path,
    )
    .actor(actor_id)
    .subject(subject.key.subject_type(), subject.key.id)
    .collapse(format!("{KIND_LIKE}:{}:{}", subject.key.kind, subject.key.id))
    .data(serde_json::json!({
        "kind": reaction.kind,
        "emoji": reaction.emoji,
        "emoji_image_url": picture,
        "others": others,
    }));
    if let Some(image_url) = &subject.info.image_url {
        request = request.image(image_url);
    }
    request
}

fn people(count: i64) -> &'static str {
    if count == 1 { "other" } else { "others" }
}

impl Engagement {
    /// Shows a new like or emoji on the screens following the subject: a
    /// `reaction_added` event without who it was from. At most
    /// [`POPS_PER_SECOND`] per subject; beyond that nothing is sent (the
    /// counters follow anyway). Best effort.
    pub(super) async fn pop(&self, subject: &Subject, reaction: Reaction) {
        let second = chrono::Utc::now().timestamp();
        let key = format!("engagement:pop:{}:{}:{second}", subject.key.kind, subject.key.id);
        let mut conn = self.core.cache.conn();
        let counted: redis::RedisResult<i64> = redis::pipe()
            .atomic()
            .incr(&key, 1)
            .expire(&key, 2)
            .ignore()
            .query_async::<(i64,)>(&mut conn)
            .await
            .map(|(count,)| count);
        match counted {
            Ok(count) if count <= POPS_PER_SECOND => {}
            Ok(_) => return,
            Err(error) => {
                tracing::warn!(%error, "reaction pop throttle unavailable");
                return;
            }
        }
        let event = EngagementEvent {
            subject: subject.key.to_ref().into(),
            event: Some(ReactionAdded { reaction: reaction.into(), ..Default::default() }.into()),
            ..Default::default()
        };
        self.core.rt.notify_topic(&subject.key.topic(), &event).await;
    }
}

#[cfg(test)]
mod tests {
    use tokuly_core::subject::SubjectInfo;
    use tokuly_proto::common::SubjectType;

    use super::*;
    use crate::subjects::SubjectKey;

    #[test]
    fn applause_is_collapsed_per_subject_grouped_and_worded_by_kind() {
        let (owner, actor) = (Uuid::now_v7(), Uuid::now_v7());
        let subject = |kind| Subject {
            key: SubjectKey::new(kind, Uuid::now_v7()),
            info: SubjectInfo {
                id: Uuid::now_v7(),
                owner_id: owner,
                title: "今日のランチ".into(),
                path: "/notes/abc".into(),
                image_url: Some("https://media.test/a.webp".into()),
                interactions_enabled: true,
            },
        };
        let like = Stored { kind: LIKE, emoji: String::new() };
        let note = subject(SubjectType::SUBJECT_TYPE_NOTE);
        let first = applause(owner, actor, "アリス", &note, &like, 0, None);
        assert_eq!(
            (first.kind.as_str(), first.title.ja.as_str(), first.body.ja.as_str()),
            (KIND_LIKE, "アリスがいいねしたよ!", "今日のランチ")
        );
        assert_eq!(first.collapse_key.as_deref(), Some(format!("engagement.like:3:{}", note.key.id).as_str()));
        assert_eq!((first.actor_id, first.user_id, first.path.as_str()), (Some(actor), owner, "/notes/abc"));
        assert_eq!(first.image_url.as_deref(), Some("https://media.test/a.webp"));

        let party = Stored { kind: EMOJI, emoji: "🎉".into() };
        assert_eq!(applause(owner, actor, "Alice", &note, &party, 0, None).title.en, "Alice reacted 🎉");
        let grouped = applause(owner, actor, "アリス", &note, &party, 2, None);
        assert_eq!(grouped.title.ja, "アリスと他2人がリアクションしたよ! 🎉");
        assert_eq!(applause(owner, actor, "Alice", &note, &party, 1, None).title.en, "Alice and 1 other reacted 🎉");

        let neko = Stored { kind: EMOJI, emoji: ":neko:".into() };
        let custom = applause(owner, actor, "Alice", &note, &neko, 3, Some("https://media.test/neko.webp"));
        assert_eq!(custom.title.en, "Alice and 3 others reacted :neko:");
        assert_eq!(custom.data["emoji_image_url"], "https://media.test/neko.webp");
        assert_eq!((custom.data["emoji"].as_str(), custom.data["others"].as_i64()), (Some(":neko:"), Some(3)));

        let comment = subject(SubjectType::SUBJECT_TYPE_COMMENT);
        assert_eq!(applause(owner, actor, "Alice", &comment, &like, 0, None).title.en, "Alice liked your comment");
        assert_eq!(
            applause(owner, actor, "アリス", &comment, &like, 4, None).title.ja,
            "アリスと他4人がコメントにいいねしたよ!"
        );
    }
}
