//! Who may see what. Every read in this crate goes through here.
//!
//! The rules live in two SQL conditions, [`sees_note`] and [`sees_story`],
//! which every query that returns notes or stories embeds. There is no
//! second implementation in Rust to drift away from them: "may this user see
//! that note?" is answered by running the same condition for one row
//! ([`Viewer::can_see_note`]).
//!
//! A [`Viewer`] is the caller as these rules need them: their ID, their
//! friends, and everyone hidden from them by a block in either direction
//! (both lists come from the social domain). The conditions refer to the
//! viewer through three fixed parameters, which [`Viewer::query_as`] and
//! [`Viewer::query_scalar`] bind:
//!
//! | parameter | value |
//! |---|---|
//! | `$1` | the viewer's ID, `NULL` for anonymous callers |
//! | `$2` | IDs of the viewer's friends |
//! | `$3` | IDs hidden from the viewer by a block |
//!
//! A query's own parameters therefore start at `$4`.
use sqlx::{
    FromRow, PgExecutor, Postgres,
    postgres::{PgArguments, PgRow},
    query::{QueryAs, QueryScalar},
};
use tokuly_core::prelude::*;
use tokuly_proto::{common::Visibility, notes::StoryAudience};
use tokuly_social::graph;

/// The visibility values a note can have, as stored.
pub const PUBLIC: i32 = Visibility::VISIBILITY_PUBLIC as i32;
pub const FRIENDS: i32 = Visibility::VISIBILITY_FRIENDS as i32;
pub const PRIVATE: i32 = Visibility::VISIBILITY_PRIVATE as i32;

/// The audiences a story can have, as stored.
pub const AUDIENCE_FRIENDS: i32 = StoryAudience::STORY_AUDIENCE_FRIENDS as i32;
pub const AUDIENCE_CLOSE_FRIENDS: i32 = StoryAudience::STORY_AUDIENCE_CLOSE_FRIENDS as i32;

/// The author (or owner) `{user}` is an account in good standing: content of
/// suspended and deleted accounts is shown to nobody else.
fn active(user: &str) -> String {
    format!("EXISTS (SELECT 1 FROM users u_ WHERE u_.id = {user} AND u_.banned_at IS NULL AND u_.deleted_at IS NULL)")
}

/// SQL condition: the viewer may see the note in table alias `n`.
///
/// - the author always sees their own notes;
/// - nobody sees notes across a block, or notes of a suspended account;
/// - PUBLIC notes are visible to everyone, signed in or not;
/// - FRIENDS notes are visible to the author's friends, and a FRIENDS reply
///   also to the author of the note it answers;
/// - PRIVATE notes are visible to nobody else.
pub fn sees_note(n: &str) -> String {
    format!(
        "({n}.author_id = $1 OR ({n}.author_id <> ALL($3) AND {active} AND ({n}.visibility = {PUBLIC} OR ({n}.visibility = {FRIENDS} AND ({n}.author_id = ANY($2) OR {n}.reply_to_author_id = $1)))))",
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

/// The caller, as the visibility rules need them.
#[derive(Debug, Clone, Default)]
pub struct Viewer {
    pub id: Option<Uuid>,
    friends: Vec<Uuid>,
    hidden: Vec<Uuid>,
}

impl Viewer {
    /// Someone who is not signed in: sees public content only.
    pub fn anonymous() -> Self {
        Self::default()
    }

    /// The viewer for a caller (`None` = anonymous), with their friends and
    /// blocks as of now.
    pub async fn load(core: &Core, id: Option<Uuid>) -> AppResult<Self> {
        let Some(user_id) = id else { return Ok(Self::anonymous()) };
        let (friends, hidden) =
            tokio::try_join!(graph::friend_ids(core, user_id), graph::blocked_ids_either(core, user_id))?;
        Ok(Self { id, friends, hidden })
    }

    pub fn is(&self, user_id: Uuid) -> bool {
        self.id == Some(user_id)
    }

    pub fn friends(&self) -> &[Uuid] {
        &self.friends
    }

    pub fn is_friend(&self, user_id: Uuid) -> bool {
        self.friends.contains(&user_id)
    }

    /// Whether a block (in either direction) hides `user_id` from the viewer.
    pub fn hides(&self, user_id: Uuid) -> bool {
        self.hidden.contains(&user_id)
    }

    /// The viewer and their friends: the people whose posts make up a home
    /// timeline or a story tray.
    pub fn circle(&self) -> Vec<Uuid> {
        self.id.into_iter().chain(self.friends.iter().copied()).collect()
    }

    /// A row query with the viewer bound as `$1`–`$3` (see the module docs).
    /// `sql` is assembled from this crate's own fragments, never from input.
    pub fn query_as<'q, T>(&'q self, sql: String) -> QueryAs<'q, Postgres, T, PgArguments>
    where
        T: for<'r> FromRow<'r, PgRow>,
    {
        sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(self.id).bind(&self.friends).bind(&self.hidden)
    }

    /// A single-value query with the viewer bound as `$1`–`$3`.
    pub fn query_scalar<'q, T>(&'q self, sql: String) -> QueryScalar<'q, Postgres, T, PgArguments>
    where
        (T,): for<'r> FromRow<'r, PgRow>,
    {
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql)).bind(self.id).bind(&self.friends).bind(&self.hidden)
    }

    /// Whether the viewer may see a note. Takes an executor so it can run
    /// inside the transaction that is creating the note.
    pub async fn can_see_note<'e, E: PgExecutor<'e>>(&self, executor: E, note_id: Uuid) -> AppResult<bool> {
        let sql = format!("SELECT EXISTS (SELECT 1 FROM notes n WHERE n.id = $4 AND {})", sees_note("n"));
        Ok(self.query_scalar(sql).bind(note_id).fetch_one(executor).await?)
    }

    /// Whether the viewer may see a story.
    pub async fn can_see_story<'e, E: PgExecutor<'e>>(&self, executor: E, story_id: Uuid) -> AppResult<bool> {
        let sql = format!("SELECT EXISTS (SELECT 1 FROM stories s WHERE s.id = $4 AND {})", sees_story("s"));
        Ok(self.query_scalar(sql).bind(story_id).fetch_one(executor).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_only_use_the_viewer_parameters() {
        for sql in [sees_note("n"), sees_story("s"), not_hidden("l.user_id")] {
            for parameter in sql.split('$').skip(1) {
                let number: String = parameter.chars().take_while(char::is_ascii_digit).collect();
                assert!(matches!(number.as_str(), "1" | "2" | "3"), "unexpected parameter ${number} in {sql}");
            }
            assert_eq!(sql.matches('(').count(), sql.matches(')').count(), "unbalanced: {sql}");
        }
    }

    #[test]
    fn the_circle_is_the_viewer_and_their_friends() {
        let (me, friend) = (Uuid::now_v7(), Uuid::now_v7());
        let viewer = Viewer { id: Some(me), friends: vec![friend], hidden: vec![] };
        assert_eq!(viewer.circle(), vec![me, friend]);
        assert!(viewer.is(me) && viewer.is_friend(friend) && !viewer.hides(friend));
        assert!(Viewer::anonymous().circle().is_empty());
    }
}
