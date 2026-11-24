//! Who may see a piece of content, for every audience the social graph can
//! express. Re-exported from [`crate::graph`]; this is what content domains
//! (notes, video, shorts, live, community, feeds) ask instead of each
//! reinventing it.
//!
//! The audiences ([`ContentAudience`]) and who is in them, for content of
//! `owner` and a signed-in viewer who is not in a block with the owner:
//!
//! | audience | who sees it |
//! |---|---|
//! | `Public` | everyone, signed in or not — but a **locked** owner's public content only reaches their followers and friends |
//! | `Followers` | the owner's followers **and** friends |
//! | `Friends` | the owner's friends |
//! | `CloseFriends` | members of the owner's close-friends circle |
//! | `Circle(id)` | members of that circle of the owner |
//! | `Private` | nobody but the owner |
//!
//! Why "followers" includes friends: a friendship does not imply a follow
//! (the two are separate relationships with separate lists and counts), but
//! as an audience friends are the closer tier, and something shared with
//! followers is not hidden from friends.
//!
//! Always: the owner sees their own content; nobody sees anything across a
//! block in either direction; signed-out viewers see `Public` content of
//! unlocked owners only. A circle member counts only while they are still a
//! friend or a follower of the owner (members are taken out when that ends;
//! the check here also covers a member added at the moment the tie ended).
//!
//! What this module does not decide: the owner's account state (content of
//! suspended or deleted accounts), link-only access, age restrictions and
//! the like stay with the content domain, as today.
//!
//! The close-friends mark: content shared with `CloseFriends` may show its
//! viewers a mark (they can tell they are a close friend by seeing it).
//! Custom circles carry no mark: their members never learn about them.
//!
//! Three ways to ask:
//! - one item: [`may_see`] (reads PostgreSQL, so a change applies at once);
//! - a page of a feed: [`may_see_many`], or [`AudienceViewer::load`] once and
//!   [`AudienceViewer::sees`] per item, with [`locked_among`] for the owners
//!   (cached lists, dropped on every change). Its accessors also give the ID
//!   lists for embedding the rules in SQL;
//! - fan-out ("who is in this audience"): [`audience_member_ids`].
use std::collections::HashSet;

use tokuly_core::prelude::*;
use tokuly_proto::common::Visibility;

use crate::{
    graph,
    idcache::{self, Record},
};

/// The audience of a piece of content. See the module docs for who is in each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContentAudience {
    Public,
    Followers,
    Friends,
    CloseFriends,
    /// One of the owner's custom circles (a circle of someone else reaches nobody).
    Circle(Uuid),
    Private,
}

impl ContentAudience {
    /// The audience of content stored with the shared `Visibility` enum
    /// (`circle_id`: `AudienceRef.circle_id`, for `VISIBILITY_CIRCLE`).
    /// `NO_FEED` and `UNLISTED` are as visible as `PUBLIC` to someone who has
    /// the item (where it is listed is the content domain's business). An
    /// unset value, or a circle without its ID, reads as `Private`: the
    /// cautious reading.
    pub fn from_visibility(visibility: Visibility, circle_id: Option<Uuid>) -> Self {
        match visibility {
            Visibility::VISIBILITY_PUBLIC | Visibility::VISIBILITY_NO_FEED | Visibility::VISIBILITY_UNLISTED => {
                Self::Public
            }
            Visibility::VISIBILITY_FOLLOWERS => Self::Followers,
            Visibility::VISIBILITY_FRIENDS => Self::Friends,
            Visibility::VISIBILITY_CLOSE_FRIENDS => Self::CloseFriends,
            Visibility::VISIBILITY_CIRCLE => circle_id.map_or(Self::Private, Self::Circle),
            Visibility::VISIBILITY_PRIVATE | Visibility::VISIBILITY_UNSPECIFIED => Self::Private,
        }
    }

    /// [`Self::from_visibility`] for a stored number; a number this build
    /// does not know reads as `Private`, never as public.
    pub fn from_stored(visibility: i32, circle_id: Option<Uuid>) -> Self {
        buffa::EnumValue::<Visibility>::from(visibility)
            .as_known()
            .map_or(Self::Private, |visibility| Self::from_visibility(visibility, circle_id))
    }
}

/// SQL: whether the account `{user}` (an expression) is locked. Without a
/// stored setting an account is locked when its owner is a minor (18, by
/// the birthdate on the account, as for nearby discovery and bumps).
macro_rules! locked_sql {
    ($user:literal) => {
        concat!(
            "COALESCE((SELECT fs_.locked FROM follow_settings fs_ WHERE fs_.user_id = ",
            $user,
            "), (SELECT u_.birthdate > current_date - interval '18 years' FROM users u_ WHERE u_.id = ",
            $user,
            "), false)"
        )
    };
}
pub(crate) use locked_sql;

/// Which of `owners` have locked accounts.
pub async fn locked_among(core: &Core, owners: &[Uuid]) -> AppResult<HashSet<Uuid>> {
    if owners.is_empty() {
        return Ok(HashSet::new());
    }
    let locked: Vec<Uuid> =
        sqlx::query_scalar(concat!("SELECT o.id FROM unnest($1::uuid[]) AS o(id) WHERE ", locked_sql!("o.id")))
            .bind(owners)
            .fetch_all(&core.db)
            .await?;
    Ok(locked.into_iter().collect())
}

/// A circle a user is a member of, as cached for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Membership {
    pub circle_id: Uuid,
    pub owner_id: Uuid,
    pub close_friends: bool,
}

impl Record for Membership {
    const SIZE: usize = 33;

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.circle_id.as_bytes());
        out.extend_from_slice(self.owner_id.as_bytes());
        out.push(u8::from(self.close_friends));
    }

    fn read(bytes: &[u8]) -> Option<Self> {
        Some(Self {
            circle_id: Uuid::from_slice(&bytes[..16]).ok()?,
            owner_id: Uuid::from_slice(&bytes[16..32]).ok()?,
            close_friends: bytes[32] != 0,
        })
    }
}

/// The circles `user_id` is in. Cached; never shown to the user.
pub(crate) async fn memberships(core: &Core, user_id: Uuid) -> AppResult<Vec<Membership>> {
    idcache::MEMBER_OF
        .get_or_load(core, user_id, || async {
            let rows: Vec<(Uuid, Uuid, i16)> = sqlx::query_as(
                "SELECT c.id, c.owner_id, c.kind FROM circle_members m JOIN circles c ON c.id = m.circle_id WHERE m.member_id = $1",
            )
            .bind(user_id)
            .fetch_all(&core.db)
            .await?;
            Ok(rows.into_iter().map(|(circle_id, owner_id, kind)| Membership { circle_id, owner_id, close_friends: kind == crate::circles::CLOSE_FRIENDS }).collect())
        })
        .await
}

/// IDs of a circle's members as stored (not filtered by their ties). Cached.
pub(crate) async fn circle_member_ids(core: &Core, circle_id: Uuid) -> AppResult<Vec<Uuid>> {
    idcache::CIRCLE_MEMBERS
        .get_or_load(core, circle_id, || async {
            Ok(sqlx::query_scalar("SELECT member_id FROM circle_members WHERE circle_id = $1")
                .bind(circle_id)
                .fetch_all(&core.db)
                .await?)
        })
        .await
}

/// A viewer as the audience rules need them, loaded once for a page of
/// content: their friends, the accounts they follow, everyone hidden from
/// them by a block, and the circles they are in.
#[derive(Debug, Clone, Default)]
pub struct AudienceViewer {
    id: Option<Uuid>,
    friends: HashSet<Uuid>,
    following: HashSet<Uuid>,
    hidden: HashSet<Uuid>,
    circles: HashSet<(Uuid, Uuid)>,
    close_friend_of: HashSet<Uuid>,
}

impl AudienceViewer {
    /// Someone who is not signed in.
    pub fn anonymous() -> Self {
        Self::default()
    }

    /// The viewer for a caller (`None` = anonymous), as of now.
    pub async fn load(core: &Core, viewer: Option<Uuid>) -> AppResult<Self> {
        let Some(user_id) = viewer else { return Ok(Self::anonymous()) };
        let (friends, following, hidden, memberships) = tokio::try_join!(
            graph::friend_ids(core, user_id),
            graph::following_ids(core, user_id),
            graph::blocked_ids_either(core, user_id),
            memberships(core, user_id),
        )?;
        Ok(Self {
            id: viewer,
            friends: friends.into_iter().collect(),
            following: following.into_iter().collect(),
            hidden: hidden.into_iter().collect(),
            circles: memberships.iter().map(|m| (m.circle_id, m.owner_id)).collect(),
            close_friend_of: memberships.iter().filter(|m| m.close_friends).map(|m| m.owner_id).collect(),
        })
    }

    /// Whether the viewer may see content of `owner` shared with `audience`.
    /// `owner_locked`: whether the owner's account is locked ([`locked_among`]).
    pub fn sees(&self, owner: Uuid, audience: ContentAudience, owner_locked: bool) -> bool {
        let Some(viewer) = self.id else {
            return audience == ContentAudience::Public && !owner_locked;
        };
        if viewer == owner {
            return true;
        }
        if self.hidden.contains(&owner) {
            return false;
        }
        let friend = self.friends.contains(&owner);
        let tied = friend || self.following.contains(&owner);
        match audience {
            ContentAudience::Public => !owner_locked || tied,
            ContentAudience::Followers => tied,
            ContentAudience::Friends => friend,
            ContentAudience::CloseFriends => tied && self.close_friend_of.contains(&owner),
            ContentAudience::Circle(circle_id) => tied && self.circles.contains(&(circle_id, owner)),
            ContentAudience::Private => false,
        }
    }

    pub fn id(&self) -> Option<Uuid> {
        self.id
    }

    /// The viewer's friends.
    pub fn friend_ids(&self) -> Vec<Uuid> {
        self.friends.iter().copied().collect()
    }

    /// The accounts the viewer follows.
    pub fn following_ids(&self) -> Vec<Uuid> {
        self.following.iter().copied().collect()
    }

    /// Everyone hidden from the viewer by a block in either direction.
    pub fn hidden_ids(&self) -> Vec<Uuid> {
        self.hidden.iter().copied().collect()
    }

    /// The circles (custom and close friends) the viewer is in. For SQL that
    /// stores a circle ID with the content; combine with the tie to the owner
    /// (friend or followed) as [`Self::sees`] does.
    pub fn circle_ids(&self) -> Vec<Uuid> {
        self.circles.iter().map(|(circle, _)| *circle).collect()
    }

    /// The owners whose close friends the viewer is.
    pub fn close_friend_of(&self) -> Vec<Uuid> {
        self.close_friend_of.iter().copied().collect()
    }

    /// Whether a block hides `user_id` from the viewer.
    pub fn hides(&self, user_id: Uuid) -> bool {
        self.hidden.contains(&user_id)
    }
}

/// Whether `viewer` (`None` = signed out) may see content of `owner` shared
/// with `audience`. Reads PostgreSQL: an unfollow, a removal from a circle
/// or a block applies at once.
pub async fn may_see(core: &Core, owner: Uuid, viewer: Option<Uuid>, audience: ContentAudience) -> AppResult<bool> {
    if viewer == Some(owner) {
        return Ok(true);
    }
    if audience == ContentAudience::Private {
        return Ok(false);
    }
    let circle = match audience {
        ContentAudience::Circle(id) => Some(id),
        _ => None,
    };
    let (hidden, friend, follower, locked, member): (bool, bool, bool, bool, bool) = sqlx::query_as(concat!(
        "SELECT
             EXISTS(SELECT 1 FROM user_blocks WHERE (blocker_id = $1 AND blocked_id = $2) OR (blocker_id = $2 AND blocked_id = $1)),
             EXISTS(SELECT 1 FROM friendships WHERE user_id = $2 AND friend_id = $1),
             EXISTS(SELECT 1 FROM follows WHERE follower_id = $2 AND followee_id = $1),
             ",
        locked_sql!("$1"),
        ",
             EXISTS(SELECT 1 FROM circle_members m JOIN circles c ON c.id = m.circle_id
                    WHERE m.member_id = $2 AND c.owner_id = $1 AND (($3::uuid IS NULL AND c.kind = 1) OR c.id = $3))"
    ))
    .bind(owner)
    .bind(viewer)
    .bind(circle)
    .fetch_one(&core.db)
    .await?;
    let Some(_) = viewer else {
        return Ok(audience == ContentAudience::Public && !locked);
    };
    if hidden {
        return Ok(false);
    }
    let tied = friend || follower;
    Ok(match audience {
        ContentAudience::Public => !locked || tied,
        ContentAudience::Followers => tied,
        ContentAudience::Friends => friend,
        ContentAudience::CloseFriends | ContentAudience::Circle(_) => tied && member,
        ContentAudience::Private => false,
    })
}

/// [`may_see`] for many items at once (a page of a feed): one answer per
/// item, in order. Uses the cached lists of [`AudienceViewer`].
pub async fn may_see_many(
    core: &Core,
    viewer: Option<Uuid>,
    items: &[(Uuid, ContentAudience)],
) -> AppResult<Vec<bool>> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let mut owners: Vec<Uuid> = items.iter().map(|(owner, _)| *owner).collect();
    owners.sort_unstable();
    owners.dedup();
    let (audience_viewer, locked) = tokio::try_join!(AudienceViewer::load(core, viewer), locked_among(core, &owners))?;
    Ok(items.iter().map(|(owner, audience)| audience_viewer.sees(*owner, *audience, locked.contains(owner))).collect())
}

/// Everyone (other than the owner) in the audience of `owner`'s content, for
/// fan-out: feeds to write into, people to notify. Active accounts only,
/// nobody in a block with the owner. `Public` content fans out to the same
/// people as `Followers` (its readers beyond them find it on their own).
pub async fn audience_member_ids(core: &Core, owner: Uuid, audience: ContentAudience) -> AppResult<Vec<Uuid>> {
    let tied = || async {
        let (friends, followers) = tokio::try_join!(graph::friend_ids(core, owner), graph::follower_ids(core, owner))?;
        AppResult::Ok(friends.into_iter().chain(followers).collect::<HashSet<Uuid>>())
    };
    let mut members: HashSet<Uuid> = match audience {
        ContentAudience::Public | ContentAudience::Followers => tied().await?,
        ContentAudience::Friends => graph::friend_ids(core, owner).await?.into_iter().collect(),
        ContentAudience::CloseFriends | ContentAudience::Circle(_) => {
            let circle = match audience {
                ContentAudience::Circle(id) => {
                    sqlx::query_scalar("SELECT id FROM circles WHERE id = $1 AND owner_id = $2")
                        .bind(id)
                        .bind(owner)
                        .fetch_optional(&core.db)
                        .await?
                }
                _ => crate::circles::close_friends_id(core, owner).await?,
            };
            let Some(circle) = circle else { return Ok(Vec::new()) };
            let tied = tied().await?;
            circle_member_ids(core, circle).await?.into_iter().filter(|member| tied.contains(member)).collect()
        }
        ContentAudience::Private => return Ok(Vec::new()),
    };
    members.remove(&owner);
    for hidden in graph::blocked_ids_either(core, owner).await? {
        members.remove(&hidden);
    }
    let members: Vec<Uuid> = members.into_iter().collect();
    tokuly_identity::users::filter_active(core, &members).await
}

/// Fails with `CIRCLE_NOT_FOUND` unless `circle_id` is a circle of `owner`.
/// For content domains storing a circle as an audience: check it when the
/// content is saved.
pub async fn ensure_circle_of(core: &Core, owner: Uuid, circle_id: Uuid) -> AppResult<()> {
    let owned: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM circles WHERE id = $1 AND owner_id = $2)")
        .bind(circle_id)
        .bind(owner)
        .fetch_one(&core.db)
        .await?;
    ensure!(owned, not_found, "CIRCLE_NOT_FOUND", "circle not found");
    Ok(())
}

/// Whether `viewer` is one of `owner`'s close friends (and still a friend or
/// follower). For the close-friends mark on content shared with them.
pub async fn is_close_friend(core: &Core, owner: Uuid, viewer: Uuid) -> AppResult<bool> {
    may_see(core, owner, Some(viewer), ContentAudience::CloseFriends).await.map(|sees| sees && viewer != owner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewer() -> (AudienceViewer, Uuid) {
        let id = Uuid::now_v7();
        (AudienceViewer { id: Some(id), ..Default::default() }, id)
    }

    #[test]
    fn signed_out_viewers_see_public_content_of_unlocked_owners_only() {
        let owner = Uuid::now_v7();
        let anonymous = AudienceViewer::anonymous();
        assert!(anonymous.sees(owner, ContentAudience::Public, false));
        assert!(!anonymous.sees(owner, ContentAudience::Public, true));
        for audience in [
            ContentAudience::Followers,
            ContentAudience::Friends,
            ContentAudience::CloseFriends,
            ContentAudience::Private,
        ] {
            assert!(!anonymous.sees(owner, audience, false), "{audience:?}");
        }
    }

    #[test]
    fn followers_include_friends_and_locks_narrow_public_content() {
        let (mut v, _) = viewer();
        let (friend_owner, followed_owner, stranger) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        v.friends.insert(friend_owner);
        v.following.insert(followed_owner);
        assert!(v.sees(friend_owner, ContentAudience::Followers, false), "friends are in the followers audience");
        assert!(v.sees(followed_owner, ContentAudience::Followers, false));
        assert!(!v.sees(followed_owner, ContentAudience::Friends, false), "a follow is not a friendship");
        assert!(!v.sees(stranger, ContentAudience::Followers, false));
        assert!(v.sees(stranger, ContentAudience::Public, false));
        assert!(!v.sees(stranger, ContentAudience::Public, true), "a locked owner's public content is for followers");
        assert!(v.sees(followed_owner, ContentAudience::Public, true));
    }

    #[test]
    fn circles_need_membership_and_a_tie_and_blocks_win() {
        let (mut v, me) = viewer();
        let (owner, circle, other_circle) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        v.circles.insert((circle, owner));
        v.close_friend_of.insert(owner);
        assert!(
            !v.sees(owner, ContentAudience::Circle(circle), false),
            "a member without a tie (any more) sees nothing"
        );
        v.following.insert(owner);
        assert!(v.sees(owner, ContentAudience::Circle(circle), false));
        assert!(v.sees(owner, ContentAudience::CloseFriends, false));
        assert!(!v.sees(owner, ContentAudience::Circle(other_circle), false));
        assert!(
            !v.sees(Uuid::now_v7(), ContentAudience::Circle(circle), false),
            "a circle reaches only its owner's content"
        );
        v.hidden.insert(owner);
        assert!(!v.sees(owner, ContentAudience::Public, false));
        assert!(v.sees(me, ContentAudience::Private, true), "the owner sees everything of their own");
    }

    #[test]
    fn shared_visibility_values_map_cautiously() {
        let circle = Uuid::now_v7();
        let map = ContentAudience::from_visibility;
        assert_eq!(map(Visibility::VISIBILITY_UNLISTED, None), ContentAudience::Public);
        assert_eq!(map(Visibility::VISIBILITY_FRIENDS, None), ContentAudience::Friends);
        assert_eq!(map(Visibility::VISIBILITY_FOLLOWERS, None), ContentAudience::Followers);
        assert_eq!(map(Visibility::VISIBILITY_CLOSE_FRIENDS, None), ContentAudience::CloseFriends);
        assert_eq!(map(Visibility::VISIBILITY_CIRCLE, Some(circle)), ContentAudience::Circle(circle));
        assert_eq!(map(Visibility::VISIBILITY_CIRCLE, None), ContentAudience::Private);
        assert_eq!(map(Visibility::VISIBILITY_UNSPECIFIED, None), ContentAudience::Private);
        assert_eq!(ContentAudience::from_stored(99, None), ContentAudience::Private, "an unknown number is never public");
    }

    #[test]
    fn memberships_roundtrip_through_bytes() {
        let m = Membership { circle_id: Uuid::now_v7(), owner_id: Uuid::now_v7(), close_friends: true };
        let mut bytes = Vec::new();
        m.write(&mut bytes);
        assert_eq!(bytes.len(), Membership::SIZE);
        assert_eq!(Membership::read(&bytes), Some(m));
    }
}
