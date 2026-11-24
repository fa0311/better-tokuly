//! `FriendService`: friend lists, per-friend preferences, removal, blocks
//! and best friends. (Requests are in `tests/requests.rs`.)
mod common;

use common::*;
use tokuly_proto::{
    common::PageRequest,
    identity::{Audience, GetProfileRequest, GetProfileResponse, Relationship, get_profile_request},
    social::{social_event::Event as Push, *},
};
use tokuly_social::graph;
use tokuly_testkit::TestUser;
use uuid::Uuid;

fn user_ref(user: &TestUser) -> String {
    user.id.to_string()
}

async fn list_friends(ctx: &Ctx, viewer: &TestUser, request: ListFriendsRequest) -> Result<ListFriendsResponse, tokuly_testkit::RpcError> {
    ctx.friends("ListFriends", viewer, &request).await
}

fn friend_ids(response: &ListFriendsResponse) -> Vec<Uuid> {
    response.friends.iter().map(|f| id_of(f.user.as_option().unwrap())).collect()
}

#[tokio::test]
async fn own_friend_list_pages_filters_and_shows_prefs() {
    let ctx = start().await;
    let me = ctx.app.user("me").await;
    let (hana, taro, jiro) = (ctx.app.user("Hanako").await, ctx.app.user("Taro").await, ctx.app.user("Jiro").await);
    for friend in [&hana, &taro, &jiro] {
        ctx.befriend(&me, friend).await;
    }
    let _: UpdateFriendPrefsResponse = ctx
        .friends("UpdateFriendPrefs", &me, &UpdateFriendPrefsRequest { user_id: user_ref(&taro), nickname: Some("たろちゃん".into()), ..Default::default() })
        .await
        .unwrap();

    let all = list_friends(&ctx, &me, ListFriendsRequest::default()).await.unwrap();
    assert_eq!(all.total, 3);
    assert_eq!(friend_ids(&all), vec![jiro.id, taro.id, hana.id], "newest friendships first");
    assert_eq!(all.friends[1].nickname, "たろちゃん");

    let first = list_friends(&ctx, &me, ListFriendsRequest { page: PageRequest { limit: 2, ..Default::default() }.into(), ..Default::default() }).await.unwrap();
    assert_eq!(friend_ids(&first), vec![jiro.id, taro.id]);
    let cursor = first.page.as_option().unwrap().next_cursor.clone();
    let second =
        list_friends(&ctx, &me, ListFriendsRequest { page: PageRequest { limit: 2, cursor, ..Default::default() }.into(), ..Default::default() }).await.unwrap();
    assert_eq!(friend_ids(&second), vec![hana.id]);
    assert!(!second.page.as_option().unwrap().has_more);

    // The filter matches display names, handles and the caller's own nicknames.
    for (query, expected) in [("hana", vec![hana.id]), ("たろ", vec![taro.id]), (jiro.handle.as_str(), vec![jiro.id]), ("zzzzzzzzzz", vec![])] {
        let found = list_friends(&ctx, &me, ListFriendsRequest { query: query.into(), ..Default::default() }).await.unwrap();
        assert_eq!(friend_ids(&found), expected, "query {query:?}");
        assert_eq!(found.total as usize, expected.len());
    }
    let err = list_friends(&ctx, &me, ListFriendsRequest { query: "x".repeat(65), ..Default::default() }).await.unwrap_err();
    assert_eq!(err.reason, "QUERY_TOO_LONG");
    let err = list_friends(&ctx, &me, ListFriendsRequest { page: PageRequest { cursor: "!!".into(), ..Default::default() }.into(), ..Default::default() }).await.unwrap_err();
    assert_eq!(err.reason, "INVALID_CURSOR");
}

#[tokio::test]
async fn someone_elses_friend_list_follows_their_privacy() {
    let ctx = start().await;
    let (owner, friend, other_friend, stranger) =
        (ctx.app.user("owner").await, ctx.app.user("friend").await, ctx.app.user("other").await, ctx.app.user("stranger").await);
    ctx.befriend(&owner, &friend).await;
    ctx.befriend(&owner, &other_friend).await;
    let _: UpdateFriendPrefsResponse = ctx
        .friends(
            "UpdateFriendPrefs",
            &owner,
            &UpdateFriendPrefsRequest { user_id: user_ref(&other_friend), nickname: Some("secret".into()), favorite: Some(true), muted: Some(true), ..Default::default() },
        )
        .await
        .unwrap();
    let of_owner = || ListFriendsRequest { user_id: user_ref(&owner), ..Default::default() };

    // Default: friends only.
    let denied = list_friends(&ctx, &stranger, of_owner()).await.unwrap_err();
    assert_eq!((denied.status, denied.reason.as_str()), (403, "FRIEND_LIST_PRIVATE"));
    let seen = list_friends(&ctx, &friend, of_owner()).await.unwrap();
    assert_eq!(seen.total, 2);
    let other = seen.friends.iter().find(|f| id_of(f.user.as_option().unwrap()) == other_friend.id).unwrap();
    assert!(other.nickname.is_empty() && !other.favorite && !other.muted, "the owner's private notes about friends are not shown");
    // Nicknames are not searchable by others either.
    let by_nickname = list_friends(&ctx, &friend, ListFriendsRequest { user_id: user_ref(&owner), query: "secret".into(), ..Default::default() }).await.unwrap();
    assert!(by_nickname.friends.is_empty());

    ctx.set_privacy(&owner, |p| p.friend_list_visible_to = Audience::AUDIENCE_EVERYONE.into()).await;
    assert_eq!(list_friends(&ctx, &stranger, of_owner()).await.unwrap().total, 2);

    // People in a block with the viewer are left out of what the viewer sees...
    let _: BlockUserResponse = ctx.friends("BlockUser", &other_friend, &BlockUserRequest { user_id: user_ref(&stranger), ..Default::default() }).await.unwrap();
    assert_eq!(friend_ids(&list_friends(&ctx, &stranger, of_owner()).await.unwrap()), vec![friend.id]);
    // ...and a block with the owner hides the whole list.
    let _: BlockUserResponse = ctx.friends("BlockUser", &owner, &BlockUserRequest { user_id: user_ref(&stranger), ..Default::default() }).await.unwrap();
    let hidden = list_friends(&ctx, &stranger, of_owner()).await.unwrap_err();
    assert_eq!((hidden.status, hidden.reason.as_str()), (404, "USER_NOT_FOUND"));

    ctx.set_privacy(&owner, |p| p.friend_list_visible_to = Audience::AUDIENCE_NOBODY.into()).await;
    assert_eq!(list_friends(&ctx, &friend, of_owner()).await.unwrap_err().reason, "FRIEND_LIST_PRIVATE");
    assert_eq!(list_friends(&ctx, &owner, ListFriendsRequest::default()).await.unwrap().total, 2, "the owner always sees their own list");
    let missing = list_friends(&ctx, &friend, ListFriendsRequest { user_id: Uuid::now_v7().to_string(), ..Default::default() }).await.unwrap_err();
    assert_eq!(missing.reason, "USER_NOT_FOUND");
}

#[tokio::test]
async fn removing_a_friend_ends_it_for_both() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    ctx.befriend(&alice, &bob).await;
    let remove = |user: &TestUser, target: &TestUser| {
        let (ctx, user, user_id) = (&ctx, user.clone(), user_ref(target));
        async move { ctx.friends::<_, RemoveFriendResponse>("RemoveFriend", &user, &RemoveFriendRequest { user_id, ..Default::default() }).await }
    };
    let not_friends = remove(&alice, &carol).await.unwrap_err();
    assert_eq!(not_friends.reason, "NOT_FRIENDS");

    // Bob had muted Alice; ending the friendship must lift that mute downstream.
    let _: UpdateFriendPrefsResponse =
        ctx.friends("UpdateFriendPrefs", &bob, &UpdateFriendPrefsRequest { user_id: user_ref(&alice), muted: Some(true), ..Default::default() }).await.unwrap();
    let (mut alice_devices, mut bob_devices) = (ctx.app.listen_user(alice.id).await, ctx.app.listen_user(bob.id).await);
    remove(&alice, &bob).await.unwrap();
    let mutes: Vec<bool> = ctx.app.events_by("social.friend.prefs_changed", bob.id).await.iter().map(|e| e.data["muted"] == true).collect();
    assert_eq!(mutes, vec![true, false]);
    assert!(!graph::are_friends(&ctx.app.core, bob.id, alice.id).await.unwrap());
    assert_eq!(list_friends(&ctx, &bob, ListFriendsRequest::default()).await.unwrap().total, 0);
    let removed = ctx.app.events_by("social.friend.removed", alice.id).await;
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].data["blocked"], false);
    // Both sides' devices drop the other from the map.
    assert!(matches!(next_push(&mut bob_devices).await, Some(Push::FriendRemovedUserId(id)) if id == alice.id.to_string()));
    assert!(matches!(next_push(&mut alice_devices).await, Some(Push::FriendRemovedUserId(id)) if id == bob.id.to_string()));
    assert_eq!(remove(&bob, &alice).await.unwrap_err().reason, "NOT_FRIENDS");
}

#[tokio::test]
async fn friend_prefs_are_private_and_mute_changes_are_announced() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    ctx.befriend(&alice, &bob).await;
    let update = |request: UpdateFriendPrefsRequest| {
        let (ctx, alice) = (&ctx, alice.clone());
        async move { ctx.friends::<_, UpdateFriendPrefsResponse>("UpdateFriendPrefs", &alice, &request).await }
    };
    let prefs_of_bob = || UpdateFriendPrefsRequest { user_id: user_ref(&bob), ..Default::default() };

    let updated = update(UpdateFriendPrefsRequest { nickname: Some(" ボブ ".into()), favorite: Some(true), muted: Some(true), ..prefs_of_bob() }).await.unwrap();
    let friend = updated.friend.as_option().unwrap();
    assert_eq!((friend.nickname.as_str(), friend.favorite, friend.muted), ("ボブ", true, true));
    let announced = ctx.app.events_by("social.friend.prefs_changed", alice.id).await;
    assert_eq!(announced.len(), 1);
    assert_eq!(announced[0].data, serde_json::json!({ "user_id": alice.id, "friend_id": bob.id, "muted": true }));

    // Untouched fields stay; a change that does not flip the mute is not announced.
    let updated = update(UpdateFriendPrefsRequest { favorite: Some(false), muted: Some(true), ..prefs_of_bob() }).await.unwrap();
    let friend = updated.friend.as_option().unwrap();
    assert_eq!((friend.nickname.as_str(), friend.favorite, friend.muted), ("ボブ", false, true));
    assert_eq!(ctx.app.events_by("social.friend.prefs_changed", alice.id).await.len(), 1);
    update(UpdateFriendPrefsRequest { muted: Some(false), nickname: Some(String::new()), ..prefs_of_bob() }).await.unwrap();
    let announced = ctx.app.events_by("social.friend.prefs_changed", alice.id).await;
    assert_eq!(announced.len(), 2);
    assert_eq!(announced[1].data["muted"], false);

    // Bob's view of Alice is untouched by Alice's preferences about him.
    let bobs = list_friends(&ctx, &bob, ListFriendsRequest::default()).await.unwrap();
    assert!(bobs.friends[0].nickname.is_empty() && !bobs.friends[0].muted);

    assert_eq!(update(UpdateFriendPrefsRequest { nickname: Some("あ".repeat(41)), ..prefs_of_bob() }).await.unwrap_err().reason, "NICKNAME_TOO_LONG");
    assert_eq!(update(UpdateFriendPrefsRequest { user_id: user_ref(&carol), muted: Some(true), ..Default::default() }).await.unwrap_err().reason, "NOT_FRIENDS");
}

#[tokio::test]
async fn friendship_details_need_a_friendship() {
    let ctx = start().await;
    let (alice, bob, mutual, stranger) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("mutual").await, ctx.app.user("stranger").await);
    ctx.befriend(&alice, &bob).await;
    ctx.befriend(&alice, &mutual).await;
    ctx.befriend(&bob, &mutual).await;

    let details: GetFriendshipResponse = ctx.friends("GetFriendship", &alice, &GetFriendshipRequest { user_id: user_ref(&bob), ..Default::default() }).await.unwrap();
    let friendship = details.friendship.as_option().unwrap();
    assert_eq!(id_of(friendship.friend.as_option().unwrap().user.as_option().unwrap()), bob.id);
    assert_eq!(friendship.source, FriendSource::FRIEND_SOURCE_PROFILE);
    assert_eq!(friendship.mutual_friend_count, 1);
    assert_eq!((friendship.bump_count, friendship.streak_weeks, friendship.interaction_score), (0, 0, 0));
    assert_eq!(friendship.titles, vec!["NEW_FRIENDS"]);

    let err = ctx.friends::<_, GetFriendshipResponse>("GetFriendship", &alice, &GetFriendshipRequest { user_id: user_ref(&stranger), ..Default::default() }).await.unwrap_err();
    assert_eq!(err.reason, "NOT_FRIENDS");
}

#[tokio::test]
async fn blocking_is_silent_total_and_reversible() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    ctx.befriend(&alice, &bob).await;
    ctx.request(&carol, &alice).await;
    let block = |user: &TestUser, user_id: String| {
        let (ctx, user) = (&ctx, user.clone());
        async move { ctx.friends::<_, BlockUserResponse>("BlockUser", &user, &BlockUserRequest { user_id, ..Default::default() }).await }
    };
    assert_eq!(block(&alice, user_ref(&alice)).await.unwrap_err().reason, "CANNOT_BLOCK_SELF");
    assert_eq!(block(&alice, Uuid::now_v7().to_string()).await.unwrap_err().reason, "USER_NOT_FOUND");

    // Blocking a friend ends the friendship; to Bob it looks like an unfriend.
    let mut bob_devices = ctx.app.listen_user(bob.id).await;
    block(&alice, user_ref(&bob)).await.unwrap();
    block(&alice, user_ref(&bob)).await.unwrap(); // harmless to repeat
    assert!(!graph::are_friends(&ctx.app.core, alice.id, bob.id).await.unwrap());
    assert!(matches!(next_push(&mut bob_devices).await, Some(Push::FriendRemovedUserId(id)) if id == alice.id.to_string()));
    assert_eq!(ctx.app.events_by("social.user.blocked", alice.id).await.len(), 1);
    let removed = ctx.app.events_by("social.friend.removed", alice.id).await;
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].data["blocked"], true);

    // Blocking a stranger drops their pending request and tells them nothing.
    let mut carol_devices = ctx.app.listen_user(carol.id).await;
    block(&alice, user_ref(&carol)).await.unwrap();
    assert!(pushes(&mut carol_devices, QUIET).await.is_empty());
    assert!(list_requests(&ctx, &alice, FriendRequestDirection::FRIEND_REQUEST_DIRECTION_INCOMING).await.requests.is_empty());
    assert!(list_requests(&ctx, &carol, FriendRequestDirection::FRIEND_REQUEST_DIRECTION_OUTGOING).await.requests.is_empty());

    // The blocked see the blocker as gone; the blocker must unblock before asking.
    let from_bob = ctx.friends::<_, SendFriendRequestResponse>("SendFriendRequest", &bob, &request_to(alice.id)).await.unwrap_err();
    assert_eq!((from_bob.status, from_bob.reason.as_str()), (404, "USER_NOT_FOUND"));
    let profile = ctx
        .app
        .call::<_, GetProfileResponse>(
            "tokuly.identity.v1.ProfileService/GetProfile",
            Some(&bob),
            &GetProfileRequest { by: Some(get_profile_request::By::UserId(user_ref(&alice))), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(profile.reason, "USER_NOT_FOUND");
    let from_alice = ctx.friends::<_, SendFriendRequestResponse>("SendFriendRequest", &alice, &request_to(bob.id)).await.unwrap_err();
    assert_eq!(from_alice.reason, "USER_BLOCKED");

    let blocked: ListBlockedUsersResponse = ctx.friends("ListBlockedUsers", &alice, &ListBlockedUsersRequest::default()).await.unwrap();
    assert_eq!(blocked.users.iter().map(id_of).collect::<Vec<_>>(), vec![carol.id, bob.id], "most recent first");
    let first: ListBlockedUsersResponse =
        ctx.friends("ListBlockedUsers", &alice, &ListBlockedUsersRequest { page: PageRequest { limit: 1, ..Default::default() }.into(), ..Default::default() }).await.unwrap();
    assert_eq!(first.users.len(), 1);
    assert!(first.page.as_option().unwrap().has_more);
    let blocked_by_bob: ListBlockedUsersResponse = ctx.friends("ListBlockedUsers", &bob, &ListBlockedUsersRequest::default()).await.unwrap();
    assert!(blocked_by_bob.users.is_empty(), "being blocked is not visible");

    // Unblocking lifts the wall but does not restore the friendship.
    let _: UnblockUserResponse = ctx.friends("UnblockUser", &alice, &UnblockUserRequest { user_id: user_ref(&bob), ..Default::default() }).await.unwrap();
    let _: UnblockUserResponse = ctx.friends("UnblockUser", &alice, &UnblockUserRequest { user_id: user_ref(&bob), ..Default::default() }).await.unwrap();
    assert_eq!(ctx.app.events_by("social.user.unblocked", alice.id).await.len(), 1);
    assert!(!graph::are_friends(&ctx.app.core, alice.id, bob.id).await.unwrap());
    assert_eq!(ctx.request(&bob, &alice).await.relationship, Relationship::RELATIONSHIP_REQUEST_SENT);
}

#[tokio::test]
async fn best_friends_list_starred_friends_and_is_shown_to_friends_only() {
    let ctx = start().await;
    let (alice, bob, carol, stranger) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await, ctx.app.user("stranger").await);
    ctx.befriend(&alice, &bob).await;
    ctx.befriend(&alice, &carol).await;
    let best = |viewer: &TestUser, owner: Option<&TestUser>| {
        let (ctx, viewer, user_id) = (&ctx, viewer.clone(), owner.map(user_ref).unwrap_or_default());
        async move { ctx.friends::<_, ListBestFriendsResponse>("ListBestFriends", &viewer, &ListBestFriendsRequest { user_id, ..Default::default() }).await }
    };
    assert!(best(&alice, None).await.unwrap().friends.is_empty(), "no interactions and no stars yet");

    let _: UpdateFriendPrefsResponse = ctx
        .friends("UpdateFriendPrefs", &alice, &UpdateFriendPrefsRequest { user_id: user_ref(&carol), favorite: Some(true), nickname: Some("かろ".into()), ..Default::default() })
        .await
        .unwrap();
    let own = best(&alice, None).await.unwrap();
    assert_eq!(own.friends.len(), 1);
    assert_eq!((id_of(own.friends[0].user.as_option().unwrap()), own.friends[0].favorite, own.friends[0].nickname.as_str()), (carol.id, true, "かろ"));

    // A friend sees the list without Alice's private nickname; strangers do not see it at all.
    let seen_by_bob = best(&bob, Some(&alice)).await.unwrap();
    assert_eq!(seen_by_bob.friends.len(), 1);
    assert!(seen_by_bob.friends[0].favorite && seen_by_bob.friends[0].nickname.is_empty());
    assert_eq!(best(&stranger, Some(&alice)).await.unwrap_err().reason, "NOT_FRIENDS");

    // Best friends are part of the friend list: hiding that hides them too.
    ctx.set_privacy(&alice, |p| p.friend_list_visible_to = Audience::AUDIENCE_NOBODY.into()).await;
    assert_eq!(best(&bob, Some(&alice)).await.unwrap_err().reason, "FRIEND_LIST_PRIVATE");
    assert_eq!(best(&alice, None).await.unwrap().friends.len(), 1, "the owner still sees their own");
}

#[tokio::test]
async fn stars_are_limited() {
    let ctx = start().await;
    let (fan, friend) = (ctx.app.user("fan").await, ctx.app.user("friend").await);
    ctx.befriend(&fan, &friend).await;
    let crowd = ctx.crowd(50).await;
    sqlx::query("INSERT INTO friendships (user_id, friend_id, favorite) SELECT $1, id, true FROM unnest($2::uuid[]) AS c(id)")
        .bind(fan.id)
        .bind(&crowd)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let star = |favorite: bool| {
        let (ctx, fan, user_id) = (&ctx, fan.clone(), user_ref(&friend));
        async move { ctx.friends::<_, UpdateFriendPrefsResponse>("UpdateFriendPrefs", &fan, &UpdateFriendPrefsRequest { user_id, favorite: Some(favorite), ..Default::default() }).await }
    };
    assert_eq!(star(true).await.unwrap_err().reason, "TOO_MANY_FAVORITES");
    star(false).await.unwrap(); // un-starring is always possible
}
