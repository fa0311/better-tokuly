//! Who may see what. Every read path goes through this module.
//!
//! The rules, for a short `s` and a viewer:
//!
//! - the owner sees their short in every state but WITHDRAWN (deleted by
//!   them after a report, and kept only for moderators: see
//!   [`moderator_sees`]);
//! - nobody else sees a short that is not PUBLISHED, that belongs to a
//!   suspended account, or whose owner is in a block relation with them;
//! - PUBLIC, NO_FEED and UNLISTED shorts can be *watched* by anyone who has
//!   the link; FRIENDS shorts by friends of the channel's owner; PRIVATE by
//!   nobody else;
//! - the channel grid *lists* PUBLIC and NO_FEED shorts, plus FRIENDS ones
//!   for friends of the owner (and everything published for the owner);
//! - *feeds* carry PUBLIC shorts, and FRIENDS ones from friends;
//! - hashtag pages, sound pages, remix lists and search are PUBLIC only.
//!
//! The same rules exist twice: as functions over [`ShortFacts`] for single
//! rows, and as SQL predicates ([`watchable!`], [`listed!`], [`in_feed!`],
//! [`public!`]) for queries that select many. The integration test
//! `visibility_decides_who_sees_a_short` runs both over every visibility
//! and kind of viewer and requires them to agree.
//!
//! The viewer type is the video domain's: [`Viewer`] carries the caller's
//! friends and the users hidden from them.
use tokuly_proto::{common::Visibility, shorts::ShortStatus};
pub use tokuly_video::access::Viewer;
use uuid::Uuid;

pub const PROCESSING: i32 = ShortStatus::SHORT_STATUS_PROCESSING as i32;
pub const PUBLISHED: i32 = ShortStatus::SHORT_STATUS_PUBLISHED as i32;
pub const FAILED: i32 = ShortStatus::SHORT_STATUS_FAILED as i32;
pub const WITHDRAWN: i32 = ShortStatus::SHORT_STATUS_WITHDRAWN as i32;

pub const PUBLIC: i32 = Visibility::VISIBILITY_PUBLIC as i32;
pub const NO_FEED: i32 = Visibility::VISIBILITY_NO_FEED as i32;
pub const UNLISTED: i32 = Visibility::VISIBILITY_UNLISTED as i32;
pub const FRIENDS: i32 = Visibility::VISIBILITY_FRIENDS as i32;
pub const PRIVATE: i32 = Visibility::VISIBILITY_PRIVATE as i32;

/// SQL predicate: the viewer may watch short `s` when they have its link.
/// Arguments are the placeholder numbers of the viewer's ID (`uuid`, NULL
/// when anonymous), their friends and the users hidden from them (`uuid[]`).
macro_rules! watchable {
    ($viewer:literal, $friends:literal, $blocked:literal) => {
        concat!(
            "((s.owner_id = $",
            $viewer,
            " AND s.status <> 4) OR (s.status = 2 AND NOT s.suspended AND NOT (s.owner_id = ANY($",
            $blocked,
            ")) AND (s.visibility IN (1, 2, 3) OR (s.visibility = 4 AND s.owner_id = ANY($",
            $friends,
            ")))))"
        )
    };
}

/// SQL predicate: short `s` is listed for the viewer on its channel's grid.
macro_rules! listed {
    ($viewer:literal, $friends:literal, $blocked:literal) => {
        concat!(
            "(s.status = 2 AND (s.owner_id = $",
            $viewer,
            " OR (NOT s.suspended AND NOT (s.owner_id = ANY($",
            $blocked,
            ")) AND (s.visibility IN (1, 2) OR (s.visibility = 4 AND s.owner_id = ANY($",
            $friends,
            "))))))"
        )
    };
}

/// SQL predicate: short `s` may appear in the viewer's feeds: public, or
/// friends-only from a friend.
macro_rules! in_feed {
    ($friends:literal, $blocked:literal) => {
        concat!(
            "(s.status = 2 AND NOT s.suspended AND NOT (s.owner_id = ANY($",
            $blocked,
            ")) AND (s.visibility = 1 OR (s.visibility = 4 AND s.owner_id = ANY($",
            $friends,
            "))))"
        )
    };
}

/// SQL predicate: short `s` is public and its owner is not hidden from the
/// viewer (for listings shown to everyone). `s.listed` is exactly
/// "published, PUBLIC and not suspended".
macro_rules! public {
    ($blocked:literal) => {
        concat!("(s.listed AND NOT (s.owner_id = ANY($", $blocked, ")))")
    };
}

pub(crate) use {in_feed, listed, public, watchable};

/// What access decisions need to know about a short.
#[derive(Debug, Clone, Copy)]
pub struct ShortFacts {
    pub owner_id: Uuid,
    pub status: i32,
    pub visibility: i32,
    pub suspended: bool,
}

impl ShortFacts {
    /// Published by an account in good standing that the viewer is not in a
    /// block with: the precondition of everything a non-owner may see.
    fn reachable(&self, viewer: &Viewer) -> bool {
        self.status == PUBLISHED && !self.suspended && !viewer.hides(self.owner_id)
    }

    fn from_friend(&self, viewer: &Viewer) -> bool {
        viewer.friends.contains(&self.owner_id)
    }

    /// Published, PUBLIC and not suspended: what everyone may find.
    pub fn is_public(&self) -> bool {
        self.status == PUBLISHED && self.visibility == PUBLIC && !self.suspended
    }
}

/// Whether the viewer may watch the short given its link ([`watchable!`]).
pub fn can_watch(viewer: &Viewer, short: &ShortFacts) -> bool {
    if viewer.is(short.owner_id) {
        return short.status != WITHDRAWN;
    }
    short.reachable(viewer)
        && match short.visibility {
            PUBLIC | NO_FEED | UNLISTED => true,
            FRIENDS => short.from_friend(viewer),
            _ => false,
        }
}

/// Whether the short is listed for the viewer on its channel's grid ([`listed!`]).
pub fn sees_listed(viewer: &Viewer, short: &ShortFacts) -> bool {
    if viewer.is(short.owner_id) {
        return short.status == PUBLISHED;
    }
    short.reachable(viewer)
        && match short.visibility {
            PUBLIC | NO_FEED => true,
            FRIENDS => short.from_friend(viewer),
            _ => false,
        }
}

/// Whether the short may appear in the viewer's feeds ([`in_feed!`]). Their
/// own shorts qualify by the same rule as anyone's; the feeds that leave
/// them out say so themselves.
pub fn feeds_carry(viewer: &Viewer, short: &ShortFacts) -> bool {
    short.reachable(viewer)
        && match short.visibility {
            PUBLIC => true,
            FRIENDS => short.from_friend(viewer),
            _ => false,
        }
}

/// Whether the short is shown to this viewer on pages for everyone
/// ([`public!`]).
pub fn sees_public(viewer: &Viewer, short: &ShortFacts) -> bool {
    short.is_public() && !viewer.hides(short.owner_id)
}

/// Whether a moderator may look at a short that they could not watch as
/// an ordinary viewer: only one that has been reported or that its owner
/// withdrew (there is something to review), and never their own (staff
/// powers are not for one's own content). A moderator cannot open
/// somebody's private or friends-only short just by knowing its ID.
pub fn moderator_sees(moderator_id: Uuid, short: &crate::shorts::ShortRow) -> bool {
    short.owner_id != moderator_id && (short.reported_at.is_some() || short.is_withdrawn())
}

/// Whether an account other than the owner can see the short at all, given
/// only its relation to the owner (for deciding whom to notify).
pub fn reaches(short: &ShortFacts, is_friend_of_owner: bool) -> bool {
    short.status == PUBLISHED
        && !short.suspended
        && match short.visibility {
            PUBLIC | NO_FEED | UNLISTED => true,
            FRIENDS => is_friend_of_owner,
            _ => false,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(owner_id: Uuid, status: i32, visibility: i32) -> ShortFacts {
        ShortFacts { owner_id, status, visibility, suspended: false }
    }

    #[test]
    fn owners_see_everything_others_only_what_is_published() {
        let owner = Uuid::now_v7();
        let me = Viewer { user_id: Some(owner), ..Default::default() };
        let stranger = Viewer { user_id: Some(Uuid::now_v7()), ..Default::default() };
        for status in [PROCESSING, PUBLISHED, FAILED, WITHDRAWN] {
            for visibility in PUBLIC..=PRIVATE {
                let short = facts(owner, status, visibility);
                assert_eq!(can_watch(&me, &short), status != WITHDRAWN, "a withdrawn short is gone for its owner too");
                let expected = status == PUBLISHED && matches!(visibility, PUBLIC | NO_FEED | UNLISTED);
                assert_eq!(can_watch(&stranger, &short), expected, "status {status} visibility {visibility}");
                assert_eq!(can_watch(&Viewer::anonymous(), &short), expected);
            }
        }
    }

    #[test]
    fn a_withdrawn_short_is_nowhere() {
        let owner = Uuid::now_v7();
        let withdrawn = facts(owner, WITHDRAWN, PRIVATE);
        assert!(!sees_listed(&Viewer { user_id: Some(owner), ..Default::default() }, &withdrawn));
        assert!(!feeds_carry(&Viewer::anonymous(), &withdrawn) && !withdrawn.is_public() && !reaches(&withdrawn, true));
    }

    #[test]
    fn friends_only_shorts_are_for_friends_everywhere() {
        let owner = Uuid::now_v7();
        let friend = Viewer { user_id: Some(Uuid::now_v7()), friends: vec![owner], ..Default::default() };
        let stranger = Viewer { user_id: Some(Uuid::now_v7()), ..Default::default() };
        let short = facts(owner, PUBLISHED, FRIENDS);
        assert!(can_watch(&friend, &short) && sees_listed(&friend, &short) && feeds_carry(&friend, &short));
        assert!(!can_watch(&stranger, &short) && !sees_listed(&stranger, &short) && !feeds_carry(&stranger, &short));
        assert!(!sees_public(&friend, &short), "hashtag and sound pages are for public shorts");
        assert!(reaches(&short, true) && !reaches(&short, false));
    }

    #[test]
    fn listing_and_feeds_are_narrower_than_watching() {
        let owner = Uuid::now_v7();
        let anyone = Viewer::anonymous();
        let hidden = facts(owner, PUBLISHED, NO_FEED);
        assert!(can_watch(&anyone, &hidden) && sees_listed(&anyone, &hidden) && !feeds_carry(&anyone, &hidden));
        let unlisted = facts(owner, PUBLISHED, UNLISTED);
        assert!(can_watch(&anyone, &unlisted) && !sees_listed(&anyone, &unlisted) && !feeds_carry(&anyone, &unlisted));
        let me = Viewer { user_id: Some(owner), ..Default::default() };
        assert!(sees_listed(&me, &facts(owner, PUBLISHED, PRIVATE)), "the owner's grid shows their private shorts");
        assert!(!sees_listed(&me, &facts(owner, PROCESSING, PUBLIC)), "drafts are not on the grid");
        assert!(!reaches(&facts(owner, PUBLISHED, PRIVATE), true));
    }

    #[test]
    fn blocks_and_suspensions_hide_even_public_shorts() {
        let owner = Uuid::now_v7();
        let blocked = Viewer { user_id: Some(Uuid::now_v7()), blocked: vec![owner], friends: vec![owner] };
        let mut short = facts(owner, PUBLISHED, PUBLIC);
        assert!(!can_watch(&blocked, &short) && !sees_listed(&blocked, &short));
        assert!(!feeds_carry(&blocked, &short) && !sees_public(&blocked, &short));
        short.suspended = true;
        assert!(!can_watch(&Viewer::anonymous(), &short) && !short.is_public());
        let me = Viewer { user_id: Some(owner), ..Default::default() };
        assert!(can_watch(&me, &short) && sees_listed(&me, &short));
    }
}
