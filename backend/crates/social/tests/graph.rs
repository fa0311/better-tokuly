//! The graph API other domains call, and what the social domain adds to
//! identity's profiles and user search.
mod common;

use chrono::{Timelike as _, Utc};
use common::*;
use tokuly_proto::{
    identity::{Audience, GetProfileRequest, GetProfileResponse, Profile, Relationship, SearchUsersRequest, SearchUsersResponse, get_profile_request},
    social::*,
};
use tokuly_social::{InteractionKind, graph};
use tokuly_testkit::TestUser;
use uuid::Uuid;

async fn profile(ctx: &Ctx, viewer: Option<&TestUser>, owner: &TestUser) -> Result<Profile, tokuly_testkit::RpcError> {
    let response: GetProfileResponse = ctx
        .app
        .call(
            "tokuly.identity.v1.ProfileService/GetProfile",
            viewer,
            &GetProfileRequest { by: Some(get_profile_request::By::UserId(owner.id.to_string())), ..Default::default() },
        )
        .await?;
    Ok(response.profile.into_option().expect("profile"))
}

#[tokio::test]
async fn friend_ids_follow_the_graph_through_the_cache() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);

    assert!(graph::friend_ids(core, alice.id).await.unwrap().is_empty());
    assert!(!graph::are_friends(core, alice.id, bob.id).await.unwrap());

    ctx.befriend(&alice, &bob).await;
    // The empty list was cached a moment ago; becoming friends must replace it.
    assert_eq!(graph::friend_ids(core, alice.id).await.unwrap(), vec![bob.id]);
    assert_eq!(graph::friend_ids(core, bob.id).await.unwrap(), vec![alice.id]);
    assert!(graph::are_friends(core, alice.id, bob.id).await.unwrap() && graph::are_friends(core, bob.id, alice.id).await.unwrap());
    assert!(!graph::are_friends(core, alice.id, alice.id).await.unwrap());

    ctx.befriend(&carol, &alice).await;
    let mut friends = graph::friend_ids(core, alice.id).await.unwrap();
    friends.sort();
    let mut expected = vec![bob.id, carol.id];
    expected.sort();
    assert_eq!(friends, expected);
    // A second read is served from Valkey and agrees.
    let mut cached = graph::friend_ids(core, alice.id).await.unwrap();
    cached.sort();
    assert_eq!(cached, expected);

    let _: RemoveFriendResponse = ctx.friends("RemoveFriend", &alice, &RemoveFriendRequest { user_id: bob.id.to_string(), ..Default::default() }).await.unwrap();
    assert_eq!(graph::friend_ids(core, alice.id).await.unwrap(), vec![carol.id]);
    assert!(graph::friend_ids(core, bob.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn relationship_reflects_requests_friendship_and_blocks() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let relationship = |viewer: Uuid, other: Uuid| async move { graph::relationship(core, viewer, other).await.unwrap() };

    assert_eq!(relationship(alice.id, alice.id).await, Relationship::RELATIONSHIP_SELF);
    assert_eq!(relationship(alice.id, bob.id).await, Relationship::RELATIONSHIP_NONE);
    assert_eq!(relationship(alice.id, Uuid::now_v7()).await, Relationship::RELATIONSHIP_NONE, "unknown users are simply unrelated");

    let sent = ctx.request(&alice, &bob).await;
    assert_eq!(relationship(alice.id, bob.id).await, Relationship::RELATIONSHIP_REQUEST_SENT);
    assert_eq!(relationship(bob.id, alice.id).await, Relationship::RELATIONSHIP_REQUEST_RECEIVED);

    // A declined request stays "sent" for the sender but is gone for the recipient.
    let request_id = sent.request.as_option().unwrap().id.clone();
    let _: RespondFriendRequestResponse =
        ctx.friends("RespondFriendRequest", &bob, &RespondFriendRequestRequest { request_id, accept: false, ..Default::default() }).await.unwrap();
    assert_eq!(relationship(alice.id, bob.id).await, Relationship::RELATIONSHIP_REQUEST_SENT);
    assert_eq!(relationship(bob.id, alice.id).await, Relationship::RELATIONSHIP_NONE);

    // Bob changes his mind and asks himself: the crossed request makes them friends.
    assert_eq!(ctx.request(&bob, &alice).await.relationship, Relationship::RELATIONSHIP_FRIENDS);
    assert_eq!(relationship(alice.id, bob.id).await, Relationship::RELATIONSHIP_FRIENDS);
    assert_eq!(relationship(bob.id, alice.id).await, Relationship::RELATIONSHIP_FRIENDS);

    let _: BlockUserResponse = ctx.friends("BlockUser", &alice, &BlockUserRequest { user_id: bob.id.to_string(), ..Default::default() }).await.unwrap();
    assert_eq!(relationship(alice.id, bob.id).await, Relationship::RELATIONSHIP_BLOCKED);
    assert_eq!(relationship(bob.id, alice.id).await, Relationship::RELATIONSHIP_NONE, "the blocked side is not told");
}

#[tokio::test]
async fn blocks_hide_both_directions_and_unblocking_restores() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);

    assert!(!graph::is_blocked_either(core, alice.id, bob.id).await.unwrap());
    assert!(graph::blocked_ids_either(core, alice.id).await.unwrap().is_empty());

    let _: BlockUserResponse = ctx.friends("BlockUser", &alice, &BlockUserRequest { user_id: bob.id.to_string(), ..Default::default() }).await.unwrap();
    let _: BlockUserResponse = ctx.friends("BlockUser", &carol, &BlockUserRequest { user_id: alice.id.to_string(), ..Default::default() }).await.unwrap();
    assert!(graph::is_blocked_either(core, alice.id, bob.id).await.unwrap());
    assert!(graph::is_blocked_either(core, bob.id, alice.id).await.unwrap(), "either direction counts");
    assert!(!graph::is_blocked_either(core, bob.id, carol.id).await.unwrap());

    // Alice is hidden from both the user she blocked and the user who blocked her.
    let mut hidden = graph::blocked_ids_either(core, alice.id).await.unwrap();
    hidden.sort();
    let mut expected = vec![bob.id, carol.id];
    expected.sort();
    assert_eq!(hidden, expected);
    assert_eq!(graph::blocked_ids_either(core, bob.id).await.unwrap(), vec![alice.id]);

    let _: UnblockUserResponse = ctx.friends("UnblockUser", &alice, &UnblockUserRequest { user_id: bob.id.to_string(), ..Default::default() }).await.unwrap();
    assert!(!graph::is_blocked_either(core, alice.id, bob.id).await.unwrap());
    assert_eq!(graph::blocked_ids_either(core, alice.id).await.unwrap(), vec![carol.id]);
    assert!(graph::blocked_ids_either(core, bob.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn allows_applies_each_audience() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let owner = ctx.app.user("owner").await;
    let (friend, friend_of_friend, stranger, blocked) =
        (ctx.app.user("friend").await, ctx.app.user("fof").await, ctx.app.user("stranger").await, ctx.app.user("blocked").await);
    ctx.befriend(&owner, &friend).await;
    ctx.befriend(&friend, &friend_of_friend).await;
    ctx.befriend(&owner, &blocked).await;
    let _: BlockUserResponse = ctx.friends("BlockUser", &blocked, &BlockUserRequest { user_id: owner.id.to_string(), ..Default::default() }).await.unwrap();

    let allows = |viewer: Option<Uuid>, audience: Audience| async move { graph::allows(core, owner.id, viewer, audience).await.unwrap() };
    use Audience::*;
    // (viewer, [EVERYONE, FRIENDS_OF_FRIENDS, FRIENDS, NOBODY, UNSPECIFIED])
    let cases = [
        ("self", Some(owner.id), [true, true, true, true, true]),
        ("friend", Some(friend.id), [true, true, true, false, true]),
        ("friend of friend", Some(friend_of_friend.id), [true, true, false, false, false]),
        ("stranger", Some(stranger.id), [true, false, false, false, false]),
        ("blocked either way", Some(blocked.id), [false, false, false, false, false]),
        ("anonymous", None, [true, false, false, false, false]),
    ];
    for (who, viewer, expected) in cases {
        let audiences = [AUDIENCE_EVERYONE, AUDIENCE_FRIENDS_OF_FRIENDS, AUDIENCE_FRIENDS, AUDIENCE_NOBODY, AUDIENCE_UNSPECIFIED];
        for (audience, expected) in audiences.into_iter().zip(expected) {
            assert_eq!(allows(viewer, audience).await, expected, "{who} / {audience:?}");
        }
    }
}

#[tokio::test]
async fn mutual_friends_are_counted() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    assert_eq!(graph::mutual_friend_count(core, alice.id, bob.id).await.unwrap(), 0);
    for name in ["m1", "m2", "m3"] {
        let mutual = ctx.app.user(name).await;
        ctx.befriend(&alice, &mutual).await;
        ctx.befriend(&bob, &mutual).await;
    }
    let only_alice = ctx.app.user("solo").await;
    ctx.befriend(&alice, &only_alice).await;
    assert_eq!(graph::mutual_friend_count(core, alice.id, bob.id).await.unwrap(), 3);
    assert_eq!(graph::mutual_friend_count(core, bob.id, alice.id).await.unwrap(), 3);
    // Being friends with each other does not make them their own mutual friend.
    ctx.befriend(&alice, &bob).await;
    assert_eq!(graph::mutual_friend_count(core, alice.id, bob.id).await.unwrap(), 3);
}

#[derive(sqlx::FromRow)]
struct PairRow {
    streak_weeks: i32,
    time_together_seconds: i64,
    nights_together: i32,
    dirty: bool,
}

async fn pair_row(ctx: &Ctx, a: Uuid, b: Uuid) -> Option<PairRow> {
    sqlx::query_as("SELECT streak_weeks, time_together_seconds, nights_together, dirty FROM friend_pairs WHERE user_lo = LEAST($1, $2) AND user_hi = GREATEST($1, $2)")
        .bind(a)
        .bind(b)
        .fetch_optional(&ctx.app.core.db)
        .await
        .unwrap()
}

/// `(kind, lo_weight + hi_weight)` of today's buckets of a pair.
async fn day_buckets(ctx: &Ctx, a: Uuid, b: Uuid) -> Vec<(i16, i64)> {
    sqlx::query_as(
        "SELECT kind, lo_weight + hi_weight FROM friend_interaction_days
         WHERE user_lo = LEAST($1, $2) AND user_hi = GREATEST($1, $2) ORDER BY kind",
    )
    .bind(a)
    .bind(b)
    .fetch_all(&ctx.app.core.db)
    .await
    .unwrap()
}

#[tokio::test]
async fn interactions_are_logged_for_friends_only() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let (alice, bob, stranger) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("stranger").await);
    ctx.befriend(&alice, &bob).await;

    graph::record_interaction(core, alice.id, stranger.id, InteractionKind::Message, 1).await.unwrap();
    assert!(pair_row(&ctx, alice.id, stranger.id).await.is_none(), "strangers leave no trace");

    graph::record_interaction(core, alice.id, bob.id, InteractionKind::Message, 3).await.unwrap();
    graph::record_interaction(core, bob.id, alice.id, InteractionKind::Message, 2).await.unwrap();
    graph::record_interaction(core, bob.id, alice.id, InteractionKind::Stamp, 1).await.unwrap();
    graph::record_interaction(core, alice.id, bob.id, InteractionKind::Call, 0).await.unwrap(); // nothing happened
    graph::record_interaction(core, alice.id, alice.id, InteractionKind::Message, 1).await.unwrap(); // not an interaction
    assert_eq!(day_buckets(&ctx, alice.id, bob.id).await, vec![(1, 5), (2, 1)]);
    let pair = pair_row(&ctx, alice.id, bob.id).await.unwrap();
    assert!(pair.dirty, "the ranking worker is told to rescore the pair");
    assert_eq!(pair.streak_weeks, 1);
}

#[tokio::test]
async fn streak_grows_weekly_and_restarts_after_a_missed_week() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    let interact = || async { graph::record_interaction(core, alice.id, bob.id, InteractionKind::Message, 1).await.unwrap() };
    // Pretends the streak last counted `weeks_ago` weeks ago (weeks start on Monday, UTC).
    let this_week = Utc::now().date_naive().week(chrono::Weekday::Mon).first_day();
    let rewind = |weeks_ago: i64, streak: i32| async move {
        sqlx::query("UPDATE friend_pairs SET streak_week = $3, streak_weeks = $4 WHERE user_lo = LEAST($1, $2) AND user_hi = GREATEST($1, $2)")
            .bind(alice.id)
            .bind(bob.id)
            .bind(this_week - chrono::Duration::weeks(weeks_ago))
            .bind(streak)
            .execute(&core.db)
            .await
            .unwrap();
    };

    interact().await;
    interact().await;
    assert_eq!(pair_row(&ctx, alice.id, bob.id).await.unwrap().streak_weeks, 1, "a week counts once");

    rewind(1, 3).await;
    interact().await;
    assert_eq!(pair_row(&ctx, alice.id, bob.id).await.unwrap().streak_weeks, 4, "last week counted, so this week extends the streak");

    rewind(2, 9).await;
    let friendship: GetFriendshipResponse = ctx.friends("GetFriendship", &alice, &GetFriendshipRequest { user_id: bob.id.to_string(), ..Default::default() }).await.unwrap();
    assert_eq!(friendship.friendship.as_option().unwrap().streak_weeks, 0, "a streak with a missed week reads as broken");
    interact().await;
    assert_eq!(pair_row(&ctx, alice.id, bob.id).await.unwrap().streak_weeks, 1, "and starts over");
}

/// An IANA zone in which it is currently `hour` o'clock (give or take the minutes).
fn zone_where_it_is(hour: i32) -> String {
    let mut offset = (hour - Utc::now().hour() as i32).rem_euclid(24);
    if offset > 12 {
        offset -= 24;
    }
    // POSIX-style names: "Etc/GMT-9" is nine hours *ahead* of UTC.
    match offset {
        0 => "Etc/GMT".to_owned(),
        ahead if ahead > 0 => format!("Etc/GMT-{ahead}"),
        behind => format!("Etc/GMT+{}", -behind),
    }
}

#[tokio::test]
async fn time_together_adds_up_and_nights_count_once() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let (owl, lark, friend) = (ctx.app.user("owl").await, ctx.app.user("lark").await, ctx.app.user("friend").await);
    ctx.befriend(&owl, &friend).await;
    ctx.befriend(&lark, &friend).await;
    for (user, hour) in [(&owl, 2), (&lark, 14)] {
        sqlx::query("UPDATE users SET time_zone = $2 WHERE id = $1").bind(user.id).bind(zone_where_it_is(hour)).execute(&core.db).await.unwrap();
    }

    // 02:00 for the owl: 45 + 30 minutes together make one night, 30 more do not make a second.
    for minutes in [45, 30, 30] {
        graph::record_interaction(core, owl.id, friend.id, InteractionKind::TimeTogether, minutes).await.unwrap();
    }
    let pair = pair_row(&ctx, owl.id, friend.id).await.unwrap();
    assert_eq!(pair.time_together_seconds, 105 * 60);
    assert_eq!(pair.nights_together, 1);
    assert_eq!(day_buckets(&ctx, owl.id, friend.id).await, vec![(7, 105), (8, 1)], "the night also enters the 30-day window");

    // 14:00 for the lark: the same time together is just an afternoon.
    graph::record_interaction(core, lark.id, friend.id, InteractionKind::TimeTogether, 120).await.unwrap();
    let pair = pair_row(&ctx, lark.id, friend.id).await.unwrap();
    assert_eq!((pair.time_together_seconds, pair.nights_together), (120 * 60, 0));
}

#[tokio::test]
async fn profiles_show_relationship_mutual_friends_and_counters() {
    let ctx = start().await;
    let (owner, viewer, mutual, stranger) = (ctx.app.user("owner").await, ctx.app.user("viewer").await, ctx.app.user("mutual").await, ctx.app.user("stranger").await);
    ctx.befriend(&owner, &mutual).await;
    ctx.befriend(&viewer, &mutual).await;

    // Anonymous: no relationship, and no friend or intro counts either (a
    // count that ignores blocks, next to the viewer's own, would tell them
    // someone blocked them).
    let anonymous = profile(&ctx, None, &owner).await.unwrap();
    assert_eq!(anonymous.relationship, Relationship::RELATIONSHIP_NONE);
    let stats = anonymous.stats.as_option().unwrap();
    assert_eq!((stats.friend_count, stats.intro_count), (0, 0));
    assert_eq!(profile(&ctx, Some(&stranger), &owner).await.unwrap().stats.as_option().unwrap().friend_count, 1);
    assert!(anonymous.mutual_friends.is_empty());

    // A stranger with a mutual friend. Mutual friends are part of the
    // owner's friend list, so they follow its audience: friends only (the
    // default) hides them from a stranger...
    let hidden = profile(&ctx, Some(&viewer), &owner).await.unwrap();
    assert_eq!((hidden.mutual_friend_count, hidden.mutual_friends.len()), (0, 0));
    // ...friends of friends shows them.
    ctx.set_privacy(&owner, |p| p.friend_list_visible_to = Audience::AUDIENCE_FRIENDS_OF_FRIENDS.into()).await;
    let before = profile(&ctx, Some(&viewer), &owner).await.unwrap();
    assert_eq!(before.relationship, Relationship::RELATIONSHIP_NONE);
    assert_eq!(before.mutual_friend_count, 1);
    assert_eq!(before.mutual_friends.iter().map(id_of).collect::<Vec<_>>(), vec![mutual.id]);
    assert!(before.friends_since.is_unset());

    ctx.request(&viewer, &owner).await;
    assert_eq!(profile(&ctx, Some(&viewer), &owner).await.unwrap().relationship, Relationship::RELATIONSHIP_REQUEST_SENT);
    assert_eq!(profile(&ctx, Some(&owner), &viewer).await.unwrap().relationship, Relationship::RELATIONSHIP_REQUEST_RECEIVED);
    assert_eq!(profile(&ctx, Some(&owner), &owner).await.unwrap().relationship, Relationship::RELATIONSHIP_SELF);

    ctx.befriend(&owner, &viewer).await; // the crossed request completes the friendship
    let _: UpdateFriendPrefsResponse = ctx
        .friends("UpdateFriendPrefs", &viewer, &UpdateFriendPrefsRequest { user_id: owner.id.to_string(), favorite: Some(true), ..Default::default() })
        .await
        .unwrap();

    let friends = profile(&ctx, Some(&viewer), &owner).await.unwrap();
    assert_eq!(friends.relationship, Relationship::RELATIONSHIP_FRIENDS);
    assert!(friends.friends_since.is_set());
    assert!(friends.best_friend, "starred friends are best friends");
    assert_eq!(friends.stats.as_option().unwrap().friend_count, 2);
    assert!(!profile(&ctx, Some(&owner), &viewer).await.unwrap().best_friend, "the star is one-sided");

    // A stranger sees none of the friendship details. (Distance is the geo
    // domain's to fill, with ghost mode applied; this domain never sets it.)
    let strangers_view = profile(&ctx, Some(&stranger), &owner).await.unwrap();
    assert!(strangers_view.friends_since.is_unset() && !strangers_view.best_friend);
    assert!(friends.distance_meters.is_none() && strangers_view.distance_meters.is_none());
}

#[tokio::test]
async fn blocked_users_cannot_see_each_other_in_profiles_or_search() {
    let ctx = start().await;
    let name = format!("Quillon{}", tokuly_core::ids::random_base62(6));
    let (owner, viewer, friend) = (ctx.app.user(&name).await, ctx.app.user("viewer").await, ctx.app.user("friend").await);
    ctx.befriend(&viewer, &friend).await;
    ctx.befriend(&owner, &friend).await;
    ctx.set_privacy(&owner, |p| p.friend_list_visible_to = Audience::AUDIENCE_EVERYONE.into()).await;

    let search = |who: &TestUser| {
        let (ctx, who, query) = (&ctx, who.clone(), name.to_lowercase());
        async move {
            let found: SearchUsersResponse = ctx
                .app
                .call("tokuly.identity.v1.ProfileService/SearchUsers", Some(&who), &SearchUsersRequest { query, ..Default::default() })
                .await
                .unwrap();
            found.results
        }
    };
    let found = search(&viewer).await;
    let hit = found.iter().find(|r| id_of(r.user.as_option().unwrap()) == owner.id).expect("found by name");
    assert_eq!(hit.relationship, Relationship::RELATIONSHIP_NONE);
    assert_eq!(hit.mutual_friend_count, 1, "search results carry mutual friends");
    ctx.request(&viewer, &owner).await;
    let found = search(&viewer).await;
    assert_eq!(found.iter().find(|r| id_of(r.user.as_option().unwrap()) == owner.id).unwrap().relationship, Relationship::RELATIONSHIP_REQUEST_SENT);

    let _: BlockUserResponse = ctx.friends("BlockUser", &owner, &BlockUserRequest { user_id: viewer.id.to_string(), ..Default::default() }).await.unwrap();
    for (who, target) in [(&viewer, &owner), (&owner, &viewer)] {
        let err = profile(&ctx, Some(who), target).await.unwrap_err();
        assert_eq!((err.status, err.reason.as_str()), (404, "USER_NOT_FOUND"), "a block hides the profile in both directions");
    }
    assert!(search(&viewer).await.iter().all(|r| id_of(r.user.as_option().unwrap()) != owner.id));
    assert!(profile(&ctx, None, &owner).await.is_ok(), "the profile is still public to everyone else");
}
