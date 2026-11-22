//! Who may see and do what. Every read path goes through this module.
//!
//! Streams follow the rules of videos (`tokuly_video::access`):
//!
//! - the owner sees their stream in every state;
//! - nobody else sees a stream of a suspended account or of someone in a
//!   block relation with them;
//! - PUBLIC, NO_FEED and UNLISTED streams can be *opened* by anyone who has
//!   the link, FRIENDS streams by friends of the owner, PRIVATE by nobody
//!   else;
//! - being *listed* is narrower: a channel page lists PUBLIC and NO_FEED
//!   streams (plus FRIENDS ones for friends), the directories ("live now",
//!   upcoming) PUBLIC ones, the subscription and friend lists also FRIENDS
//!   ones for friends.
//!
//! The rules exist twice: as functions for single rows ([`can_watch`],
//! [`sees_listed`]) and as SQL predicates over `live_streams s` for the
//! queries that list many. The integration test
//! `visibility_decides_who_sees_a_stream` runs both over every visibility
//! and kind of viewer and requires them to agree.
//!
//! Chat standing: the channel's owner and the moderators they appointed
//! ([`Standing`]) may moderate; everyone else is a viewer.
use tokuly_core::prelude::*;
pub use tokuly_video::access::{FRIENDS, NO_FEED, PRIVATE, PUBLIC, UNLISTED, Viewer};

/// Stored values of `tokuly.live.v1.StreamStatus`.
pub const UPCOMING: i32 = 1;
pub const LIVE: i32 = 2;
pub const ENDED: i32 = 3;

/// SQL predicates over `live_streams s`. Arguments are the placeholder
/// numbers of the viewer's friends and of the users hidden from them
/// (`uuid[]`).
///
/// SQL predicate: stream `s` is listed for the viewer on its channel's page
/// (the owner's own view of their channel is a different query).
macro_rules! listed {
    ($friends:literal, $blocked:literal) => {
        concat!(
            "(NOT s.suspended AND NOT (s.owner_id = ANY($",
            $blocked,
            ")) AND (s.visibility IN (1, 2) OR (s.visibility = 4 AND s.owner_id = ANY($",
            $friends,
            "))))"
        )
    };
}

/// SQL predicate: stream `s` may appear in the viewer's personal lists
/// (subscriptions, friends): public, or friends-only from a friend.
macro_rules! in_feed {
    ($friends:literal, $blocked:literal) => {
        concat!(
            "(NOT s.suspended AND NOT (s.owner_id = ANY($",
            $blocked,
            ")) AND (s.visibility = 1 OR (s.visibility = 4 AND s.owner_id = ANY($",
            $friends,
            "))))"
        )
    };
}

/// SQL predicate: stream `s` is public (the directories shown to everyone).
macro_rules! public_stream {
    ($blocked:literal) => {
        concat!("(s.visibility = 1 AND NOT s.suspended AND NOT (s.owner_id = ANY($", $blocked, ")))")
    };
}

pub(crate) use {in_feed, listed, public_stream};

/// What access decisions need to know about a stream.
#[derive(Debug, Clone, Copy)]
pub struct StreamFacts {
    pub owner_id: Uuid,
    pub visibility: i32,
    pub suspended: bool,
}

/// Whether the viewer may open the stream given its link ([`watchable!`]).
pub fn can_watch(viewer: &Viewer, stream: &StreamFacts) -> bool {
    if viewer.is(stream.owner_id) {
        return true;
    }
    if stream.suspended || viewer.hides(stream.owner_id) {
        return false;
    }
    match stream.visibility {
        PUBLIC | NO_FEED | UNLISTED => true,
        FRIENDS => viewer.friends.contains(&stream.owner_id),
        _ => false,
    }
}

/// Whether the stream is listed for the viewer on its channel page ([`listed!`]).
pub fn sees_listed(viewer: &Viewer, stream: &StreamFacts) -> bool {
    if stream.suspended || viewer.hides(stream.owner_id) {
        return false;
    }
    match stream.visibility {
        PUBLIC | NO_FEED => true,
        FRIENDS => viewer.friends.contains(&stream.owner_id),
        _ => false,
    }
}

/// Whether anyone who knows the stream's name may play it without a token:
/// true for the visibilities that are open to everyone with the link.
pub fn open_to_everyone(visibility: i32) -> bool {
    matches!(visibility, PUBLIC | NO_FEED | UNLISTED)
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
        StreamFacts { owner_id, visibility, suspended: false }
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
            assert_eq!(open_to_everyone(visibility), matches!(visibility, PUBLIC | NO_FEED | UNLISTED));
        }
    }

    #[test]
    fn blocks_and_suspensions_hide_even_public_streams() {
        let owner = Uuid::now_v7();
        let blocked = Viewer { user_id: Some(Uuid::now_v7()), blocked: vec![owner], friends: vec![owner] };
        let mut stream = facts(owner, PUBLIC);
        assert!(!can_watch(&blocked, &stream) && !sees_listed(&blocked, &stream));
        stream.suspended = true;
        assert!(!can_watch(&Viewer::anonymous(), &stream) && !sees_listed(&Viewer::anonymous(), &stream));
        let me = Viewer { user_id: Some(owner), ..Default::default() };
        assert!(can_watch(&me, &stream));
    }

    #[test]
    fn only_owners_and_appointed_moderators_moderate() {
        assert!(Standing { is_owner: true, is_appointed: false }.can_moderate());
        assert!(Standing { is_owner: false, is_appointed: true }.can_moderate());
        assert!(!Standing { is_owner: false, is_appointed: false }.can_moderate());
    }
}
