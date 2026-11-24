//! Reactions: likes, dislikes and emoji — Unicode or custom — on any
//! subject.
//!
//! Every reaction is a row of `reactions`. A like or a dislike has an empty
//! `emoji`, so a person holds at most one of the two per subject; each
//! emoji is a row of its own. How many a person may hold, and which emoji a
//! subject takes, is the subject's [`rules::Rules`]. Custom emoji are
//! checked against the emoji catalogue when they are added and drawn from it
//! whenever they are returned ([`custom`]). The owner's notification and the
//! pop on the subject's screens are in [`notice`].
//!
//! The rows and the counters derived from them (`subject_stats`,
//! `reaction_emoji_counts`, and for comments the `like_count` /
//! `dislike_count` on the comment itself) change in one transaction; see
//! [`change`].
mod custom;
mod notice;
pub(crate) mod rules;

use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use tokuly_core::{analytics::AnalyticsEvent, events, outbound, prelude::*};
use tokuly_proto::{
    common::PageInfo,
    engagement::{Reaction, ReactionKind, ReactionSummary},
};

pub(crate) use self::custom::Pictures;
use self::rules::Rules;
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
/// How long reacting to the same subject again sends its owner no second notification.
const LIKE_NOTICE_WINDOW: Duration = Duration::from_secs(24 * 3600);

const LIKE: i32 = ReactionKind::REACTION_KIND_LIKE as i32;
const DISLIKE: i32 = ReactionKind::REACTION_KIND_DISLIKE as i32;
const EMOJI: i32 = ReactionKind::REACTION_KIND_EMOJI as i32;

const _: () = {
    // The migration's CHECK constraints use these numbers.
    assert!(LIKE == 1 && DISLIKE == 2 && EMOJI == 3);
};

/// A reaction as stored: the kind number and, for kind EMOJI, the emoji
/// (a Unicode emoji, or a custom one as `:name:`).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct Stored {
    pub kind: i32,
    pub emoji: String,
}

impl Stored {
    /// The reaction without a picture; see [`Pictures::reaction`].
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

    /// A like or a dislike: a person holds at most one of the two.
    fn is_vote(&self) -> bool {
        self.kind != EMOJI
    }

    /// Whether the owner of the subject is told about it, and the people
    /// watching see it pop: likes and emoji are, dislikes never.
    fn is_applause(&self) -> bool {
        self.kind != DISLIKE
    }
}

/// Checks the reaction of a request and brings it into its stored form.
/// Whether a custom emoji exists is checked later, against the catalogue
/// ([`custom::check`]).
pub(crate) fn requested(reaction: Option<&Reaction>) -> AppResult<Stored> {
    let kind =
        reaction.and_then(|reaction| reaction.kind.as_known()).unwrap_or(ReactionKind::REACTION_KIND_UNSPECIFIED);
    match kind {
        ReactionKind::REACTION_KIND_LIKE => Ok(Stored { kind: LIKE, emoji: String::new() }),
        ReactionKind::REACTION_KIND_DISLIKE => Ok(Stored { kind: DISLIKE, emoji: String::new() }),
        ReactionKind::REACTION_KIND_EMOJI => {
            let emoji = requested_emoji(reaction.map(|reaction| reaction.emoji.as_str()).unwrap_or_default())?;
            Ok(Stored { kind: EMOJI, emoji })
        }
        ReactionKind::REACTION_KIND_UNSPECIFIED => {
            Err(AppError::invalid("REACTION_INVALID", "a reaction is a like, a dislike or an emoji"))
        }
    }
}

/// One Unicode emoji, or a custom emoji's `:shortcode:` (lowered).
pub(crate) fn requested_emoji(input: &str) -> AppResult<String> {
    let emoji = input.trim();
    if emoji.len() > 2 && emoji.starts_with(':') && emoji.ends_with(':') {
        return match tokuly_emoji::validate::normalize_name(emoji) {
            Some(name) => Ok(format!(":{name}:")),
            None => Err(AppError::invalid("EMOJI_INVALID", "a reaction is a single emoji or a :shortcode:")),
        };
    }
    text::single_emoji(emoji)
}

/// The reactions of `user_id` to `keys`, oldest first per subject.
pub(crate) async fn of_user(
    core: &Core,
    user_id: Uuid,
    keys: &[SubjectKey],
) -> AppResult<HashMap<SubjectKey, Vec<Stored>>> {
    if keys.is_empty() {
        return Ok(HashMap::new());
    }
    let (kinds, ids): (Vec<i32>, Vec<Uuid>) = keys.iter().map(|key| (key.kind, key.id)).unzip();
    let rows: Vec<(i32, Uuid, i32, String)> = sqlx::query_as(
        "SELECT subject_type, subject_id, kind, emoji FROM reactions
         WHERE user_id = $1 AND (subject_type, subject_id) IN (SELECT * FROM unnest($2::integer[], $3::uuid[]))
         ORDER BY created_at, emoji",
    )
    .bind(user_id)
    .bind(&kinds)
    .bind(&ids)
    .fetch_all(&core.db)
    .await?;
    let mut held: HashMap<SubjectKey, Vec<Stored>> = HashMap::new();
    for (subject_type, subject_id, kind, emoji) in rows {
        held.entry(SubjectKey { kind: subject_type, id: subject_id }).or_default().push(Stored { kind, emoji });
    }
    Ok(held)
}

/// The one reaction that stands for a person's reactions to a subject
/// (`ReactionSummary.viewer_reaction`): their like or dislike, otherwise
/// their newest emoji.
pub(crate) fn representative(held: &[Stored]) -> Option<&Stored> {
    held.iter().find(|reaction| reaction.is_vote()).or_else(|| held.last())
}

/// The likes and dislikes of `user_id` on comments, by comment ID.
pub(crate) async fn of_user_on_comments(
    core: &Core,
    user_id: Uuid,
    comment_ids: &[Uuid],
) -> AppResult<HashMap<Uuid, Reaction>> {
    let keys: Vec<SubjectKey> = comment_ids.iter().map(|id| SubjectKey::of_comment(*id)).collect();
    Ok(of_user(core, user_id, &keys)
        .await?
        .into_iter()
        .filter_map(|(key, held)| Some((key.id, representative(&held)?.to_proto())))
        .collect())
}

/// A reaction entered (`delta` 1) or left (`delta` -1) the counters of its
/// subject. A comment's own counts go first: every path locks the comment
/// row before the statistics row. Returns `false` when the subject is a
/// comment that no longer exists (nothing is counted then; only possible
/// when the reaction's row was removed by other means).
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
    // The statistics row is always the last lock taken.
    if reaction.kind == EMOJI {
        stats::add_emoji(conn, key, &reaction.emoji, delta).await?;
    }
    stats::add(conn, key, owner_id, reaction.counter(), delta).await?;
    effects.count(key, reaction.counter(), delta);
    Ok(true)
}

/// Takes a reaction whose row was deleted by other means (its user's
/// account was erased) out of the counters of its subject.
pub(crate) async fn uncount(
    conn: &mut PgConnection,
    effects: &mut Effects,
    key: SubjectKey,
    reaction: &Stored,
) -> AppResult<()> {
    count(conn, effects, key, None, reaction, -1).await?;
    Ok(())
}

/// What a person wants done with their reactions to one subject.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Change<'a> {
    /// React, under the subject's rules.
    Add(&'a Stored, &'a Rules),
    /// Take this one reaction back.
    Remove(&'a Stored),
    /// Take every reaction back.
    RemoveAll,
}

/// What a [`change`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Changed {
    pub added: Option<Stored>,
    /// Reactions taken back, or replaced by `added` (the first of them).
    pub removed: Vec<Stored>,
}

impl Changed {
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.added.is_none() && self.removed.is_empty()
    }
}

/// Which of `held` go and what comes, for a change. Refuses an emoji
/// beyond what one person may hold.
fn plan(held: &[Stored], change: Change<'_>) -> AppResult<Changed> {
    match change {
        Change::RemoveAll => Ok(Changed { added: None, removed: held.to_vec() }),
        Change::Remove(reaction) => {
            Ok(Changed { added: None, removed: held.iter().filter(|held| *held == reaction).cloned().collect() })
        }
        Change::Add(new, _) if held.contains(new) => Ok(Changed::default()),
        Change::Add(new, rules) => {
            let removed: Vec<Stored> = if rules.multiple {
                // A like and a dislike replace each other; emoji sit side by side.
                held.iter().filter(|held| new.is_vote() && held.is_vote()).cloned().collect()
            } else {
                held.to_vec()
            };
            if !new.is_vote() {
                let kept = held.iter().filter(|held| !held.is_vote() && !removed.contains(held)).count();
                ensure!(
                    kept < rules.max_per_person.max(1) as usize,
                    precondition,
                    "REACTION_LIMIT",
                    "you already reacted with as many emoji as one person may here"
                );
            }
            Ok(Changed { added: Some(new.clone()), removed })
        }
    }
}

/// Changes the reactions of `user_id` to a subject and moves the counters
/// accordingly.
///
/// `owner_id` is the subject's owner, when the subject resolved. An
/// advisory lock per user and subject serializes the read-then-write, so
/// two taps at once count once; adding an emoji also takes the subject's
/// emoji lock, so that two people adding new emoji at once cannot both
/// take the last free place.
pub(crate) async fn change(
    conn: &mut PgConnection,
    effects: &mut Effects,
    user_id: Uuid,
    key: SubjectKey,
    owner_id: Option<Uuid>,
    change: Change<'_>,
) -> AppResult<Changed> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("engagement:reaction:{user_id}:{}:{}", key.kind, key.id))
        .execute(&mut *conn)
        .await?;
    let adding_emoji = matches!(change, Change::Add(new, _) if !new.is_vote());
    if adding_emoji {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("engagement:reaction-emoji:{}:{}", key.kind, key.id))
            .execute(&mut *conn)
            .await?;
    }
    if key.is_comment() {
        // A comment is locked before anything that hangs off it, here as in
        // its deletion — which this reaction therefore either precedes
        // (and is purged by) or follows (and is refused).
        let comment: Option<Uuid> = sqlx::query_scalar("SELECT id FROM comments WHERE id = $1 FOR UPDATE")
            .bind(key.id)
            .fetch_optional(&mut *conn)
            .await?;
        ensure!(
            comment.is_some() || !matches!(change, Change::Add(..)),
            not_found,
            "SUBJECT_NOT_FOUND",
            "the item does not exist or is not visible to you"
        );
    }
    let held: Vec<Stored> = sqlx::query_as(
        "SELECT kind, emoji FROM reactions WHERE user_id = $1 AND subject_type = $2 AND subject_id = $3
         ORDER BY created_at, emoji",
    )
    .bind(user_id)
    .bind(key.kind)
    .bind(key.id)
    .fetch_all(&mut *conn)
    .await?;
    let changed = plan(&held, change)?;
    for reaction in &changed.removed {
        sqlx::query(
            "DELETE FROM reactions WHERE user_id = $1 AND subject_type = $2 AND subject_id = $3 AND emoji = $4",
        )
        .bind(user_id)
        .bind(key.kind)
        .bind(key.id)
        .bind(&reaction.emoji)
        .execute(&mut *conn)
        .await?;
        count(conn, effects, key, owner_id, reaction, -1).await?;
    }
    if let (Some(new), Change::Add(_, rules)) = (&changed.added, change) {
        if !new.is_vote() {
            // Counted after the removals: replacing one's only emoji on a
            // full subject frees its place first.
            let (distinct, present): (i64, bool) = sqlx::query_as(
                "SELECT count(*), COALESCE(bool_or(emoji = $3), false) FROM reaction_emoji_counts
                 WHERE subject_type = $1 AND subject_id = $2",
            )
            .bind(key.kind)
            .bind(key.id)
            .bind(&new.emoji)
            .fetch_one(&mut *conn)
            .await?;
            ensure!(
                present || distinct < i64::from(rules.max_distinct_emoji),
                precondition,
                "REACTION_EMOJI_LIMIT",
                "this already has as many different emoji as it can take; join one of them"
            );
        }
        sqlx::query(
            "INSERT INTO reactions (user_id, subject_type, subject_id, kind, emoji) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user_id)
        .bind(key.kind)
        .bind(key.id)
        .bind(new.kind)
        .bind(&new.emoji)
        .execute(&mut *conn)
        .await?;
        count(conn, effects, key, owner_id, new, 1).await?;
    }
    Ok(changed)
}

fn changed_event(
    user_id: Uuid,
    key: SubjectKey,
    owner_id: Option<Uuid>,
    new: Option<&Stored>,
    previous: Option<&Stored>,
) -> ReactionChanged {
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

/// The `engagement.reaction.changed` events of a change: the reaction that
/// came with the one it replaced, then every other one that went.
fn change_events(user_id: Uuid, key: SubjectKey, owner_id: Option<Uuid>, changed: &Changed) -> Vec<ReactionChanged> {
    let mut removed = changed.removed.iter();
    let mut events = Vec::new();
    if let Some(added) = &changed.added {
        events.push(changed_event(user_id, key, owner_id, Some(added), removed.next()));
    }
    events.extend(removed.map(|previous| changed_event(user_id, key, owner_id, None, Some(previous))));
    events
}

/// Someone who reacted to a subject.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ReactorRow {
    pub user_id: Uuid,
    pub kind: i32,
    pub emoji: String,
    pub created_at: DateTime<Utc>,
}

impl ReactorRow {
    pub(crate) fn reaction(&self) -> Stored {
        Stored { kind: self.kind, emoji: self.emoji.clone() }
    }
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

    pub(crate) fn reaction(&self) -> Stored {
        Stored { kind: self.kind, emoji: self.emoji.clone() }
    }
}

impl Engagement {
    /// Reacts to a subject under its rules (`ReactionService.SetReaction`).
    pub(crate) async fn set_reaction(&self, user_id: Uuid, subject: &Subject, new: &Stored) -> AppResult<()> {
        ensure!(
            subject.info.interactions_enabled,
            precondition,
            "INTERACTIONS_DISABLED",
            "reactions are turned off here"
        );
        ensure!(
            !(subject.key.is_comment() && new.kind == EMOJI),
            invalid,
            "EMOJI_NOT_ALLOWED",
            "comments take likes and dislikes, not emoji"
        );
        self.core
            .cache
            .rate_limit(&format!("engagement:reaction:{user_id}"), CHANGES_PER_MINUTE, Duration::from_secs(60))
            .await?;
        // Everything that needs the pool, before the transaction holds a connection.
        let rules = self.rules_of_subject(subject).await?;
        custom::check(&self.core, new, &rules).await?;
        let pictures = Pictures::load(&self.core, [new.emoji.as_str()]).await?;
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
        let changed = change(&mut tx, &mut effects, user_id, subject.key, owner_id, Change::Add(new, &rules)).await?;
        for event in change_events(user_id, subject.key, owner_id, &changed) {
            events::emit(&mut *tx, REACTION_CHANGED, Some(user_id), &event).await?;
        }
        if changed.added.is_some() {
            // Reacting, taking it back and reacting again is not news twice.
            let notice_key = format!("engagement:liked:{user_id}:{}:{}", subject.key.kind, subject.key.id);
            if let Some(owner_id) = owner_to_tell
                && self.core.cache.set_nx(&notice_key, "1", LIKE_NOTICE_WINDOW).await.unwrap_or(true)
            {
                let others = notice::others_applauding(&mut tx, subject, user_id, owner_id).await?;
                let picture = pictures
                    .get(&new.emoji)
                    .and_then(|picture| picture.image.as_option())
                    .map(|image| image.url.as_str());
                let request = notice::applause(owner_id, user_id, &actor_name, subject, new, others, picture);
                outbound::notify(&mut *tx, &request).await?;
            }
        }
        tx.commit().await?;
        self.apply(effects).await;
        if let Some(added) = changed.added {
            let mut tracked = AnalyticsEvent::new("engagement.reaction")
                .user(Some(user_id))
                .subject("subject", subject.key.id)
                .props(serde_json::json!({
                    "subject_type": subject.key.kind,
                    "kind": added.kind,
                    "emoji": added.emoji,
                    "custom": custom::custom_name(&added.emoji).is_some(),
                }));
            if let Some(owner_id) = subject.owner() {
                tracked = tracked.owner(owner_id);
            }
            self.core.analytics.track(tracked).await;
            if added.kind == EMOJI
                && let Err(error) = tokuly_emoji::catalog::record_use(&self.core, user_id, &[added.emoji.clone()]).await
            {
                tracing::warn!(%error, "emoji use was not recorded");
            }
            if added.is_applause() {
                self.pop(subject, pictures.reaction(&added)).await;
            }
        }
        Ok(())
    }

    /// Takes the caller's reactions back (`ReactionService.RemoveReaction`):
    /// `which` one, or all of them. Works on a subject that no longer
    /// resolves, by its UUID.
    pub(crate) async fn remove_reaction(
        &self,
        user_id: Uuid,
        subject: &LooseSubject,
        which: Option<&Stored>,
    ) -> AppResult<()> {
        self.core
            .cache
            .rate_limit(&format!("engagement:reaction:{user_id}"), CHANGES_PER_MINUTE, Duration::from_secs(60))
            .await?;
        let owner_id = subject.subject.as_ref().map(|subject| subject.info.owner_id);
        let request = match which {
            Some(reaction) => Change::Remove(reaction),
            None => Change::RemoveAll,
        };
        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        let changed = change(&mut tx, &mut effects, user_id, subject.key, owner_id, request).await?;
        for event in change_events(user_id, subject.key, owner_id, &changed) {
            events::emit(&mut *tx, REACTION_CHANGED, Some(user_id), &event).await?;
        }
        tx.commit().await?;
        self.apply(effects).await;
        Ok(())
    }

    /// The reaction counts of a subject with `viewer`'s own reactions, its
    /// rules and the pictures of its custom emoji.
    pub(crate) async fn reaction_summary(&self, subject: &Subject, viewer: Option<Uuid>) -> AppResult<ReactionSummary> {
        let key = subject.key;
        let stats = stats::load(&self.core, &[key]).await?;
        let own = match viewer {
            Some(viewer) => of_user(&self.core, viewer, &[key]).await?.remove(&key).unwrap_or_default(),
            None => Vec::new(),
        };
        let mut summary = stats.reactions(key, &own);
        summary.rules = self.rules_of_subject(subject).await?.to_proto().into();
        Pictures::for_summaries(&self.core, &[&summary]).await?.dress_summary(&mut summary);
        Ok(summary)
    }

    /// Who reacted to a subject, most recent first
    /// (`ReactionService.ListReactions`). Emoji reactions are public, likes
    /// are the owner's to see, dislikes nobody's. People in a block with the
    /// viewer and people the viewer muted are left out.
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
        let emoji = match kind {
            ReactionKind::REACTION_KIND_EMOJI if !emoji.trim().is_empty() => requested_emoji(emoji)?,
            _ => String::new(),
        };
        let mut hidden = self.hidden_from(viewer).await?;
        hidden.extend(self.muted_by(viewer).await?);
        let after: Option<(DateTime<Utc>, Uuid, String)> = page.decode()?;
        let (after_time, after_user, after_emoji) = match after {
            Some((time, user, emoji)) => (Some(time), Some(user), emoji),
            None => (None, None, String::new()),
        };
        let mut rows: Vec<ReactorRow> = sqlx::query_as(
            "SELECT user_id, kind, emoji, created_at FROM reactions
             WHERE subject_type = $1 AND subject_id = $2 AND kind = ANY($3) AND ($4 = '' OR emoji = $4)
               AND user_id <> ALL($5)
               AND ($6::timestamptz IS NULL OR (created_at, user_id, emoji) < ($6, $7, $8))
             ORDER BY created_at DESC, user_id DESC, emoji DESC LIMIT $9",
        )
        .bind(subject.key.kind)
        .bind(subject.key.id)
        .bind(kinds)
        .bind(&emoji)
        .bind(&hidden)
        .bind(after_time)
        .bind(after_user)
        .bind(&after_emoji)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.created_at, row.user_id, row.emoji.clone()));
        Ok((rows, info))
    }

    /// The subjects `user_id` reacted to, most recent first
    /// (`ReactionService.ListMyReactions`). `subject_type` and `kind` of 0
    /// mean "any".
    pub(crate) async fn reacted_subjects(
        &self,
        user_id: Uuid,
        subject_type: i32,
        kind: i32,
        page: &Page,
    ) -> AppResult<(Vec<ReactedRow>, PageInfo)> {
        let after: Option<(DateTime<Utc>, Uuid, String)> = page.decode()?;
        let (after_time, after_subject, after_emoji) = match after {
            Some((time, subject, emoji)) => (Some(time), Some(subject), emoji),
            None => (None, None, String::new()),
        };
        let mut rows: Vec<ReactedRow> = sqlx::query_as(
            "SELECT subject_type, subject_id, kind, emoji, created_at FROM reactions
             WHERE user_id = $1 AND ($2 = 0 OR subject_type = $2) AND ($3 = 0 OR kind = $3)
               AND ($4::timestamptz IS NULL OR (created_at, subject_id, emoji) < ($4, $5, $6))
             ORDER BY created_at DESC, subject_id DESC, emoji DESC LIMIT $7",
        )
        .bind(user_id)
        .bind(subject_type)
        .bind(kind)
        .bind(after_time)
        .bind(after_subject)
        .bind(&after_emoji)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.created_at, row.subject_id, row.emoji.clone()));
        Ok((rows, info))
    }
}

#[cfg(test)]
mod tests {
    use tokuly_proto::common::SubjectType;

    use super::*;

    fn reaction(kind: ReactionKind, emoji: &str) -> Reaction {
        Reaction { kind: kind.into(), emoji: emoji.into(), ..Default::default() }
    }

    fn like() -> Stored {
        Stored { kind: LIKE, emoji: String::new() }
    }

    fn dislike() -> Stored {
        Stored { kind: DISLIKE, emoji: String::new() }
    }

    fn emoji(emoji: &str) -> Stored {
        Stored { kind: EMOJI, emoji: emoji.into() }
    }

    fn rules(multiple: bool, max_per_person: i32) -> Rules {
        Rules {
            emoji: true,
            multiple,
            max_per_person,
            max_distinct_emoji: 50,
            custom_emoji: true,
            sensitive_emoji: false,
        }
    }

    #[test]
    fn requests_are_checked_and_normalized() {
        assert_eq!(requested(Some(&reaction(ReactionKind::REACTION_KIND_LIKE, "🎉"))).unwrap(), like());
        assert_eq!(requested(Some(&reaction(ReactionKind::REACTION_KIND_DISLIKE, ""))).unwrap().kind, DISLIKE);
        assert_eq!(requested(Some(&reaction(ReactionKind::REACTION_KIND_EMOJI, " 🎉 "))).unwrap(), emoji("🎉"));
        assert_eq!(
            requested(Some(&reaction(ReactionKind::REACTION_KIND_EMOJI, " :Neko_2: "))).unwrap(),
            emoji(":neko_2:")
        );
        for wrong in ["nope", "", ":", "::", ":a b:", ":ねこ:", "neko:", ":__:"] {
            assert_eq!(
                requested(Some(&reaction(ReactionKind::REACTION_KIND_EMOJI, wrong))).unwrap_err().reason,
                "EMOJI_INVALID",
                "{wrong:?}"
            );
        }
        assert_eq!(
            requested(Some(&reaction(ReactionKind::REACTION_KIND_UNSPECIFIED, ""))).unwrap_err().reason,
            "REACTION_INVALID"
        );
        assert_eq!(requested(None).unwrap_err().reason, "REACTION_INVALID");
    }

    #[test]
    fn each_kind_moves_its_own_counter() {
        assert_eq!(like().counter(), Counter::Like);
        assert_eq!(dislike().counter(), Counter::Dislike);
        assert_eq!(emoji("🎉").counter(), Counter::Emoji);
        assert!(!dislike().is_applause());
        assert!(emoji(":neko:").is_applause());
    }

    #[test]
    fn one_reaction_subjects_replace_what_was_there() {
        let single = rules(false, 1);
        let changed = plan(&[like()], Change::Add(&emoji("🎉"), &single)).unwrap();
        assert_eq!(changed, Changed { added: Some(emoji("🎉")), removed: vec![like()] });
        assert_eq!(plan(&[emoji("🎉")], Change::Add(&emoji("🎉"), &single)).unwrap(), Changed::default());
        // A subject whose type took several before takes them all back.
        let changed = plan(&[emoji("🎉"), emoji("👀")], Change::Add(&emoji("🙏"), &single)).unwrap();
        assert_eq!(changed.removed.len(), 2);
    }

    #[test]
    fn several_reaction_subjects_add_emoji_side_by_side() {
        let multiple = rules(true, 2);
        let held = [like(), emoji("🎉")];
        let changed = plan(&held, Change::Add(&emoji("👀"), &multiple)).unwrap();
        assert_eq!(changed, Changed { added: Some(emoji("👀")), removed: vec![] });
        // A like and a dislike replace each other.
        let changed = plan(&held, Change::Add(&dislike(), &multiple)).unwrap();
        assert_eq!(changed, Changed { added: Some(dislike()), removed: vec![like()] });
        // Votes do not count towards the emoji a person may hold.
        let full = [like(), emoji("🎉"), emoji("👀")];
        assert_eq!(plan(&full, Change::Add(&emoji("🙏"), &multiple)).unwrap_err().reason, "REACTION_LIMIT");
        assert!(plan(&full, Change::Add(&dislike(), &multiple)).is_ok());
        assert_eq!(plan(&full, Change::Add(&emoji("🎉"), &multiple)).unwrap(), Changed::default());
    }

    #[test]
    fn taking_back_removes_one_or_all() {
        let held = [like(), emoji("🎉")];
        assert_eq!(plan(&held, Change::Remove(&emoji("🎉"))).unwrap().removed, [emoji("🎉")]);
        assert!(plan(&held, Change::Remove(&dislike())).unwrap().is_empty());
        assert_eq!(plan(&held, Change::RemoveAll).unwrap().removed.len(), 2);
        assert!(plan(&[], Change::RemoveAll).unwrap().is_empty());
    }

    #[test]
    fn the_representative_is_the_vote_or_the_newest_emoji() {
        assert_eq!(representative(&[emoji("🎉"), like(), emoji("👀")]), Some(&like()));
        assert_eq!(representative(&[emoji("🎉"), emoji("👀")]), Some(&emoji("👀")));
        assert_eq!(representative(&[]), None);
    }

    #[test]
    fn change_events_describe_before_and_after() {
        let (user, owner) = (Uuid::now_v7(), Uuid::now_v7());
        let key = SubjectKey::new(SubjectType::SUBJECT_TYPE_VIDEO, Uuid::now_v7());
        let switched = change_events(
            user,
            key,
            Some(owner),
            &Changed { added: Some(emoji("🎉")), removed: vec![like(), emoji("👀")] },
        );
        assert_eq!(switched.len(), 2);
        assert_eq!((switched[0].kind, switched[0].emoji.as_str(), switched[0].previous_kind), (EMOJI, "🎉", LIKE));
        assert_eq!((switched[1].kind, switched[1].previous_emoji.as_str()), (0, "👀"));
        let added = change_events(user, key, None, &Changed { added: Some(emoji(":neko:")), removed: vec![] });
        assert_eq!((added[0].previous_kind, added[0].emoji.as_str()), (0, ":neko:"));
        assert!(added[0].subject_owner_id.is_none());
        assert!(change_events(user, key, None, &Changed::default()).is_empty());
    }
}
