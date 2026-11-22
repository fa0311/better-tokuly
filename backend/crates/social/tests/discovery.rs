//! Finding people: suggestions, contact matching, invites and intros, and
//! the nearby discovery stream.
mod common;

use std::time::Duration;

use common::*;
use sha2::{Digest as _, Sha256};
use tokuly_core::events::Event;
use tokuly_proto::{
    common::PageRequest,
    identity::{Audience, GetProfileRequest, GetProfileResponse, Relationship, get_profile_request},
    social::*,
};
use tokuly_social::suggestions::{self, SuggestionReason};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

async fn suggestions_for(ctx: &Ctx, user: &TestUser, limit: i32) -> Vec<Suggestion> {
    let response: ListSuggestionsResponse =
        ctx.friends("ListSuggestions", user, &ListSuggestionsRequest { limit, ..Default::default() }).await.unwrap();
    response.suggestions
}

fn suggested(suggestions: &[Suggestion]) -> Vec<(Uuid, &str)> {
    suggestions.iter().map(|s| (id_of(s.user.as_option().unwrap()), s.reason.as_str())).collect()
}

/// What a client uploads for one address-book email.
fn contact_hash(email: &str) -> String {
    hex::encode(Sha256::digest(email.trim().to_lowercase().as_bytes()))
}

fn email_of(user: &TestUser) -> String {
    format!("{}@test.tokuly.localhost", user.handle)
}

#[tokio::test]
async fn friends_of_friends_are_suggested_until_related_or_dismissed() {
    let ctx = start().await;
    let me = ctx.app.user("me").await;
    let (f1, f2) = (ctx.app.user("f1").await, ctx.app.user("f2").await);
    ctx.befriend(&me, &f1).await;
    ctx.befriend(&me, &f2).await;
    // Friends of my friends, each in a different situation.
    let close = ctx.app.user("close").await; // knows both of my friends
    let distant = ctx.app.user("distant").await;
    let dismissed = ctx.app.user("dismissed").await;
    let private = ctx.app.user("private").await; // not discoverable
    let requested = ctx.app.user("requested").await; // I already asked
    let blocker = ctx.app.user("blocker").await; // blocked me
    for fof in [&close, &distant, &private, &requested, &blocker] {
        ctx.befriend(&f1, fof).await;
    }
    ctx.befriend(&f2, &close).await;
    ctx.befriend(&f2, &dismissed).await;
    ctx.set_privacy(&private, |p| p.discoverable = false).await;
    ctx.request(&me, &requested).await;
    let _: BlockUserResponse = ctx
        .friends("BlockUser", &blocker, &BlockUserRequest { user_id: me.id.to_string(), ..Default::default() })
        .await
        .unwrap();

    let found = suggestions_for(&ctx, &me, 0).await;
    assert_eq!(found[0].reason, "FRIEND_OF_FRIEND");
    assert_eq!(id_of(found[0].user.as_option().unwrap()), close.id, "most mutual friends first");
    assert_eq!(found[0].mutual_friend_count, 2);
    let mut preview: Vec<Uuid> = found[0].mutual_friends.iter().map(id_of).collect();
    preview.sort();
    let mut both = vec![f1.id, f2.id];
    both.sort();
    assert_eq!(preview, both);
    let mut rest: Vec<Uuid> = found[1..].iter().map(|s| id_of(s.user.as_option().unwrap())).collect();
    rest.sort();
    let mut expected = vec![distant.id, dismissed.id];
    expected.sort();
    assert_eq!(rest, expected, "friends, pending requests, blocks and hidden accounts are left out");

    assert_eq!(suggestions_for(&ctx, &me, 1).await.len(), 1);

    let _: DismissSuggestionResponse = ctx
        .friends(
            "DismissSuggestion",
            &me,
            &DismissSuggestionRequest { user_id: dismissed.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let after = suggestions_for(&ctx, &me, 0).await;
    assert_eq!(suggested(&after).len(), 2);
    assert!(after.iter().all(|s| id_of(s.user.as_option().unwrap()) != dismissed.id));
    let err = ctx
        .friends::<_, DismissSuggestionResponse>(
            "DismissSuggestion",
            &me,
            &DismissSuggestionRequest { user_id: Uuid::now_v7().to_string(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(err.reason, "USER_NOT_FOUND");

    // Becoming friends removes a suggestion as well.
    ctx.befriend(&me, &close).await;
    assert_eq!(suggested(&suggestions_for(&ctx, &me, 0).await), vec![(distant.id, "FRIEND_OF_FRIEND")]);
}

#[tokio::test]
async fn hints_from_other_domains_rank_above_friends_of_friends() {
    let ctx = start().await;
    let (me, friend, fof, regular) = (
        ctx.app.user("me").await,
        ctx.app.user("friend").await,
        ctx.app.user("fof").await,
        ctx.app.user("regular").await,
    );
    ctx.befriend(&me, &friend).await;
    ctx.befriend(&friend, &fof).await;
    suggestions::suggest(&ctx.app.core, me.id, regular.id, SuggestionReason::NearbyOften).await.unwrap();
    suggestions::suggest(&ctx.app.core, me.id, regular.id, SuggestionReason::NearbyOften).await.unwrap(); // repeat is harmless
    suggestions::suggest(&ctx.app.core, me.id, me.id, SuggestionReason::NearbyOften).await.unwrap(); // ignored
    assert_eq!(
        suggested(&suggestions_for(&ctx, &me, 0).await),
        vec![(regular.id, "NEARBY_OFTEN"), (fof.id, "FRIEND_OF_FRIEND")]
    );
}

#[tokio::test]
async fn contacts_match_verified_discoverable_accounts_by_email_hash() {
    let ctx = start().await;
    let me = ctx.app.user("me").await;
    let (known, friend, hidden, unverified) = (
        ctx.app.user("known").await,
        ctx.app.user("friend").await,
        ctx.app.user("hidden").await,
        ctx.app.user("unverified").await,
    );
    ctx.befriend(&me, &friend).await;
    ctx.set_privacy(&hidden, |p| p.discoverable = false).await;
    sqlx::query("UPDATE users SET email_verified_at = NULL WHERE id = $1")
        .bind(unverified.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let match_contacts = |hashes: Vec<String>| {
        let (ctx, me) = (&ctx, me.clone());
        async move {
            ctx.friends::<_, MatchContactsResponse>(
                "MatchContacts",
                &me,
                &MatchContactsRequest { contact_hashes: hashes, ..Default::default() },
            )
            .await
        }
    };

    let known_hash = contact_hash(&email_of(&known));
    let hashes = vec![
        known_hash.to_uppercase(), // hex case does not matter
        contact_hash(&format!("  {}  ", email_of(&friend).to_uppercase())),
        contact_hash(&email_of(&hidden)),
        contact_hash(&email_of(&unverified)),
        contact_hash(&email_of(&me)),
        contact_hash("stranger@example.com"),
        contact_hash("+819012345678"),
    ];
    let matched = match_contacts(hashes).await.unwrap().matches;
    let mut found: Vec<(Uuid, Relationship)> =
        matched.iter().map(|m| (id_of(m.user.as_option().unwrap()), m.relationship.as_known().unwrap())).collect();
    found.sort_by_key(|(id, _)| *id);
    let mut expected =
        vec![(known.id, Relationship::RELATIONSHIP_NONE), (friend.id, Relationship::RELATIONSHIP_FRIENDS)];
    expected.sort_by_key(|(id, _)| *id);
    assert_eq!(found, expected, "only verified, discoverable accounts other than the caller match");
    assert_eq!(
        matched.iter().find(|m| id_of(m.user.as_option().unwrap()) == known.id).unwrap().contact_hash,
        known_hash
    );

    // A match becomes a suggestion (friends are filtered out as usual).
    assert_eq!(suggested(&suggestions_for(&ctx, &me, 0).await), vec![(known.id, "CONTACT")]);

    assert!(match_contacts(Vec::new()).await.unwrap().matches.is_empty());
    assert_eq!(match_contacts(vec!["not-a-hash".into()]).await.unwrap_err().reason, "INVALID_CONTACT_HASH");
    assert_eq!(match_contacts(vec![known_hash; 2001]).await.unwrap_err().reason, "TOO_MANY_CONTACTS");
}

async fn my_invite(ctx: &Ctx, user: &TestUser) -> Invite {
    let response: GetMyInviteResponse = ctx.friends("GetMyInvite", user, &GetMyInviteRequest::default()).await.unwrap();
    response.invite.into_option().unwrap()
}

async fn resolve(ctx: &Ctx, viewer: Option<&TestUser>, code: &str) -> Result<ResolveInviteResponse, RpcError> {
    ctx.app
        .call(
            &format!("{FRIENDS}/ResolveInvite"),
            viewer,
            &ResolveInviteRequest { code: code.to_owned(), ..Default::default() },
        )
        .await
}

/// The event identity emits when someone signs up with an invite code.
fn signed_up(user: &TestUser, invite_code: &str) -> Event {
    Event {
        id: Uuid::now_v7(),
        topic: "identity.user.created".into(),
        actor_id: Some(user.id),
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({ "user_id": user.id, "handle": user.handle, "invite_code": invite_code }),
    }
}

#[tokio::test]
async fn invite_codes_are_stable_resolvable_and_resettable() {
    let ctx = start().await;
    let (inviter, visitor) = (ctx.app.user("inviter").await, ctx.app.user("visitor").await);

    let invite = my_invite(&ctx, &inviter).await;
    assert_eq!(invite.url, format!("http://localhost:8080/invite/{}", invite.code));
    assert_eq!(invite.intro_count, 0);
    assert_eq!(my_invite(&ctx, &inviter).await.code, invite.code, "the code is created once");

    // Anyone can look a code up; signed-in visitors also learn their relationship.
    let anonymous = resolve(&ctx, None, &invite.code).await.unwrap();
    assert_eq!(id_of(anonymous.inviter.as_option().unwrap()), inviter.id);
    assert_eq!(anonymous.relationship, Relationship::RELATIONSHIP_UNSPECIFIED);
    assert_eq!(
        resolve(&ctx, Some(&visitor), &invite.code).await.unwrap().relationship,
        Relationship::RELATIONSHIP_NONE
    );
    assert_eq!(
        resolve(&ctx, Some(&inviter), &invite.code).await.unwrap().relationship,
        Relationship::RELATIONSHIP_SELF
    );
    assert!(resolve(&ctx, None, &invite.url).await.is_ok(), "a pasted link works like the code");
    for bad in ["", "nope", "zzzzzzzzzz", "' OR 1=1 --"] {
        let err = resolve(&ctx, None, bad).await.unwrap_err();
        assert_eq!((err.status, err.reason.as_str()), (404, "INVITE_NOT_FOUND"), "{bad:?}");
    }

    // A block hides the invite like everything else.
    let _: BlockUserResponse = ctx
        .friends("BlockUser", &inviter, &BlockUserRequest { user_id: visitor.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(resolve(&ctx, Some(&visitor), &invite.code).await.unwrap_err().reason, "INVITE_NOT_FOUND");

    let reset: ResetMyInviteResponse =
        ctx.friends("ResetMyInvite", &inviter, &ResetMyInviteRequest::default()).await.unwrap();
    let fresh = reset.invite.into_option().unwrap();
    assert_ne!(fresh.code, invite.code);
    assert_eq!(
        resolve(&ctx, None, &invite.code).await.unwrap_err().reason,
        "INVITE_NOT_FOUND",
        "the old link stops working"
    );
    assert!(resolve(&ctx, None, &fresh.code).await.is_ok());
    assert_eq!(my_invite(&ctx, &inviter).await.code, fresh.code);

    let err = ctx
        .app
        .call::<_, GetMyInviteResponse>(&format!("{FRIENDS}/GetMyInvite"), None, &GetMyInviteRequest::default())
        .await
        .unwrap_err();
    assert_eq!(err.reason, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn signing_up_through_an_invite_counts_as_an_intro() {
    let ctx = start().await;
    let (inviter, newcomer, another) =
        (ctx.app.user("inviter").await, ctx.app.user("newcomer").await, ctx.app.user("another").await);
    let invite = my_invite(&ctx, &inviter).await;

    // Sign-ups without a code, with an unknown code or with one's own code change nothing.
    ctx.social.handle_event(&signed_up(&another, "")).await.unwrap();
    ctx.social.handle_event(&signed_up(&another, "doesnotexist")).await.unwrap();
    ctx.social.handle_event(&signed_up(&inviter, &invite.code)).await.unwrap();
    assert_eq!(my_invite(&ctx, &inviter).await.intro_count, 0);

    let event = signed_up(&newcomer, &invite.code);
    ctx.social.handle_event(&event).await.unwrap();
    ctx.social.handle_event(&event).await.unwrap(); // redelivery
    assert_eq!(my_invite(&ctx, &inviter).await.intro_count, 1);
    let recorded = ctx.app.events_by("social.intro.created", newcomer.id).await;
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].data["inviter_id"], serde_json::json!(inviter.id));

    // The newcomer's friend request is waiting for the inviter, worded accordingly.
    let incoming: ListFriendRequestsResponse =
        ctx.friends("ListFriendRequests", &inviter, &ListFriendRequestsRequest::default()).await.unwrap();
    assert_eq!(incoming.requests.len(), 1);
    assert_eq!(id_of(incoming.requests[0].from.as_option().unwrap()), newcomer.id);
    assert_eq!(incoming.requests[0].source, FriendSource::FRIEND_SOURCE_INVITE_LINK);
    let notices = ctx.notifications_for(inviter.id, "social.friend_request").await;
    assert_eq!(notices.len(), 1);
    assert!(notices[0].data["body"]["ja"].as_str().unwrap().contains("招待"));

    // The intro shows on the inviter's profile and in their intro list.
    let profile: GetProfileResponse = ctx
        .app
        .call(
            "tokuly.identity.v1.ProfileService/GetProfile",
            None,
            &GetProfileRequest {
                by: Some(get_profile_request::By::UserId(inviter.id.to_string())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(profile.profile.as_option().unwrap().stats.as_option().unwrap().intro_count, 1);
    let intros: ListIntrosResponse = ctx.friends("ListIntros", &inviter, &ListIntrosRequest::default()).await.unwrap();
    assert_eq!((intros.total, intros.users.iter().map(id_of).collect::<Vec<_>>()), (1, vec![newcomer.id]));

    // If the request is withdrawn, the two stay suggested to each other.
    let _: CancelFriendRequestResponse = ctx
        .friends(
            "CancelFriendRequest",
            &newcomer,
            &CancelFriendRequestRequest { request_id: incoming.requests[0].id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(suggested(&suggestions_for(&ctx, &newcomer, 0).await), vec![(inviter.id, "INVITED_BY")]);
    assert_eq!(suggested(&suggestions_for(&ctx, &inviter, 0).await), vec![(newcomer.id, "INVITED")]);
}

#[tokio::test]
async fn intro_lists_follow_friend_list_privacy() {
    let ctx = start().await;
    let (inviter, friend, stranger) =
        (ctx.app.user("inviter").await, ctx.app.user("friend").await, ctx.app.user("stranger").await);
    ctx.befriend(&inviter, &friend).await;
    let invite = my_invite(&ctx, &inviter).await;
    let mut newcomers = Vec::new();
    for i in 0..3 {
        let newcomer = ctx.app.user(&format!("new{i}")).await;
        ctx.social.handle_event(&signed_up(&newcomer, &invite.code)).await.unwrap();
        newcomers.push(newcomer);
    }
    let intros_of_inviter = |viewer: &TestUser, page: PageRequest| {
        let (ctx, viewer, user_id) = (&ctx, viewer.clone(), inviter.id.to_string());
        async move {
            ctx.friends::<_, ListIntrosResponse>(
                "ListIntros",
                &viewer,
                &ListIntrosRequest { user_id, page: page.into(), ..Default::default() },
            )
            .await
        }
    };

    let first = intros_of_inviter(&friend, PageRequest { limit: 2, ..Default::default() }).await.unwrap();
    assert_eq!(first.total, 3);
    assert_eq!(
        first.users.iter().map(id_of).collect::<Vec<_>>(),
        vec![newcomers[2].id, newcomers[1].id],
        "newest first"
    );
    let cursor = first.page.as_option().unwrap().next_cursor.clone();
    let rest = intros_of_inviter(&friend, PageRequest { limit: 2, cursor, ..Default::default() }).await.unwrap();
    assert_eq!(rest.users.iter().map(id_of).collect::<Vec<_>>(), vec![newcomers[0].id]);

    let denied = intros_of_inviter(&stranger, PageRequest::default()).await.unwrap_err();
    assert_eq!(denied.reason, "FRIEND_LIST_PRIVATE");
    ctx.set_privacy(&inviter, |p| p.friend_list_visible_to = Audience::AUDIENCE_EVERYONE.into()).await;
    assert_eq!(intros_of_inviter(&stranger, PageRequest::default()).await.unwrap().total, 3);
}

const NEARBY: &str = "tokuly.social.v1.FriendService/DiscoverNearby";

fn nearby_at(spot: (f64, f64)) -> DiscoverNearbyRequest {
    DiscoverNearbyRequest { location: lat_lng(spot).into(), accuracy_meters: 5.0, ..Default::default() }
}

fn seen(snapshot: &DiscoverNearbyResponse) -> Vec<(Uuid, Relationship)> {
    snapshot.users.iter().map(|u| (id_of(u.user.as_option().unwrap()), u.relationship.as_known().unwrap())).collect()
}

#[tokio::test]
async fn nearby_stream_reports_arrivals_and_departures() {
    let ctx = start().await;
    let (alice, bob, blocked, far) = (
        ctx.app.user("alice").await,
        ctx.app.user("bob").await,
        ctx.app.user("blocked").await,
        ctx.app.user("far").await,
    );
    ctx.request(&alice, &bob).await;
    let _: BlockUserResponse = ctx
        .friends("BlockUser", &alice, &BlockUserRequest { user_id: blocked.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    let spot = somewhere();
    let app = &ctx.app;

    // Alice opens the add-friend screen first and stays; the others arrive
    // half a second later and leave after four seconds.
    let alice_request = nearby_at(spot);
    let alice_stream =
        app.stream::<_, DiscoverNearbyResponse>(NEARBY, Some(&alice), &alice_request, 3, Duration::from_secs(20));
    let visit = |user: &TestUser, at: (f64, f64)| {
        let user = user.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            app.stream::<_, DiscoverNearbyResponse>(NEARBY, Some(&user), &nearby_at(at), 2, Duration::from_secs(4))
                .await
                .unwrap()
        }
    };
    let (alice_saw, bob_saw, blocked_saw, far_saw) = tokio::join!(
        alice_stream,
        visit(&bob, north_of(spot, 20.0)),
        visit(&blocked, north_of(spot, 10.0)),
        visit(&far, north_of(spot, 400.0))
    );
    let alice_saw = alice_saw.unwrap();

    assert_eq!(alice_saw.len(), 3, "an empty start, Bob arriving, Bob leaving");
    assert!(alice_saw[0].users.is_empty());
    assert_eq!(
        seen(&alice_saw[1]),
        vec![(bob.id, Relationship::RELATIONSHIP_REQUEST_SENT)],
        "the blocked and the far-away user never show up"
    );
    let distance = alice_saw[1].users[0].distance_meters;
    assert!((18.0..=22.0).contains(&distance), "{distance}");
    assert!(alice_saw[2].users.is_empty());

    // Bob sees Alice from his side of the pending request (and possibly the
    // third visitor, who is a stranger to him).
    let bob_last = seen(bob_saw.last().expect("Bob got a snapshot"));
    assert!(bob_last.contains(&(alice.id, Relationship::RELATIONSHIP_REQUEST_RECEIVED)), "{bob_last:?}");
    assert!(bob_last.iter().all(|(id, _)| *id != far.id));
    assert!(
        blocked_saw.iter().all(|snapshot| seen(snapshot).iter().all(|(id, _)| *id != alice.id)),
        "blocks hide in both directions"
    );
    assert_eq!(far_saw.len(), 1);
    assert!(far_saw[0].users.is_empty());
}

#[tokio::test]
async fn nearby_stream_rejects_bad_input_and_opted_out_users() {
    let ctx = start().await;
    let (user, hermit) = (ctx.app.user("user").await, ctx.app.user("hermit").await);
    let open = |user: Option<&TestUser>, request: DiscoverNearbyRequest| {
        let (app, user) = (&ctx.app, user.cloned());
        async move {
            app.stream::<_, DiscoverNearbyResponse>(NEARBY, user.as_ref(), &request, 1, Duration::from_secs(3)).await
        }
    };

    assert_eq!(open(Some(&user), DiscoverNearbyRequest::default()).await.unwrap_err().reason, "INVALID_LOCATION");
    assert_eq!(open(Some(&user), nearby_at((91.0, 0.0))).await.unwrap_err().reason, "INVALID_LOCATION");
    assert_eq!(open(None, nearby_at(somewhere())).await.unwrap_err().reason, "SIGN_IN_REQUIRED");

    ctx.set_privacy(&hermit, |p| p.nearby_discoverable = false).await;
    assert_eq!(open(Some(&hermit), nearby_at(somewhere())).await.unwrap_err().reason, "NEARBY_DISCOVERY_DISABLED");
}
