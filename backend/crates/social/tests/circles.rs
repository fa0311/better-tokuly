//! Circles (close friends and custom lists), and the audience API built on
//! follows, friends and circles that other domains ask.
mod common;

use common::*;
use tokuly_proto::social::*;
use tokuly_social::graph::{self, AudienceViewer, ContentAudience};
use tokuly_testkit::TestUser;
use uuid::Uuid;

async fn create(ctx: &Ctx, owner: &TestUser, name: &str, emoji: &str) -> Result<Circle, tokuly_testkit::RpcError> {
    let request = CreateCircleRequest {
        name: name.into(),
        emoji: emoji.into(),
        color: CircleColor::CIRCLE_COLOR_LIME.into(),
        ..Default::default()
    };
    let response: CreateCircleResponse = ctx.circles("CreateCircle", owner, &request).await?;
    Ok(response.circle.into_option().unwrap())
}

async fn delete(ctx: &Ctx, user: &TestUser, id: &str) -> Result<DeleteCircleResponse, tokuly_testkit::RpcError> {
    ctx.circles("DeleteCircle", user, &DeleteCircleRequest { circle_id: id.into(), ..Default::default() }).await
}

async fn circles_of(ctx: &Ctx, owner: &TestUser) -> Vec<Circle> {
    let list: ListCirclesResponse = ctx.circles("ListCircles", owner, &ListCirclesRequest::default()).await.unwrap();
    list.circles
}

async fn member_ids(ctx: &Ctx, owner: &TestUser, circle_id: &str) -> Vec<Uuid> {
    let list: ListCircleMembersResponse = ctx
        .circles(
            "ListCircleMembers",
            owner,
            &ListCircleMembersRequest { circle_id: circle_id.into(), ..Default::default() },
        )
        .await
        .unwrap();
    list.members.iter().map(|m| id_of(m.user.as_option().unwrap())).collect()
}

#[tokio::test]
async fn circles_are_made_named_ordered_and_deleted_by_their_owner() {
    let ctx = start().await;
    let (owner, other) = (ctx.app.user("owner").await, ctx.app.user("other").await);
    let all = circles_of(&ctx, &owner).await;
    assert_eq!(all.len(), 1, "everyone starts with close friends");
    let close = &all[0];
    assert_eq!(close.kind.as_known(), Some(CircleKind::CIRCLE_KIND_CLOSE_FRIENDS));
    assert_eq!(circles_of(&ctx, &owner).await[0].id, close.id, "made once");

    let club = create(&ctx, &owner, "  部活  ", "🏀").await.unwrap();
    assert_eq!(
        (club.name.as_str(), club.emoji.as_str(), club.kind.as_known()),
        ("部活", "🏀", Some(CircleKind::CIRCLE_KIND_CUSTOM))
    );
    let class = create(&ctx, &owner, "クラス", "").await.unwrap();
    assert_eq!(create(&ctx, &owner, " ", "").await.unwrap_err().reason, "NAME_REQUIRED");
    assert_eq!(create(&ctx, &owner, &"あ".repeat(31), "").await.unwrap_err().reason, "NAME_TOO_LONG");
    assert_eq!(create(&ctx, &owner, "x", "ab").await.unwrap_err().reason, "INVALID_EMOJI");
    let order: Vec<String> = circles_of(&ctx, &owner).await.into_iter().map(|c| c.id).collect();
    assert_eq!(order, vec![close.id.clone(), club.id.clone(), class.id.clone()]);

    let reordered: ReorderCirclesResponse = ctx
        .circles(
            "ReorderCircles",
            &owner,
            &ReorderCirclesRequest { circle_ids: vec![class.id.clone(), club.id.clone()], ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(
        reordered.circles.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
        vec![close.id.clone(), class.id.clone(), club.id.clone()]
    );
    let incomplete: Result<ReorderCirclesResponse, _> = ctx
        .circles(
            "ReorderCircles",
            &owner,
            &ReorderCirclesRequest { circle_ids: vec![class.id.clone()], ..Default::default() },
        )
        .await;
    assert_eq!(incomplete.unwrap_err().reason, "INVALID_ORDER");

    let renamed: UpdateCircleResponse = ctx
        .circles(
            "UpdateCircle",
            &owner,
            &UpdateCircleRequest {
                circle_id: club.id.clone(),
                name: Some("バスケ部".into()),
                emoji: Some(String::new()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let renamed = renamed.circle.into_option().unwrap();
    assert_eq!((renamed.name.as_str(), renamed.emoji.as_str()), ("バスケ部", ""));
    let fixed: Result<UpdateCircleResponse, _> = ctx
        .circles(
            "UpdateCircle",
            &owner,
            &UpdateCircleRequest { circle_id: close.id.clone(), name: Some("x".into()), ..Default::default() },
        )
        .await;
    assert_eq!(fixed.unwrap_err().reason, "CLOSE_FRIENDS_FIXED");
    let recolored: Result<UpdateCircleResponse, _> = ctx
        .circles(
            "UpdateCircle",
            &owner,
            &UpdateCircleRequest {
                circle_id: close.id.clone(),
                color: Some(CircleColor::CIRCLE_COLOR_MINT.into()),
                ..Default::default()
            },
        )
        .await;
    assert!(recolored.is_ok(), "close friends takes a colour");

    // Someone else's circle does not exist for them.
    let theirs: Result<GetCircleResponse, _> =
        ctx.circles("GetCircle", &other, &GetCircleRequest { circle_id: club.id.clone(), ..Default::default() }).await;
    assert_eq!(theirs.unwrap_err().reason, "CIRCLE_NOT_FOUND");
    assert_eq!(delete(&ctx, &other, &club.id).await.unwrap_err().reason, "CIRCLE_NOT_FOUND");
    assert_eq!(delete(&ctx, &owner, &close.id).await.unwrap_err().reason, "CLOSE_FRIENDS_FIXED");
    delete(&ctx, &owner, &club.id).await.unwrap();
    assert_eq!(circles_of(&ctx, &owner).await.len(), 2);
}

#[tokio::test]
async fn there_is_a_limit_on_circles_even_when_made_at_once() {
    let ctx = start().await;
    let owner = ctx.app.user("owner").await;
    let results = futures::future::join_all((0..24).map(|i| {
        let owner = &owner;
        let ctx = &ctx;
        async move { create(ctx, owner, &format!("list {i}"), "").await }
    }))
    .await;
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 20);
    assert!(results.iter().filter_map(|r| r.as_ref().err()).all(|e| e.reason == "TOO_MANY_CIRCLES"));
}

#[tokio::test]
async fn members_are_friends_or_followers_and_are_never_told() {
    let ctx = start().await;
    let (owner, friend, follower, followed, stranger) = (
        ctx.app.user("owner").await,
        ctx.app.user("friend").await,
        ctx.app.user("follower").await,
        ctx.app.user("followed").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&owner, &friend).await;
    ctx.follow(&follower, &owner).await;
    ctx.follow(&owner, &followed).await;
    let close = ctx.close_friends(&owner).await;

    assert_eq!(ctx.add_to_circle(&owner, &close.id, &[&stranger]).await.unwrap_err().reason, "NOT_CONNECTED");
    assert_eq!(
        ctx.add_to_circle(&owner, &close.id, &[&followed]).await.unwrap_err().reason,
        "NOT_CONNECTED",
        "following someone is not a tie the other way"
    );
    assert_eq!(ctx.add_to_circle(&owner, &close.id, &[&owner]).await.unwrap_err().reason, "NOT_CONNECTED");
    assert_eq!(ctx.add_to_circle(&friend, &close.id, &[&owner]).await.unwrap_err().reason, "CIRCLE_NOT_FOUND");

    let mut friend_devices = ctx.app.listen_user(friend.id).await;
    let added = ctx.add_to_circle(&owner, &close.id, &[&friend, &follower]).await.unwrap();
    assert_eq!(added.circle.as_option().unwrap().member_count, 2);
    ctx.add_to_circle(&owner, &close.id, &[&friend]).await.unwrap(); // again: harmless
    assert!(pushes(&mut friend_devices, QUIET).await.is_empty(), "members are not told");
    assert!(ctx.notifications_for(friend.id, "social.circle").await.is_empty());
    let mut members = member_ids(&ctx, &owner, &close.id).await;
    members.sort();
    let mut expected = vec![friend.id, follower.id];
    expected.sort();
    assert_eq!(members, expected);

    let candidates: ListCircleCandidatesResponse = ctx
        .circles(
            "ListCircleCandidates",
            &owner,
            &ListCircleCandidatesRequest { circle_id: close.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let ids: Vec<Uuid> = candidates.candidates.iter().map(|c| id_of(c.user.as_option().unwrap())).collect();
    assert!(
        ids.contains(&friend.id)
            && ids.contains(&follower.id)
            && !ids.contains(&stranger.id)
            && !ids.contains(&followed.id)
    );
    assert!(candidates.candidates.iter().all(|c| c.member));

    let removed: RemoveCircleMembersResponse = ctx
        .circles(
            "RemoveCircleMembers",
            &owner,
            &RemoveCircleMembersRequest {
                circle_id: close.id.clone(),
                user_ids: vec![follower.id.to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(removed.circle.as_option().unwrap().member_count, 1);
    let too_many = AddCircleMembersRequest {
        circle_id: close.id.clone(),
        user_ids: (0..101).map(|_| Uuid::now_v7().to_string()).collect(),
        ..Default::default()
    };
    let too_many: Result<AddCircleMembersResponse, _> = ctx.circles("AddCircleMembers", &owner, &too_many).await;
    assert_eq!(too_many.unwrap_err().reason, "TOO_MANY_IDS");
}

#[tokio::test]
async fn many_member_changes_at_once_from_one_account_go_through() {
    let ctx = start().await;
    let owner = ctx.app.user("owner").await;
    let mut fans = Vec::new();
    for i in 0..12 {
        let fan = ctx.app.user(&format!("fan{i}")).await;
        ctx.follow(&fan, &owner).await;
        fans.push(fan);
    }
    let close = ctx.close_friends(&owner).await;
    // More concurrent calls than the test pool has connections, each holding locks.
    let results = futures::future::join_all(fans.iter().map(|fan| async { ctx.add_to_circle(&owner, &close.id, &[fan]).await })).await;
    assert!(results.iter().all(|r| r.is_ok()), "{results:?}");
    assert_eq!(member_ids(&ctx, &owner, &close.id).await.len(), 12);
    // Unfollows racing with adds leave no member without a tie.
    let late = ctx.app.user("late").await;
    ctx.follow(&late, &owner).await;
    let unfollow = UnfollowRequest { user_id: owner.id.to_string(), ..Default::default() };
    let (added, _) = tokio::join!(async { ctx.add_to_circle(&owner, &close.id, &[&late]).await }, async {
        ctx.follows::<_, UnfollowResponse>("Unfollow", &late, &unfollow).await.unwrap()
    });
    let members = member_ids(&ctx, &owner, &close.id).await;
    assert!(!members.contains(&late.id), "added: {:?}", added.map(|_| ()));
}

#[tokio::test]
async fn members_leave_circles_when_their_tie_ends() {
    let ctx = start().await;
    let (owner, friend, both, follower, blocker) = (
        ctx.app.user("owner").await,
        ctx.app.user("friend").await,
        ctx.app.user("both").await,
        ctx.app.user("follower").await,
        ctx.app.user("blocker").await,
    );
    ctx.befriend(&owner, &friend).await;
    ctx.befriend(&owner, &both).await;
    ctx.follow(&both, &owner).await;
    ctx.follow(&follower, &owner).await;
    ctx.befriend(&owner, &blocker).await;
    let close = ctx.close_friends(&owner).await;
    let custom = create(&ctx, &owner, "仲良し", "").await.unwrap();
    for circle in [&close.id, &custom.id] {
        ctx.add_to_circle(&owner, circle, &[&friend, &both, &follower, &blocker]).await.unwrap();
    }
    // The blocker also lists the owner in their own close friends.
    let theirs = ctx.close_friends(&blocker).await;
    ctx.add_to_circle(&blocker, &theirs.id, &[&owner]).await.unwrap();

    let _: RemoveFriendResponse = ctx
        .friends("RemoveFriend", &owner, &RemoveFriendRequest { user_id: friend.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    let _: RemoveFriendResponse = ctx
        .friends("RemoveFriend", &both, &RemoveFriendRequest { user_id: owner.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    let _: UnfollowResponse = ctx
        .follows("Unfollow", &follower, &UnfollowRequest { user_id: owner.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    ctx.block(&blocker, &owner).await;
    for circle in [&close.id, &custom.id] {
        assert_eq!(member_ids(&ctx, &owner, circle).await, vec![both.id], "only the one who still follows stays");
    }
    assert!(member_ids(&ctx, &blocker, &theirs.id).await.is_empty(), "a block empties both sides' circles");
}

#[tokio::test]
async fn the_audience_api_covers_followers_friends_and_circles() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let (owner, friend, follower, close, stranger, blocked) = (
        ctx.app.user("owner").await,
        ctx.app.user("friend").await,
        ctx.app.user("follower").await,
        ctx.app.user("close").await,
        ctx.app.user("stranger").await,
        ctx.app.user("blocked").await,
    );
    ctx.befriend(&owner, &friend).await;
    ctx.follow(&follower, &owner).await;
    ctx.follow(&close, &owner).await;
    ctx.follow(&blocked, &owner).await;
    let close_friends = ctx.close_friends(&owner).await;
    ctx.add_to_circle(&owner, &close_friends.id, &[&close]).await.unwrap();
    let club = create(&ctx, &owner, "部活", "").await.unwrap();
    ctx.add_to_circle(&owner, &club.id, &[&friend]).await.unwrap();
    let club_id: Uuid = club.id.parse().unwrap();
    ctx.block(&owner, &blocked).await;

    use ContentAudience::*;
    let audiences = [Public, Followers, Friends, CloseFriends, Circle(club_id), Circle(Uuid::now_v7()), Private];
    // (viewer, what they see of each audience above)
    let expect: [(Option<&TestUser>, [bool; 7]); 7] = [
        (Some(&owner), [true, true, true, true, true, true, true]),
        (Some(&friend), [true, true, true, false, true, false, false]),
        (Some(&follower), [true, true, false, false, false, false, false]),
        (Some(&close), [true, true, false, true, false, false, false]),
        (Some(&stranger), [true, false, false, false, false, false, false]),
        (Some(&blocked), [false; 7]),
        (None, [true, false, false, false, false, false, false]),
    ];
    let check = |locked: bool| {
        let expect = &expect;
        async move {
            for (viewer, sees) in expect {
                let viewer_id = viewer.map(|v| v.id);
                let loaded = AudienceViewer::load(core, viewer_id).await.unwrap();
                let items: Vec<(Uuid, ContentAudience)> = audiences.iter().map(|a| (owner.id, *a)).collect();
                let many = graph::may_see_many(core, viewer_id, &items).await.unwrap();
                for (i, audience) in audiences.iter().enumerate() {
                    let mut wanted = sees[i];
                    if locked && *audience == Public && matches!(viewer, None)
                        || locked && *audience == Public && viewer.is_some_and(|v| v.id == stranger.id)
                    {
                        wanted = false; // a locked owner's public content is for followers and friends
                    }
                    let single = graph::may_see(core, owner.id, viewer_id, *audience).await.unwrap();
                    assert_eq!(
                        single,
                        wanted,
                        "may_see {audience:?} for {:?} (locked {locked})",
                        viewer.map(|v| &v.handle)
                    );
                    assert_eq!(many[i], wanted, "may_see_many {audience:?} for {:?}", viewer.map(|v| &v.handle));
                    assert_eq!(loaded.sees(owner.id, *audience, locked), wanted, "AudienceViewer {audience:?}");
                }
            }
        }
    };
    check(false).await;
    ctx.set_locked(&owner, true).await;
    check(true).await;

    let mut followers = graph::audience_member_ids(core, owner.id, Followers).await.unwrap();
    followers.sort();
    let mut expected = vec![friend.id, follower.id, close.id];
    expected.sort();
    assert_eq!(followers, expected, "followers and friends, without the blocked one");
    assert_eq!(graph::audience_member_ids(core, owner.id, CloseFriends).await.unwrap(), vec![close.id]);
    assert_eq!(graph::audience_member_ids(core, owner.id, Circle(club_id)).await.unwrap(), vec![friend.id]);
    assert!(
        graph::audience_member_ids(core, stranger.id, Circle(club_id)).await.unwrap().is_empty(),
        "a circle reaches only its owner's content"
    );
    assert!(graph::audience_member_ids(core, owner.id, Private).await.unwrap().is_empty());
    assert!(
        graph::is_close_friend(core, owner.id, close.id).await.unwrap()
            && !graph::is_close_friend(core, owner.id, friend.id).await.unwrap()
    );
    assert!(graph::ensure_circle_of(core, owner.id, club_id).await.is_ok());
    assert_eq!(graph::ensure_circle_of(core, stranger.id, club_id).await.unwrap_err().reason, "CIRCLE_NOT_FOUND");

    // Leaving the circle applies at once, cached or not.
    let _: UnfollowResponse = ctx
        .follows("Unfollow", &close, &UnfollowRequest { user_id: owner.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    assert!(!graph::may_see(core, owner.id, Some(close.id), CloseFriends).await.unwrap());
    assert!(!AudienceViewer::load(core, Some(close.id)).await.unwrap().sees(owner.id, CloseFriends, true));
    assert!(graph::audience_member_ids(core, owner.id, CloseFriends).await.unwrap().is_empty());
}

#[tokio::test]
async fn friendship_details_are_one_cached_call_away() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let (alice, bob, carol, dave) = (
        ctx.app.user("alice").await,
        ctx.app.user("bob").await,
        ctx.app.user("carol").await,
        ctx.app.user("dave").await,
    );
    ctx.befriend(&alice, &bob).await;
    ctx.befriend(&alice, &carol).await;
    assert!(graph::friends_since(core, alice.id, dave.id).await.unwrap().is_none());
    let since = graph::friends_since(core, alice.id, bob.id).await.unwrap().expect("friends");
    assert_eq!(graph::friends_since(core, bob.id, alice.id).await.unwrap(), Some(since));
    let many = graph::friends_since_many(core, alice.id, &[bob.id, carol.id, dave.id]).await.unwrap();
    assert_eq!(many.len(), 2);
    assert!(graph::best_friend_ids(core, alice.id).await.unwrap().is_empty());

    // Starring and muting reach the cached details right away.
    let prefs = UpdateFriendPrefsRequest {
        user_id: carol.id.to_string(),
        favorite: Some(true),
        muted: Some(true),
        ..Default::default()
    };
    let _: UpdateFriendPrefsResponse = ctx.friends("UpdateFriendPrefs", &alice, &prefs).await.unwrap();
    assert_eq!(graph::best_friend_ids(core, alice.id).await.unwrap(), vec![carol.id]);
    assert_eq!(graph::muted_friend_ids(core, alice.id).await.unwrap(), vec![carol.id]);
    assert_eq!(graph::muted_by(core, carol.id, &[alice.id, bob.id]).await.unwrap(), vec![alice.id]);
    let link = graph::friend_link(core, alice.id, carol.id).await.unwrap().unwrap();
    assert!(link.favorite && link.muted && link.is_best_friend());

    let _: RemoveFriendResponse = ctx
        .friends("RemoveFriend", &alice, &RemoveFriendRequest { user_id: carol.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    assert!(graph::best_friend_ids(core, alice.id).await.unwrap().is_empty());
    assert!(graph::friends_since(core, alice.id, carol.id).await.unwrap().is_none());
}
