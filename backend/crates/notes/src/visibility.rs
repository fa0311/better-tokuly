//! Who may see what. Every read in this crate goes through here.
//!
//! The rules live in two SQL conditions, [`sees_note`] and [`sees_story`],
//! which every query that returns notes or stories embeds. There is no
//! second implementation in Rust to drift away from them: "may this user see
//! that note?" is answered by running the same condition for one row
//! ([`Viewer::can_see_note`]).
//!
//! The audiences are the social graph's ([`tokuly_social::graph::ContentAudience`]):
//! a [`Viewer`] is the caller as `graph::AudienceViewer` loads them — their
//! friends, the accounts they follow, everyone hidden from them by a block
//! in either direction, and the circles they are in — turned into ID lists
//! the conditions refer to through six fixed parameters, which
//! [`Viewer::query_as`] and [`Viewer::query_scalar`] bind:
//!
//! | parameter | value |
//! |---|---|
//! | `$1` | the viewer's ID, `NULL` for anonymous callers |
//! | `$2` | IDs of the viewer's friends |
//! | `$3` | IDs hidden from the viewer by a block |
//! | `$4` | IDs the viewer is tied to: friends and the accounts they follow |
//! | `$5` | IDs of the accounts whose close friends the viewer is |
//! | `$6` | IDs of the custom circles the viewer is in |
//!
//! A query's own parameters therefore start at `$7`.
//!
//! Locked accounts: a public note reaches everyone only while it is
//! `listed` (its author is not locked, as this domain's copy in
//! `note_authors` knows it; [`crate::authors`] keeps both). Between the
//! moment an account is locked and the moment the copy hears of it, the SQL
//! would still treat its public notes as everyone's;
//! [`Viewer::drop_newly_locked`] asks the social domain about exactly those
//! rows, so the lock applies at once wherever notes are returned.
use std::collections::HashSet;

use sqlx::{
    FromRow, PgExecutor, Postgres,
    postgres::{PgArguments, PgRow},
    query::{QueryAs, QueryScalar},
};
use tokuly_core::prelude::*;
use tokuly_proto::{common::Visibility, notes::StoryAudience};
use tokuly_social::graph::{self, AudienceViewer, ContentAudience};

use crate::model::NoteRow;

/// The visibility values a note can have, as stored.
pub const PUBLIC: i32 = Visibility::VISIBILITY_PUBLIC as i32;
pub const FOLLOWERS: i32 = Visibility::VISIBILITY_FOLLOWERS as i32;
pub const FRIENDS: i32 = Visibility::VISIBILITY_FRIENDS as i32;
pub const CLOSE_FRIENDS: i32 = Visibility::VISIBILITY_CLOSE_FRIENDS as i32;
pub const CIRCLE: i32 = Visibility::VISIBILITY_CIRCLE as i32;
/// Private; a direct note when it names people (`direct_user_ids`).
pub const PRIVATE: i32 = Visibility::VISIBILITY_PRIVATE as i32;

/// The audiences a story can have, as stored.
pub const AUDIENCE_FRIENDS: i32 = StoryAudience::STORY_AUDIENCE_FRIENDS as i32;
pub const AUDIENCE_CLOSE_FRIENDS: i32 = StoryAudience::STORY_AUDIENCE_CLOSE_FRIENDS as i32;

/// The author (or owner) `{user}` is an account in good standing: content of
/// suspended and deleted accounts is shown to nobody else.
fn active(user: &str) -> String {
    format!("EXISTS (SELECT 1 FROM users u_ WHERE u_.id = {user} AND u_.banned_at IS NULL AND u_.deleted_at IS NULL)")
}

/// SQL condition: everyone may see the note in alias `n` (public, author
/// not locked), whatever their relationship to the author. What search,
/// hashtag counts, trends, quotes and other people's renotes go by.
pub fn listed(n: &str) -> String {
    format!("{n}.listed")
}

/// SQL condition: the viewer may see the note in table alias `n`.
///
/// - the author always sees their own notes;
/// - nobody sees notes across a block, or notes of a suspended account;
/// - PUBLIC notes: everyone — a locked author's only their followers and friends;
/// - FOLLOWERS: followers and friends; FRIENDS: friends;
/// - CLOSE_FRIENDS / CIRCLE: members of the author's close-friends circle /
///   of that circle, while they are a friend or follower;
/// - a reply narrower than public (and not private) also reaches the author
///   of the note it answers;
/// - direct notes reach the people they name; PRIVATE notes nobody else.
pub fn sees_note(n: &str) -> String {
    format!(
        "({n}.author_id = $1 OR ({n}.author_id <> ALL($3) AND {active} AND (\
            ({n}.visibility = {PUBLIC} AND ({n}.listed OR {n}.author_id = ANY($4))) \
            OR ({n}.visibility = {FOLLOWERS} AND {n}.author_id = ANY($4)) \
            OR ({n}.visibility = {FRIENDS} AND {n}.author_id = ANY($2)) \
            OR ({n}.visibility = {CLOSE_FRIENDS} AND {n}.author_id = ANY($4) AND {n}.author_id = ANY($5)) \
            OR ({n}.visibility = {CIRCLE} AND {n}.author_id = ANY($4) AND {n}.circle_id = ANY($6)) \
            OR ({n}.visibility NOT IN ({PUBLIC}, {PRIVATE}) AND {n}.reply_to_author_id = $1) \
            OR ({n}.visibility = {PRIVATE} AND $1 = ANY({n}.direct_user_ids)))))",
        active = active(&format!("{n}.author_id")),
    )
}

/// SQL condition: the viewer may see the story in table alias `s`.
///
/// - the owner always sees their own stories (the archive);
/// - friends of the owner see a story while it is live, or for as long as
///   the owner keeps it in a highlight — and a close-friends story only if
///   the owner chose them for it;
/// - nobody else does, and nobody across a block.
pub fn sees_story(s: &str) -> String {
    format!(
        "({s}.owner_id = $1 OR ({s}.owner_id = ANY($2) AND {s}.owner_id <> ALL($3) AND {active} \
           AND ({s}.expires_at > now() OR EXISTS (SELECT 1 FROM story_highlight_items hi_ WHERE hi_.story_id = {s}.id)) \
           AND ({s}.audience = {AUDIENCE_FRIENDS} OR EXISTS (SELECT 1 FROM story_audience sa_ WHERE sa_.story_id = {s}.id AND sa_.user_id = $1))))",
        active = active(&format!("{s}.owner_id")),
    )
}

/// SQL condition: the user in `column` is not hidden from the viewer by a
/// block. For lists of people (likers, renoters, story viewers).
pub fn not_hidden(column: &str) -> String {
    format!("{column} <> ALL($3)")
}

/// The audience a stored note is shared with, as the social graph names it.
/// A direct note is `Private` here (its recipients are this domain's own).
pub fn audience_of(visibility: i32, circle_id: Option<Uuid>) -> ContentAudience {
    ContentAudience::from_stored(visibility, circle_id)
}

/// The caller, as the visibility rules need them.
#[derive(Debug, Clone, Default)]
pub struct Viewer {
    pub id: Option<Uuid>,
    audience: AudienceViewer,
    friends: Vec<Uuid>,
    hidden: Vec<Uuid>,
    tied: Vec<Uuid>,
    close_friend_of: Vec<Uuid>,
    circles: Vec<Uuid>,
}

impl Viewer {
    /// Someone who is not signed in: sees what everyone may see.
    pub fn anonymous() -> Self {
        Self::default()
    }

    /// The viewer for a caller (`None` = anonymous), with their ties and
    /// blocks as of now.
    pub async fn load(core: &Core, id: Option<Uuid>) -> AppResult<Self> {
        let Some(_) = id else { return Ok(Self::anonymous()) };
        Ok(Self::from_audience(AudienceViewer::load(core, id).await?))
    }

    fn from_audience(audience: AudienceViewer) -> Self {
        let mut friends = audience.friend_ids();
        friends.sort_unstable();
        let mut tied: Vec<Uuid> = friends.iter().copied().chain(audience.following_ids()).collect();
        tied.sort_unstable();
        tied.dedup();
        Self {
            id: audience.id(),
            friends,
            hidden: audience.hidden_ids(),
            tied,
            close_friend_of: audience.close_friend_of(),
            circles: audience.circle_ids(),
            audience,
        }
    }

    pub fn is(&self, user_id: Uuid) -> bool {
        self.id == Some(user_id)
    }

    pub fn friends(&self) -> &[Uuid] {
        &self.friends
    }

    pub fn is_friend(&self, user_id: Uuid) -> bool {
        self.friends.binary_search(&user_id).is_ok()
    }

    /// Whether the viewer is a friend or a follower of `user_id`.
    pub fn is_tied_to(&self, user_id: Uuid) -> bool {
        self.tied.binary_search(&user_id).is_ok()
    }

    /// Whether a block (in either direction) hides `user_id` from the viewer.
    pub fn hides(&self, user_id: Uuid) -> bool {
        self.audience.hides(user_id)
    }

    /// The viewer and their friends: the people whose posts make up the
    /// home (friends) timeline or a story tray.
    pub fn me_and_friends(&self) -> Vec<Uuid> {
        self.id.into_iter().chain(self.friends.iter().copied()).collect()
    }

    /// Whether the viewer may see content of `owner` shared with `audience`
    /// (the social graph's rule, for things that are not note rows).
    pub fn sees(&self, owner: Uuid, audience: ContentAudience, owner_locked: bool) -> bool {
        self.audience.sees(owner, audience, owner_locked)
    }

    /// A row query with the viewer bound as `$1`–`$6` (see the module docs).
    /// `sql` is assembled from this crate's own fragments, never from input.
    pub fn query_as<'q, T>(&'q self, sql: String) -> QueryAs<'q, Postgres, T, PgArguments>
    where
        T: for<'r> FromRow<'r, PgRow>,
    {
        sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(self.id)
            .bind(&self.friends)
            .bind(&self.hidden)
            .bind(&self.tied)
            .bind(&self.close_friend_of)
            .bind(&self.circles)
    }

    /// A single-value query with the viewer bound as `$1`–`$6`.
    pub fn query_scalar<'q, T>(&'q self, sql: String) -> QueryScalar<'q, Postgres, T, PgArguments>
    where
        (T,): for<'r> FromRow<'r, PgRow>,
    {
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(self.id)
            .bind(&self.friends)
            .bind(&self.hidden)
            .bind(&self.tied)
            .bind(&self.close_friend_of)
            .bind(&self.circles)
    }

    /// Whether the viewer may see a note. Takes an executor so it can run
    /// inside the transaction that is creating the note.
    pub async fn can_see_note<'e, E: PgExecutor<'e>>(&self, executor: E, note_id: Uuid) -> AppResult<bool> {
        let sql = format!("SELECT EXISTS (SELECT 1 FROM notes n WHERE n.id = $7 AND {})", sees_note("n"));
        Ok(self.query_scalar(sql).bind(note_id).fetch_one(executor).await?)
    }

    /// Whether the viewer may see a story.
    pub async fn can_see_story<'e, E: PgExecutor<'e>>(&self, executor: E, story_id: Uuid) -> AppResult<bool> {
        let sql = format!("SELECT EXISTS (SELECT 1 FROM stories s WHERE s.id = $7 AND {})", sees_story("s"));
        Ok(self.query_scalar(sql).bind(story_id).fetch_one(executor).await?)
    }

    /// Of notes the SQL let through, the authors whose account was locked a
    /// moment ago (the social domain knows, this domain's copy does not yet):
    /// their public notes must not reach the viewer unless they are tied to
    /// the author. Asks the social domain only about such rows.
    pub async fn newly_locked(&self, core: &Core, rows: &[&NoteRow]) -> AppResult<HashSet<Uuid>> {
        let mut doubtful: Vec<Uuid> = rows
            .iter()
            .filter(|row| row.listed && !self.is(row.author_id) && !self.is_tied_to(row.author_id))
            .map(|row| row.author_id)
            .collect();
        if doubtful.is_empty() {
            return Ok(HashSet::new());
        }
        doubtful.sort_unstable();
        doubtful.dedup();
        graph::locked_among(core, &doubtful).await
    }

    /// Takes the rows of [`Self::newly_locked`] authors out of `rows`.
    pub async fn drop_newly_locked<T>(
        &self,
        core: &Core,
        rows: &mut Vec<T>,
        note: impl Fn(&T) -> &NoteRow,
    ) -> AppResult<()> {
        let refs: Vec<&NoteRow> = rows.iter().map(&note).collect();
        let locked = self.newly_locked(core, &refs).await?;
        if !locked.is_empty() {
            rows.retain(|row| {
                let row = note(row);
                !(row.listed && locked.contains(&row.author_id) && !self.is_tied_to(row.author_id))
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_only_use_the_viewer_parameters() {
        for sql in [sees_note("n"), sees_story("s"), not_hidden("l.user_id"), listed("n")] {
            for parameter in sql.split('$').skip(1) {
                let number: String = parameter.chars().take_while(char::is_ascii_digit).collect();
                assert!(
                    matches!(number.as_str(), "1" | "2" | "3" | "4" | "5" | "6"),
                    "unexpected parameter ${number} in {sql}"
                );
            }
            assert_eq!(sql.matches('(').count(), sql.matches(')').count(), "unbalanced: {sql}");
        }
    }

    #[test]
    fn the_home_circle_is_the_viewer_and_their_friends() {
        let (me, friend) = (Uuid::now_v7(), Uuid::now_v7());
        let viewer = Viewer { id: Some(me), friends: vec![friend], tied: vec![friend], ..Default::default() };
        assert_eq!(viewer.me_and_friends(), vec![me, friend]);
        assert!(viewer.is(me) && viewer.is_friend(friend) && viewer.is_tied_to(friend) && !viewer.hides(friend));
        assert!(Viewer::anonymous().me_and_friends().is_empty());
    }

    #[test]
    fn audiences_map_like_the_social_graph() {
        let circle = Uuid::now_v7();
        assert_eq!(audience_of(PUBLIC, None), ContentAudience::Public);
        assert_eq!(audience_of(FOLLOWERS, None), ContentAudience::Followers);
        assert_eq!(audience_of(CIRCLE, Some(circle)), ContentAudience::Circle(circle));
        assert_eq!(audience_of(PRIVATE, None), ContentAudience::Private);
        assert_eq!(audience_of(42, None), ContentAudience::Private);
    }
}
