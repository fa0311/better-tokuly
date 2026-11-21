//! Who may see what. Every read path goes through this module.
//!
//! The rules, for a video `v` and a viewer:
//!
//! - the owner sees their video in every state;
//! - nobody else sees a video that is not PUBLISHED, that belongs to a
//!   suspended account, or whose owner is in a block relation with them;
//! - PUBLIC, NO_FEED and UNLISTED videos can be *watched* by anyone who has
//!   the link; FRIENDS videos by friends of the owner (and by anyone with
//!   the link when the owner set `allow_link`); PRIVATE by nobody else;
//! - being *listed* is narrower: a channel page lists PUBLIC and NO_FEED
//!   videos (plus FRIENDS ones for friends), feeds list PUBLIC ones (the
//!   subscriptions feed also FRIENDS ones for friends).
//!
//! The same rules exist twice: as methods of [`Viewer`] for single rows, and
//! as SQL predicates ([`watchable!`], [`listed!`], [`in_feed!`]) for queries
//! that select many. `tests/…` and the unit tests below keep them in step.
use chrono::{Datelike as _, NaiveDate};
use tokuly_core::prelude::*;
use tokuly_proto::{common::Visibility, video::PlaybackBlock};
use tokuly_social::graph;

/// Stored value of `VideoStatus.VIDEO_STATUS_PUBLISHED`.
pub const PUBLISHED: i32 = 3;
pub const PUBLIC: i32 = Visibility::VISIBILITY_PUBLIC as i32;
pub const NO_FEED: i32 = Visibility::VISIBILITY_NO_FEED as i32;
pub const UNLISTED: i32 = Visibility::VISIBILITY_UNLISTED as i32;
pub const FRIENDS: i32 = Visibility::VISIBILITY_FRIENDS as i32;
pub const PRIVATE: i32 = Visibility::VISIBILITY_PRIVATE as i32;

/// Age from which age-restricted videos may be watched.
pub const ADULT_AGE: u32 = 18;

/// SQL predicate: the viewer may watch video `v` when they have its link.
/// Arguments are the placeholder numbers of the viewer's ID (`uuid`, NULL
/// when anonymous), their friends and the users hidden from them (`uuid[]`).
macro_rules! watchable {
    ($viewer:literal, $friends:literal, $blocked:literal) => {
        concat!(
            "(v.owner_id = $",
            $viewer,
            " OR (v.status = 3 AND NOT v.suspended AND NOT (v.owner_id = ANY($",
            $blocked,
            ")) AND (v.visibility IN (1, 2, 3) OR (v.visibility = 4 AND (v.allow_link OR v.owner_id = ANY($",
            $friends,
            "))))))"
        )
    };
}

/// SQL predicate: video `v` is listed for the viewer on its channel's page.
macro_rules! listed {
    ($viewer:literal, $friends:literal, $blocked:literal) => {
        concat!(
            "(v.status = 3 AND (NOT v.suspended OR v.owner_id = $",
            $viewer,
            ") AND NOT (v.owner_id = ANY($",
            $blocked,
            ")) AND (v.visibility IN (1, 2) OR (v.visibility = 4 AND (v.owner_id = $",
            $viewer,
            " OR v.owner_id = ANY($",
            $friends,
            ")))))"
        )
    };
}

/// SQL predicate: video `v` may appear in the viewer's feeds: public, or
/// friends-only from a friend.
macro_rules! in_feed {
    ($friends:literal, $blocked:literal) => {
        concat!(
            "(v.status = 3 AND NOT v.suspended AND NOT (v.owner_id = ANY($",
            $blocked,
            ")) AND (v.visibility = 1 OR (v.visibility = 4 AND v.owner_id = ANY($",
            $friends,
            "))))"
        )
    };
}

/// SQL predicate: video `v` is public (for listings shown to everyone).
macro_rules! public_video {
    ($blocked:literal) => {
        concat!("(v.status = 3 AND v.visibility = 1 AND NOT v.suspended AND NOT (v.owner_id = ANY($", $blocked, ")))")
    };
}

pub(crate) use {in_feed, listed, public_video, watchable};

/// What access decisions need to know about a video.
#[derive(Debug, Clone, Copy)]
pub struct VideoFacts {
    pub owner_id: Uuid,
    pub status: i32,
    pub visibility: i32,
    pub allow_link: bool,
    pub suspended: bool,
}

/// The caller of a read, with the parts of the social graph the rules need.
#[derive(Debug, Clone, Default)]
pub struct Viewer {
    pub user_id: Option<Uuid>,
    /// The viewer's friends.
    pub friends: Vec<Uuid>,
    /// Users hidden from the viewer: blocked by them or blocking them.
    pub blocked: Vec<Uuid>,
}

impl Viewer {
    pub fn anonymous() -> Self {
        Self::default()
    }

    /// Loads the viewer's friends and blocks (both cached by the social domain).
    pub async fn load(core: &Core, user_id: Option<Uuid>) -> AppResult<Self> {
        let Some(user_id) = user_id else { return Ok(Self::anonymous()) };
        let (friends, blocked) = tokio::try_join!(graph::friend_ids(core, user_id), graph::blocked_ids_either(core, user_id))?;
        Ok(Self { user_id: Some(user_id), friends, blocked })
    }

    pub fn is(&self, user_id: Uuid) -> bool {
        self.user_id == Some(user_id)
    }

    /// Whether `owner_id` and the viewer hide each other (a block either way).
    pub fn hides(&self, owner_id: Uuid) -> bool {
        self.blocked.contains(&owner_id)
    }

    fn is_friend_of(&self, owner_id: Uuid) -> bool {
        self.friends.contains(&owner_id)
    }

    /// Whether the viewer may see the things of `owner_id` at all: their own,
    /// or those of an account that is neither suspended nor blocked.
    pub fn may_see_owner(&self, owner_id: Uuid, suspended: bool) -> bool {
        self.is(owner_id) || (!suspended && !self.hides(owner_id))
    }

    /// Whether the viewer may watch the video given its link ([`watchable!`]).
    pub fn can_watch(&self, video: &VideoFacts) -> bool {
        if self.is(video.owner_id) {
            return true;
        }
        if video.status != PUBLISHED || video.suspended || self.hides(video.owner_id) {
            return false;
        }
        match video.visibility {
            PUBLIC | NO_FEED | UNLISTED => true,
            FRIENDS => video.allow_link || self.is_friend_of(video.owner_id),
            _ => false,
        }
    }

    /// Whether the video is listed for the viewer on its channel page ([`listed!`]).
    pub fn sees_listed(&self, video: &VideoFacts) -> bool {
        if video.status != PUBLISHED || self.hides(video.owner_id) || (video.suspended && !self.is(video.owner_id)) {
            return false;
        }
        match video.visibility {
            PUBLIC | NO_FEED => true,
            FRIENDS => self.is(video.owner_id) || self.is_friend_of(video.owner_id),
            _ => false,
        }
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
pub async fn age_block(core: &Core, viewer: &Viewer, owner_id: Uuid, age_restricted: bool) -> AppResult<Option<PlaybackBlock>> {
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
        VideoFacts { owner_id, status, visibility, allow_link: false, suspended: false }
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
        let blocked = Viewer { user_id: Some(Uuid::now_v7()), blocked: vec![owner], friends: vec![owner] };
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
        assert!(anyone.can_watch(&facts(owner, PUBLISHED, UNLISTED)) && !anyone.sees_listed(&facts(owner, PUBLISHED, UNLISTED)));
        let me = Viewer { user_id: Some(owner), ..Default::default() };
        assert!(!me.sees_listed(&facts(owner, PUBLISHED, PRIVATE)), "private videos live in the studio, not on the page");
        assert!(!me.sees_listed(&facts(owner, 1, PUBLIC)), "drafts are not listed");
    }

    #[test]
    fn age_is_counted_in_full_years() {
        let date = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(age_on(date(2008, 11, 21), date(2026, 11, 21)), 18);
        assert_eq!(age_on(date(2008, 11, 22), date(2026, 11, 21)), 17);
        assert_eq!(age_on(date(2030, 1, 1), date(2026, 11, 21)), 0);
    }
}
