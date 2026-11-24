//! What a new reaction tells others: the owner's notification, grouped per
//! subject, and the "pop" on the screens showing the subject.
use tokuly_core::{
    outbound::{Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::engagement::{EngagementEvent, Reaction, ReactionAdded};

use tokuly_social::graph;

use super::{DISLIKE, EMOJI, KIND_LIKE, LIKE, Stored, rules::Rules};
use crate::{Engagement, stats, subjects::Subject, text};

/// Pops one subject's topic carries per second at most. A burst beyond
/// that is still counted; only the animation is skipped.
const POPS_PER_SECOND: i64 = 10;

/// The other people a notification names next to the newest one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Others {
    Exact(i64),
    /// More than [`OTHERS_SHOWN`]: "他99+人".
    More,
}

/// The largest exact number of others a notification names.
pub(super) const OTHERS_SHOWN: i64 = 99;

impl Others {
    fn text(self) -> String {
        match self {
            Self::Exact(count) => count.to_string(),
            Self::More => format!("{OTHERS_SHOWN}+"),
        }
    }

    fn is_none(self) -> bool {
        self == Self::Exact(0)
    }
}

impl Engagement {
    /// How many other people than `actor_id` and `owner_id` hold an applause
    /// reaction — a like, or for anything but a comment an emoji — on the
    /// subject: the "他N人" of the owner's notification. People in a block
    /// with the owner and people the owner muted are not counted, as in the
    /// lists the owner sees.
    ///
    /// Read through the pool before the reaction's transaction, so that it
    /// never runs while the subject's statistics row is locked, and bounded:
    /// on a subject that takes one reaction per person it is the counters
    /// minus the excluded people's own rows (looked up by user); on one that
    /// takes several it reads at most enough rows to tell 99 people from
    /// more.
    pub(super) async fn others_applauding(
        &self,
        subject: &Subject,
        rules: &Rules,
        actor_id: Uuid,
        owner_id: Uuid,
    ) -> AppResult<Others> {
        let mut excluded = graph::blocked_ids_either(&self.core, owner_id).await?;
        excluded.extend(self.muted_by(Some(owner_id)).await?);
        excluded.extend([actor_id, owner_id]);
        excluded.sort();
        excluded.dedup();
        let key = subject.key;
        let kinds: &[i32] = if key.is_comment() { &[LIKE] } else { &[LIKE, EMOJI] };
        if !rules.multiple {
            // One reaction per person: every applause row is a person.
            let row = stats::load(&self.core, &[key], self.cfg.reaction_emoji_per_subject).await?.of(key);
            let total = row.like_count + if key.is_comment() { 0 } else { row.emoji_count };
            let theirs: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM reactions
                 WHERE user_id = ANY($1) AND subject_type = $2 AND subject_id = $3 AND kind = ANY($4)",
            )
            .bind(&excluded)
            .bind(key.kind)
            .bind(key.id)
            .bind(kinds)
            .fetch_one(&self.core.db)
            .await?;
            let others = (total - theirs).max(0);
            return Ok(if others > OTHERS_SHOWN { Others::More } else { Others::Exact(others) });
        }
        // Several per person: a person holds at most `max_per_person` emoji
        // and one like, so this many rows always show more than 99 people.
        let rows = (OTHERS_SHOWN + 1) * (i64::from(rules.max_per_person) + 1);
        let people: i64 = sqlx::query_scalar(
            "SELECT count(DISTINCT user_id) FROM (
                 SELECT user_id FROM reactions
                 WHERE subject_type = $1 AND subject_id = $2 AND kind = ANY($3) AND user_id <> ALL($4)
                 LIMIT $5
             ) bounded",
        )
        .bind(key.kind)
        .bind(key.id)
        .bind(kinds)
        .bind(&excluded)
        .bind(rows)
        .fetch_one(&self.core.db)
        .await?;
        Ok(if people > OTHERS_SHOWN { Others::More } else { Others::Exact(people) })
    }
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
    others: Others,
    picture: Option<&str>,
) -> NotificationRequest {
    debug_assert_ne!(reaction.kind, DISLIKE);
    let count = others.text();
    let people = people(others);
    let title = match (reaction.kind == EMOJI, subject.key.is_comment(), others.is_none()) {
        (true, _, true) => Localized::new(
            format!("{actor_name}が{}でリアクションしたよ!", reaction.emoji),
            format!("{actor_name} reacted {}", reaction.emoji),
        ),
        (true, _, false) => Localized::new(
            format!("{actor_name}と他{count}人がリアクションしたよ! {}", reaction.emoji),
            format!("{actor_name} and {count} {people} reacted {}", reaction.emoji),
        ),
        (false, true, true) => {
            Localized::new(format!("{actor_name}がコメントにいいねしたよ!"), format!("{actor_name} liked your comment"))
        }
        (false, true, false) => Localized::new(
            format!("{actor_name}と他{count}人がコメントにいいねしたよ!"),
            format!("{actor_name} and {count} {people} liked your comment"),
        ),
        (false, false, true) => {
            Localized::new(format!("{actor_name}がいいねしたよ!"), format!("{actor_name} liked your post"))
        }
        (false, false, false) => Localized::new(
            format!("{actor_name}と他{count}人がリアクションしたよ!"),
            format!("{actor_name} and {count} {people} reacted to your post"),
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
        // At most 99; `others_more` when there are more.
        "others": match others { Others::Exact(count) => count, Others::More => OTHERS_SHOWN },
        "others_more": others == Others::More,
    }));
    if let Some(image_url) = &subject.info.image_url {
        request = request.image(image_url);
    }
    request
}

fn people(others: Others) -> &'static str {
    if others == Others::Exact(1) { "other" } else { "others" }
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
        let first = applause(owner, actor, "アリス", &note, &like, Others::Exact(0), None);
        assert_eq!(
            (first.kind.as_str(), first.title.ja.as_str(), first.body.ja.as_str()),
            (KIND_LIKE, "アリスがいいねしたよ!", "今日のランチ")
        );
        assert_eq!(first.collapse_key.as_deref(), Some(format!("engagement.like:3:{}", note.key.id).as_str()));
        assert_eq!((first.actor_id, first.user_id, first.path.as_str()), (Some(actor), owner, "/notes/abc"));
        assert_eq!(first.image_url.as_deref(), Some("https://media.test/a.webp"));

        let party = Stored { kind: EMOJI, emoji: "🎉".into() };
        assert_eq!(applause(owner, actor, "Alice", &note, &party, Others::Exact(0), None).title.en, "Alice reacted 🎉");
        let grouped = applause(owner, actor, "アリス", &note, &party, Others::Exact(2), None);
        assert_eq!(grouped.title.ja, "アリスと他2人がリアクションしたよ! 🎉");
        assert_eq!(
            applause(owner, actor, "Alice", &note, &party, Others::Exact(1), None).title.en,
            "Alice and 1 other reacted 🎉"
        );

        let neko = Stored { kind: EMOJI, emoji: ":neko:".into() };
        let custom =
            applause(owner, actor, "Alice", &note, &neko, Others::Exact(3), Some("https://media.test/neko.webp"));
        assert_eq!(custom.title.en, "Alice and 3 others reacted :neko:");
        assert_eq!(custom.data["emoji_image_url"], "https://media.test/neko.webp");
        assert_eq!((custom.data["emoji"].as_str(), custom.data["others"].as_i64()), (Some(":neko:"), Some(3)));

        let comment = subject(SubjectType::SUBJECT_TYPE_COMMENT);
        assert_eq!(
            applause(owner, actor, "Alice", &comment, &like, Others::Exact(0), None).title.en,
            "Alice liked your comment"
        );
        assert_eq!(
            applause(owner, actor, "アリス", &comment, &like, Others::Exact(4), None).title.ja,
            "アリスと他4人がコメントにいいねしたよ!"
        );
        let crowd = applause(owner, actor, "アリス", &note, &party, Others::More, None);
        assert_eq!(crowd.title.ja, "アリスと他99+人がリアクションしたよ! 🎉");
        assert_eq!((crowd.data["others"].as_i64(), crowd.data["others_more"].as_bool()), (Some(99), Some(true)));
    }
}
