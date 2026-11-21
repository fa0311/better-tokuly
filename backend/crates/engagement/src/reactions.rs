//! Reactions: a user's one like, dislike or emoji on a subject.
//!
//! The row in `reactions` and the counters derived from it
//! (`subject_stats`, `reaction_emoji_counts`, and for comments the
//! `like_count` / `dislike_count` on the comment itself) change in one
//! transaction; see [`replace`].
use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use tokuly_core::{
    analytics::AnalyticsEvent,
    events,
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    common::PageInfo,
    engagement::{Reaction, ReactionKind, ReactionSummary},
};

use crate::{
    Engagement,
    effects::Effects,
    events::{REACTION_CHANGED, ReactionChanged},
    stats::{self, Counter},
    subjects::{LooseSubject, Subject, SubjectKey},
    text,
};

pub(crate) const KIND_LIKE: &str = "engagement.like";

/// Reactions one user may set or remove per minute.
const CHANGES_PER_MINUTE: u32 = 120;
/// How long liking the same subject again sends its owner no second notification.
const LIKE_NOTICE_WINDOW: Duration = Duration::from_secs(24 * 3600);

const LIKE: i32 = ReactionKind::REACTION_KIND_LIKE as i32;
const DISLIKE: i32 = ReactionKind::REACTION_KIND_DISLIKE as i32;
const EMOJI: i32 = ReactionKind::REACTION_KIND_EMOJI as i32;

const _: () = {
    // The migration's CHECK constraints use these numbers.
    assert!(LIKE == 1 && DISLIKE == 2 && EMOJI == 3);
};

/// A reaction as stored: the kind number and, for kind EMOJI, the emoji.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct Stored {
    pub kind: i32,
    pub emoji: String,
}

impl Stored {
    pub(crate) fn to_proto(&self) -> Reaction {
        Reaction { kind: self.kind.into(), emoji: self.emoji.clone(), ..Default::default() }
    }

    fn counter(&self) -> Counter {
        match self.kind {
            LIKE => Counter::Like,
            DISLIKE => Counter::Dislike,
            _ => Counter::Emoji,
        }
    }

    /// Whether the owner of the subject is told about it: likes and emoji
    /// are, dislikes never.
    fn is_applause(&self) -> bool {
        self.kind != DISLIKE
    }
}

/// Checks the reaction of a `SetReaction` request.
pub(crate) fn requested(reaction: Option<&Reaction>) -> AppResult<Stored> {
    let kind = reaction.and_then(|reaction| reaction.kind.as_known()).unwrap_or(ReactionKind::REACTION_KIND_UNSPECIFIED);
    match kind {
        ReactionKind::REACTION_KIND_LIKE => Ok(Stored { kind: LIKE, emoji: String::new() }),
        ReactionKind::REACTION_KIND_DISLIKE => Ok(Stored { kind: DISLIKE, emoji: String::new() }),
        ReactionKind::REACTION_KIND_EMOJI => {
            let emoji = text::single_emoji(reaction.map(|reaction| reaction.emoji.as_str()).unwrap_or_default())?;
            Ok(Stored { kind: EMOJI, emoji })
        }
        ReactionKind::REACTION_KIND_UNSPECIFIED => {
            Err(AppError::invalid("REACTION_INVALID", "a reaction is a like, a dislike or an emoji"))
        }
    }
}

/// The reactions of `user_id` to `keys`.
pub(crate) async fn of_user(core: &Core, user_id: Uuid, keys: &[SubjectKey]) -> AppResult<HashMap<SubjectKey, Reaction>> {
    if keys.is_empty() {
        return Ok(HashMap::new());
    }
    let (kinds, ids): (Vec<i32>, Vec<Uuid>) = keys.iter().map(|key| (key.kind, key.id)).unzip();
    let rows: Vec<(i32, Uuid, i32, String)> = sqlx::query_as(
        "SELECT subject_type, subject_id, kind, emoji FROM reactions
         WHERE user_id = $1 AND (subject_type, subject_id) IN (SELECT * FROM unnest($2::integer[], $3::uuid[]))",
    )
    .bind(user_id)
    .bind(&kinds)
    .bind(&ids)
    .fetch_all(&core.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(subject_type, subject_id, kind, emoji)| (SubjectKey { kind: subject_type, id: subject_id }, Stored { kind, emoji }.to_proto()))
        .collect())
}

/// The reactions of `user_id` to comments, by comment ID.
pub(crate) async fn of_user_on_comments(core: &Core, user_id: Uuid, comment_ids: &[Uuid]) -> AppResult<HashMap<Uuid, Reaction>> {
    let keys: Vec<SubjectKey> = comment_ids.iter().map(|id| SubjectKey::of_comment(*id)).collect();
    Ok(of_user(core, user_id, &keys).await?.into_iter().map(|(key, reaction)| (key.id, reaction)).collect())
}

/// A reaction entered (`delta` 1) or left (`delta` -1) the counters of its
/// subject. A comment's own counts go first: every path locks the comment
/// row before the statistics row. Returns `false` when the subject is a
/// comment that no longer exists (nothing is counted then).
async fn count(
    conn: &mut PgConnection,
    effects: &mut Effects,
    key: SubjectKey,
    owner_id: Option<Uuid>,
    reaction: &Stored,
    delta: i64,
) -> AppResult<bool> {
    if key.is_comment() && reaction.kind != EMOJI {
        let (likes, dislikes) = if reaction.kind == LIKE { (delta, 0) } else { (0, delta) };
        let counted = sqlx::query(
            "UPDATE comments SET like_count = GREATEST(like_count + $2, 0), dislike_count = GREATEST(dislike_count + $3, 0) WHERE id = $1",
        )
        .bind(key.id)
        .bind(likes)
        .bind(dislikes)
        .execute(&mut *conn)
        .await?;
        if counted.rows_affected() == 0 {
            return Ok(false);
        }
    }
    stats::add(conn, key, owner_id, reaction.counter(), delta).await?;
    if reaction.kind == EMOJI {
        stats::add_emoji(conn, key, &reaction.emoji, delta).await?;
    }
    effects.count(key, reaction.counter(), delta);
    Ok(true)
}

/// Takes a reaction whose row was deleted by other means (its user's
/// account was erased) out of the counters of its subject.
pub(crate) async fn uncount(conn: &mut PgConnection, effects: &mut Effects, key: SubjectKey, reaction: &Stored) -> AppResult<()> {
    count(conn, effects, key, None, reaction, -1).await?;
    Ok(())
}

/// Makes `new` the reaction of `user_id` to a subject (`None`: none) and
/// moves the counters accordingly. Returns the reaction it replaced.
///
/// `owner_id` is the subject's owner, when the subject resolved. An
/// advisory lock per user and subject serializes the read-then-write, so
/// two taps at once count once.
pub(crate) async fn replace(
    conn: &mut PgConnection,
    effects: &mut Effects,
    user_id: Uuid,
    key: SubjectKey,
    owner_id: Option<Uuid>,
    new: Option<&Stored>,
) -> AppResult<Option<Stored>> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("engagement:reaction:{user_id}:{}:{}", key.kind, key.id))
        .execute(&mut *conn)
        .await?;
    let previous: Option<Stored> =
        sqlx::query_as("SELECT kind, emoji FROM reactions WHERE user_id = $1 AND subject_type = $2 AND subject_id = $3")
            .bind(user_id)
            .bind(key.kind)
            .bind(key.id)
            .fetch_optional(&mut *conn)
            .await?;
    if previous.as_ref() == new {
        return Ok(previous);
    }
    match new {
        Some(new) => {
            sqlx::query(
                "INSERT INTO reactions (user_id, subject_type, subject_id, kind, emoji) VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (user_id, subject_type, subject_id) DO UPDATE SET kind = $4, emoji = $5, created_at = now()",
            )
            .bind(user_id)
            .bind(key.kind)
            .bind(key.id)
            .bind(new.kind)
            .bind(&new.emoji)
            .execute(&mut *conn)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM reactions WHERE user_id = $1 AND subject_type = $2 AND subject_id = $3")
                .bind(user_id)
                .bind(key.kind)
                .bind(key.id)
                .execute(&mut *conn)
                .await?;
        }
    }
    if let Some(previous) = &previous {
        count(conn, effects, key, owner_id, previous, -1).await?;
    }
    if let Some(new) = new {
        // The comment was deleted while this reaction was on its way: the
        // error rolls the reaction back with the rest of the transaction.
        ensure!(
            count(conn, effects, key, owner_id, new, 1).await?,
            not_found,
            "SUBJECT_NOT_FOUND",
            "the item does not exist or is not visible to you"
        );
    }
    Ok(previous)
}

fn changed(user_id: Uuid, key: SubjectKey, owner_id: Option<Uuid>, new: Option<&Stored>, previous: Option<&Stored>) -> ReactionChanged {
    ReactionChanged {
        user_id,
        subject_type: key.kind,
        subject_id: key.id,
        subject_owner_id: owner_id,
        kind: new.map(|reaction| reaction.kind).unwrap_or_default(),
        emoji: new.map(|reaction| reaction.emoji.clone()).unwrap_or_default(),
        previous_kind: previous.map(|reaction| reaction.kind).unwrap_or_default(),
        previous_emoji: previous.map(|reaction| reaction.emoji.clone()).unwrap_or_default(),
    }
}

/// The notification a subject's owner gets about a like or an emoji.
fn applause(owner_id: Uuid, actor_id: Uuid, actor_name: &str, subject: &Subject, reaction: &Stored) -> NotificationRequest {
    let title = if reaction.kind == EMOJI {
        Localized::new(format!("{actor_name}が{}でリアクションしたよ!", reaction.emoji), format!("{actor_name} reacted {}", reaction.emoji))
    } else if subject.key.is_comment() {
        Localized::new(format!("{actor_name}がコメントにいいねしたよ!"), format!("{actor_name} liked your comment"))
    } else {
        Localized::new(format!("{actor_name}がいいねしたよ!"), format!("{actor_name} liked your post"))
    };
    let mut request = NotificationRequest::new(owner_id, KIND_LIKE, title, Localized::same(text::excerpt(&subject.info.title, 100)), &subject.info.path)
        .actor(actor_id)
        .subject(subject.key.subject_type(), subject.key.id)
        .collapse(format!("{KIND_LIKE}:{}:{}", subject.key.kind, subject.key.id));
    if let Some(image_url) = &subject.info.image_url {
        request = request.image(image_url);
    }
    request
}

/// Someone who reacted to a subject.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ReactorRow {
    pub user_id: Uuid,
    pub kind: i32,
    pub emoji: String,
    pub created_at: DateTime<Utc>,
}

/// A subject a user reacted to.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ReactedRow {
    pub subject_type: i32,
    pub subject_id: Uuid,
    pub kind: i32,
    pub emoji: String,
    pub created_at: DateTime<Utc>,
}

impl ReactedRow {
    pub(crate) fn key(&self) -> SubjectKey {
        SubjectKey { kind: self.subject_type, id: self.subject_id }
    }
}

impl Engagement {
    /// Reacts to a subject, replacing the caller's previous reaction
    /// (`ReactionService.SetReaction`).
    pub(crate) async fn set_reaction(&self, user_id: Uuid, subject: &Subject, new: &Stored) -> AppResult<()> {
        ensure!(subject.info.interactions_enabled, precondition, "INTERACTIONS_DISABLED", "reactions are turned off here");
        ensure!(
            !(subject.key.is_comment() && new.kind == EMOJI),
            invalid,
            "EMOJI_NOT_ALLOWED",
            "comments take likes and dislikes, not emoji"
        );
        self.core.cache.rate_limit(&format!("engagement:reaction:{user_id}"), CHANGES_PER_MINUTE, Duration::from_secs(60)).await?;
        let owner_to_tell = subject.owner().filter(|owner_id| *owner_id != user_id && new.is_applause());
        let actor_name = match owner_to_tell {
            Some(_) => {
                let user = tokuly_identity::summary(&self.core, user_id).await?;
                if user.display_name.is_empty() { format!("@{}", user.handle) } else { user.display_name }
            }
            None => String::new(),
        };

        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        let owner_id = Some(subject.info.owner_id);
        let previous = replace(&mut tx, &mut effects, user_id, subject.key, owner_id, Some(new)).await?;
        let is_change = previous.as_ref() != Some(new);
        if is_change {
            let event = changed(user_id, subject.key, owner_id, Some(new), previous.as_ref());
            events::emit(&mut *tx, REACTION_CHANGED, Some(user_id), &event).await?;
            // Liking, unliking and liking again is not news twice.
            let notice_key = format!("engagement:liked:{user_id}:{}:{}", subject.key.kind, subject.key.id);
            if let Some(owner_id) = owner_to_tell
                && self.core.cache.set_nx(&notice_key, "1", LIKE_NOTICE_WINDOW).await.unwrap_or(true)
            {
                outbound::notify(&mut *tx, &applause(owner_id, user_id, &actor_name, subject, new)).await?;
            }
        }
        tx.commit().await?;
        self.apply(effects).await;
        if is_change {
            let mut tracked = AnalyticsEvent::new("engagement.reaction")
                .user(Some(user_id))
                .subject("subject", subject.key.id)
                .props(serde_json::json!({ "subject_type": subject.key.kind, "kind": new.kind, "emoji": new.emoji }));
            if let Some(owner_id) = subject.owner() {
                tracked = tracked.owner(owner_id);
            }
            self.core.analytics.track(tracked).await;
        }
        Ok(())
    }

    /// Takes the caller's reaction back (`ReactionService.RemoveReaction`).
    /// Works on a subject that no longer resolves, by its UUID.
    pub(crate) async fn remove_reaction(&self, user_id: Uuid, subject: &LooseSubject) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("engagement:reaction:{user_id}"), CHANGES_PER_MINUTE, Duration::from_secs(60)).await?;
        let owner_id = subject.subject.as_ref().map(|subject| subject.info.owner_id);
        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        let previous = replace(&mut tx, &mut effects, user_id, subject.key, owner_id, None).await?;
        if previous.is_some() {
            let event = changed(user_id, subject.key, owner_id, None, previous.as_ref());
            events::emit(&mut *tx, REACTION_CHANGED, Some(user_id), &event).await?;
        }
        tx.commit().await?;
        self.apply(effects).await;
        Ok(())
    }

    /// The reaction counts of a subject with `viewer`'s own reaction.
    pub(crate) async fn reaction_summary(&self, key: SubjectKey, viewer: Option<Uuid>) -> AppResult<ReactionSummary> {
        let stats = stats::load(&self.core, &[key]).await?;
        let own = match viewer {
            Some(viewer) => of_user(&self.core, viewer, &[key]).await?.remove(&key),
            None => None,
        };
        Ok(stats.reactions(key, own))
    }

    /// Who reacted to a subject, most recent first
    /// (`ReactionService.ListReactions`). Emoji reactions are public, likes
    /// are the owner's to see, dislikes nobody's.
    pub(crate) async fn reactors(
        &self,
        subject: &Subject,
        viewer: Option<Uuid>,
        kind: ReactionKind,
        emoji: &str,
        page: &Page,
    ) -> AppResult<(Vec<ReactorRow>, PageInfo)> {
        let is_owner = viewer.is_some_and(|viewer| subject.is_owned_by(viewer));
        let private = || AppError::denied("REACTION_LIST_PRIVATE", "this list is not public");
        let kinds: &[i32] = match kind {
            ReactionKind::REACTION_KIND_LIKE if is_owner => &[LIKE],
            ReactionKind::REACTION_KIND_LIKE | ReactionKind::REACTION_KIND_DISLIKE => return Err(private()),
            ReactionKind::REACTION_KIND_EMOJI => &[EMOJI],
            ReactionKind::REACTION_KIND_UNSPECIFIED if is_owner => &[LIKE, EMOJI],
            ReactionKind::REACTION_KIND_UNSPECIFIED => &[EMOJI],
        };
        let emoji = if kind == ReactionKind::REACTION_KIND_EMOJI { emoji.trim() } else { "" };
        let hidden = self.hidden_from(viewer).await?;
        let after: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
        let (after_time, after_user) = after.unzip();
        let mut rows: Vec<ReactorRow> = sqlx::query_as(
            "SELECT user_id, kind, emoji, created_at FROM reactions
             WHERE subject_type = $1 AND subject_id = $2 AND kind = ANY($3) AND ($4 = '' OR emoji = $4)
               AND user_id <> ALL($5)
               AND ($6::timestamptz IS NULL OR (created_at, user_id) < ($6, $7))
             ORDER BY created_at DESC, user_id DESC LIMIT $8",
        )
        .bind(subject.key.kind)
        .bind(subject.key.id)
        .bind(kinds)
        .bind(emoji)
        .bind(&hidden)
        .bind(after_time)
        .bind(after_user)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.created_at, row.user_id));
        Ok((rows, info))
    }

    /// The subjects `user_id` reacted to, most recent first
    /// (`ReactionService.ListMyReactions`). `subject_type` and `kind` of 0
    /// mean "any".
    pub(crate) async fn reacted_subjects(&self, user_id: Uuid, subject_type: i32, kind: i32, page: &Page) -> AppResult<(Vec<ReactedRow>, PageInfo)> {
        let after: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
        let (after_time, after_subject) = after.unzip();
        let mut rows: Vec<ReactedRow> = sqlx::query_as(
            "SELECT subject_type, subject_id, kind, emoji, created_at FROM reactions
             WHERE user_id = $1 AND ($2 = 0 OR subject_type = $2) AND ($3 = 0 OR kind = $3)
               AND ($4::timestamptz IS NULL OR (created_at, subject_id) < ($4, $5))
             ORDER BY created_at DESC, subject_id DESC LIMIT $6",
        )
        .bind(user_id)
        .bind(subject_type)
        .bind(kind)
        .bind(after_time)
        .bind(after_subject)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.created_at, row.subject_id));
        Ok((rows, info))
    }
}

#[cfg(test)]
mod tests {
    use tokuly_core::subject::SubjectInfo;
    use tokuly_proto::common::SubjectType;

    use super::*;

    fn reaction(kind: ReactionKind, emoji: &str) -> Reaction {
        Reaction { kind: kind.into(), emoji: emoji.into(), ..Default::default() }
    }

    #[test]
    fn requests_are_checked_and_normalized() {
        assert_eq!(requested(Some(&reaction(ReactionKind::REACTION_KIND_LIKE, "🎉"))).unwrap(), Stored { kind: LIKE, emoji: String::new() });
        assert_eq!(requested(Some(&reaction(ReactionKind::REACTION_KIND_DISLIKE, ""))).unwrap().kind, DISLIKE);
        assert_eq!(requested(Some(&reaction(ReactionKind::REACTION_KIND_EMOJI, " 🎉 "))).unwrap(), Stored { kind: EMOJI, emoji: "🎉".into() });
        assert_eq!(requested(Some(&reaction(ReactionKind::REACTION_KIND_EMOJI, "nope"))).unwrap_err().reason, "EMOJI_INVALID");
        assert_eq!(requested(Some(&reaction(ReactionKind::REACTION_KIND_EMOJI, ""))).unwrap_err().reason, "EMOJI_INVALID");
        assert_eq!(requested(Some(&reaction(ReactionKind::REACTION_KIND_UNSPECIFIED, ""))).unwrap_err().reason, "REACTION_INVALID");
        assert_eq!(requested(None).unwrap_err().reason, "REACTION_INVALID");
    }

    #[test]
    fn each_kind_moves_its_own_counter() {
        assert_eq!(Stored { kind: LIKE, emoji: String::new() }.counter(), Counter::Like);
        assert_eq!(Stored { kind: DISLIKE, emoji: String::new() }.counter(), Counter::Dislike);
        assert_eq!(Stored { kind: EMOJI, emoji: "🎉".into() }.counter(), Counter::Emoji);
        assert!(!Stored { kind: DISLIKE, emoji: String::new() }.is_applause());
        assert!(Stored { kind: EMOJI, emoji: "🎉".into() }.is_applause());
    }

    #[test]
    fn change_events_describe_before_and_after() {
        let (user, owner) = (Uuid::now_v7(), Uuid::now_v7());
        let key = SubjectKey::new(SubjectType::SUBJECT_TYPE_VIDEO, Uuid::now_v7());
        let like = Stored { kind: LIKE, emoji: String::new() };
        let party = Stored { kind: EMOJI, emoji: "🎉".into() };
        let switched = changed(user, key, Some(owner), Some(&party), Some(&like));
        assert_eq!((switched.kind, switched.emoji.as_str(), switched.previous_kind), (EMOJI, "🎉", LIKE));
        let removed = changed(user, key, None, None, Some(&party));
        assert_eq!((removed.kind, removed.previous_kind, removed.previous_emoji.as_str()), (0, EMOJI, "🎉"));
        assert!(removed.subject_owner_id.is_none());
    }

    #[test]
    fn applause_is_collapsed_per_subject_and_worded_by_kind() {
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
        let note = subject(SubjectType::SUBJECT_TYPE_NOTE);
        let like = applause(owner, actor, "アリス", &note, &Stored { kind: LIKE, emoji: String::new() });
        assert_eq!((like.kind.as_str(), like.title.ja.as_str(), like.body.ja.as_str()), (KIND_LIKE, "アリスがいいねしたよ!", "今日のランチ"));
        assert_eq!(like.collapse_key.as_deref(), Some(format!("engagement.like:3:{}", note.key.id).as_str()));
        assert_eq!((like.actor_id, like.user_id, like.path.as_str()), (Some(actor), owner, "/notes/abc"));
        assert_eq!(like.image_url.as_deref(), Some("https://media.test/a.webp"));

        let emoji = applause(owner, actor, "Alice", &note, &Stored { kind: EMOJI, emoji: "🎉".into() });
        assert_eq!(emoji.title.en, "Alice reacted 🎉");
        let comment = subject(SubjectType::SUBJECT_TYPE_COMMENT);
        let liked_comment = applause(owner, actor, "Alice", &comment, &Stored { kind: LIKE, emoji: String::new() });
        assert_eq!(liked_comment.title.en, "Alice liked your comment");
    }
}
