//! Who may see what. Every read path goes through this module.
//!
//! The rules, for a short `s` and a viewer:
//!
//! - the owner sees their short in every state but WITHDRAWN (deleted by
//!   them after a report, and kept only for moderators: see
//!   [`moderator_sees`]);
//! - nobody else sees a short that is not PUBLISHED, that belongs to a
//!   suspended account, or whose owner is in a block relation with them;
//! - the audiences are the video domain's (`tokuly_video::access`, the
//!   social graph's): PUBLIC, NO_FEED and UNLISTED shorts can be *watched*
//!   by anyone who has the link — a **locked** owner's only by their
//!   followers and friends; FOLLOWERS shorts by followers and friends;
//!   FRIENDS by friends; CLOSE_FRIENDS by the owner's close friends; CIRCLE
//!   by the members of that circle (shown to them as FOLLOWERS); PRIVATE by
//!   nobody else;
//! - the channel grid *lists* PUBLIC and NO_FEED shorts, plus the audiences
//!   above the viewer is in (and everything published for the owner);
//! - *feeds* carry PUBLIC shorts, and the ones shared with an audience the
//!   viewer is in;
//! - hashtag pages, sound pages, remix lists and search carry `listed`
//!   shorts only: PUBLIC, not suspended, owner not locked.
//!
//! Locked accounts: `shorts.owner_locked` is this domain's copy of the
//! social domain's setting, kept by `Shorts::sync_owner_lock`; until it
//! catches up, `Viewer::drop_newly_locked` asks the social domain about
//! the rows that reach a viewer only because the copy says "unlocked".
//!
//! The same rules exist twice: as functions over [`ShortFacts`] for single
//! rows, and as SQL predicates ([`watchable!`], [`listed!`], [`in_feed!`],
//! [`public!`]) for queries that select many, which refer to the viewer as
//! `$1`–`$6` (bound by `Viewer::query_as`, see `tokuly_video::access`). The
//! integration test `visibility_decides_who_sees_a_short` runs both over
//! every visibility and kind of viewer and requires them to agree.
use tokuly_proto::{common::Visibility, shorts::ShortStatus};
pub use tokuly_video::access::Viewer;
use tokuly_video::access::is_open;
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
pub const FOLLOWERS: i32 = Visibility::VISIBILITY_FOLLOWERS as i32;
pub const CLOSE_FRIENDS: i32 = Visibility::VISIBILITY_CLOSE_FRIENDS as i32;
pub const CIRCLE: i32 = Visibility::VISIBILITY_CIRCLE as i32;

/// SQL predicate: the viewer may watch short `s` when they have its link.
macro_rules! watchable {
    () => {
        concat!(
            "((s.owner_id = $1 AND s.status <> 4) OR (s.status = 2 AND NOT s.suspended AND NOT (s.owner_id = ANY($3)) AND ",
            tokuly_video::reach!("s", "1, 2, 3"),
            "))"
        )
    };
}

/// SQL predicate: short `s` is listed for the viewer on its channel's grid.
macro_rules! listed {
    () => {
        concat!(
            "(s.status = 2 AND (s.owner_id = $1 OR (NOT s.suspended AND NOT (s.owner_id = ANY($3)) AND ",
            tokuly_video::reach!("s", "1, 2"),
            ")))"
        )
    };
}

/// SQL predicate: short `s` may appear in the viewer's feeds: public, or
/// shared with an audience they are in.
macro_rules! in_feed {
    () => {
        concat!(
            "(s.status = 2 AND NOT s.suspended AND NOT (s.owner_id = ANY($3)) AND ",
            tokuly_video::reach!("s", "1"),
            ")"
        )
    };
}

/// SQL predicate: short `s` is listed for everyone and its owner is not
/// hidden from the viewer (for listings shown to everyone). `s.listed` is
/// exactly "published, PUBLIC, not suspended, owner not locked".
macro_rules! public {
    () => {
        "(s.listed AND NOT (s.owner_id = ANY($3)))"
    };
}

pub(crate) use {in_feed, listed, public, watchable};

/// What access decisions need to know about a short.
#[derive(Debug, Clone, Copy)]
pub struct ShortFacts {
    pub owner_id: Uuid,
    pub status: i32,
    pub visibility: i32,
    pub circle_id: Option<Uuid>,
    pub suspended: bool,
    /// The owner's account is locked, as this domain's copy knows it.
    pub owner_locked: bool,
}

impl ShortFacts {
    /// Published by an account in good standing that the viewer is not in a
    /// block with: the precondition of everything a non-owner may see.
    fn reachable(&self, viewer: &Viewer) -> bool {
        self.status == PUBLISHED && !self.suspended && !viewer.hides(self.owner_id)
    }

    /// Whether the viewer is in the short's audience, `open` naming the
    /// visibilities that count as open to everyone where it is shown.
    fn reaches(&self, viewer: &Viewer, open: bool) -> bool {
        viewer.reaches(self.owner_id, self.visibility, self.circle_id, self.owner_locked, open)
    }

    /// Published, PUBLIC, not suspended and the owner not locked: what
    /// everyone may find.
    pub fn is_public(&self) -> bool {
        self.status == PUBLISHED && self.visibility == PUBLIC && !self.suspended && !self.owner_locked
    }

    /// The owner, whether the short is open to everyone with the link, and
    /// the copy of the owner's lock (for `Viewer::drop_newly_locked`).
    pub fn lock_facts(&self) -> (Uuid, bool, bool) {
        (self.owner_id, is_open(self.visibility), self.owner_locked)
    }
}

/// Whether the viewer may watch the short given its link ([`watchable!`]).
pub fn can_watch(viewer: &Viewer, short: &ShortFacts) -> bool {
    if viewer.is(short.owner_id) {
        return short.status != WITHDRAWN;
    }
    short.reachable(viewer) && short.reaches(viewer, is_open(short.visibility))
}

/// Whether the short is listed for the viewer on its channel's grid ([`listed!`]).
pub fn sees_listed(viewer: &Viewer, short: &ShortFacts) -> bool {
    if viewer.is(short.owner_id) {
        return short.status == PUBLISHED;
    }
    short.reachable(viewer) && short.reaches(viewer, matches!(short.visibility, PUBLIC | NO_FEED))
}

/// Whether the short may appear in the viewer's feeds ([`in_feed!`]). Their
/// own shorts qualify by the same rule as anyone's; the feeds that leave
/// them out say so themselves.
pub fn feeds_carry(viewer: &Viewer, short: &ShortFacts) -> bool {
    short.reachable(viewer) && short.reaches(viewer, short.visibility == PUBLIC)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(owner_id: Uuid, status: i32, visibility: i32) -> ShortFacts {
        ShortFacts { owner_id, status, visibility, circle_id: None, suspended: false, owner_locked: false }
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
        assert!(!feeds_carry(&Viewer::anonymous(), &withdrawn) && !withdrawn.is_public());
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
    }

    #[test]
    fn blocks_and_suspensions_hide_even_public_shorts() {
        let owner = Uuid::now_v7();
        let blocked =
            Viewer { user_id: Some(Uuid::now_v7()), blocked: vec![owner], friends: vec![owner], ..Default::default() };
        let mut short = facts(owner, PUBLISHED, PUBLIC);
        assert!(!can_watch(&blocked, &short) && !sees_listed(&blocked, &short));
        assert!(!feeds_carry(&blocked, &short) && !sees_public(&blocked, &short));
        short.suspended = true;
        assert!(!can_watch(&Viewer::anonymous(), &short) && !short.is_public());
        let me = Viewer { user_id: Some(owner), ..Default::default() };
        assert!(can_watch(&me, &short) && sees_listed(&me, &short));
    }

    #[test]
    fn audiences_and_locks_apply_to_shorts_everywhere() {
        let owner = Uuid::now_v7();
        let follower = Viewer { user_id: Some(Uuid::now_v7()), following: vec![owner], ..Default::default() };
        let stranger = Viewer { user_id: Some(Uuid::now_v7()), ..Default::default() };
        let followers = facts(owner, PUBLISHED, FOLLOWERS);
        assert!(
            can_watch(&follower, &followers)
                && sees_listed(&follower, &followers)
                && feeds_carry(&follower, &followers)
        );
        assert!(!can_watch(&stranger, &followers) && !sees_listed(&stranger, &followers));
        let mut locked = facts(owner, PUBLISHED, PUBLIC);
        locked.owner_locked = true;
        assert!(!locked.is_public(), "a locked owner's shorts are not listed for everyone");
        assert!(can_watch(&follower, &locked) && feeds_carry(&follower, &locked) && !sees_public(&follower, &locked));
        assert!(!can_watch(&stranger, &locked) && !can_watch(&Viewer::anonymous(), &locked));
        let circle = Uuid::now_v7();
        let member = Viewer { circles: vec![circle], ..follower.clone() };
        let mut circled = facts(owner, PUBLISHED, CIRCLE);
        circled.circle_id = Some(circle);
        assert!(can_watch(&member, &circled) && !can_watch(&follower, &circled));
    }
}
