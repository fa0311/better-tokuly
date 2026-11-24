//! Follows: following, locked accounts and follow requests, lists, counts,
//! preferences, blocks, suggestions and the limits.
mod common;

use common::*;
use tokuly_proto::social::{social_event::Event as Push, *};
use tokuly_social::{SocialConfig, graph};
use tokuly_testkit::TestUser;
use uuid::Uuid;

async fn summary(ctx: &Ctx, viewer: &TestUser, owner: &TestUser) -> FollowSummary {
    let response: GetFollowSummaryResponse = ctx
        .follows(
            "GetFollowSummary",
            viewer,
            &GetFollowSummaryRequest { user_id: owner.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    response.summary.into_option().unwrap()
}

async fn requests(ctx: &Ctx, user: &TestUser, direction: FollowRequestDirection) -> ListFollowRequestsResponse {
    ctx.follows(
        "ListFollowRequests",
        user,
        &ListFollowRequestsRequest { direction: direction.into(), ..Default::default() },
    )
    .await
    .unwrap()
}

async fn respond(
    ctx: &Ctx,
    user: &TestUser,
    request_id: &str,
    approve: bool,
) -> Result<RespondFollowRequestResponse, tokuly_testkit::RpcError> {
    ctx.follows(
        "RespondFollowRequest",
        user,
        &RespondFollowRequestRequest { request_id: request_id.into(), approve, ..Default::default() },
    )
    .await
}

async fn unfollow(ctx: &Ctx, follower: &TestUser, followee: &TestUser) {
    let _: UnfollowResponse = ctx
        .follows("Unfollow", follower, &UnfollowRequest { user_id: followee.id.to_string(), ..Default::default() })
        .await
        .unwrap();
}

fn state(response: &FollowResponse) -> FollowState {
    response.state.as_known().unwrap()
}

#[tokio::test]
async fn following_an_open_account_is_immediate_and_one_way() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let mut bob_devices = ctx.app.listen_user(bob.id).await;

    let followed = ctx.follow(&alice, &bob).await;
    assert_eq!(state(&followed), FollowState::FOLLOW_STATE_FOLLOWING);
    assert!(
        matches!(next_push(&mut bob_devices).await, Some(Push::NewFollower(entry)) if id_of(entry.user.as_option().unwrap()) == alice.id)
    );
    assert_eq!(ctx.notifications_for(bob.id, "social.new_follower").await.len(), 1);
    assert_eq!(ctx.app.events_by("social.follow.created", alice.id).await.len(), 1);

    // Following again changes nothing and notifies nobody.
    assert_eq!(state(&ctx.follow(&alice, &bob).await), FollowState::FOLLOW_STATE_FOLLOWING);
    assert_eq!(ctx.notifications_for(bob.id, "social.new_follower").await.len(), 1);

    assert!(
        graph::follows(core, alice.id, bob.id).await.unwrap() && !graph::follows(core, bob.id, alice.id).await.unwrap()
    );
    assert_eq!(graph::follower_ids(core, bob.id).await.unwrap(), vec![alice.id]);
    assert_eq!(graph::following_ids(core, alice.id).await.unwrap(), vec![bob.id]);
    assert!(!graph::are_friends(core, alice.id, bob.id).await.unwrap(), "a follow is not a friendship");

    let bobs = summary(&ctx, &alice, &bob).await;
    assert_eq!((bobs.follower_count, bobs.following_count), (1, 0));
    assert_eq!(bobs.relation.as_option().unwrap().following.as_known(), Some(FollowState::FOLLOW_STATE_FOLLOWING));
    let alices = summary(&ctx, &bob, &alice).await;
    assert!(alices.relation.as_option().unwrap().follows_you, "bob sees that alice follows him");

    unfollow(&ctx, &alice, &bob).await;
    unfollow(&ctx, &alice, &bob).await; // nothing left to undo: still fine
    assert!(graph::follower_ids(core, bob.id).await.unwrap().is_empty(), "the cached list follows the unfollow");
    assert_eq!(summary(&ctx, &alice, &bob).await.follower_count, 0);
}

#[tokio::test]
async fn follows_are_refused_for_oneself_blocks_and_missing_accounts() {
    let ctx = start().await;
    let (alice, bob, carol, gone) = (
        ctx.app.user("alice").await,
        ctx.app.user("bob").await,
        ctx.app.user("carol").await,
        ctx.app.user("gone").await,
    );
    let reason = |r: Result<FollowResponse, tokuly_testkit::RpcError>| r.unwrap_err().reason;

    assert_eq!(reason(ctx.follows("Follow", &alice, &follow_request(alice.id)).await), "CANNOT_FOLLOW_SELF");
    assert_eq!(reason(ctx.follows("Follow", &alice, &FollowRequest::default()).await), "MISSING_TARGET");
    assert_eq!(reason(ctx.follows("Follow", &alice, &follow_request(Uuid::now_v7())).await), "USER_NOT_FOUND");
    ctx.block(&bob, &alice).await;
    assert_eq!(
        reason(ctx.follows("Follow", &alice, &follow_request(bob.id)).await),
        "USER_NOT_FOUND",
        "a blocker looks like nobody"
    );
    assert_eq!(reason(ctx.follows("Follow", &bob, &follow_request(alice.id)).await), "USER_BLOCKED");
    sqlx::query("UPDATE users SET banned_at = now() WHERE id = $1")
        .bind(gone.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(reason(ctx.follows("Follow", &alice, &follow_request(gone.id)).await), "USER_NOT_FOUND");
    let by_handle = FollowRequest {
        target: Some(follow_request::Target::Handle(format!("@{}", carol.handle))),
        ..Default::default()
    };
    let followed: FollowResponse = ctx.follows("Follow", &alice, &by_handle).await.unwrap();
    assert_eq!(state(&followed), FollowState::FOLLOW_STATE_FOLLOWING);
}

#[tokio::test]
async fn locked_accounts_approve_followers_and_declining_is_silent() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let (lock, fan, other) = (ctx.app.user("lock").await, ctx.app.user("fan").await, ctx.app.user("other").await);
    ctx.set_locked(&lock, true).await;
    let mut lock_devices = ctx.app.listen_user(lock.id).await;

    let asked = ctx.follow(&fan, &lock).await;
    assert_eq!(state(&asked), FollowState::FOLLOW_STATE_REQUESTED);
    let request = asked.request.into_option().unwrap();
    assert!(matches!(next_push(&mut lock_devices).await, Some(Push::FollowRequestReceived(r)) if r.id == request.id));
    assert_eq!(ctx.notifications_for(lock.id, "social.follow_request").await.len(), 1);
    assert!(!graph::follows(core, fan.id, lock.id).await.unwrap());
    assert_eq!(
        state(&ctx.follow(&fan, &lock).await),
        FollowState::FOLLOW_STATE_REQUESTED,
        "asking again returns the request"
    );
    let incoming = requests(&ctx, &lock, FollowRequestDirection::FOLLOW_REQUEST_DIRECTION_INCOMING).await;
    assert_eq!((incoming.requests.len(), incoming.incoming_count), (1, 1));
    assert_eq!(
        respond(&ctx, &other, &request.id, true).await.unwrap_err().reason,
        "REQUEST_NOT_FOUND",
        "only the recipient answers"
    );

    let mut fan_devices = ctx.app.listen_user(fan.id).await;
    let approved = respond(&ctx, &lock, &request.id, true).await.unwrap();
    assert_eq!(id_of(approved.follower.as_option().unwrap().user.as_option().unwrap()), fan.id);
    assert!(graph::follows(core, fan.id, lock.id).await.unwrap());
    assert!(
        matches!(next_push(&mut fan_devices).await, Some(Push::FollowRequestApproved(e)) if id_of(e.user.as_option().unwrap()) == lock.id)
    );
    assert_eq!(ctx.notifications_for(fan.id, "social.follow_approved").await.len(), 1);
    assert_eq!(respond(&ctx, &lock, &request.id, true).await.unwrap_err().reason, "REQUEST_NOT_FOUND", "answered once");

    // Declining: the requester keeps seeing a pending request.
    let declined = ctx.follow(&other, &lock).await.request.into_option().unwrap();
    assert!(respond(&ctx, &lock, &declined.id, false).await.unwrap().follower.as_option().is_none());
    assert_eq!(
        requests(&ctx, &lock, FollowRequestDirection::FOLLOW_REQUEST_DIRECTION_INCOMING).await.incoming_count,
        0
    );
    assert_eq!(
        requests(&ctx, &other, FollowRequestDirection::FOLLOW_REQUEST_DIRECTION_OUTGOING).await.requests.len(),
        1
    );
    assert_eq!(state(&ctx.follow(&other, &lock).await), FollowState::FOLLOW_STATE_REQUESTED);
    assert_eq!(
        summary(&ctx, &other, &lock).await.relation.as_option().unwrap().following.as_known(),
        Some(FollowState::FOLLOW_STATE_REQUESTED)
    );
}

#[tokio::test]
async fn requests_can_be_withdrawn_and_unlocking_approves_the_waiting_ones() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let (lock, waiting, declined, withdrawn) = (
        ctx.app.user("lock").await,
        ctx.app.user("waiting").await,
        ctx.app.user("declined").await,
        ctx.app.user("withdrawn").await,
    );
    ctx.set_locked(&lock, true).await;
    ctx.follow(&waiting, &lock).await;
    let no = ctx.follow(&declined, &lock).await.request.into_option().unwrap();
    respond(&ctx, &lock, &no.id, false).await.unwrap();
    let gone = ctx.follow(&withdrawn, &lock).await.request.into_option().unwrap();

    let mut lock_devices = ctx.app.listen_user(lock.id).await;
    let _: CancelFollowRequestResponse = ctx
        .follows(
            "CancelFollowRequest",
            &withdrawn,
            &CancelFollowRequestRequest { request_id: gone.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(matches!(next_push(&mut lock_devices).await, Some(Push::FollowRequestCancelledId(id)) if id == gone.id));
    assert_eq!(ctx.app.events_by("social.follow.request_withdrawn", withdrawn.id).await.len(), 1);
    let again: Result<CancelFollowRequestResponse, _> = ctx
        .follows(
            "CancelFollowRequest",
            &withdrawn,
            &CancelFollowRequestRequest { request_id: gone.id.clone(), ..Default::default() },
        )
        .await;
    assert_eq!(again.unwrap_err().reason, "REQUEST_NOT_FOUND");

    let unlocked = ctx.set_locked(&lock, false).await;
    assert_eq!(unlocked.approved_request_count, 1);
    assert!(graph::follows(core, waiting.id, lock.id).await.unwrap());
    assert!(!graph::follows(core, declined.id, lock.id).await.unwrap(), "a declined request stays declined");
    assert_eq!(ctx.notifications_for(waiting.id, "social.follow_approved").await.len(), 1);
    assert_eq!(state(&ctx.follow(&withdrawn, &lock).await), FollowState::FOLLOW_STATE_FOLLOWING, "open now");
    assert_eq!(ctx.app.events_by("social.follow.settings_changed", lock.id).await.len(), 2);
}

#[tokio::test]
async fn minors_start_locked() {
    let ctx = start().await;
    let (teen, adult) = (ctx.app.user("teen").await, ctx.app.user("adult").await);
    ctx.make_minor(&teen).await;
    let settings: GetFollowSettingsResponse =
        ctx.follows("GetFollowSettings", &teen, &GetFollowSettingsRequest::default()).await.unwrap();
    assert!(settings.settings.as_option().unwrap().locked);
    assert_eq!(state(&ctx.follow(&adult, &teen).await), FollowState::FOLLOW_STATE_REQUESTED);
    ctx.set_locked(&teen, false).await;
    assert_eq!(
        summary(&ctx, &adult, &teen).await.relation.as_option().unwrap().following.as_known(),
        Some(FollowState::FOLLOW_STATE_FOLLOWING)
    );
}

#[tokio::test]
async fn lists_show_followers_following_mutuals_and_known_followers() {
    let ctx = start().await;
    let (star, a, b, c, viewer) = (
        ctx.app.user("star").await,
        ctx.app.user("a").await,
        ctx.app.user("b").await,
        ctx.app.user("c").await,
        ctx.app.user("viewer").await,
    );
    for fan in [&a, &b, &c] {
        ctx.follow(fan, &star).await;
    }
    ctx.follow(&star, &a).await;
    ctx.follow(&viewer, &b).await;

    let mut followers = follow_list_ids(&ctx, &viewer, star.id, FollowList::FOLLOW_LIST_FOLLOWERS).await;
    followers.sort();
    let mut expected = vec![a.id, b.id, c.id];
    expected.sort();
    assert_eq!(followers, expected);
    assert_eq!(follow_list_ids(&ctx, &viewer, star.id, FollowList::FOLLOW_LIST_FOLLOWING).await, vec![a.id]);
    assert_eq!(follow_list_ids(&ctx, &viewer, star.id, FollowList::FOLLOW_LIST_MUTUALS).await, vec![a.id]);
    assert_eq!(follow_list_ids(&ctx, &viewer, star.id, FollowList::FOLLOW_LIST_KNOWN_FOLLOWERS).await, vec![b.id]);
    let s = summary(&ctx, &viewer, &star).await;
    assert_eq!((s.follower_count, s.known_follower_count, s.known_followers.len()), (3, 1, 1));

    // Each row carries the viewer's relationship to that person.
    let list = follow_list(&ctx, &viewer, star.id, FollowList::FOLLOW_LIST_FOLLOWERS).await.unwrap();
    let row_b = list.entries.iter().find(|e| id_of(e.user.as_option().unwrap()) == b.id).unwrap();
    assert_eq!(row_b.relation.as_option().unwrap().following.as_known(), Some(FollowState::FOLLOW_STATE_FOLLOWING));
    assert_eq!(list.total, 3);

    // Paging and the name filter.
    let first: ListFollowsResponse = ctx
        .follows(
            "ListFollows",
            &viewer,
            &ListFollowsRequest {
                user_id: star.id.to_string(),
                page: tokuly_proto::common::PageRequest { limit: 2, ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(first.entries.len(), 2);
    let cursor = first.page.as_option().unwrap().next_cursor.clone();
    assert!(!cursor.is_empty());
    let second: ListFollowsResponse = ctx
        .follows(
            "ListFollows",
            &viewer,
            &ListFollowsRequest {
                user_id: star.id.to_string(),
                page: tokuly_proto::common::PageRequest { limit: 2, cursor, ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(second.entries.len(), 1);
    let filtered: ListFollowsResponse = ctx
        .follows(
            "ListFollows",
            &viewer,
            &ListFollowsRequest { user_id: star.id.to_string(), query: c.handle.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!((filtered.entries.len(), filtered.total), (1, 1));

    // Someone in a block with the viewer is left out of list and count alike.
    ctx.block(&c, &viewer).await;
    assert_eq!(follow_list(&ctx, &viewer, star.id, FollowList::FOLLOW_LIST_FOLLOWERS).await.unwrap().total, 2);
    assert_eq!(summary(&ctx, &viewer, &star).await.follower_count, 2);
    assert_eq!(summary(&ctx, &star, &star).await.follower_count, 3, "the owner's own count is whole");
    // Suspended accounts drop out for everyone.
    sqlx::query("UPDATE users SET banned_at = now() WHERE id = $1").bind(a.id).execute(&ctx.app.core.db).await.unwrap();
    assert_eq!(summary(&ctx, &star, &star).await.follower_count, 2);
}

#[tokio::test]
async fn a_locked_accounts_lists_are_for_its_followers_and_friends() {
    let ctx = start().await;
    let (lock, follower, friend, stranger) = (
        ctx.app.user("lock").await,
        ctx.app.user("follower").await,
        ctx.app.user("friend").await,
        ctx.app.user("stranger").await,
    );
    ctx.follow(&follower, &lock).await;
    ctx.befriend(&friend, &lock).await;
    ctx.set_locked(&lock, true).await;
    assert_eq!(
        follow_list(&ctx, &stranger, lock.id, FollowList::FOLLOW_LIST_FOLLOWERS).await.unwrap_err().reason,
        "FOLLOW_LIST_PRIVATE"
    );
    assert!(follow_list(&ctx, &follower, lock.id, FollowList::FOLLOW_LIST_FOLLOWING).await.is_ok());
    assert!(follow_list(&ctx, &friend, lock.id, FollowList::FOLLOW_LIST_FOLLOWERS).await.is_ok());
    assert!(follow_list(&ctx, &lock, lock.id, FollowList::FOLLOW_LIST_FOLLOWERS).await.is_ok());
    let s = summary(&ctx, &stranger, &lock).await;
    assert!(s.locked && s.known_followers.is_empty());
    assert_eq!(s.follower_count, 1, "counts stay visible");
    ctx.block(&lock, &stranger).await;
    assert_eq!(
        follow_list(&ctx, &stranger, lock.id, FollowList::FOLLOW_LIST_FOLLOWERS).await.unwrap_err().reason,
        "USER_NOT_FOUND"
    );
}

#[tokio::test]
async fn removing_a_follower_and_blocking_end_follows_silently() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let (owner, fan, rival, asker) = (
        ctx.app.user("owner").await,
        ctx.app.user("fan").await,
        ctx.app.user("rival").await,
        ctx.app.user("asker").await,
    );
    ctx.follow(&fan, &owner).await;
    ctx.follow(&rival, &owner).await;
    ctx.follow(&owner, &rival).await;
    let mut fan_devices = ctx.app.listen_user(fan.id).await;
    let _: RemoveFollowerResponse = ctx
        .follows("RemoveFollower", &owner, &RemoveFollowerRequest { user_id: fan.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    assert!(!graph::follows(core, fan.id, owner.id).await.unwrap());
    assert!(pushes(&mut fan_devices, QUIET).await.is_empty(), "the removed follower is not told");
    assert_eq!(state(&ctx.follow(&fan, &owner).await), FollowState::FOLLOW_STATE_FOLLOWING, "they may follow again");

    // A block ends follows in both directions and requests too.
    ctx.set_locked(&rival, true).await;
    ctx.follow(&asker, &rival).await;
    ctx.follow(&rival, &asker).await;
    ctx.block(&owner, &rival).await;
    assert!(
        !graph::follows(core, rival.id, owner.id).await.unwrap()
            && !graph::follows(core, owner.id, rival.id).await.unwrap()
    );
    assert!(graph::following_ids(core, owner.id).await.unwrap().is_empty());
    let removed = ctx.app.events_by("social.follow.removed", owner.id).await;
    assert_eq!(removed.iter().filter(|e| e.data["reason"] == "blocked").count(), 2);
    ctx.block(&rival, &asker).await;
    assert!(
        requests(&ctx, &rival, FollowRequestDirection::FOLLOW_REQUEST_DIRECTION_INCOMING).await.requests.is_empty()
    );
    assert!(
        requests(&ctx, &asker, FollowRequestDirection::FOLLOW_REQUEST_DIRECTION_OUTGOING).await.requests.is_empty()
    );
}

#[tokio::test]
async fn followers_mute_and_choose_notifications_privately() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let (fan, star, quiet) = (ctx.app.user("fan").await, ctx.app.user("star").await, ctx.app.user("quiet").await);
    let prefs = |user_id: Uuid, muted: Option<bool>, mute_seconds: i64, notify: Option<FollowNotifyLevel>| {
        UpdateFollowPrefsRequest {
            user_id: user_id.to_string(),
            muted,
            mute_seconds,
            notify_level: notify.map(Into::into),
            ..Default::default()
        }
    };
    let not_following: Result<UpdateFollowPrefsResponse, _> =
        ctx.follows("UpdateFollowPrefs", &fan, &prefs(star.id, Some(true), 0, None)).await;
    assert_eq!(not_following.unwrap_err().reason, "NOT_FOLLOWING");
    ctx.follow(&fan, &star).await;
    ctx.follow(&quiet, &star).await;

    let bad: Result<UpdateFollowPrefsResponse, _> =
        ctx.follows("UpdateFollowPrefs", &fan, &prefs(star.id, Some(true), 60, None)).await;
    assert_eq!(bad.unwrap_err().reason, "INVALID_MUTE_DURATION");
    let timed: UpdateFollowPrefsResponse =
        ctx.follows("UpdateFollowPrefs", &fan, &prefs(star.id, Some(true), 3600, None)).await.unwrap();
    let timed = timed.prefs.into_option().unwrap();
    assert!(timed.muted && timed.muted_until.as_option().is_some());
    assert_eq!(graph::muted_followee_ids(core, fan.id).await.unwrap(), vec![star.id]);
    sqlx::query("UPDATE follows SET muted_until = now() - interval '1 minute' WHERE follower_id = $1")
        .bind(fan.id)
        .execute(&core.db)
        .await
        .unwrap();
    assert!(graph::muted_followee_ids(core, fan.id).await.unwrap().is_empty(), "a timed mute runs out by itself");

    let _: UpdateFollowPrefsResponse = ctx
        .follows("UpdateFollowPrefs", &fan, &prefs(star.id, None, 0, Some(FollowNotifyLevel::FOLLOW_NOTIFY_LEVEL_ALL)))
        .await
        .unwrap();
    let _: UpdateFollowPrefsResponse = ctx
        .follows(
            "UpdateFollowPrefs",
            &quiet,
            &prefs(star.id, Some(true), 0, Some(FollowNotifyLevel::FOLLOW_NOTIFY_LEVEL_ALL)),
        )
        .await
        .unwrap();
    assert_eq!(
        graph::post_notification_recipients(core, star.id).await.unwrap(),
        vec![fan.id],
        "a muted follower gets no post notifications"
    );
    let own = summary(&ctx, &fan, &star).await.prefs.into_option().unwrap();
    assert_eq!(own.notify_level.as_known(), Some(FollowNotifyLevel::FOLLOW_NOTIFY_LEVEL_ALL));
    assert!(summary(&ctx, &star, &fan).await.prefs.as_option().is_none(), "the followee learns nothing");
    // The follower's own following list carries the preferences.
    let mine = follow_list(&ctx, &fan, fan.id, FollowList::FOLLOW_LIST_FOLLOWING).await.unwrap();
    assert!(mine.entries[0].prefs.as_option().is_some());
    assert_eq!(
        ctx.app.events_by("social.follow.prefs_changed", fan.id).await.len()
            + ctx.app.events_by("social.follow.prefs_changed", quiet.id).await.len(),
        3
    );
}

#[tokio::test]
async fn the_follow_limit_holds_under_concurrent_follows() {
    let ctx = start_with(SocialConfig { max_following: 3, ..SocialConfig::default() }).await;
    let fan = ctx.app.user("fan").await;
    let mut stars = Vec::new();
    for i in 0..6 {
        stars.push(ctx.app.user(&format!("star{i}")).await);
    }
    // Half of them locked: requests count towards the limit too.
    for star in &stars[..3] {
        ctx.set_locked(star, true).await;
    }
    let results = futures::future::join_all(
        stars
            .iter()
            .map(|star| async { ctx.follows::<_, FollowResponse>("Follow", &fan, &follow_request(star.id)).await }),
    )
    .await;
    let succeeded = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(succeeded, 3, "{results:?}");
    assert!(results.iter().filter_map(|r| r.as_ref().err()).all(|e| e.reason == "FOLLOW_LIMIT_REACHED"));
}

#[tokio::test]
async fn one_account_following_many_at_once_does_not_starve_the_pool() {
    let ctx = start().await;
    let fan = ctx.app.user("fan").await;
    let mut stars = Vec::new();
    for i in 0..12 {
        let star = ctx.app.user(&format!("star{i}")).await;
        if i % 2 == 0 {
            ctx.set_locked(&star, true).await;
        }
        stars.push(star);
    }
    // More concurrent calls than the test pool has connections, each taking locks.
    let started = std::time::Instant::now();
    let results = futures::future::join_all(
        stars
            .iter()
            .map(|star| async { ctx.follows::<_, FollowResponse>("Follow", &fan, &follow_request(star.id)).await }),
    )
    .await;
    assert!(results.iter().all(|r| r.is_ok()), "{results:?}");
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    // Unlocking while requests arrive: none is left waiting on an open account.
    let lock = &stars[0];
    let late = ctx.app.user("late").await;
    let (_, _) = tokio::join!(ctx.set_locked(lock, false), async {
        ctx.follows::<_, FollowResponse>("Follow", &late, &follow_request(lock.id)).await
    });
    let left = requests(&ctx, lock, FollowRequestDirection::FOLLOW_REQUEST_DIRECTION_INCOMING).await;
    assert_eq!(left.incoming_count, 0, "a request stored during the unlock was approved by it");
}

#[tokio::test]
async fn suggestions_come_from_several_signals_and_respect_boundaries() {
    let ctx = start().await;
    let (me, friend, fan, fof, teen, dismissed) = (
        ctx.app.user("me").await,
        ctx.app.user("friend").await,
        ctx.app.user("fan").await,
        ctx.app.user("fof").await,
        ctx.app.user("teen").await,
        ctx.app.user("dismissed").await,
    );
    ctx.befriend(&me, &friend).await;
    ctx.befriend(&friend, &fof).await;
    ctx.befriend(&friend, &teen).await;
    ctx.befriend(&friend, &dismissed).await;
    ctx.make_minor(&teen).await;
    ctx.follow(&fan, &me).await;
    let _: DismissFollowSuggestionResponse = ctx
        .follows(
            "DismissFollowSuggestion",
            &me,
            &DismissFollowSuggestionRequest { user_id: dismissed.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();

    let list: ListFollowSuggestionsResponse =
        ctx.follows("ListFollowSuggestions", &me, &ListFollowSuggestionsRequest::default()).await.unwrap();
    let by_id: std::collections::HashMap<Uuid, &FollowSuggestion> =
        list.suggestions.iter().map(|s| (id_of(s.user.as_option().unwrap()), s)).collect();
    assert_eq!(by_id[&fan.id].reasons, vec!["FOLLOWS_YOU"]);
    assert_eq!(by_id[&fof.id].reasons, vec!["FRIEND_OF_FRIEND"]);
    assert!(!by_id.contains_key(&friend.id), "friends are not suggested");
    assert!(!by_id.contains_key(&teen.id), "an adult is not shown minors they do not know");
    assert!(!by_id.contains_key(&dismissed.id));
    assert!(!by_id.contains_key(&me.id));
    ctx.follow(&me, &fan).await;
    let list: ListFollowSuggestionsResponse =
        ctx.follows("ListFollowSuggestions", &me, &ListFollowSuggestionsRequest::default()).await.unwrap();
    assert!(
        list.suggestions.iter().all(|s| id_of(s.user.as_option().unwrap()) != fan.id),
        "followed accounts drop out"
    );
}

#[tokio::test]
async fn profiles_carry_a_follow_card() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.follow(&alice, &bob).await;
    let response: tokuly_proto::identity::GetProfileResponse = ctx
        .app
        .call(
            "tokuly.identity.v1.ProfileService/GetProfile",
            Some(&alice),
            &tokuly_proto::identity::GetProfileRequest {
                by: Some(tokuly_proto::identity::get_profile_request::By::UserId(bob.id.to_string())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let profile = response.profile.into_option().unwrap();
    let card = profile.cards.iter().find(|c| c.kind == "social.follow").expect("follow card");
    assert_eq!(card.data["follower_count"], "1");
    assert_eq!(card.data["following"], "FOLLOWING");
    assert_eq!(card.path, format!("/u/{}/followers", bob.handle));
}
