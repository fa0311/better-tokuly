//! Who may see and do what. Every read path goes through this module.
//!
//! Streams follow the rules of videos (`tokuly_video::access`, the social
//! graph's audiences):
//!
//! - the owner sees their stream in every state;
//! - nobody else sees a stream of a suspended account or of someone in a
//!   block relation with them;
//! - PUBLIC, NO_FEED and UNLISTED streams can be *opened* by anyone who has
//!   the link — a **locked** owner's only by their followers and friends;
//!   FOLLOWERS streams by followers and friends, FRIENDS by friends,
//!   CLOSE_FRIENDS by the owner's close friends, CIRCLE by that circle's
//!   members (shown to them as FOLLOWERS), PRIVATE by nobody else;
//! - being *listed* is narrower: a channel page lists PUBLIC and NO_FEED
//!   streams (plus the audiences above the viewer is in), the directories
//!   ("live now", upcoming) PUBLIC ones of unlocked owners, the
//!   subscription and friend lists also the audiences the viewer is in.
//!
//! Locked accounts: `live_streams.owner_locked` is this domain's copy of
//! the social domain's setting (kept by `Live::sync_owner_lock`); until it
//! catches up, `Viewer::drop_newly_locked` / [`can_watch_now`] ask the
//! social domain about streams that reach a viewer only because the copy
//! says "unlocked". The media server treats a stream as open (no playback
//! token) only while it is open and its owner unlocked.
//!
//! The rules exist twice: as functions for single rows ([`can_watch`],
//! [`sees_listed`]) and as SQL predicates over `live_streams s` for the
//! queries that list many (the viewer as `$1`–`$6`, bound by
//! `Viewer::query_as`). The integration test
//! `visibility_decides_who_sees_a_stream` runs both over every visibility
//! and kind of viewer and requires them to agree.
//!
//! Chat standing: the channel's owner and the moderators they appointed
//! ([`Standing`]) may moderate; everyone else is a viewer.
use tokuly_core::prelude::*;
use tokuly_social::graph::ContentAudience;
pub use tokuly_video::access::{
    CIRCLE, CLOSE_FRIENDS, FOLLOWERS, FRIENDS, NO_FEED, PRIVATE, PUBLIC, UNLISTED, Viewer, is_open,
};

/// Stored values of `tokuly.live.v1.StreamStatus`.
pub const UPCOMING: i32 = 1;
pub const LIVE: i32 = 2;
pub const ENDED: i32 = 3;

/// SQL predicate: stream `s` is listed for the viewer on its channel's page
/// (the owner's own view of their channel is a different query).
macro_rules! listed {
    () => {
        concat!("(NOT s.suspended AND NOT (s.owner_id = ANY($3)) AND ", tokuly_video::reach!("s", "1, 2"), ")")
    };
}

/// SQL predicate: stream `s` may appear in the viewer's personal lists
/// (subscriptions, friends): public, or shared with an audience they are in.
macro_rules! in_feed {
    () => {
        concat!("(NOT s.suspended AND NOT (s.owner_id = ANY($3)) AND ", tokuly_video::reach!("s", "1"), ")")
    };
}

/// SQL predicate: stream `s` is public and its owner unlocked (the
/// directories shown to everyone).
macro_rules! public_stream {
    () => {
        "(s.visibility = 1 AND NOT s.suspended AND NOT s.owner_locked AND NOT (s.owner_id = ANY($3)))"
    };
}

/// SQL predicate: stream `s` needs a playback token: not open to everyone,
/// or its owner is locked (for the passes that cut viewers who lost access).
macro_rules! restricted {
    () => {
        "(s.visibility NOT IN (1, 2, 3) OR s.owner_locked)"
    };
}

pub(crate) use {in_feed, listed, public_stream, restricted};

/// What access decisions need to know about a stream.
#[derive(Debug, Clone, Copy)]
pub struct StreamFacts {
    pub owner_id: Uuid,
    pub visibility: i32,
    pub circle_id: Option<Uuid>,
    pub suspended: bool,
    /// The owner's account is locked, as this domain's copy knows it.
    pub owner_locked: bool,
}

impl StreamFacts {
    /// The owner, whether the stream is open to everyone with the link,
    /// and the copy of the owner's lock (for `Viewer::drop_newly_locked`).
    pub fn lock_facts(&self) -> (Uuid, bool, bool) {
        (self.owner_id, is_open(self.visibility), self.owner_locked)
    }
}

/// Whether the viewer may open the stream given its link.
pub fn can_watch(viewer: &Viewer, stream: &StreamFacts) -> bool {
    if viewer.is(stream.owner_id) {
        return true;
    }
    if stream.suspended || viewer.hides(stream.owner_id) {
        return false;
    }
    viewer.reaches(
        stream.owner_id,
        stream.visibility,
        stream.circle_id,
        stream.owner_locked,
        is_open(stream.visibility),
    )
}

/// [`can_watch`], with the owner's lock as the social domain knows it now.
pub async fn can_watch_now(core: &Core, viewer: &Viewer, stream: &StreamFacts) -> AppResult<bool> {
    if !can_watch(viewer, stream) {
        return Ok(false);
    }
    let (owner, open, locked) = stream.lock_facts();
    if !viewer.doubts_lock(owner, open, locked) {
        return Ok(true);
    }
    Ok(!tokuly_video::access::is_locked_now(core, owner).await?)
}

/// Whether the stream is listed for the viewer on its channel page ([`listed!`]).
pub fn sees_listed(viewer: &Viewer, stream: &StreamFacts) -> bool {
    if stream.suspended || viewer.hides(stream.owner_id) {
        return false;
    }
    let open = matches!(stream.visibility, PUBLIC | NO_FEED);
    viewer.reaches(stream.owner_id, stream.visibility, stream.circle_id, stream.owner_locked, open)
}

/// The audience of a stream as the social graph names it, given whether
/// its owner is `locked`: open streams of a locked owner reach followers.
pub fn audience(visibility: i32, circle_id: Option<Uuid>, locked: bool) -> ContentAudience {
    if is_open(visibility) {
        if locked { ContentAudience::Followers } else { ContentAudience::Public }
    } else {
        tokuly_video::access::audience_of(visibility, circle_id)
    }
}

/// Whether going from audience `before` to `after` may leave someone who
/// could watch out.
pub fn narrower(before: ContentAudience, after: ContentAudience) -> bool {
    let contains = after == before
        || after == ContentAudience::Public
        || (after == ContentAudience::Followers && before != ContentAudience::Public);
    !contains
}

/// Whether anyone who knows the stream's name may play it without a token:
/// open to everyone with the link, and its owner not locked.
pub fn open_to_everyone(visibility: i32, owner_locked: bool) -> bool {
    is_open(visibility) && !owner_locked
}

/// A user's standing in the chats of a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    pub is_owner: bool,
    /// Appointed moderator (false for the owner, who needs no appointment).
    pub is_appointed: bool,
}

impl Standing {
    /// May delete messages, restrict users, pin, run polls.
    pub fn can_moderate(&self) -> bool {
        self.is_owner || self.is_appointed
    }
}

/// The standing of `user_id` on the channel owned by `owner_id`.
pub async fn standing(core: &Core, channel_id: Uuid, owner_id: Uuid, user_id: Uuid) -> AppResult<Standing> {
    if user_id == owner_id {
        return Ok(Standing { is_owner: true, is_appointed: false });
    }
    let is_appointed: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM live_moderators WHERE channel_id = $1 AND user_id = $2)")
            .bind(channel_id)
            .bind(user_id)
            .fetch_one(&core.db)
            .await?;
    Ok(Standing { is_owner: false, is_appointed })
}

/// [`standing`], refusing everyone who may not moderate (`NOT_A_MODERATOR`).
pub async fn require_moderator(core: &Core, channel_id: Uuid, owner_id: Uuid, user_id: Uuid) -> AppResult<Standing> {
    let standing = standing(core, channel_id, owner_id, user_id).await?;
    ensure!(standing.can_moderate(), denied, "NOT_A_MODERATOR", "only the streamer and moderators can do this");
    Ok(standing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(owner_id: Uuid, visibility: i32) -> StreamFacts {
        StreamFacts { owner_id, visibility, circle_id: None, suspended: false, owner_locked: false }
    }

    #[test]
    fn owners_see_everything_others_follow_the_visibility() {
        let owner = Uuid::now_v7();
        let me = Viewer { user_id: Some(owner), ..Default::default() };
        let friend = Viewer { user_id: Some(Uuid::now_v7()), friends: vec![owner], ..Default::default() };
        let stranger = Viewer { user_id: Some(Uuid::now_v7()), ..Default::default() };
        for visibility in 1..=5 {
            let stream = facts(owner, visibility);
            assert!(can_watch(&me, &stream));
            assert_eq!(can_watch(&stranger, &stream), matches!(visibility, PUBLIC | NO_FEED | UNLISTED));
            assert_eq!(can_watch(&Viewer::anonymous(), &stream), matches!(visibility, PUBLIC | NO_FEED | UNLISTED));
            assert_eq!(can_watch(&friend, &stream), visibility != PRIVATE);
            assert_eq!(sees_listed(&stranger, &stream), matches!(visibility, PUBLIC | NO_FEED));
            assert_eq!(sees_listed(&friend, &stream), matches!(visibility, PUBLIC | NO_FEED | FRIENDS));
            assert_eq!(open_to_everyone(visibility, false), matches!(visibility, PUBLIC | NO_FEED | UNLISTED));
            assert!(!open_to_everyone(visibility, true), "a locked owner's streams need a token");
        }
    }

    #[test]
    fn blocks_and_suspensions_hide_even_public_streams() {
        let owner = Uuid::now_v7();
        let blocked =
            Viewer { user_id: Some(Uuid::now_v7()), blocked: vec![owner], friends: vec![owner], ..Default::default() };
        let mut stream = facts(owner, PUBLIC);
        assert!(!can_watch(&blocked, &stream) && !sees_listed(&blocked, &stream));
        stream.suspended = true;
        assert!(!can_watch(&Viewer::anonymous(), &stream) && !sees_listed(&Viewer::anonymous(), &stream));
        let me = Viewer { user_id: Some(owner), ..Default::default() };
        assert!(can_watch(&me, &stream));
    }

    #[test]
    fn audiences_and_locks_decide_who_watches_a_stream() {
        let owner = Uuid::now_v7();
        let follower = Viewer { user_id: Some(Uuid::now_v7()), following: vec![owner], ..Default::default() };
        let stranger = Viewer { user_id: Some(Uuid::now_v7()), ..Default::default() };
        let followers = facts(owner, FOLLOWERS);
        assert!(can_watch(&follower, &followers) && sees_listed(&follower, &followers));
        assert!(!can_watch(&stranger, &followers) && !can_watch(&Viewer::anonymous(), &followers));
        let mut locked = facts(owner, PUBLIC);
        locked.owner_locked = true;
        assert!(can_watch(&follower, &locked) && !can_watch(&stranger, &locked));
        let (public, friends) = (ContentAudience::Public, ContentAudience::Friends);
        assert!(narrower(public, ContentAudience::Followers) && narrower(ContentAudience::Followers, friends));
        assert!(!narrower(friends, ContentAudience::Followers) && !narrower(friends, public));
        assert!(narrower(ContentAudience::CloseFriends, friends), "close friends may be followers only");
        assert_eq!(audience(PUBLIC, None, true), ContentAudience::Followers);
    }

    #[test]
    fn only_owners_and_appointed_moderators_moderate() {
        assert!(Standing { is_owner: true, is_appointed: false }.can_moderate());
        assert!(Standing { is_owner: false, is_appointed: true }.can_moderate());
        assert!(!Standing { is_owner: false, is_appointed: false }.can_moderate());
    }
}
