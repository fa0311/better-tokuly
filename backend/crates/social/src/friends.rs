//! Friendships: listing them, per-friend preferences, shared history and
//! ending them.
use std::collections::HashSet;

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{Postgres, Transaction};
use tokuly_core::{events, prelude::*};
use tokuly_identity::{settings, users};
use tokuly_proto::{
    common::PageInfo,
    identity::Audience,
    social::{Friend, Friendship, social_event::Event as Push},
};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    Social, circles,
    events::{FRIEND_PREFS_CHANGED, FRIEND_REMOVED, FriendPrefsChanged, FriendRemoved},
    graph::{self, Pair},
    idcache, not_friends, people, ranking,
};

/// Most friends one user can star.
const MAX_FAVORITES: i64 = 50;
const MAX_NICKNAME_GRAPHEMES: usize = 40;
const MAX_QUERY_CHARS: usize = 64;
/// Largest page of `ListFriends`; big enough to load a whole map in few calls.
const MAX_FRIENDS_PAGE: i64 = 200;

/// Columns of [`FriendRow`], for queries joining `friendships f` with `users u`.
macro_rules! friend_columns {
    () => {
        "f.friend_id, f.created_at, f.nickname, f.favorite, f.muted, f.best_friend_rank, \
         u.status_emoji, u.status_text, u.status_set_at, u.status_expires_at"
    };
}
pub(crate) use friend_columns;

/// One side of a friendship plus the friend's status.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct FriendRow {
    pub friend_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub nickname: String,
    pub favorite: bool,
    pub muted: bool,
    pub best_friend_rank: i32,
    pub status_emoji: String,
    pub status_text: String,
    pub status_set_at: Option<DateTime<Utc>>,
    pub status_expires_at: Option<DateTime<Utc>>,
}

/// How much of a friendship row the viewer may see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Detail {
    /// The viewer's own friend: everything, including online state.
    Own,
    /// A row of a friend's best-friend list: rank and star, nothing private.
    Ranking,
    /// A row of someone else's friend list: who, and since when.
    Public,
}

/// What two friends have been through, as stored on their pair.
#[derive(Debug, Default, sqlx::FromRow)]
struct HistoryRow {
    bump_count: i32,
    last_bump_at: Option<DateTime<Utc>>,
    time_together_seconds: i64,
    nights_together: i32,
    streak_weeks: i32,
    streak_week: Option<NaiveDate>,
    titles: Vec<String>,
    interaction_score: i32,
}

/// The changes of one `UpdateFriendPrefs` call; `None` leaves a field alone.
pub(crate) struct PrefsChange<'a> {
    pub nickname: Option<&'a str>,
    pub favorite: Option<bool>,
    pub muted: Option<bool>,
}

/// Trims a nickname and drops control characters; empty clears it.
fn normalize_nickname(input: &str) -> AppResult<String> {
    let nickname: String = input.chars().filter(|c| !c.is_control()).collect();
    let nickname = nickname.trim();
    ensure!(
        nickname.graphemes(true).count() <= MAX_NICKNAME_GRAPHEMES,
        invalid,
        "NICKNAME_TOO_LONG",
        "nicknames are at most {MAX_NICKNAME_GRAPHEMES} characters"
    );
    Ok(nickname.to_owned())
}

/// Deletes both rows of the friendship between `a` and `b` inside `tx`.
/// Returns false if the two were not friends.
///
/// A side that had muted the other is announced as unmuted: the notify
/// domain mirrors mutes from these events and must not keep one for someone
/// who is no longer a friend (it would silently survive a later re-friending,
/// where the friendship starts unmuted).
pub(crate) async fn end_friendship(tx: &mut Transaction<'_, Postgres>, a: Uuid, b: Uuid) -> AppResult<bool> {
    let sides: Vec<(Uuid, Uuid, bool)> = sqlx::query_as(
        "DELETE FROM friendships WHERE (user_id = $1 AND friend_id = $2) OR (user_id = $2 AND friend_id = $1)
         RETURNING user_id, friend_id, muted",
    )
    .bind(a)
    .bind(b)
    .fetch_all(&mut **tx)
    .await?;
    for (user_id, friend_id, muted) in &sides {
        if *muted {
            let unmuted = FriendPrefsChanged { user_id: *user_id, friend_id: *friend_id, muted: false };
            events::emit(&mut **tx, FRIEND_PREFS_CHANGED, Some(*user_id), &unmuted).await?;
        }
    }
    Ok(!sides.is_empty())
}

/// A `LIKE` pattern matching `query` anywhere, with its wildcards escaped.
fn contains_pattern(query: &str) -> String {
    format!("%{}%", query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))
}

impl Social {
    /// Turns friendship rows into API messages, in the same order.
    pub(crate) async fn friends_to_proto(&self, rows: Vec<FriendRow>, detail: Detail) -> AppResult<Vec<Friend>> {
        let ids: Vec<Uuid> = rows.iter().map(|r| r.friend_id).collect();
        let summaries = tokuly_identity::summaries(&self.core, &ids).await?;
        let presence = if detail == Detail::Own {
            people::presence_of(&self.core, &ids).await
        } else {
            people::Presence::default()
        };
        Ok(rows
            .into_iter()
            .map(|row| {
                let mut friend = Friend {
                    user: users::pick(&summaries, row.friend_id).into(),
                    friends_since: ts(row.created_at),
                    status: users::status_of(
                        &row.status_emoji,
                        &row.status_text,
                        row.status_set_at,
                        row.status_expires_at,
                    )
                    .into(),
                    ..Default::default()
                };
                if detail != Detail::Public {
                    friend.favorite = row.favorite;
                    friend.best_friend_rank = row.best_friend_rank;
                }
                if detail == Detail::Own {
                    friend.nickname = row.nickname;
                    friend.muted = row.muted;
                    friend.online = presence.is_online(row.friend_id);
                    friend.last_seen_at = opt_ts(presence.last_seen(row.friend_id));
                }
                friend
            })
            .collect())
    }

    /// `friend_id` as their friend `user_id` sees them. Fails with `NOT_FRIENDS`.
    pub(crate) async fn friend_as_seen_by(&self, user_id: Uuid, friend_id: Uuid) -> AppResult<Friend> {
        let row: Option<FriendRow> = sqlx::query_as(concat!(
            "SELECT ",
            friend_columns!(),
            " FROM friendships f JOIN users u ON u.id = f.friend_id WHERE f.user_id = $1 AND f.friend_id = $2"
        ))
        .bind(user_id)
        .bind(friend_id)
        .fetch_optional(&self.core.db)
        .await?;
        let row = row.ok_or_else(not_friends)?;
        let mut friends = self.friends_to_proto(vec![row], Detail::Own).await?;
        Ok(friends.remove(0))
    }

    /// Fails unless `viewer` may look at `owner`'s friend list (also used for
    /// their intros): the owner must be visible to the viewer at all, and the
    /// owner's `friend_list_visible_to` audience must include the viewer.
    pub(crate) async fn ensure_can_see_friends_of(&self, viewer: Uuid, owner: Uuid) -> AppResult<()> {
        if viewer == owner {
            return Ok(());
        }
        people::ensure_visible(&self.core, viewer, owner).await?;
        let privacy = settings::privacy_of(&self.core, owner).await?;
        let allowed =
            graph::allows(&self.core, owner, Some(viewer), people::audience(&privacy.friend_list_visible_to)).await?;
        ensure!(allowed, denied, "FRIEND_LIST_PRIVATE", "this user's friend list is private");
        Ok(())
    }

    /// Which of `owners` let `viewer` see their friend list, and with it the
    /// friends the two have in common: mutual friends follow the same rule
    /// as the list they are taken from. (The batch form of
    /// [`Self::ensure_can_see_friends_of`]'s audience check.)
    pub(crate) async fn friend_lists_visible_to(&self, viewer: Uuid, owners: &[Uuid]) -> AppResult<HashSet<Uuid>> {
        let core = &self.core;
        if owners.is_empty() {
            return Ok(HashSet::new());
        }
        let privacy = settings::privacy_of_many(core, owners).await?;
        let hidden: HashSet<Uuid> = graph::blocked_ids_either(core, viewer).await?.into_iter().collect();
        let friends: HashSet<Uuid> = sqlx::query_scalar::<_, Uuid>(
            "SELECT friend_id FROM friendships WHERE user_id = $1 AND friend_id = ANY($2)",
        )
        .bind(viewer)
        .bind(owners)
        .fetch_all(&core.db)
        .await?
        .into_iter()
        .collect();
        let mutual = graph::mutual_friend_counts(core, viewer, owners).await?;
        Ok(owners
            .iter()
            .copied()
            .filter(|owner| {
                if *owner == viewer {
                    return true;
                }
                if hidden.contains(owner) {
                    return false;
                }
                let audience = privacy
                    .get(owner)
                    .map(|p| people::audience(&p.friend_list_visible_to))
                    .unwrap_or(Audience::AUDIENCE_UNSPECIFIED);
                match audience {
                    Audience::AUDIENCE_EVERYONE => true,
                    Audience::AUDIENCE_NOBODY => false,
                    Audience::AUDIENCE_FRIENDS | Audience::AUDIENCE_UNSPECIFIED => friends.contains(owner),
                    Audience::AUDIENCE_FRIENDS_OF_FRIENDS => friends.contains(owner) || mutual.contains_key(owner),
                }
            })
            .collect())
    }

    /// One page of `owner`'s friends (newest friendships first) as `viewer`
    /// sees them, plus the total matching `query`.
    pub(crate) async fn list_friends(
        &self,
        viewer: Uuid,
        owner: Uuid,
        query: &str,
        page: &Page,
    ) -> AppResult<(Vec<Friend>, PageInfo, i32)> {
        self.ensure_can_see_friends_of(viewer, owner).await?;
        let own = viewer == owner;
        let query = query.trim().trim_start_matches('@');
        ensure!(query.chars().count() <= MAX_QUERY_CHARS, invalid, "QUERY_TOO_LONG", "search query is too long");
        let pattern = contains_pattern(query);
        // In someone else's list the viewer does not meet people hidden from
        // them by a block. (Nobody in their own list can be: blocking unfriends.)
        let hidden = if own { Vec::new() } else { graph::blocked_ids_either(&self.core, viewer).await? };
        let cursor: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
        let (after_time, after_id) = cursor.unzip();

        // $3: the name filter is off; $5: nicknames are private to the owner.
        let mut rows: Vec<FriendRow> = sqlx::query_as(concat!(
            "SELECT ",
            friend_columns!(),
            " FROM friendships f JOIN users u ON u.id = f.friend_id
              WHERE f.user_id = $1 AND u.deleted_at IS NULL AND NOT (f.friend_id = ANY($2))
                AND ($3 OR u.display_name ILIKE $4 OR u.handle::text ILIKE $4 OR ($5 AND f.nickname ILIKE $4))
                AND ($6::timestamptz IS NULL OR (f.created_at, f.friend_id) < ($6, $7))
              ORDER BY f.created_at DESC, f.friend_id DESC
              LIMIT $8"
        ))
        .bind(owner)
        .bind(&hidden)
        .bind(query.is_empty())
        .bind(&pattern)
        .bind(own)
        .bind(after_time)
        .bind(after_id)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM friendships f JOIN users u ON u.id = f.friend_id
             WHERE f.user_id = $1 AND u.deleted_at IS NULL AND NOT (f.friend_id = ANY($2))
               AND ($3 OR u.display_name ILIKE $4 OR u.handle::text ILIKE $4 OR ($5 AND f.nickname ILIKE $4))",
        )
        .bind(owner)
        .bind(&hidden)
        .bind(query.is_empty())
        .bind(&pattern)
        .bind(own)
        .fetch_one(&self.core.db)
        .await?;

        let info = page.finish(&mut rows, |r| (r.created_at, r.friend_id));
        let friends = self.friends_to_proto(rows, if own { Detail::Own } else { Detail::Public }).await?;
        Ok((friends, info, total as i32))
    }

    pub(crate) fn friends_page(page: Option<&tokuly_proto::common::PageRequestView<'_>>) -> Page {
        Page::from_view(page, MAX_FRIENDS_PAGE)
    }

    /// Ends the friendship between `user_id` and `friend_id` for both sides.
    pub(crate) async fn remove_friend(&self, user_id: Uuid, friend_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        graph::lock_pair(&mut tx, user_id, friend_id).await?;
        if !end_friendship(&mut tx, user_id, friend_id).await? {
            return Err(not_friends());
        }
        events::emit(&mut *tx, FRIEND_REMOVED, Some(user_id), &FriendRemoved { user_id, friend_id, blocked: false })
            .await?;
        // A friend who does not follow is no longer someone to share with.
        let exits = circles::untie_both(&mut tx, user_id, friend_id).await?;
        tx.commit().await?;
        exits.invalidate(&self.core).await;
        self.after_friendship_ended(user_id, friend_id).await;
        Ok(())
    }

    /// Housekeeping once a friendship is gone (unfriend or block), after the
    /// commit: drop cached friend lists, close the gap in both best-friend
    /// rankings, and tell both sides' devices to forget the other.
    pub(crate) async fn after_friendship_ended(&self, actor: Uuid, other: Uuid) {
        idcache::friendships_changed(&self.core, &[actor, other]).await;
        if let Err(e) = self.recompute_ranks(&[actor, other]).await {
            tracing::warn!(error = %e, "best-friend ranks not refreshed after a friendship ended; the ranking worker will catch up");
        }
        self.push(other, Push::FriendRemovedUserId(actor.to_string())).await;
        self.push(actor, Push::FriendRemovedUserId(other.to_string())).await;
    }

    /// The friendship between `user_id` and `friend_id` with its shared history.
    pub(crate) async fn friendship(&self, user_id: Uuid, friend_id: Uuid) -> AppResult<Friendship> {
        let friend = self.friend_as_seen_by(user_id, friend_id).await?;
        let pair = Pair::of(user_id, friend_id);
        let history: HistoryRow = sqlx::query_as(
            "SELECT bump_count, last_bump_at, time_together_seconds, nights_together, streak_weeks, streak_week, titles, interaction_score
             FROM friend_pairs WHERE user_lo = $1 AND user_hi = $2",
        )
        .bind(pair.lo)
        .bind(pair.hi)
        .fetch_optional(&self.core.db)
        .await?
        .unwrap_or_default();
        // (how they met, where the friend ranks the caller)
        let (source, rank_for_friend): (i32, Option<i32>) = sqlx::query_as(
            "SELECT f.source, (SELECT best_friend_rank FROM friendships WHERE user_id = $2 AND friend_id = $1)
             FROM friendships f WHERE f.user_id = $1 AND f.friend_id = $2",
        )
        .bind(user_id)
        .bind(friend_id)
        .fetch_one(&self.core.db)
        .await?;

        let now = Utc::now();
        let streak_weeks = ranking::effective_streak(history.streak_weeks, history.streak_week, now.date_naive());
        let mutual_closest = friend.best_friend_rank == 1 && rank_for_friend == Some(1);
        let friends_since = friend.friends_since.as_option().map(from_ts).unwrap_or(now);
        Ok(Friendship {
            source: source.into(),
            bump_count: history.bump_count,
            last_bump_at: opt_ts(history.last_bump_at),
            time_together_seconds: history.time_together_seconds,
            nights_together: history.nights_together,
            streak_weeks,
            titles: ranking::all_titles(&history.titles, mutual_closest, streak_weeks, friends_since, now),
            mutual_friend_count: if self.friend_lists_visible_to(user_id, &[friend_id]).await?.is_empty() {
                0
            } else {
                graph::mutual_friend_count(&self.core, user_id, friend_id).await?
            },
            interaction_score: history.interaction_score,
            friend: friend.into(),
            ..Default::default()
        })
    }

    /// Changes what `user_id` keeps about their friend `friend_id`.
    pub(crate) async fn update_friend_prefs(
        &self,
        user_id: Uuid,
        friend_id: Uuid,
        change: PrefsChange<'_>,
    ) -> AppResult<Friend> {
        let nickname = change.nickname.map(normalize_nickname).transpose()?;
        let mut tx = self.core.db.begin().await?;
        let current: Option<(bool, bool)> =
            sqlx::query_as("SELECT favorite, muted FROM friendships WHERE user_id = $1 AND friend_id = $2 FOR UPDATE")
                .bind(user_id)
                .bind(friend_id)
                .fetch_optional(&mut *tx)
                .await?;
        let (was_favorite, was_muted) = current.ok_or_else(not_friends)?;
        if change.favorite == Some(true) && !was_favorite {
            let favorites: i64 = sqlx::query_scalar("SELECT count(*) FROM friendships WHERE user_id = $1 AND favorite")
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await?;
            ensure!(
                favorites < MAX_FAVORITES,
                precondition,
                "TOO_MANY_FAVORITES",
                "you can star at most {MAX_FAVORITES} friends"
            );
        }
        sqlx::query(
            "UPDATE friendships SET nickname = COALESCE($3, nickname), favorite = COALESCE($4, favorite), muted = COALESCE($5, muted)
             WHERE user_id = $1 AND friend_id = $2",
        )
        .bind(user_id)
        .bind(friend_id)
        .bind(nickname)
        .bind(change.favorite)
        .bind(change.muted)
        .execute(&mut *tx)
        .await?;
        if let Some(muted) = change.muted.filter(|muted| *muted != was_muted) {
            // The notify domain mirrors mutes from this event.
            events::emit(
                &mut *tx,
                FRIEND_PREFS_CHANGED,
                Some(user_id),
                &FriendPrefsChanged { user_id, friend_id, muted },
            )
            .await?;
        }
        tx.commit().await?;
        idcache::FRIEND_LINKS.invalidate(&self.core, &[user_id]).await;
        self.friend_as_seen_by(user_id, friend_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nicknames_are_trimmed_and_limited() {
        assert_eq!(normalize_nickname("  たろう\u{0007}  ").unwrap(), "たろう");
        assert_eq!(normalize_nickname("   ").unwrap(), "");
        assert!(normalize_nickname(&"あ".repeat(40)).is_ok());
        assert_eq!(normalize_nickname(&"あ".repeat(41)).unwrap_err().reason, "NICKNAME_TOO_LONG");
    }

    #[test]
    fn like_wildcards_in_queries_are_literal() {
        assert_eq!(contains_pattern("50%_off\\"), "%50\\%\\_off\\\\%");
    }
}
