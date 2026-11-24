//! Work that happens outside RPCs: the best-friend ranking, presence
//! fan-out, account deletion, pruning and the data export. The worker entry
//! points are called directly; `tests/workers.rs` covers the wiring.
mod common;

use chrono::Utc;
use common::*;
use tokuly_core::{
    events::Event,
    presence::{self, PresenceChange},
};
use tokuly_proto::social::{social_event::Event as Push, *};
use tokuly_social::{InteractionKind, graph};
use tokuly_testkit::TestUser;
use uuid::Uuid;

/// Ranking passes work on every dirty pair in the (shared) test database, so
/// tests that run one take turns: a pass started by one test must not be
/// half-way through another test's pairs when that test looks at its ranks.
static RANKING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn best_friends(ctx: &Ctx, viewer: &TestUser, owner: &TestUser) -> Vec<Friend> {
    let response: ListBestFriendsResponse = ctx
        .friends(
            "ListBestFriends",
            viewer,
            &ListBestFriendsRequest { user_id: owner.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    response.friends
}

fn ranks(friends: &[Friend]) -> Vec<(Uuid, i32)> {
    friends.iter().map(|f| (id_of(f.user.as_option().unwrap()), f.best_friend_rank)).collect()
}

async fn friendship(ctx: &Ctx, user: &TestUser, friend: &TestUser) -> Friendship {
    let response: GetFriendshipResponse = ctx
        .friends("GetFriendship", user, &GetFriendshipRequest { user_id: friend.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    response.friendship.into_option().unwrap()
}

async fn interact(ctx: &Ctx, from: &TestUser, to: &TestUser, kind: InteractionKind, weight: i32) {
    graph::record_interaction(&ctx.app.core, from.id, to.id, kind, weight).await.unwrap();
}

#[tokio::test]
async fn interactions_rank_best_friends_and_earn_titles() {
    let _turn = RANKING.lock().await;
    let ctx = start().await;
    let me = ctx.app.user("me").await;
    let (chatty, caller, quiet) =
        (ctx.app.user("chatty").await, ctx.app.user("caller").await, ctx.app.user("quiet").await);
    for friend in [&chatty, &caller, &quiet] {
        ctx.befriend(&me, friend).await;
    }
    assert!(best_friends(&ctx, &me, &me).await.is_empty());

    // chatty: a long two-way conversation, a bump and an evening together.
    interact(&ctx, &me, &chatty, InteractionKind::Message, 120).await;
    interact(&ctx, &chatty, &me, InteractionKind::Message, 90).await;
    interact(&ctx, &chatty, &me, InteractionKind::Stamp, 10).await;
    interact(&ctx, &me, &chatty, InteractionKind::Bump, 1).await;
    interact(&ctx, &chatty, &me, InteractionKind::TimeTogether, 200).await;
    // caller: half an hour on the phone.
    interact(&ctx, &caller, &me, InteractionKind::Call, 30).await;
    // quiet: I looked at their pin a couple of times; they never answered.
    interact(&ctx, &me, &quiet, InteractionKind::MapView, 2).await;

    assert!(ctx.social.rescore_pending().await.unwrap() >= 3);
    assert_eq!(
        ranks(&best_friends(&ctx, &me, &me).await),
        vec![(chatty.id, 1), (caller.id, 2)],
        "one-sided glances do not make a best friend"
    );
    assert_eq!(ranks(&best_friends(&ctx, &chatty, &chatty).await), vec![(me.id, 1)]);
    // A friend may look at the list and finds themselves in it.
    assert_eq!(ranks(&best_friends(&ctx, &caller, &me).await), vec![(chatty.id, 1), (caller.id, 2)]);

    let with_chatty = friendship(&ctx, &me, &chatty).await;
    assert_eq!(with_chatty.interaction_score, 220, "messages and stamps of the last 30 days");
    assert_eq!(with_chatty.time_together_seconds, 200 * 60);
    assert_eq!(with_chatty.streak_weeks, 1);
    assert_eq!(with_chatty.friend.as_option().unwrap().best_friend_rank, 1);
    assert_eq!(with_chatty.titles, vec!["BFF", "MUTUAL_LOVE", "CHATTERBOX", "NEW_FRIENDS"]);
    assert_eq!(friendship(&ctx, &me, &caller).await.titles, vec!["NEW_FRIENDS"]);

    // When a best friend leaves, the others move up.
    let _: RemoveFriendResponse = ctx
        .friends("RemoveFriend", &me, &RemoveFriendRequest { user_id: chatty.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(ranks(&best_friends(&ctx, &me, &me).await), vec![(caller.id, 1)]);

    // Scores fade: six weeks later, with nothing new, nobody is a best friend.
    sqlx::query("UPDATE friend_interaction_days SET day = day - 42 WHERE user_lo = $1 OR user_hi = $1")
        .bind(me.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    sqlx::query("UPDATE friend_pairs SET scored_at = now() - interval '7 hours' WHERE user_lo = $1 OR user_hi = $1")
        .bind(me.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.social.rescore_pending().await.unwrap();
    assert!(best_friends(&ctx, &me, &me).await.is_empty());
    assert_eq!(friendship(&ctx, &me, &caller).await.interaction_score, 0);
}

#[tokio::test]
async fn only_the_closest_friends_are_ranked_and_stars_follow() {
    let _turn = RANKING.lock().await;
    let ctx = start().await;
    let me = ctx.app.user("me").await;
    // Ten friends, each a little closer than the one before.
    let mut friends = Vec::new();
    for i in 0..10 {
        let friend = ctx.app.user(&format!("friend{i}")).await;
        ctx.befriend(&me, &friend).await;
        interact(&ctx, &me, &friend, InteractionKind::Message, 10 + i).await;
        interact(&ctx, &friend, &me, InteractionKind::Message, 10 + i).await;
        friends.push(friend);
    }
    // The least close one is starred by hand.
    let _: UpdateFriendPrefsResponse = ctx
        .friends(
            "UpdateFriendPrefs",
            &me,
            &UpdateFriendPrefsRequest {
                user_id: friends[0].id.to_string(),
                favorite: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    ctx.social.rescore_pending().await.unwrap();

    let listed = best_friends(&ctx, &me, &me).await;
    let mut expected: Vec<(Uuid, i32)> =
        friends[2..].iter().rev().zip(1..).map(|(friend, rank)| (friend.id, rank)).collect();
    expected.push((friends[0].id, 0));
    assert_eq!(ranks(&listed), expected, "the eight closest in order, then starred friends without a rank");
    assert!(listed.last().unwrap().favorite);
}

#[tokio::test]
async fn presence_is_fanned_out_to_friends_unless_hidden() {
    let ctx = start().await;
    let (alice, bob, stranger) =
        (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("stranger").await);
    ctx.befriend(&alice, &bob).await;
    let (mut bob_devices, mut stranger_devices) =
        (ctx.app.listen_user(bob.id).await, ctx.app.listen_user(stranger.id).await);
    let came_online = PresenceChange { user_id: alice.id, online: true, at: Utc::now() };

    ctx.social.fan_out_presence(&came_online).await.unwrap();
    let seen = next_presence(&mut bob_devices).await.expect("friends are told");
    assert!(seen.user_id == alice.id.to_string() && seen.online && seen.last_seen_at.is_set());
    assert!(presence_events(&mut stranger_devices, QUIET).await.is_empty());

    // Hiding the online status: friends see Alice go offline, without a "last seen"...
    presence::mark_online(&ctx.app.core.cache, alice.id).await.unwrap();
    ctx.set_privacy(&alice, |p| p.share_online_status = false).await;
    let privacy_changed = |user: &TestUser| Event {
        id: Uuid::now_v7(),
        topic: "identity.user.privacy_changed".into(),
        actor_id: Some(user.id),
        occurred_at: Utc::now(),
        data: serde_json::json!({ "user_id": user.id }),
    };
    ctx.social.handle_event(&privacy_changed(&alice)).await.unwrap();
    let seen = next_presence(&mut bob_devices).await.expect("friends see her go offline");
    assert!(!seen.online && seen.last_seen_at.is_unset());
    // ...and later transitions are not announced at all.
    ctx.social.fan_out_presence(&came_online).await.unwrap();
    assert!(presence_events(&mut bob_devices, QUIET).await.is_empty());

    // Sharing again while online reveals her at once.
    ctx.set_privacy(&alice, |p| p.share_online_status = true).await;
    ctx.social.handle_event(&privacy_changed(&alice)).await.unwrap();
    assert!(next_presence(&mut bob_devices).await.expect("friends see her again").online);
}

#[tokio::test]
async fn friend_lists_carry_online_state_when_shared() {
    let ctx = start().await;
    let (me, open, hidden) = (ctx.app.user("me").await, ctx.app.user("open").await, ctx.app.user("hidden").await);
    ctx.befriend(&me, &open).await;
    ctx.befriend(&me, &hidden).await;
    ctx.set_privacy(&hidden, |p| p.share_online_status = false).await;
    let cache = &ctx.app.core.cache;
    presence::mark_online(cache, open.id).await.unwrap();
    presence::mark_online(cache, hidden.id).await.unwrap();
    let friend = |id: Uuid| {
        let (ctx, me) = (&ctx, me.clone());
        async move {
            let list: ListFriendsResponse =
                ctx.friends("ListFriends", &me, &ListFriendsRequest::default()).await.unwrap();
            list.friends.into_iter().find(|f| id_of(f.user.as_option().unwrap()) == id).unwrap()
        }
    };

    assert!(friend(open.id).await.online);
    let secretive = friend(hidden.id).await;
    assert!(!secretive.online && secretive.last_seen_at.is_unset(), "online state is only shown when shared");

    presence::mark_offline(cache, open.id).await.unwrap();
    let gone = friend(open.id).await;
    assert!(!gone.online && gone.last_seen_at.is_set());
}

#[tokio::test]
async fn mutes_made_in_notification_settings_show_on_the_friend() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    let mute_changed = |muted: bool| Event {
        id: Uuid::now_v7(),
        topic: "notify.mute.changed".into(),
        actor_id: Some(alice.id),
        occurred_at: Utc::now(),
        data: serde_json::json!({ "user_id": alice.id, "muted_user_id": bob.id, "muted": muted }),
    };
    let bob_is_muted = || async {
        let list: ListFriendsResponse =
            ctx.friends("ListFriends", &alice, &ListFriendsRequest::default()).await.unwrap();
        list.friends[0].muted
    };

    ctx.social.handle_event(&mute_changed(true)).await.unwrap();
    assert!(bob_is_muted().await);
    ctx.social.handle_event(&mute_changed(false)).await.unwrap();
    assert!(!bob_is_muted().await);
    // The change came from notify; echoing it back would loop.
    assert!(ctx.app.events_by("social.friend.prefs_changed", alice.id).await.is_empty());
}

/// How many rows across all social tables still mention a user.
async fn rows_about(ctx: &Ctx, user: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM friendships WHERE user_id = $1 OR friend_id = $1)
              + (SELECT count(*) FROM friend_requests WHERE from_user_id = $1 OR to_user_id = $1)
              + (SELECT count(*) FROM user_blocks WHERE blocker_id = $1 OR blocked_id = $1)
              + (SELECT count(*) FROM friend_suggestions WHERE user_id = $1 OR suggested_id = $1)
              + (SELECT count(*) FROM friend_suggestion_dismissals WHERE user_id = $1 OR dismissed_id = $1)
              + (SELECT count(*) FROM friend_invites WHERE user_id = $1)
              + (SELECT count(*) FROM friend_intros WHERE invitee_id = $1 OR inviter_id = $1)
              + (SELECT count(*) FROM friend_pairs WHERE user_lo = $1 OR user_hi = $1)
              + (SELECT count(*) FROM friend_interaction_days WHERE user_lo = $1 OR user_hi = $1)
              + (SELECT count(*) FROM bumps WHERE user_lo = $1 OR user_hi = $1)
              + (SELECT count(*) FROM map_reactions WHERE from_user_id = $1 OR to_user_id = $1)
              + (SELECT count(*) FROM map_views WHERE viewed_id = $1 OR viewer_id = $1)
              + (SELECT count(*) FROM follows WHERE follower_id = $1 OR followee_id = $1)
              + (SELECT count(*) FROM follow_requests WHERE follower_id = $1 OR followee_id = $1)
              + (SELECT count(*) FROM follow_settings WHERE user_id = $1)
              + (SELECT count(*) FROM follow_suggestion_dismissals WHERE user_id = $1 OR dismissed_id = $1)
              + (SELECT count(*) FROM circles WHERE owner_id = $1)
              + (SELECT count(*) FROM circle_members WHERE owner_id = $1 OR member_id = $1)",
    )
    .bind(user)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap()
}

/// Gives `user` a bit of everything the social domain stores, with `friend`
/// as their friend. Returns the user's invite code.
async fn live_a_little(ctx: &Ctx, user: &TestUser, friend: &TestUser) -> String {
    let (admirer, blocked, suggested, newcomer) = (
        ctx.app.user("admirer").await,
        ctx.app.user("blocked").await,
        ctx.app.user("suggested").await,
        ctx.app.user("newcomer").await,
    );
    ctx.befriend(user, friend).await;
    // Follows and circles: the admirer follows the user, the user asks to
    // follow a locked account, keeps the friend among close friends and is
    // in the friend's.
    ctx.follow(&admirer, user).await;
    ctx.follow(user, friend).await;
    ctx.set_locked(&suggested, true).await;
    ctx.follow(user, &suggested).await;
    ctx.set_locked(user, false).await;
    let close = ctx.close_friends(user).await;
    ctx.add_to_circle(user, &close.id, &[friend]).await.unwrap();
    let theirs = ctx.close_friends(friend).await;
    ctx.add_to_circle(friend, &theirs.id, &[user]).await.unwrap();
    let _: DismissFollowSuggestionResponse = ctx
        .follows(
            "DismissFollowSuggestion",
            user,
            &DismissFollowSuggestionRequest { user_id: newcomer.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    ctx.request(&admirer, user).await;
    ctx.request(user, &suggested).await;
    let _: BlockUserResponse = ctx
        .friends("BlockUser", user, &BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    let _: DismissSuggestionResponse = ctx
        .friends(
            "DismissSuggestion",
            user,
            &DismissSuggestionRequest { user_id: admirer.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let invite: GetMyInviteResponse = ctx.friends("GetMyInvite", user, &GetMyInviteRequest::default()).await.unwrap();
    let code = invite.invite.into_option().unwrap().code;
    let signed_up = Event {
        id: Uuid::now_v7(),
        topic: "identity.user.created".into(),
        actor_id: Some(newcomer.id),
        occurred_at: Utc::now(),
        data: serde_json::json!({ "user_id": newcomer.id, "handle": newcomer.handle, "invite_code": code }),
    };
    ctx.social.handle_event(&signed_up).await.unwrap();

    let spot = somewhere();
    for shaker in [user, friend] {
        let _: BumpResponse = ctx
            .interact("Bump", shaker, &BumpRequest { location: lat_lng(spot).into(), ..Default::default() })
            .await
            .unwrap();
    }
    let reaction = SendMapReactionRequest {
        user_ids: vec![friend.id.to_string()],
        kind: MapReactionKind::MAP_REACTION_KIND_HEARTS.into(),
        ..Default::default()
    };
    let _: SendMapReactionResponse = ctx.interact("SendMapReaction", user, &reaction).await.unwrap();
    let _: RecordMapViewResponse = ctx
        .interact("RecordMapView", friend, &RecordMapViewRequest { user_id: user.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    code
}

#[tokio::test]
async fn a_deleted_account_leaves_nothing_behind() {
    let ctx = start().await;
    let (leaver, friend) = (ctx.app.user("leaver").await, ctx.app.user("friend").await);
    live_a_little(&ctx, &leaver, &friend).await;
    assert!(rows_about(&ctx, leaver.id).await >= 12, "the account has data in every table");
    assert_eq!(graph::friend_ids(&ctx.app.core, friend.id).await.unwrap(), vec![leaver.id]);

    let deleted = Event {
        id: Uuid::now_v7(),
        topic: "identity.user.deleted".into(),
        actor_id: Some(leaver.id),
        occurred_at: Utc::now(),
        data: serde_json::json!({ "user_id": leaver.id }),
    };
    let mut friend_devices = ctx.app.listen_user(friend.id).await;
    ctx.social.handle_event(&deleted).await.unwrap();
    assert_eq!(rows_about(&ctx, leaver.id).await, 0);
    assert!(
        graph::friend_ids(&ctx.app.core, friend.id).await.unwrap().is_empty(),
        "cached friend lists forget the account too"
    );
    assert!(graph::blocked_ids_either(&ctx.app.core, leaver.id).await.unwrap().is_empty());
    assert!(
        graph::follower_ids(&ctx.app.core, friend.id).await.unwrap().is_empty(),
        "cached follower lists forget the account too"
    );
    assert!(
        matches!(next_push(&mut friend_devices).await, Some(Push::FriendRemovedUserId(id)) if id == leaver.id.to_string())
    );

    ctx.social.handle_event(&deleted).await.unwrap(); // redelivery is harmless
    // Events this domain does not care about are ignored.
    let unrelated = Event { topic: "identity.user.updated".into(), ..deleted };
    ctx.social.handle_event(&unrelated).await.unwrap();
}

#[tokio::test]
async fn the_data_export_has_a_social_section() {
    let ctx = start().await;
    let (user, friend) = (ctx.app.user("user").await, ctx.app.user("friend").await);
    let code = live_a_little(&ctx, &user, &friend).await;

    let exporter = ctx
        .app
        .core
        .exporters
        .all()
        .into_iter()
        .find(|e| e.name() == "social")
        .expect("the social exporter is registered");
    let export = exporter.export(user.id).await.unwrap();
    assert_eq!(export["friends"].as_array().unwrap().len(), 1);
    assert_eq!(export["friends"][0]["user_id"], serde_json::json!(friend.id));
    assert_eq!(export["invite_code"], serde_json::json!(code));
    for (section, count) in [
        ("friend_requests_sent", 1),
        ("friend_requests_received", 2), // the admirer and the newcomer
        ("blocked_users", 1),
        ("dismissed_suggestions", 1),
        ("intros", 1),
        ("shared_history", 1),
        ("bumps", 1),
        ("map_reactions_sent", 1),
        ("map_reactions_received", 0),
        ("map_views_made", 0),
        ("following", 1),
        ("followers", 1),
        ("follow_requests_sent", 1),
        ("follow_requests_received", 0),
        ("dismissed_follow_suggestions", 1),
        ("circles", 1),
    ] {
        assert_eq!(
            export[section].as_array().unwrap_or_else(|| panic!("{section} is a list")).len(),
            count,
            "{section}"
        );
    }
    // The friend's export shows their side, not the user's private data.
    let friends_export = exporter.export(friend.id).await.unwrap();
    assert_eq!(friends_export["map_views_made"].as_array().unwrap().len(), 1);
    assert_eq!(friends_export["map_reactions_received"].as_array().unwrap().len(), 1);
    assert!(friends_export["blocked_users"].as_array().unwrap().is_empty());
    assert_eq!(export["circles"][0]["members"].as_array().unwrap().len(), 1);
    assert_eq!(export["follow_settings"]["locked"], serde_json::json!(false));
    // Whose circles someone is in is not part of their export.
    assert_eq!(friends_export["circles"][0]["members"][0]["user_id"], serde_json::json!(user.id));
    assert_eq!(friends_export["circles"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn pruning_removes_only_what_has_aged_out() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    let db = &ctx.app.core.db;
    // One fresh and one aged-out row of each prunable kind.
    for kind in [InteractionKind::Message, InteractionKind::Stamp] {
        graph::record_interaction(&ctx.app.core, alice.id, bob.id, kind, 1).await.unwrap();
    }
    sqlx::query("UPDATE friend_interaction_days SET day = day - 60 WHERE kind = 2 AND (user_lo = $1 OR user_hi = $1)")
        .bind(alice.id)
        .execute(db)
        .await
        .unwrap();
    for age in ["1 hour", "3 days"] {
        sqlx::query("INSERT INTO map_views (viewed_id, viewer_id, viewed_at) VALUES ($1, $2, now() - $3::interval)")
            .bind(alice.id)
            .bind(bob.id)
            .bind(age)
            .execute(db)
            .await
            .unwrap();
    }
    for age in ["1 day", "8 days"] {
        sqlx::query("INSERT INTO map_reactions (from_user_id, to_user_id, kind, created_at) VALUES ($1, $2, 1, now() - $3::interval)")
            .bind(alice.id)
            .bind(bob.id)
            .bind(age)
            .execute(db)
            .await
            .unwrap();
    }

    // Two declined requests to Alice: one from last week, one from last spring.
    for (name, age) in [("recent", "7 days"), ("ancient", "120 days")] {
        let asker = ctx.app.user(name).await;
        ctx.request(&asker, &alice).await;
        sqlx::query("UPDATE friend_requests SET declined_at = now() - $2::interval WHERE from_user_id = $1")
            .bind(asker.id)
            .bind(age)
            .execute(db)
            .await
            .unwrap();
    }

    ctx.social.prune().await.unwrap();
    let remaining: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM friend_interaction_days WHERE user_lo = $1 OR user_hi = $1),
                (SELECT count(*) FROM map_views WHERE viewed_id = $1),
                (SELECT count(*) FROM map_reactions WHERE from_user_id = $1),
                (SELECT count(*) FROM friend_requests WHERE to_user_id = $1)",
    )
    .bind(alice.id)
    .fetch_one(db)
    .await
    .unwrap();
    assert_eq!(remaining, (1, 1, 1, 1));
}

/// An event as the outbox relay delivers it.
fn delivered<T: serde::Serialize>(topic: &str, actor: Uuid, payload: &T) -> tokuly_core::events::Event {
    tokuly_core::events::Event {
        id: Uuid::now_v7(),
        topic: topic.into(),
        actor_id: Some(actor),
        occurred_at: Utc::now(),
        data: serde_json::to_value(payload).unwrap(),
    }
}

#[tokio::test]
async fn graph_events_drop_cached_lists_that_a_cut_off_request_left_behind() {
    use tokuly_social::events::*;
    let ctx = start().await;
    let core = &ctx.app.core;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    assert_eq!(graph::friend_ids(core, alice.id).await.unwrap(), vec![bob.id]); // cached now
    assert!(graph::blocked_ids_either(core, bob.id).await.unwrap().is_empty()); // this too

    // A block commits, but its request is dropped (the phone lost its
    // connection) before it got to the cache: the lists are stale...
    let mut tx = core.db.begin().await.unwrap();
    sqlx::query("DELETE FROM friendships WHERE user_id IN ($1, $2)")
        .bind(alice.id)
        .bind(bob.id)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2)")
        .bind(alice.id)
        .bind(bob.id)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(graph::friend_ids(core, alice.id).await.unwrap(), vec![bob.id], "stale until the events arrive");
    assert!(graph::blocked_ids_either(core, bob.id).await.unwrap().is_empty(), "stale until the events arrive");

    // ...until the events written by the same transaction arrive.
    ctx.social
        .handle_event(&delivered(
            FRIEND_REMOVED,
            alice.id,
            &FriendRemoved { user_id: alice.id, friend_id: bob.id, blocked: true },
        ))
        .await
        .unwrap();
    ctx.social
        .handle_event(&delivered(USER_BLOCKED, alice.id, &BlockChanged { user_id: alice.id, blocked_id: bob.id }))
        .await
        .unwrap();
    assert!(graph::friend_ids(core, alice.id).await.unwrap().is_empty());
    assert!(graph::friend_ids(core, bob.id).await.unwrap().is_empty());
    assert_eq!(graph::blocked_ids_either(core, bob.id).await.unwrap(), vec![alice.id]);

    // The same for an unblock and a new friendship.
    sqlx::query("DELETE FROM user_blocks WHERE blocker_id = $1").bind(alice.id).execute(&core.db).await.unwrap();
    sqlx::query("INSERT INTO friendships (user_id, friend_id) VALUES ($1, $2), ($2, $1)")
        .bind(alice.id)
        .bind(bob.id)
        .execute(&core.db)
        .await
        .unwrap();
    ctx.social
        .handle_event(&delivered(USER_UNBLOCKED, alice.id, &BlockChanged { user_id: alice.id, blocked_id: bob.id }))
        .await
        .unwrap();
    ctx.social
        .handle_event(&delivered(
            FRIEND_ACCEPTED,
            alice.id,
            &FriendAccepted { user_id: alice.id, friend_id: bob.id, source: 0 },
        ))
        .await
        .unwrap();
    assert!(graph::blocked_ids_either(core, bob.id).await.unwrap().is_empty());
    assert_eq!(graph::friend_ids(core, bob.id).await.unwrap(), vec![alice.id]);
}
