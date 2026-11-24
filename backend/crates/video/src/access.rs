//! Who may see what. Every read path goes through this module, and the
//! shorts and live domains use it too (their content follows the same
//! audiences).
//!
//! The rules, for a video `v` and a viewer:
//!
//! - the owner sees their video in every state;
//! - nobody else sees a video that is not PUBLISHED, that belongs to a
//!   suspended account, or whose owner is in a block relation with them;
//! - the audiences are the social graph's ([`ContentAudience`]):
//!
//! | visibility | may watch it (given the link) |
//! |---|---|
//! | PUBLIC, NO_FEED, UNLISTED | everyone — but a **locked** owner's only their followers and friends |
//! | FOLLOWERS (6) | the owner's followers and friends |
//! | FRIENDS (4) | the owner's friends; with `allow_link` also anyone with the link (a locked owner's: followers) |
//! | CLOSE_FRIENDS (7) | members of the owner's close-friends circle who are still a friend or follower |
//! | CIRCLE (8) | members of that custom circle of the owner (same); to anyone but the owner the video reads as FOLLOWERS |
//! | PRIVATE | nobody else |
//!
//! - being *listed* is narrower: a channel page lists PUBLIC and NO_FEED
//!   videos plus those of the audiences above the viewer is in; the
//!   subscriptions feed PUBLIC ones plus the same audiences; directories
//!   for everyone (trending, search, related, hashtag-like lists) only
//!   PUBLIC videos of unlocked owners.
//!
//! Locked accounts: whether an owner is locked is the social domain's. This
//! domain copies it onto its rows (`owner_locked` on channels, videos and
//! playlists, kept by [`crate::Video::sync_owner_lock`] on
//! `social.follow.settings_changed`) so SQL can decide. Between a lock and
//! its event the copy is behind; [`Viewer::newly_locked`] asks the social
//! domain about exactly the rows that reach the viewer only because the
//! copy says "unlocked", so a lock applies at the next read.
//!
//! The rules exist twice: as methods of [`Viewer`] for single rows, and as
//! SQL predicates ([`watchable!`], [`listed!`], [`in_feed!`],
//! [`public_video!`], and [`reach!`] for the other domains) for queries that
//! select many. The SQL refers to the viewer through six fixed placeholders,
//! which [`Viewer::query_as`], [`Viewer::query_scalar`] and [`Viewer::query`]
//! bind:
//!
//! | placeholder | value |
//! |---|---|
//! | `$1` | the viewer's ID, `NULL` for anonymous callers |
//! | `$2` | the viewer's friends |
//! | `$3` | users hidden from the viewer by a block (either way) |
//! | `$4` | users the viewer is tied to: friends and accounts they follow |
//! | `$5` | users whose close-friends circle the viewer is in |
//! | `$6` | the custom circles the viewer is in |
//!
//! A query's own parameters therefore start at `$7`. The integration test
//! `visibility_decides_who_can_watch_by_link` runs both over every
//! visibility and kind of viewer and requires them to agree.
use std::collections::HashSet;

use chrono::{Datelike as _, NaiveDate};
use sqlx::{
    FromRow, Postgres,
    postgres::{PgArguments, PgRow},
    query::{Query, QueryAs, QueryScalar},
};
use tokuly_core::prelude::*;
use tokuly_proto::{common::Visibility, video::PlaybackBlock};
use tokuly_social::graph::{self, AudienceViewer, ContentAudience};

/// Stored value of `VideoStatus.VIDEO_STATUS_PUBLISHED`.
pub const PUBLISHED: i32 = 3;
pub const PUBLIC: i32 = Visibility::VISIBILITY_PUBLIC as i32;
pub const NO_FEED: i32 = Visibility::VISIBILITY_NO_FEED as i32;
pub const UNLISTED: i32 = Visibility::VISIBILITY_UNLISTED as i32;
pub const FRIENDS: i32 = Visibility::VISIBILITY_FRIENDS as i32;
pub const PRIVATE: i32 = Visibility::VISIBILITY_PRIVATE as i32;
pub const FOLLOWERS: i32 = Visibility::VISIBILITY_FOLLOWERS as i32;
pub const CLOSE_FRIENDS: i32 = Visibility::VISIBILITY_CLOSE_FRIENDS as i32;
pub const CIRCLE: i32 = Visibility::VISIBILITY_CIRCLE as i32;

/// Age from which age-restricted videos may be watched.
pub const ADULT_AGE: u32 = 18;

/// SQL predicate: a viewer who is neither the owner nor hidden from them is
/// in the audience of the row in table alias `$t` (columns `owner_id`,
/// `visibility`, `circle_id`, `owner_locked`). `$open` lists the
/// visibilities that count as open to everyone here (for example
/// `"1, 2, 3"` to watch by link, `"1, 2"` on a channel page). With `link`,
/// a FRIENDS row with `allow_link` is open too. Uses the viewer placeholders
/// `$2`, `$4`–`$6` (see the module docs).
#[macro_export]
macro_rules! reach {
    ($t:literal, $open:literal) => {
        concat!(
            "((",
            $t,
            ".visibility IN (",
            $open,
            ") AND (NOT ",
            $t,
            ".owner_locked OR ",
            $t,
            ".owner_id = ANY($4))) \
             OR (",
            $t,
            ".visibility = 4 AND ",
            $t,
            ".owner_id = ANY($2)) \
             OR (",
            $t,
            ".visibility = 6 AND ",
            $t,
            ".owner_id = ANY($4)) \
             OR (",
            $t,
            ".visibility = 7 AND ",
            $t,
            ".owner_id = ANY($4) AND ",
            $t,
            ".owner_id = ANY($5)) \
             OR (",
            $t,
            ".visibility = 8 AND ",
            $t,
            ".owner_id = ANY($4) AND ",
            $t,
            ".circle_id = ANY($6)))"
        )
    };
    ($t:literal, $open:literal, link) => {
        concat!(
            "(",
            $crate::reach!($t, $open),
            " OR (",
            $t,
            ".visibility = 4 AND ",
            $t,
            ".allow_link AND (NOT ",
            $t,
            ".owner_locked OR ",
            $t,
            ".owner_id = ANY($4))))"
        )
    };
}

/// SQL predicate: the viewer may watch video `v` when they have its link.
macro_rules! watchable {
    () => {
        concat!(
            "(v.owner_id = $1 OR (v.status = 3 AND NOT v.suspended AND NOT (v.owner_id = ANY($3)) AND ",
            $crate::reach!("v", "1, 2, 3", link),
            "))"
        )
    };
}

/// SQL predicate: video `v` is listed for the viewer on its channel's page.
/// The owner's page shows them everything published but PRIVATE and UNLISTED.
macro_rules! listed {
    () => {
        concat!(
            "(v.status = 3 AND NOT (v.owner_id = ANY($3)) AND ((v.owner_id = $1 AND v.visibility IN (1, 2, 4, 6, 7, 8)) \
             OR (NOT v.suspended AND ",
            $crate::reach!("v", "1, 2"),
            ")))"
        )
    };
}

/// SQL predicate: video `v` may appear in the viewer's own feeds
/// (subscriptions, friends): public, or shared with an audience they are in.
macro_rules! in_feed {
    () => {
        concat!("(v.status = 3 AND NOT v.suspended AND NOT (v.owner_id = ANY($3)) AND ", $crate::reach!("v", "1"), ")")
    };
}

/// SQL predicate: video `v` is public and its owner unlocked (for listings
/// shown to everyone), and not hidden from the viewer.
macro_rules! public_video {
    () => {
        "(v.status = 3 AND v.visibility = 1 AND NOT v.suspended AND NOT v.owner_locked AND NOT (v.owner_id = ANY($3)))"
    };
}

pub(crate) use {in_feed, listed, public_video, watchable};

/// Whether a visibility is open to everyone who has the link.
pub fn is_open(visibility: i32) -> bool {
    matches!(visibility, PUBLIC | NO_FEED | UNLISTED)
}

/// The audience of content stored with `visibility` and `circle_id`, as the
/// social graph names it. A number this build does not know is `Private`.
pub fn audience_of(visibility: i32, circle_id: Option<Uuid>) -> ContentAudience {
    ContentAudience::from_stored(visibility, circle_id)
}

/// What a non-owner is shown as the visibility of content: a custom circle
/// reads as FOLLOWERS (members never learn about the circle).
pub fn shown_visibility(visibility: i32, is_owner: bool) -> i32 {
    if visibility == CIRCLE && !is_owner { FOLLOWERS } else { visibility }
}

/// The circle stored with an audience a creator chose: for
/// VISIBILITY_CIRCLE one of the owner's circles (`CIRCLE_REQUIRED` without
/// one, `CIRCLE_NOT_FOUND` for anyone else's), nothing for the others.
pub async fn chosen_circle(
    core: &Core,
    owner_id: Uuid,
    visibility: i32,
    circle_id: Option<Uuid>,
) -> AppResult<Option<Uuid>> {
    if visibility != CIRCLE {
        return Ok(None);
    }
    let circle_id = circle_id.ok_or_else(|| AppError::invalid("CIRCLE_REQUIRED", "choose one of your circles"))?;
    graph::ensure_circle_of(core, owner_id, circle_id).await?;
    Ok(Some(circle_id))
}

/// Whether the account is locked right now, as the social domain says.
pub async fn is_locked_now(core: &Core, owner_id: Uuid) -> AppResult<bool> {
    Ok(graph::locked_among(core, &[owner_id]).await?.contains(&owner_id))
}

/// What access decisions need to know about a video.
#[derive(Debug, Clone, Copy)]
pub struct VideoFacts {
    pub owner_id: Uuid,
    pub status: i32,
    pub visibility: i32,
    pub circle_id: Option<Uuid>,
    pub allow_link: bool,
    pub suspended: bool,
    /// The owner's account is locked, as this domain's copy knows it.
    pub owner_locked: bool,
}

/// The caller of a read, with the parts of the social graph the rules need.
#[derive(Debug, Clone, Default)]
pub struct Viewer {
    pub user_id: Option<Uuid>,
    /// The viewer's friends.
    pub friends: Vec<Uuid>,
    /// Users hidden from the viewer: blocked by them or blocking them.
    pub blocked: Vec<Uuid>,
    /// Accounts the viewer follows (friends are not necessarily among them).
    pub following: Vec<Uuid>,
    /// Owners whose close-friends circle the viewer is in.
    pub close_friend_of: Vec<Uuid>,
    /// Custom (and close-friends) circles the viewer is in.
    pub circles: Vec<Uuid>,
}

impl Viewer {
    pub fn anonymous() -> Self {
        Self::default()
    }

    /// Loads the viewer's ties, blocks and circles (cached by the social domain).
    pub async fn load(core: &Core, user_id: Option<Uuid>) -> AppResult<Self> {
        let Some(_) = user_id else { return Ok(Self::anonymous()) };
        let audience = AudienceViewer::load(core, user_id).await?;
        Ok(Self {
            user_id,
            friends: audience.friend_ids(),
            blocked: audience.hidden_ids(),
            following: audience.following_ids(),
            close_friend_of: audience.close_friend_of(),
            circles: audience.circle_ids(),
        })
    }

    pub fn is(&self, user_id: Uuid) -> bool {
        self.user_id == Some(user_id)
    }

    /// Whether `owner_id` and the viewer hide each other (a block either way).
    pub fn hides(&self, owner_id: Uuid) -> bool {
        self.blocked.contains(&owner_id)
    }

    pub fn is_friend_of(&self, owner_id: Uuid) -> bool {
        self.friends.contains(&owner_id)
    }

    /// Whether the viewer is a friend or a follower of `owner_id`.
    pub fn is_tied_to(&self, owner_id: Uuid) -> bool {
        self.friends.contains(&owner_id) || self.following.contains(&owner_id)
    }

    /// Friends and followed accounts, for the SQL placeholder `$4`.
    fn tied(&self) -> Vec<Uuid> {
        let mut tied: Vec<Uuid> = self.friends.iter().chain(&self.following).copied().collect();
        tied.sort_unstable();
        tied.dedup();
        tied
    }

    /// Whether the viewer may see the things of `owner_id` at all: their own,
    /// or those of an account that is neither suspended nor blocked.
    pub fn may_see_owner(&self, owner_id: Uuid, suspended: bool) -> bool {
        self.is(owner_id) || (!suspended && !self.hides(owner_id))
    }

    /// Whether a viewer who is not the owner (and not hidden from them) is in
    /// the audience `visibility` / `circle_id` of `owner_id`'s content
    /// ([`reach!`]). `open`: whether the visibility counts as open to
    /// everyone where the content is shown.
    pub fn reaches(&self, owner_id: Uuid, visibility: i32, circle_id: Option<Uuid>, locked: bool, open: bool) -> bool {
        let tied = self.is_tied_to(owner_id);
        if open {
            return !locked || tied;
        }
        match visibility {
            FRIENDS => self.is_friend_of(owner_id),
            FOLLOWERS => tied,
            CLOSE_FRIENDS => tied && self.close_friend_of.contains(&owner_id),
            CIRCLE => tied && circle_id.is_some_and(|circle| self.circles.contains(&circle)),
            _ => false,
        }
    }

    /// Whether the viewer may watch the video given its link ([`watchable!`]).
    pub fn can_watch(&self, video: &VideoFacts) -> bool {
        if self.is(video.owner_id) {
            return true;
        }
        if video.status != PUBLISHED || video.suspended || self.hides(video.owner_id) {
            return false;
        }
        let open = is_open(video.visibility) || (video.visibility == FRIENDS && video.allow_link);
        self.reaches(video.owner_id, video.visibility, video.circle_id, video.owner_locked, open)
            || self.reaches(video.owner_id, video.visibility, video.circle_id, video.owner_locked, false)
    }

    /// Whether the video is listed for the viewer on its channel page ([`listed!`]).
    pub fn sees_listed(&self, video: &VideoFacts) -> bool {
        if video.status != PUBLISHED || self.hides(video.owner_id) {
            return false;
        }
        if self.is(video.owner_id) {
            return !matches!(video.visibility, PRIVATE | UNLISTED);
        }
        let open = matches!(video.visibility, PUBLIC | NO_FEED);
        !video.suspended && self.reaches(video.owner_id, video.visibility, video.circle_id, video.owner_locked, open)
    }

    /// Whether the viewer reaches content of `owner_id` only because this
    /// domain's copy says the owner is not locked: content open to everyone
    /// (`open`), of an owner the copy calls unlocked, shown to someone who
    /// is neither the owner nor tied to them. Only such content can change
    /// with a lock whose event has not arrived yet.
    pub fn doubts_lock(&self, owner_id: Uuid, open: bool, owner_locked: bool) -> bool {
        open && !owner_locked && !self.is(owner_id) && !self.is_tied_to(owner_id)
    }

    /// Of `owners` (for content the viewer [`doubts_lock`](Self::doubts_lock)
    /// about), those locked right now: their content must not reach the
    /// viewer. Asks the social domain only when there is something to ask.
    pub async fn newly_locked(&self, core: &Core, owners: impl IntoIterator<Item = Uuid>) -> AppResult<HashSet<Uuid>> {
        let mut owners: Vec<Uuid> = owners.into_iter().collect();
        if owners.is_empty() {
            return Ok(HashSet::new());
        }
        owners.sort_unstable();
        owners.dedup();
        graph::locked_among(core, &owners).await
    }

    /// Whether the viewer may watch the video, with the owner's lock as the
    /// social domain knows it right now ([`Self::can_watch`] plus
    /// [`Self::newly_locked`]).
    pub async fn can_watch_now(&self, core: &Core, video: &VideoFacts) -> AppResult<bool> {
        if !self.can_watch(video) {
            return Ok(false);
        }
        let open = is_open(video.visibility) || (video.visibility == FRIENDS && video.allow_link);
        if !self.doubts_lock(video.owner_id, open, video.owner_locked) {
            return Ok(true);
        }
        if self.reaches(video.owner_id, video.visibility, video.circle_id, video.owner_locked, false) {
            return Ok(true); // a friend of a FRIENDS video with a link
        }
        Ok(!is_locked_now(core, video.owner_id).await?)
    }

    /// Drops the rows of `rows` that reach the viewer only through a lock
    /// this domain has not heard of yet. `facts` gives a row's owner,
    /// whether the row is shown as open to everyone, and the copy of the
    /// owner's lock.
    pub async fn drop_newly_locked<R>(
        &self,
        core: &Core,
        rows: &mut Vec<R>,
        facts: impl Fn(&R) -> (Uuid, bool, bool),
    ) -> AppResult<()> {
        let doubtful: Vec<Uuid> = rows
            .iter()
            .map(&facts)
            .filter(|(owner, open, locked)| self.doubts_lock(*owner, *open, *locked))
            .map(|(owner, _, _)| owner)
            .collect();
        let locked = self.newly_locked(core, doubtful).await?;
        if !locked.is_empty() {
            rows.retain(|row| {
                let (owner, open, copy) = facts(row);
                !(self.doubts_lock(owner, open, copy) && locked.contains(&owner))
            });
        }
        Ok(())
    }

    /// A row query with the viewer bound as `$1`–`$6` (see the module docs).
    pub fn query_as<'q, T>(&'q self, sql: impl sqlx::SqlSafeStr) -> QueryAs<'q, Postgres, T, PgArguments>
    where
        T: for<'r> FromRow<'r, PgRow>,
    {
        sqlx::query_as(sql)
            .bind(self.user_id)
            .bind(&self.friends)
            .bind(&self.blocked)
            .bind(self.tied())
            .bind(&self.close_friend_of)
            .bind(&self.circles)
    }

    /// A single-value query with the viewer bound as `$1`–`$6`.
    pub fn query_scalar<'q, T>(&'q self, sql: impl sqlx::SqlSafeStr) -> QueryScalar<'q, Postgres, T, PgArguments>
    where
        (T,): for<'r> FromRow<'r, PgRow>,
    {
        sqlx::query_scalar(sql)
            .bind(self.user_id)
            .bind(&self.friends)
            .bind(&self.blocked)
            .bind(self.tied())
            .bind(&self.close_friend_of)
            .bind(&self.circles)
    }

    /// A statement with the viewer bound as `$1`–`$6`.
    pub fn query<'q>(&'q self, sql: impl sqlx::SqlSafeStr) -> Query<'q, Postgres, PgArguments> {
        sqlx::query(sql)
            .bind(self.user_id)
            .bind(&self.friends)
            .bind(&self.blocked)
            .bind(self.tied())
            .bind(&self.close_friend_of)
            .bind(&self.circles)
    }
}

/// Full years between `birthdate` and `today`.
pub fn age_on(birthdate: NaiveDate, today: NaiveDate) -> u32 {
    let mut years = today.year() - birthdate.year();
    if (today.month(), today.day()) < (birthdate.month(), birthdate.day()) {
        years -= 1;
    }
    years.max(0) as u32
}

/// Why the viewer may not play an age-restricted video, if so. The owner
/// always may; everyone else needs an account with a birthdate that makes
/// them an adult.
pub async fn age_block(
    core: &Core,
    viewer: &Viewer,
    owner_id: Uuid,
    age_restricted: bool,
) -> AppResult<Option<PlaybackBlock>> {
    if !age_restricted || viewer.is(owner_id) {
        return Ok(None);
    }
    let Some(user_id) = viewer.user_id else {
        return Ok(Some(PlaybackBlock::PLAYBACK_BLOCK_SIGN_IN_REQUIRED));
    };
    let birthdate: Option<Option<NaiveDate>> =
        sqlx::query_scalar("SELECT birthdate FROM users WHERE id = $1").bind(user_id).fetch_optional(&core.db).await?;
    let adult = birthdate.flatten().is_some_and(|date| age_on(date, now().date_naive()) >= ADULT_AGE);
    Ok((!adult).then_some(PlaybackBlock::PLAYBACK_BLOCK_AGE_RESTRICTED))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(owner_id: Uuid, status: i32, visibility: i32) -> VideoFacts {
        VideoFacts {
            owner_id,
            status,
            visibility,
            circle_id: None,
            allow_link: false,
            suspended: false,
            owner_locked: false,
        }
    }

    #[test]
    fn owners_see_everything_others_only_what_is_published() {
        let owner = Uuid::now_v7();
        let me = Viewer { user_id: Some(owner), ..Default::default() };
        let stranger = Viewer { user_id: Some(Uuid::now_v7()), ..Default::default() };
        for status in 1..=3 {
            for visibility in 1..=5 {
                let video = facts(owner, status, visibility);
                assert!(me.can_watch(&video));
                let expected = status == PUBLISHED && matches!(visibility, PUBLIC | NO_FEED | UNLISTED);
                assert_eq!(stranger.can_watch(&video), expected, "status {status} visibility {visibility}");
                assert_eq!(Viewer::anonymous().can_watch(&video), expected);
            }
        }
    }

    #[test]
    fn friends_only_videos_need_friendship_or_an_allowed_link() {
        let owner = Uuid::now_v7();
        let friend = Viewer { user_id: Some(Uuid::now_v7()), friends: vec![owner], ..Default::default() };
        let stranger = Viewer { user_id: Some(Uuid::now_v7()), ..Default::default() };
        let mut video = facts(owner, PUBLISHED, FRIENDS);
        assert!(friend.can_watch(&video) && friend.sees_listed(&video));
        assert!(!stranger.can_watch(&video) && !stranger.sees_listed(&video));
        video.allow_link = true;
        // The link opens the video; the listing stays friends-only.
        assert!(stranger.can_watch(&video) && Viewer::anonymous().can_watch(&video));
        assert!(!stranger.sees_listed(&video));
    }

    #[test]
    fn blocks_and_suspensions_hide_even_public_videos() {
        let owner = Uuid::now_v7();
        let blocked =
            Viewer { user_id: Some(Uuid::now_v7()), blocked: vec![owner], friends: vec![owner], ..Default::default() };
        let mut video = facts(owner, PUBLISHED, PUBLIC);
        assert!(!blocked.can_watch(&video) && !blocked.sees_listed(&video));
        video.suspended = true;
        assert!(!Viewer::anonymous().can_watch(&video));
        let me = Viewer { user_id: Some(owner), ..Default::default() };
        assert!(me.can_watch(&video) && me.sees_listed(&video));
    }

    #[test]
    fn listing_is_narrower_than_watching() {
        let owner = Uuid::now_v7();
        let anyone = Viewer::anonymous();
        assert!(anyone.sees_listed(&facts(owner, PUBLISHED, NO_FEED)));
        assert!(
            anyone.can_watch(&facts(owner, PUBLISHED, UNLISTED))
                && !anyone.sees_listed(&facts(owner, PUBLISHED, UNLISTED))
        );
        let me = Viewer { user_id: Some(owner), ..Default::default() };
        assert!(
            !me.sees_listed(&facts(owner, PUBLISHED, PRIVATE)),
            "private videos live in the studio, not on the page"
        );
        assert!(!me.sees_listed(&facts(owner, 1, PUBLIC)), "drafts are not listed");
    }

    #[test]
    fn followers_close_friends_and_circles_reach_their_members_only() {
        let owner = Uuid::now_v7();
        let circle = Uuid::now_v7();
        let follower = Viewer { user_id: Some(Uuid::now_v7()), following: vec![owner], ..Default::default() };
        let friend = Viewer { user_id: Some(Uuid::now_v7()), friends: vec![owner], ..Default::default() };
        let close = Viewer {
            user_id: Some(Uuid::now_v7()),
            following: vec![owner],
            close_friend_of: vec![owner],
            circles: vec![circle],
            ..Default::default()
        };
        let untied_member = Viewer { user_id: Some(Uuid::now_v7()), circles: vec![circle], ..Default::default() };
        let stranger = Viewer { user_id: Some(Uuid::now_v7()), ..Default::default() };
        let followers = facts(owner, PUBLISHED, FOLLOWERS);
        assert!(follower.can_watch(&followers) && friend.can_watch(&followers) && follower.sees_listed(&followers));
        assert!(!stranger.can_watch(&followers) && !Viewer::anonymous().can_watch(&followers));
        let close_friends = facts(owner, PUBLISHED, CLOSE_FRIENDS);
        assert!(close.can_watch(&close_friends) && !follower.can_watch(&close_friends));
        let mut in_circle = facts(owner, PUBLISHED, CIRCLE);
        in_circle.circle_id = Some(circle);
        assert!(close.can_watch(&in_circle) && close.sees_listed(&in_circle));
        assert!(!untied_member.can_watch(&in_circle), "a member who is no longer tied to the owner sees nothing");
        assert!(!follower.can_watch(&in_circle));
        in_circle.circle_id = None;
        assert!(!close.can_watch(&in_circle), "a circle video without its circle reaches nobody");
        assert_eq!(shown_visibility(CIRCLE, false), FOLLOWERS);
        assert_eq!(shown_visibility(CIRCLE, true), CIRCLE);
    }

    #[test]
    fn a_locked_owners_open_videos_reach_followers_and_friends_only() {
        let owner = Uuid::now_v7();
        let follower = Viewer { user_id: Some(Uuid::now_v7()), following: vec![owner], ..Default::default() };
        let friend = Viewer { user_id: Some(Uuid::now_v7()), friends: vec![owner], ..Default::default() };
        let stranger = Viewer { user_id: Some(Uuid::now_v7()), ..Default::default() };
        for visibility in [PUBLIC, NO_FEED, UNLISTED] {
            let mut video = facts(owner, PUBLISHED, visibility);
            video.owner_locked = true;
            assert!(follower.can_watch(&video) && friend.can_watch(&video), "visibility {visibility}");
            assert!(!stranger.can_watch(&video) && !Viewer::anonymous().can_watch(&video), "visibility {visibility}");
            assert!(stranger.doubts_lock(owner, true, false) && !follower.doubts_lock(owner, true, false));
        }
        let mut linked = facts(owner, PUBLISHED, FRIENDS);
        linked.allow_link = true;
        linked.owner_locked = true;
        assert!(friend.can_watch(&linked) && follower.can_watch(&linked) && !stranger.can_watch(&linked));
    }

    #[test]
    fn age_is_counted_in_full_years() {
        let date = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(age_on(date(2008, 11, 21), date(2026, 11, 21)), 18);
        assert_eq!(age_on(date(2008, 11, 22), date(2026, 11, 21)), 17);
        assert_eq!(age_on(date(2030, 1, 1), date(2026, 11, 21)), 0);
    }
}
