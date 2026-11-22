//! `FriendService`: the life of a friend request — sending, crossing,
//! accepting, declining, cancelling, listing, and the limits around it.
mod common;

use common::*;
use tokuly_proto::{
    common::PageRequest,
    identity::{Audience, Relationship},
    social::{social_event::Event as Push, *},
};
use tokuly_social::{SocialConfig, graph};
use tokuly_testkit::TestUser;
use uuid::Uuid;

#[tokio::test]
async fn a_request_notifies_the_recipient_and_is_idempotent() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("ありす").await, ctx.app.user("bob").await);
    let mut bob_devices = ctx.app.listen_user(bob.id).await;

    let mut request = request_to(bob.id);
    request.message = "  よろしく!  ".into();
    let sent: SendFriendRequestResponse = ctx.friends("SendFriendRequest", &alice, &request).await.unwrap();
    assert_eq!(sent.relationship, Relationship::RELATIONSHIP_REQUEST_SENT);
    let pending = sent.request.as_option().unwrap();
    assert_eq!((id_of(pending.from.as_option().unwrap()), id_of(pending.to.as_option().unwrap())), (alice.id, bob.id));
    assert_eq!(pending.message, "よろしく!");
    assert_eq!(pending.source, FriendSource::FRIEND_SOURCE_PROFILE);

    // Realtime push, domain event and a notification for Bob.
    match next_push(&mut bob_devices).await {
        Some(Push::FriendRequestReceived(pushed)) => assert_eq!(pushed.id, pending.id),
        other => panic!("expected a friend request push, got {other:?}"),
    }
    let events = ctx.app.events_by("social.friend.requested", alice.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["to_user_id"], serde_json::json!(bob.id));
    let notices = ctx.notifications_for(bob.id, "social.friend_request").await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].data["actor_id"], serde_json::json!(alice.id));
    assert_eq!(notices[0].data["body"]["ja"], "ありすさんから友達リクエストが届いたよ!");

    // Asking again returns the same request and tells nobody.
    let again = ctx.request(&alice, &bob).await;
    assert_eq!(again.request.as_option().unwrap().id, pending.id);
    assert_eq!(ctx.notifications_for(bob.id, "social.friend_request").await.len(), 1);
    assert!(pushes(&mut bob_devices, QUIET).await.is_empty());

    // By handle works too (here: Bob's handle, from a third user).
    let carol = ctx.app.user("carol").await;
    let by_handle: SendFriendRequestResponse = ctx
        .friends(
            "SendFriendRequest",
            &carol,
            &SendFriendRequestRequest {
                target: Some(send_friend_request_request::Target::Handle(format!("@{}", bob.handle))),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(id_of(by_handle.request.as_option().unwrap().to.as_option().unwrap()), bob.id);
}

#[tokio::test]
async fn request_validation_and_authorization() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let send = |request: SendFriendRequestRequest| {
        let (ctx, alice) = (&ctx, alice.clone());
        async move { ctx.friends::<_, SendFriendRequestResponse>("SendFriendRequest", &alice, &request).await.unwrap_err() }
    };

    assert_eq!(send(SendFriendRequestRequest::default()).await.reason, "MISSING_TARGET");
    assert_eq!(send(request_to(alice.id)).await.reason, "CANNOT_FRIEND_SELF");
    let ghost = send(request_to(Uuid::now_v7())).await;
    assert_eq!((ghost.status, ghost.reason.as_str()), (404, "USER_NOT_FOUND"));
    let unknown_handle = SendFriendRequestRequest {
        target: Some(send_friend_request_request::Target::Handle("nobody-here-000".into())),
        ..Default::default()
    };
    assert_eq!(send(unknown_handle).await.reason, "USER_NOT_FOUND");
    let bad_id = SendFriendRequestRequest {
        target: Some(send_friend_request_request::Target::UserId("nope".into())),
        ..Default::default()
    };
    assert_eq!(send(bad_id).await.reason, "INVALID_ID");
    let mut wordy = request_to(bob.id);
    wordy.message = "あ".repeat(141);
    assert_eq!(send(wordy).await.reason, "MESSAGE_TOO_LONG");

    let anonymous = ctx
        .app
        .call::<_, SendFriendRequestResponse>(&format!("{FRIENDS}/SendFriendRequest"), None, &request_to(bob.id))
        .await
        .unwrap_err();
    assert_eq!((anonymous.status, anonymous.reason.as_str()), (401, "SIGN_IN_REQUIRED"));

    ctx.befriend(&alice, &bob).await;
    let dup = send(request_to(bob.id)).await;
    assert_eq!((dup.code.as_str(), dup.reason.as_str()), ("already_exists", "ALREADY_FRIENDS"));
}

#[tokio::test]
async fn crossed_requests_make_friends_immediately() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.request(&alice, &bob).await;
    let mut alice_devices = ctx.app.listen_user(alice.id).await;

    let crossed = ctx.request(&bob, &alice).await;
    assert_eq!(crossed.relationship, Relationship::RELATIONSHIP_FRIENDS);
    assert!(crossed.request.is_unset());
    assert!(graph::are_friends(&ctx.app.core, alice.id, bob.id).await.unwrap());
    // No request is left over in either direction.
    assert!(
        list_requests(&ctx, &alice, FriendRequestDirection::FRIEND_REQUEST_DIRECTION_OUTGOING)
            .await
            .requests
            .is_empty()
    );
    assert!(
        list_requests(&ctx, &alice, FriendRequestDirection::FRIEND_REQUEST_DIRECTION_INCOMING)
            .await
            .requests
            .is_empty()
    );
    // Alice asked first, so she is the one told "accepted".
    match next_push(&mut alice_devices).await {
        Some(Push::FriendRequestAccepted(friend)) => assert_eq!(id_of(friend.user.as_option().unwrap()), bob.id),
        other => panic!("expected an accepted push, got {other:?}"),
    }
    assert_eq!(ctx.notifications_for(alice.id, "social.friend_accepted").await.len(), 1);
}

#[tokio::test]
async fn privacy_decides_who_may_send_requests() {
    let ctx = start().await;
    let (picky, friend, friend_of_friend, stranger) = (
        ctx.app.user("picky").await,
        ctx.app.user("friend").await,
        ctx.app.user("fof").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&picky, &friend).await;
    ctx.befriend(&friend, &friend_of_friend).await;

    ctx.set_privacy(&picky, |p| p.friend_requests_from = Audience::AUDIENCE_FRIENDS_OF_FRIENDS.into()).await;
    let denied = ctx
        .friends::<_, SendFriendRequestResponse>("SendFriendRequest", &stranger, &request_to(picky.id))
        .await
        .unwrap_err();
    assert_eq!((denied.status, denied.reason.as_str()), (403, "FRIEND_REQUESTS_RESTRICTED"));
    assert_eq!(ctx.request(&friend_of_friend, &picky).await.relationship, Relationship::RELATIONSHIP_REQUEST_SENT);

    ctx.set_privacy(&picky, |p| p.friend_requests_from = Audience::AUDIENCE_NOBODY.into()).await;
    let denied = ctx
        .friends::<_, SendFriendRequestResponse>("SendFriendRequest", &stranger, &request_to(picky.id))
        .await
        .unwrap_err();
    assert_eq!(denied.reason, "FRIEND_REQUESTS_RESTRICTED");
    // Picky can still reach out, and an existing request to them can still be answered by them.
    assert_eq!(ctx.request(&picky, &stranger).await.relationship, Relationship::RELATIONSHIP_REQUEST_SENT);
}

#[tokio::test]
async fn friend_limit_applies_to_both_sides() {
    let ctx = start_with(SocialConfig { max_friends: 1, ..Default::default() }).await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    let pending = ctx.request(&carol, &alice).await.request.into_option().unwrap();
    ctx.befriend(&alice, &bob).await;

    // Alice is full: she can neither ask nor accept.
    let full = ctx
        .friends::<_, SendFriendRequestResponse>(
            "SendFriendRequest",
            &alice,
            &request_to(ctx.app.user("dave").await.id),
        )
        .await
        .unwrap_err();
    assert_eq!(full.reason, "FRIEND_LIMIT_REACHED");
    let full = ctx
        .friends::<_, RespondFriendRequestResponse>(
            "RespondFriendRequest",
            &alice,
            &RespondFriendRequestRequest { request_id: pending.id.clone(), accept: true, ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(full.reason, "FRIEND_LIMIT_REACHED");
    assert!(!graph::are_friends(&ctx.app.core, alice.id, carol.id).await.unwrap());
    assert_eq!(
        list_requests(&ctx, &alice, FriendRequestDirection::FRIEND_REQUEST_DIRECTION_INCOMING).await.requests.len(),
        1,
        "the request survives"
    );
}

#[tokio::test]
async fn accepting_creates_the_friendship_with_events_and_activity() {
    let ctx = start().await;
    let (alice, bob, eve) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("eve").await);
    let request_id = ctx.request(&alice, &bob).await.request.into_option().unwrap().id;
    let mut alice_devices = ctx.app.listen_user(alice.id).await;
    let respond = |user: &TestUser, accept: bool| {
        let (ctx, user, request_id) = (&ctx, user.clone(), request_id.clone());
        async move {
            ctx.friends::<_, RespondFriendRequestResponse>(
                "RespondFriendRequest",
                &user,
                &RespondFriendRequestRequest { request_id, accept, ..Default::default() },
            )
            .await
        }
    };

    // Only the recipient can answer.
    for outsider in [&alice, &eve] {
        let err = respond(outsider, true).await.unwrap_err();
        assert_eq!((err.status, err.reason.as_str()), (404, "REQUEST_NOT_FOUND"));
    }
    let accepted = respond(&bob, true).await.unwrap();
    let friend = accepted.friend.as_option().expect("the new friend");
    assert_eq!(id_of(friend.user.as_option().unwrap()), alice.id);
    assert!(friend.friends_since.is_set());

    assert_eq!(ctx.app.events_by("social.friend.accepted", bob.id).await.len(), 1);
    assert_eq!(ctx.notifications_for(alice.id, "social.friend_accepted").await.len(), 1);
    assert!(
        ctx.notifications_for(bob.id, "social.friend_accepted").await.is_empty(),
        "the accepter needs no notification"
    );
    for (user, friend) in [(&alice, &bob), (&bob, &alice)] {
        let activity = ctx.activities_of(user.id, "social.friend_added").await;
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].data["with_user_ids"], serde_json::json!([friend.id]));
    }
    match next_push(&mut alice_devices).await {
        Some(Push::FriendRequestAccepted(friend)) => assert_eq!(id_of(friend.user.as_option().unwrap()), bob.id),
        other => panic!("expected an accepted push, got {other:?}"),
    }
    assert_eq!(respond(&bob, true).await.unwrap_err().reason, "REQUEST_NOT_FOUND", "a request is answered once");
}

#[tokio::test]
async fn declining_is_silent_and_cancelling_withdraws() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    let to_bob = ctx.request(&alice, &bob).await.request.into_option().unwrap().id;
    let to_carol = ctx.request(&alice, &carol).await.request.into_option().unwrap().id;
    assert_eq!(
        list_requests(&ctx, &bob, FriendRequestDirection::FRIEND_REQUEST_DIRECTION_UNSPECIFIED).await.incoming_count,
        1
    );

    // Bob declines: nothing for Alice to see, nothing left for Bob.
    let mut alice_devices = ctx.app.listen_user(alice.id).await;
    let declined: RespondFriendRequestResponse = ctx
        .friends(
            "RespondFriendRequest",
            &bob,
            &RespondFriendRequestRequest { request_id: to_bob.clone(), accept: false, ..Default::default() },
        )
        .await
        .unwrap();
    assert!(declined.friend.is_unset());
    assert!(pushes(&mut alice_devices, QUIET).await.is_empty());
    let incoming = list_requests(&ctx, &bob, FriendRequestDirection::FRIEND_REQUEST_DIRECTION_INCOMING).await;
    assert!(incoming.requests.is_empty() && incoming.incoming_count == 0);
    let outgoing = list_requests(&ctx, &alice, FriendRequestDirection::FRIEND_REQUEST_DIRECTION_OUTGOING).await;
    assert_eq!(outgoing.requests.len(), 2, "the sender still sees the declined request as pending");
    assert_eq!(outgoing.requests[0].id, to_carol, "newest first");

    // Carol's request is withdrawn; only the sender may do that.
    let mut carol_devices = ctx.app.listen_user(carol.id).await;
    let cancel = |user: &TestUser| {
        let (ctx, user, request_id) = (&ctx, user.clone(), to_carol.clone());
        async move {
            ctx.friends::<_, CancelFriendRequestResponse>(
                "CancelFriendRequest",
                &user,
                &CancelFriendRequestRequest { request_id, ..Default::default() },
            )
            .await
        }
    };
    assert_eq!(cancel(&carol).await.unwrap_err().reason, "REQUEST_NOT_FOUND");
    cancel(&alice).await.unwrap();
    match next_push(&mut carol_devices).await {
        Some(Push::FriendRequestCancelledId(id)) => assert_eq!(id, to_carol),
        other => panic!("expected a cancellation push, got {other:?}"),
    }
    assert!(
        list_requests(&ctx, &carol, FriendRequestDirection::FRIEND_REQUEST_DIRECTION_INCOMING)
            .await
            .requests
            .is_empty()
    );
    assert_eq!(cancel(&alice).await.unwrap_err().reason, "REQUEST_NOT_FOUND");

    // Withdrawing the declined one tells nobody: Bob no longer had it on screen.
    let mut bob_devices = ctx.app.listen_user(bob.id).await;
    let _: CancelFriendRequestResponse = ctx
        .friends(
            "CancelFriendRequest",
            &alice,
            &CancelFriendRequestRequest { request_id: to_bob, ..Default::default() },
        )
        .await
        .unwrap();
    assert!(pushes(&mut bob_devices, QUIET).await.is_empty());
}

#[tokio::test]
async fn requests_are_listed_per_direction_with_pagination_and_mutual_friends() {
    let ctx = start().await;
    let (star, mutual) = (ctx.app.user("star").await, ctx.app.user("mutual").await);
    ctx.befriend(&star, &mutual).await;
    let mut fans = Vec::new();
    for i in 0..3 {
        let fan = ctx.app.user(&format!("fan{i}")).await;
        ctx.request(&fan, &star).await;
        fans.push(fan);
    }
    ctx.befriend(&fans[2], &mutual).await;

    let first: ListFriendRequestsResponse = ctx
        .friends(
            "ListFriendRequests",
            &star,
            &ListFriendRequestsRequest {
                direction: FriendRequestDirection::FRIEND_REQUEST_DIRECTION_INCOMING.into(),
                page: PageRequest { limit: 2, ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(first.incoming_count, 3);
    assert_eq!(
        first.requests.iter().map(|r| id_of(r.from.as_option().unwrap())).collect::<Vec<_>>(),
        vec![fans[2].id, fans[1].id]
    );
    assert_eq!(first.requests[0].mutual_friend_count, 1);
    assert_eq!(first.requests[1].mutual_friend_count, 0);
    let page = first.page.as_option().unwrap();
    assert!(page.has_more);
    let rest: ListFriendRequestsResponse = ctx
        .friends(
            "ListFriendRequests",
            &star,
            &ListFriendRequestsRequest {
                direction: FriendRequestDirection::FRIEND_REQUEST_DIRECTION_INCOMING.into(),
                page: PageRequest { limit: 2, cursor: page.next_cursor.clone(), ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(rest.requests.iter().map(|r| id_of(r.from.as_option().unwrap())).collect::<Vec<_>>(), vec![fans[0].id]);
    assert!(!rest.page.as_option().unwrap().has_more);

    assert_eq!(
        list_requests(&ctx, &fans[0], FriendRequestDirection::FRIEND_REQUEST_DIRECTION_OUTGOING).await.requests.len(),
        1
    );
    assert!(
        list_requests(&ctx, &fans[0], FriendRequestDirection::FRIEND_REQUEST_DIRECTION_INCOMING)
            .await
            .requests
            .is_empty()
    );
}

#[tokio::test]
async fn new_friendships_are_announced_only_by_those_who_show_their_friend_list() {
    let ctx = start().await;
    let (open, private) = (ctx.app.user("open").await, ctx.app.user("private").await);
    ctx.set_privacy(&private, |p| p.friend_list_visible_to = Audience::AUDIENCE_NOBODY.into()).await;
    ctx.befriend(&open, &private).await;
    assert_eq!(ctx.activities_of(open.id, "social.friend_added").await[0].data["audience"], "friends");
    assert_eq!(
        ctx.activities_of(private.id, "social.friend_added").await[0].data["audience"],
        "private",
        "a hidden friend list stays hidden"
    );
}

#[tokio::test]
async fn request_limits_stop_pestering_and_hoarding() {
    let ctx = start().await;
    let (pest, target) = (ctx.app.user("pest").await, ctx.app.user("target").await);
    // Withdrawing and re-sending works a few times, then the pair is rate limited.
    let mut refusal = None;
    for _ in 0..6 {
        match ctx.friends::<_, SendFriendRequestResponse>("SendFriendRequest", &pest, &request_to(target.id)).await {
            Ok(sent) => {
                let request_id = sent.request.into_option().unwrap().id;
                let _: CancelFriendRequestResponse = ctx
                    .friends(
                        "CancelFriendRequest",
                        &pest,
                        &CancelFriendRequestRequest { request_id, ..Default::default() },
                    )
                    .await
                    .unwrap();
            }
            Err(e) => refusal = Some(e),
        }
    }
    let refusal = refusal.expect("the sixth request to the same person within a day is refused");
    assert_eq!((refusal.status, refusal.reason.as_str()), (429, "RATE_LIMITED"));

    // Nobody can sit on hundreds of unanswered requests.
    let hoarder = ctx.app.user("hoarder").await;
    let crowd = ctx.crowd(500).await;
    sqlx::query(
        "INSERT INTO friend_requests (from_user_id, to_user_id) SELECT $1, id FROM unnest($2::uuid[]) AS c(id)",
    )
    .bind(hoarder.id)
    .bind(&crowd)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    let full = ctx
        .friends::<_, SendFriendRequestResponse>("SendFriendRequest", &hoarder, &request_to(target.id))
        .await
        .unwrap_err();
    assert_eq!(full.reason, "TOO_MANY_PENDING_REQUESTS");
}
