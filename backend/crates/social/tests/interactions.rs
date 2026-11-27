//! `InteractionService`: bumps, buzzes, knocks, map reactions and map views.
mod common;

use std::time::Duration;

use chrono::Utc;
use common::*;
use tokuly_core::geoindex;
use tokuly_proto::{
    common::PageRequest,
    identity::{GetProfileRequest, GetProfileResponse, Relationship, get_profile_request},
    social::{social_event::Event as Push, *},
};
use tokuly_social::{SocialConfig, bumps, graph};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

async fn shake(ctx: &Ctx, user: &TestUser, at: (f64, f64)) -> Result<Vec<Bump>, RpcError> {
    let request = BumpRequest { location: lat_lng(at).into(), accuracy_meters: 5.0, intensity: 2.5, ..Default::default() };
    ctx.interact::<_, BumpResponse>("Bump", user, &request).await.map(|response| response.bumps)
}

fn participants(bump: &Bump) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = bump.participants.iter().map(id_of).collect();
    ids.sort();
    ids
}

fn sorted(mut ids: Vec<Uuid>) -> Vec<Uuid> {
    ids.sort();
    ids
}

#[tokio::test]
async fn two_friends_shaking_together_bump_once() {
    let ctx = start().await;
    let (alice, bob, mutual, bystander) = (ctx.app.user("ありす").await, ctx.app.user("ぼぶ").await, ctx.app.user("mutual").await, ctx.app.user("bystander").await);
    ctx.befriend(&alice, &bob).await;
    ctx.befriend(&alice, &mutual).await;
    ctx.befriend(&bob, &mutual).await;
    ctx.befriend(&alice, &bystander).await; // a friend of only one of them
    let spot = somewhere();
    let (mut alice_devices, mut mutual_devices, mut bystander_devices) =
        (ctx.app.listen_user(alice.id).await, ctx.app.listen_user(mutual.id).await, ctx.app.listen_user(bystander.id).await);

    // Alice shakes first: nobody to bump with yet.
    assert!(shake(&ctx, &alice, spot).await.unwrap().is_empty());
    // Bob shakes a moment later, a few meters away: that is a bump.
    let bumps = shake(&ctx, &bob, north_of(spot, 8.0)).await.unwrap();
    assert_eq!(bumps.len(), 1);
    let bump = &bumps[0];
    assert_eq!(participants(bump), sorted(vec![alice.id, bob.id]));
    assert_eq!(bump.pair_count, 1);
    assert!(!bump.friend_request_created);
    let location = bump.location.as_option().unwrap();
    assert!(geoindex::distance_meters(location.lat, location.lng, spot.0, spot.1) < 10.0, "the bump is placed between the two phones");

    // Alice's phone learns about it over realtime; her retry returns the same bump.
    match next_push(&mut alice_devices).await {
        Some(Push::Bump(pushed)) => assert_eq!(pushed.id, bump.id),
        other => panic!("expected a bump push, got {other:?}"),
    }
    let retried = shake(&ctx, &alice, spot).await.unwrap();
    assert_eq!(retried.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), vec![bump.id.as_str()]);
    assert_eq!(retried[0].pair_count, 1, "a retry does not count twice");

    // Mutual friends are told "A and B bumped!"; other friends are not.
    match next_push(&mut mutual_devices).await {
        Some(Push::FriendsBumped(pushed)) => {
            assert_eq!(pushed.id, bump.id);
            assert!(pushed.location.is_unset(), "who bumped, not where (ghost mode may hide either of them from this friend)");
        }
        other => panic!("expected a friends-bumped push, got {other:?}"),
    }
    assert!(pushes(&mut bystander_devices, QUIET).await.is_empty());
    let told = ctx.notifications_for(mutual.id, "social.friends_bumped").await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["body"]["ja"], "ありすさんとぼぶさんがBumpしたよ!");
    assert!(ctx.notifications_for(bystander.id, "social.friends_bumped").await.is_empty());

    // Both get an inbox entry and an activity entry, without the position.
    for (user, partner) in [(&alice, &bob), (&bob, &alice)] {
        let notices = ctx.notifications_for(user.id, "social.bump").await;
        assert_eq!(notices.len(), 1);
        assert_eq!((notices[0].data["inbox"].as_bool(), notices[0].data["push"].as_bool()), (Some(true), Some(false)));
        let activity = ctx.activities_of(user.id, "social.bumped").await;
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].data["with_user_ids"], serde_json::json!([partner.id]));
        assert!(activity[0].data["location"].is_null(), "{}", activity[0].data);
    }
    let created = ctx.app.events("social.bump.created").await.into_iter().filter(|e| e.data["bump_id"] == bump.id.as_str()).count();
    assert_eq!(created, 1);

    // The bump counts on the friendship and on both profiles' statistics.
    let details: GetFriendshipResponse = ctx.friends("GetFriendship", &alice, &GetFriendshipRequest { user_id: bob.id.to_string(), ..Default::default() }).await.unwrap();
    let friendship = details.friendship.as_option().unwrap();
    assert_eq!(friendship.bump_count, 1);
    assert!(friendship.last_bump_at.is_set());
    let profile: GetProfileResponse = ctx
        .app
        .call(
            "tokuly.identity.v1.ProfileService/GetProfile",
            None,
            &GetProfileRequest { by: Some(get_profile_request::By::UserId(alice.id.to_string())), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(profile.profile.as_option().unwrap().stats.as_option().unwrap().bump_count, 1);
}

#[tokio::test]
async fn shakes_too_far_apart_in_space_or_time_do_not_bump() {
    // A shorter match window, so "too late" can be tested without a long sleep.
    let ctx = start_with(SocialConfig { bump_window_ms: 2000, ..Default::default() }).await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    let spot = somewhere();

    assert!(shake(&ctx, &alice, spot).await.unwrap().is_empty());
    assert!(shake(&ctx, &bob, north_of(spot, 150.0)).await.unwrap().is_empty(), "150 m apart is not a bump");

    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(shake(&ctx, &carol, north_of(spot, 3.0)).await.unwrap().is_empty(), "Alice shook too long ago");
    // Within the window it works, also for strangers.
    assert_eq!(shake(&ctx, &alice, spot).await.unwrap().len(), 1);

    let invalid = ctx.interact::<_, BumpResponse>("Bump", &alice, &BumpRequest::default()).await.unwrap_err();
    assert_eq!(invalid.reason, "INVALID_LOCATION");
    let invalid = shake(&ctx, &alice, (0.0, 200.0)).await.unwrap_err();
    assert_eq!(invalid.reason, "INVALID_LOCATION");
    let anonymous = ctx.app.call::<_, BumpResponse>(&format!("{INTERACTIONS}/Bump"), None, &BumpRequest::default()).await.unwrap_err();
    assert_eq!(anonymous.reason, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn strangers_who_bump_get_a_friend_request_and_blocked_users_never_match() {
    let ctx = start().await;
    let (first, second, blocked) = (ctx.app.user("first").await, ctx.app.user("second").await, ctx.app.user("blocked").await);
    let _: BlockUserResponse = ctx.friends("BlockUser", &first, &BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() }).await.unwrap();
    let spot = somewhere();
    let mut first_devices = ctx.app.listen_user(first.id).await;

    assert!(shake(&ctx, &blocked, north_of(spot, 2.0)).await.unwrap().is_empty());
    assert!(shake(&ctx, &first, spot).await.unwrap().is_empty(), "people in a block never bump");
    let bumps = shake(&ctx, &second, north_of(spot, 4.0)).await.unwrap();
    // `second` and `first` are each other's best match; `blocked`, who shook
    // earlier, is not (one stranger per shake).
    assert_eq!(bumps.len(), 1);
    let with_first = bumps.iter().find(|b| participants(b) == sorted(vec![first.id, second.id])).expect("a bump with the first shaker");
    assert!(with_first.friend_request_created);
    assert_eq!(with_first.pair_count, 1);

    // Whoever shook first is the one asking.
    assert_eq!(graph::relationship(&ctx.app.core, first.id, second.id).await.unwrap(), Relationship::RELATIONSHIP_REQUEST_SENT);
    let incoming: ListFriendRequestsResponse = ctx.friends("ListFriendRequests", &second, &ListFriendRequestsRequest::default()).await.unwrap();
    let from_first = incoming.requests.iter().find(|r| id_of(r.from.as_option().unwrap()) == first.id).expect("the bump's friend request");
    assert_eq!(from_first.source, FriendSource::FRIEND_SOURCE_BUMP);
    assert!(matches!(next_push(&mut first_devices).await, Some(Push::Bump(pushed)) if pushed.id == with_first.id));
    // Not friends: no activity entries, no "friends bumped".
    assert!(ctx.activities_of(first.id, "social.bumped").await.is_empty());
    assert!(ctx.notifications_for(first.id, "social.bump").await.is_empty());
}

#[tokio::test]
async fn a_stranger_who_bumps_you_learns_nothing_of_where_you_are() {
    let ctx = start().await;
    let (alice, bob, lurker, stranger) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("lurker").await, ctx.app.user("stranger").await);
    ctx.befriend(&alice, &bob).await;

    // Two friends bump; someone shaking at a made-up point next to them a
    // moment later is nobody's best match and bumps no one.
    let spot = somewhere();
    assert!(shake(&ctx, &alice, spot).await.unwrap().is_empty());
    assert_eq!(shake(&ctx, &bob, north_of(spot, 4.0)).await.unwrap().len(), 1);
    assert!(shake(&ctx, &lurker, north_of(spot, 20.0)).await.unwrap().is_empty());

    // Strangers who do bump each get their own position back, never the
    // point between the phones (which, with one's own, gives away the other's).
    let (stranger_at, lurker_at) = { let spot = somewhere(); (spot, north_of(spot, 12.0)) };
    let mut stranger_devices = ctx.app.listen_user(stranger.id).await;
    assert!(shake(&ctx, &stranger, stranger_at).await.unwrap().is_empty());
    let bumps = shake(&ctx, &lurker, lurker_at).await.unwrap();
    assert_eq!(bumps.len(), 1);
    // (Equal up to the last bit of a float stored in jsonb.)
    let is_at = |bump: &Bump, (lat, lng): (f64, f64)| {
        let l = bump.location.as_option().unwrap();
        (l.lat - lat).abs() < 1e-9 && (l.lng - lng).abs() < 1e-9
    };
    assert!(is_at(&bumps[0], lurker_at), "the response: {:?}", bumps[0].location);
    match next_push(&mut stranger_devices).await {
        Some(Push::Bump(pushed)) => assert!(is_at(&pushed, stranger_at), "the push: {:?}", pushed.location),
        other => panic!("expected a bump, got {other:?}"),
    }
    for (user, own) in [(&lurker, lurker_at), (&stranger, stranger_at)] {
        let listed: ListBumpsResponse = ctx.interact("ListBumps", user, &ListBumpsRequest::default()).await.unwrap();
        assert!(is_at(&listed.bumps[0], own), "the list: {:?}", listed.bumps[0].location);
    }
}

#[tokio::test]
async fn strangers_bump_only_when_both_can_be_found_nearby() {
    let ctx = start().await;
    let (hermit, stranger, friend) = (ctx.app.user("hermit").await, ctx.app.user("stranger").await, ctx.app.user("friend").await);
    ctx.set_privacy(&hermit, |p| p.nearby_discoverable = false).await;
    ctx.befriend(&hermit, &friend).await;

    // A bump tells a stranger who you are: not for someone who keeps out of
    // nearby discovery, whichever of the two shakes first.
    let spot = somewhere();
    assert!(shake(&ctx, &hermit, spot).await.unwrap().is_empty());
    assert!(shake(&ctx, &stranger, north_of(spot, 3.0)).await.unwrap().is_empty());
    let spot = somewhere();
    assert!(shake(&ctx, &stranger, spot).await.unwrap().is_empty());
    assert!(shake(&ctx, &hermit, north_of(spot, 3.0)).await.unwrap().is_empty());
    assert_eq!(graph::relationship(&ctx.app.core, hermit.id, stranger.id).await.unwrap(), Relationship::RELATIONSHIP_NONE, "no friend request either");

    // Friends bump regardless.
    let spot = somewhere();
    assert!(shake(&ctx, &hermit, spot).await.unwrap().is_empty());
    assert_eq!(shake(&ctx, &friend, north_of(spot, 3.0)).await.unwrap().len(), 1);
}

#[tokio::test]
async fn minors_bump_adults_only_when_they_are_friends() {
    let ctx = start().await;
    let (kid, adult, other_kid, parent) = (ctx.app.user("kid").await, ctx.app.user("adult").await, ctx.app.user("kid2").await, ctx.app.user("parent").await);
    ctx.make_minor(&kid).await;
    ctx.make_minor(&other_kid).await;
    ctx.befriend(&kid, &parent).await;

    let spot = somewhere();
    assert!(shake(&ctx, &kid, spot).await.unwrap().is_empty());
    assert!(shake(&ctx, &adult, north_of(spot, 3.0)).await.unwrap().is_empty(), "an adult stranger never bumps a minor");
    let spot = somewhere();
    assert!(shake(&ctx, &kid, spot).await.unwrap().is_empty());
    assert_eq!(shake(&ctx, &other_kid, north_of(spot, 3.0)).await.unwrap().len(), 1, "two minors may");
    let spot = somewhere();
    assert!(shake(&ctx, &kid, spot).await.unwrap().is_empty());
    assert_eq!(shake(&ctx, &parent, north_of(spot, 3.0)).await.unwrap().len(), 1, "friends always may");
}

#[tokio::test]
async fn an_account_without_a_birthdate_bumps_friends_only() {
    let ctx = start().await;
    let (unknown, kid, adult, friend) = (ctx.app.user("unknown").await, ctx.app.user("kid").await, ctx.app.user("adult").await, ctx.app.user("friend").await);
    sqlx::query("UPDATE users SET birthdate = NULL WHERE id = $1").bind(unknown.id).execute(&ctx.app.core.db).await.unwrap();
    ctx.make_minor(&kid).await;
    ctx.befriend(&unknown, &friend).await;

    for stranger in [&kid, &adult] {
        let spot = somewhere();
        assert!(shake(&ctx, &unknown, spot).await.unwrap().is_empty());
        assert!(shake(&ctx, stranger, north_of(spot, 3.0)).await.unwrap().is_empty(), "in neither age group");
    }
    let spot = somewhere();
    assert!(shake(&ctx, &unknown, spot).await.unwrap().is_empty());
    assert_eq!(shake(&ctx, &friend, north_of(spot, 3.0)).await.unwrap().len(), 1, "friends always may");
}

#[tokio::test]
async fn a_bump_puts_a_declined_request_back_on_the_table() {
    let ctx = start().await;
    let (asker, decliner) = (ctx.app.user("asker").await, ctx.app.user("decliner").await);
    let request_id = ctx.request(&asker, &decliner).await.request.into_option().unwrap().id;
    let _: RespondFriendRequestResponse =
        ctx.friends("RespondFriendRequest", &decliner, &RespondFriendRequestRequest { request_id: request_id.clone(), accept: false, ..Default::default() }).await.unwrap();
    // Asking again by hand changes nothing: the decline stands.
    ctx.request(&asker, &decliner).await;
    let incoming: ListFriendRequestsResponse = ctx.friends("ListFriendRequests", &decliner, &ListFriendRequestsRequest::default()).await.unwrap();
    assert!(incoming.requests.is_empty());

    // Shaking phones together is a fresh sign from both sides.
    let spot = somewhere();
    shake(&ctx, &asker, spot).await.unwrap();
    let bumps = shake(&ctx, &decliner, north_of(spot, 2.0)).await.unwrap();
    assert!(bumps[0].friend_request_created);
    let incoming: ListFriendRequestsResponse = ctx.friends("ListFriendRequests", &decliner, &ListFriendRequestsRequest::default()).await.unwrap();
    assert_eq!(incoming.requests.iter().map(|r| (r.id.clone(), r.source.as_known())).collect::<Vec<_>>(), vec![(request_id, Some(FriendSource::FRIEND_SOURCE_BUMP))]);
    assert_eq!(ctx.notifications_for(decliner.id, "social.friend_request").await.len(), 2, "the original request and its revival");
}

#[tokio::test]
async fn bumps_are_listed_newest_first_and_filtered_by_partner() {
    let ctx = start_with(SocialConfig { bump_cooldown_seconds: 1, ..Default::default() }).await;
    let (me, bob, carol, outsider) = (ctx.app.user("me").await, ctx.app.user("bob").await, ctx.app.user("carol").await, ctx.app.user("outsider").await);
    ctx.befriend(&me, &bob).await;
    ctx.befriend(&me, &carol).await;
    // Three bumps in three different places: with Bob, with Carol, with Bob again.
    let mut ids = Vec::new();
    for partner in [&bob, &carol, &bob] {
        let spot = somewhere();
        shake(&ctx, &me, spot).await.unwrap();
        let bumps = shake(&ctx, partner, north_of(spot, 5.0)).await.unwrap();
        assert_eq!(bumps.len(), 1);
        ids.push(bumps[0].id.clone());
        tokio::time::sleep(Duration::from_millis(1100)).await; // let the pair's cool-down pass
    }
    let list = |user: &TestUser, request: ListBumpsRequest| {
        let (ctx, user) = (&ctx, user.clone());
        async move { ctx.interact::<_, ListBumpsResponse>("ListBumps", &user, &request).await }
    };

    let all = list(&me, ListBumpsRequest::default()).await.unwrap();
    assert_eq!(all.bumps.iter().map(|b| b.id.clone()).collect::<Vec<_>>(), vec![ids[2].clone(), ids[1].clone(), ids[0].clone()]);
    assert_eq!(all.bumps[0].pair_count, 2, "every bump shows the pair's running total");
    assert_eq!(all.bumps[1].pair_count, 1);

    let with_bob = list(&me, ListBumpsRequest { with_user_id: bob.id.to_string(), ..Default::default() }).await.unwrap();
    assert_eq!(with_bob.bumps.iter().map(|b| b.id.clone()).collect::<Vec<_>>(), vec![ids[2].clone(), ids[0].clone()]);

    let first = list(&me, ListBumpsRequest { page: PageRequest { limit: 2, ..Default::default() }.into(), ..Default::default() }).await.unwrap();
    assert_eq!(first.bumps.len(), 2);
    let cursor = first.page.as_option().unwrap().next_cursor.clone();
    let rest = list(&me, ListBumpsRequest { page: PageRequest { limit: 2, cursor, ..Default::default() }.into(), ..Default::default() }).await.unwrap();
    assert_eq!(rest.bumps.iter().map(|b| b.id.clone()).collect::<Vec<_>>(), vec![ids[0].clone()]);

    // Other people's bumps are theirs alone.
    assert_eq!(list(&carol, ListBumpsRequest::default()).await.unwrap().bumps.len(), 1);
    assert!(list(&outsider, ListBumpsRequest::default()).await.unwrap().bumps.is_empty());
    assert!(list(&outsider, ListBumpsRequest { with_user_id: me.id.to_string(), ..Default::default() }).await.unwrap().bumps.is_empty());
    assert_eq!(list(&me, ListBumpsRequest { with_user_id: "x".into(), ..Default::default() }).await.unwrap_err().reason, "INVALID_ID");

    // The geo domain can name the place afterwards.
    bumps::set_bump_place(&ctx.app.core, ids[2].parse().unwrap(), "  渋谷スクランブル交差点 ").await.unwrap();
    assert_eq!(list(&me, ListBumpsRequest::default()).await.unwrap().bumps[0].place_name, "渋谷スクランブル交差点");
}

#[tokio::test]
async fn buzz_needs_a_friend_who_is_physically_near() {
    let ctx = start().await;
    let (alice, bob, stranger) = (ctx.app.user("ありす").await, ctx.app.user("bob").await, ctx.app.user("stranger").await);
    ctx.befriend(&alice, &bob).await;
    let cache = &ctx.app.core.cache;
    let buzz = |from: &TestUser, to: &TestUser| {
        let (ctx, from, user_id) = (&ctx, from.clone(), to.id.to_string());
        async move { ctx.interact::<_, BuzzResponse>("Buzz", &from, &BuzzRequest { user_id, ..Default::default() }).await }
    };
    let spot = somewhere();

    assert_eq!(buzz(&alice, &stranger).await.unwrap_err().reason, "NOT_FRIENDS");
    assert_eq!(buzz(&alice, &bob).await.unwrap_err().reason, "NOT_NEARBY", "without known positions nobody is near");

    geoindex::set(cache, alice.id, spot.0, spot.1, Utc::now()).await.unwrap();
    let far = north_of(spot, 350.0);
    geoindex::set(cache, bob.id, far.0, far.1, Utc::now()).await.unwrap();
    assert_eq!(buzz(&alice, &bob).await.unwrap_err().reason, "NOT_NEARBY", "350 m is too far");

    let near = north_of(spot, 150.0);
    geoindex::set(cache, bob.id, near.0, near.1, Utc::now() - chrono::Duration::minutes(11)).await.unwrap();
    assert_eq!(buzz(&alice, &bob).await.unwrap_err().reason, "NOT_NEARBY", "an old position proves nothing");

    geoindex::set(cache, bob.id, near.0, near.1, Utc::now()).await.unwrap();
    let mut bob_devices = ctx.app.listen_user(bob.id).await;
    buzz(&alice, &bob).await.unwrap();
    match next_push(&mut bob_devices).await {
        Some(Push::Buzz(event)) => assert_eq!(id_of(event.from.as_option().unwrap()), alice.id),
        other => panic!("expected a buzz, got {other:?}"),
    }
    let notices = ctx.notifications_for(bob.id, "social.buzz").await;
    assert_eq!(notices.len(), 1);
    assert_eq!((notices[0].data["inbox"].as_bool(), notices[0].data["push"].as_bool()), (Some(false), Some(true)), "a buzz is push only");

    // A burst is fine; a flood is not.
    let mut outcomes = Vec::new();
    for _ in 0..12 {
        outcomes.push(buzz(&alice, &bob).await);
    }
    assert!(outcomes.iter().any(|o| o.as_ref().is_err_and(|e| e.reason == "RATE_LIMITED")));
}

#[tokio::test]
async fn knocks_reach_friends_only() {
    let ctx = start().await;
    let (alice, bob, stranger) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("stranger").await);
    ctx.befriend(&alice, &bob).await;
    let knock = |to: &TestUser, count: i32| {
        let (ctx, alice, user_id) = (&ctx, alice.clone(), to.id.to_string());
        async move { ctx.interact::<_, KnockResponse>("Knock", &alice, &KnockRequest { user_id, count, ..Default::default() }).await }
    };
    assert_eq!(knock(&stranger, 1).await.unwrap_err().reason, "NOT_FRIENDS");
    for count in [0, 21, -3] {
        assert_eq!(knock(&bob, count).await.unwrap_err().reason, "INVALID_KNOCK_COUNT");
    }

    let mut bob_devices = ctx.app.listen_user(bob.id).await;
    knock(&bob, 3).await.unwrap();
    match next_push(&mut bob_devices).await {
        Some(Push::Knock(event)) => assert_eq!((id_of(event.from.as_option().unwrap()), event.count), (alice.id, 3)),
        other => panic!("expected a knock, got {other:?}"),
    }
    let notices = ctx.notifications_for(bob.id, "social.knock").await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].data["data"]["count"], 3);
}

fn reaction_to(users: &[&TestUser], kind: MapReactionKind) -> SendMapReactionRequest {
    SendMapReactionRequest { user_ids: users.iter().map(|u| u.id.to_string()).collect(), kind: kind.into(), intensity: 2, ..Default::default() }
}

#[tokio::test]
async fn map_reactions_play_live_and_wait_until_acknowledged() {
    let ctx = start().await;
    let (alice, bob, carol, stranger) = (ctx.app.user("ありす").await, ctx.app.user("bob").await, ctx.app.user("carol").await, ctx.app.user("stranger").await);
    ctx.befriend(&alice, &bob).await;
    ctx.befriend(&alice, &carol).await;
    let send = |request: SendMapReactionRequest| {
        let (ctx, alice) = (&ctx, alice.clone());
        async move { ctx.interact::<_, SendMapReactionResponse>("SendMapReaction", &alice, &request).await }
    };
    let pending = |user: &TestUser| {
        let (ctx, user) = (&ctx, user.clone());
        async move { ctx.interact::<_, ListPendingMapReactionsResponse>("ListPendingMapReactions", &user, &ListPendingMapReactionsRequest::default()).await.unwrap().reactions }
    };

    // An emoji for a whole gathering (duplicates collapse).
    let mut bob_devices = ctx.app.listen_user(bob.id).await;
    let mut emoji = reaction_to(&[&bob, &carol, &bob], MapReactionKind::MAP_REACTION_KIND_EMOJI);
    emoji.emoji = "🍜".into();
    let sent = send(emoji).await.unwrap().reactions;
    assert_eq!(sent.len(), 2);
    assert!(sent.iter().all(|r| r.emoji == "🍜" && r.intensity == 2 && id_of(r.from.as_option().unwrap()) == alice.id));
    match next_push(&mut bob_devices).await {
        Some(Push::MapReaction(reaction)) => assert_eq!((reaction.to_user_id.as_str(), reaction.kind.as_known()), (bob.id.to_string().as_str(), Some(MapReactionKind::MAP_REACTION_KIND_EMOJI))),
        other => panic!("expected a map reaction, got {other:?}"),
    }
    let notices = ctx.notifications_for(bob.id, "social.map_reaction").await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].data["body"]["ja"], "ありすさんから🍜が届いたよ!");

    // A stamp carries only its ID; the stamp domain resolves the rest.
    let stamp_id = Uuid::now_v7();
    let mut stamp = reaction_to(&[&bob], MapReactionKind::MAP_REACTION_KIND_STAMP);
    stamp.stamp_id = stamp_id.to_string();
    let sent_stamp = send(stamp).await.unwrap().reactions;
    let stamp_ref = sent_stamp[0].stamp.as_option().unwrap();
    assert_eq!(stamp_ref.id, stamp_id.to_string());
    assert!(stamp_ref.image_url.is_empty() && stamp_ref.pack_id.is_empty());

    // Both wait for Bob, oldest first, until he acknowledges them.
    let waiting = pending(&bob).await;
    assert_eq!(waiting.len(), 2);
    assert_eq!(waiting[0].emoji, "🍜");
    assert_eq!(waiting[1].id, sent_stamp[0].id);
    // Carol cannot acknowledge Bob's reactions, and junk IDs are ignored.
    let ack = |user: &TestUser, reaction_ids: Vec<String>| {
        let (ctx, user) = (&ctx, user.clone());
        async move { ctx.interact::<_, AckMapReactionsResponse>("AckMapReactions", &user, &AckMapReactionsRequest { reaction_ids, ..Default::default() }).await }
    };
    ack(&carol, vec![waiting[0].id.clone()]).await.unwrap();
    assert_eq!(pending(&bob).await.len(), 2);
    ack(&bob, vec![waiting[0].id.clone(), Uuid::now_v7().to_string()]).await.unwrap();
    assert_eq!(pending(&bob).await.iter().map(|r| r.id.clone()).collect::<Vec<_>>(), vec![sent_stamp[0].id.clone()]);
    assert_eq!(pending(&carol).await.len(), 1);
    assert!(pending(&alice).await.is_empty());
    assert_eq!(ack(&bob, vec!["nope".into()]).await.unwrap_err().reason, "INVALID_ID");

    // Validation.
    assert_eq!(send(reaction_to(&[], MapReactionKind::MAP_REACTION_KIND_HEARTS)).await.unwrap_err().reason, "NO_RECIPIENTS");
    assert_eq!(send(reaction_to(&[&bob], MapReactionKind::MAP_REACTION_KIND_UNSPECIFIED)).await.unwrap_err().reason, "INVALID_REACTION_KIND");
    assert_eq!(send(reaction_to(&[&bob], MapReactionKind::MAP_REACTION_KIND_EMOJI)).await.unwrap_err().reason, "INVALID_EMOJI");
    assert_eq!(send(reaction_to(&[&bob], MapReactionKind::MAP_REACTION_KIND_STAMP)).await.unwrap_err().reason, "INVALID_ID");
    let mut wild = reaction_to(&[&bob], MapReactionKind::MAP_REACTION_KIND_HEARTS);
    wild.intensity = 9;
    assert_eq!(send(wild).await.unwrap_err().reason, "INVALID_INTENSITY");
    assert_eq!(send(reaction_to(&[&bob, &stranger], MapReactionKind::MAP_REACTION_KIND_EYES)).await.unwrap_err().reason, "NOT_FRIENDS");
    let crowd = SendMapReactionRequest {
        user_ids: (0..51).map(|_| Uuid::now_v7().to_string()).collect(),
        kind: MapReactionKind::MAP_REACTION_KIND_HEARTS.into(),
        ..Default::default()
    };
    assert_eq!(send(crowd).await.unwrap_err().reason, "TOO_MANY_RECIPIENTS");

    // A reaction from someone blocked since then is not replayed.
    let _: BlockUserResponse = ctx.friends("BlockUser", &carol, &BlockUserRequest { user_id: alice.id.to_string(), ..Default::default() }).await.unwrap();
    assert!(pending(&carol).await.is_empty());
}

#[tokio::test]
async fn map_views_count_everyone_but_name_only_those_who_share() {
    let ctx = start().await;
    let (owner, open, shy, stranger) = (ctx.app.user("owner").await, ctx.app.user("open").await, ctx.app.user("shy").await, ctx.app.user("stranger").await);
    ctx.befriend(&owner, &open).await;
    ctx.befriend(&owner, &shy).await;
    ctx.set_privacy(&shy, |p| p.share_views = false).await;
    let view = |viewer: &TestUser| {
        let (ctx, viewer, user_id) = (&ctx, viewer.clone(), owner.id.to_string());
        async move { ctx.interact::<_, RecordMapViewResponse>("RecordMapView", &viewer, &RecordMapViewRequest { user_id, ..Default::default() }).await }
    };
    assert_eq!(view(&stranger).await.unwrap_err().reason, "NOT_FRIENDS");

    let mut owner_devices = ctx.app.listen_user(owner.id).await;
    view(&open).await.unwrap();
    match next_push(&mut owner_devices).await {
        Some(Push::MapView(event)) => {
            assert_eq!(id_of(event.viewer.as_option().expect("a sharing viewer is named")), open.id);
            assert_eq!(event.viewer_count_24h, 1);
        }
        other => panic!("expected a map view, got {other:?}"),
    }
    view(&open).await.unwrap(); // looking again right away is the same look
    assert!(pushes(&mut owner_devices, QUIET).await.is_empty());

    view(&shy).await.unwrap();
    match next_push(&mut owner_devices).await {
        Some(Push::MapView(event)) => {
            assert!(event.viewer.is_unset(), "a viewer who does not share views stays anonymous");
            assert_eq!(event.viewer_count_24h, 2);
        }
        other => panic!("expected a map view, got {other:?}"),
    }

    let views: GetMapViewsResponse = ctx.interact("GetMapViews", &owner, &GetMapViewsRequest::default()).await.unwrap();
    assert_eq!((views.viewer_count, views.view_count), (2, 2));
    assert_eq!(views.viewers.iter().map(id_of).collect::<Vec<_>>(), vec![open.id]);
    let nobody: GetMapViewsResponse = ctx.interact("GetMapViews", &open, &GetMapViewsRequest::default()).await.unwrap();
    assert_eq!((nobody.viewer_count, nobody.view_count, nobody.viewers.len()), (0, 0, 0));

    // Views older than a day drop out.
    sqlx::query("UPDATE map_views SET viewed_at = now() - interval '25 hours' WHERE viewer_id = $1").bind(open.id).execute(&ctx.app.core.db).await.unwrap();
    let views: GetMapViewsResponse = ctx.interact("GetMapViews", &owner, &GetMapViewsRequest::default()).await.unwrap();
    assert_eq!((views.viewer_count, views.viewers.len()), (1, 0));
}

#[tokio::test]
async fn knocks_reactions_and_views_have_limits_per_sender_and_recipient() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let mut friends = Vec::new();
    for i in 0..4 {
        let friend = ctx.app.user(&format!("f{i}")).await;
        ctx.befriend(&alice, &friend).await;
        friends.push(friend);
    }

    // Knocks: 20 a minute per friend, and 60 per 10 minutes in all.
    let knock = |to: &TestUser| {
        let (ctx, alice, user_id) = (&ctx, alice.clone(), to.id.to_string());
        async move { ctx.interact::<_, KnockResponse>("Knock", &alice, &KnockRequest { user_id, count: 1, ..Default::default() }).await }
    };
    for friend in &friends[..3] {
        for _ in 0..20 {
            knock(friend).await.unwrap();
        }
    }
    assert_eq!(knock(&friends[3]).await.unwrap_err().reason, "RATE_LIMITED");

    // Map reactions: 60 a minute to the same friend.
    let react = |to: &TestUser| {
        let (ctx, alice) = (&ctx, alice.clone());
        let request = reaction_to(&[to], MapReactionKind::MAP_REACTION_KIND_HEARTS);
        async move { ctx.interact::<_, SendMapReactionResponse>("SendMapReaction", &alice, &request).await }
    };
    for _ in 0..60 {
        react(&friends[0]).await.unwrap();
    }
    assert_eq!(react(&friends[0]).await.unwrap_err().reason, "RATE_LIMITED");
    react(&friends[1]).await.unwrap();
    // A throw at a gathering that includes a friend at the limit is refused
    // as a whole and counts against nobody.
    let gathering = reaction_to(&[&friends[0], &friends[1]], MapReactionKind::MAP_REACTION_KIND_HEARTS);
    let refused = ctx.interact::<_, SendMapReactionResponse>("SendMapReaction", &alice, &gathering).await.unwrap_err();
    assert_eq!(refused.reason, "RATE_LIMITED");
    let counted: Option<i64> = redis::cmd("GET").arg(format!("rl:map-reaction:{}:{}", alice.id, friends[1].id)).query_async(&mut ctx.app.core.cache.conn()).await.unwrap();
    assert_eq!(counted, Some(1), "only the one reaction that went through");

    // Map views: 120 different pins a minute.
    let viewer = ctx.app.user("viewer").await;
    let crowd = ctx.crowd(121).await;
    sqlx::query("INSERT INTO friendships (user_id, friend_id) SELECT $1, id FROM unnest($2::uuid[]) AS c(id) UNION ALL SELECT id, $1 FROM unnest($2::uuid[]) AS c(id)")
        .bind(viewer.id)
        .bind(&crowd)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let view = |user_id: Uuid| {
        let (ctx, viewer) = (&ctx, viewer.clone());
        async move { ctx.interact::<_, RecordMapViewResponse>("RecordMapView", &viewer, &RecordMapViewRequest { user_id: user_id.to_string(), ..Default::default() }).await }
    };
    for friend in &crowd[..120] {
        view(*friend).await.unwrap();
    }
    view(crowd[0]).await.unwrap(); // the same pin again is the same look
    assert_eq!(view(crowd[120]).await.unwrap_err().reason, "RATE_LIMITED");
    // The refused look is not marked as seen, so a retry later still counts.
    let marked = ctx.app.core.cache.exists(&format!("social:map-view:{}:{}", viewer.id.simple(), crowd[120].simple())).await.unwrap();
    assert!(!marked);
}

#[tokio::test]
async fn a_shake_where_the_shaker_is_not_is_refused_before_anyone_can_match_it() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    let spot = somewhere();
    ctx.claims.refuse(alice.id);

    assert!(shake(&ctx, &bob, spot).await.unwrap().is_empty());
    assert_eq!(shake(&ctx, &alice, north_of(spot, 2.0)).await.unwrap_err().reason, "LOCATION_NOT_VERIFIED");
    assert!(shake(&ctx, &bob, spot).await.unwrap().is_empty(), "the refused shake was never published");
    let asked = ctx.claims.asked.lock().unwrap().clone();
    let (_, claim) = asked.iter().find(|(user, _)| *user == alice.id).copied().expect("Alice's claim was checked");
    let expected = north_of(spot, 2.0);
    let error_meters = geoindex::distance_meters(claim.lat, claim.lng, expected.0, expected.1);
    assert!(
        error_meters < 0.001,
        "claim should preserve the requested position within 1 mm, got {error_meters} m"
    );
    assert_eq!(claim.accuracy_meters, 5.0);
}

#[tokio::test]
async fn a_device_with_only_approximate_locations_is_told_so() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    ctx.claims.coarse(alice.id);
    assert_eq!(shake(&ctx, &alice, somewhere()).await.unwrap_err().reason, "LOCATION_TOO_COARSE");
}

#[tokio::test]
async fn without_a_position_check_no_shake_is_taken() {
    let ctx = start_unchecked().await;
    let alice = ctx.app.user("alice").await;
    assert_eq!(shake(&ctx, &alice, somewhere()).await.unwrap_err().reason, "LOCATION_NOT_VERIFIED");
}
