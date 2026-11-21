//! The activity log: ingestion, the caller's own log, profile lists, the
//! friends' feed and per-kind visibility.
mod common;

use chrono::{Duration, Utc};
use common::*;
use tokuly_core::outbound::{ActivityAudience, ActivityEntry, Localized};
use tokuly_feed::IngestOutcome;
use tokuly_proto::{common::SubjectType, feed::*};
use tokuly_testkit::TestUser;
use uuid::Uuid;

#[tokio::test]
async fn an_event_is_stored_once_and_a_deleted_entry_stays_deleted() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let posted = note(&alice);
    let event = activity_event(&posted, Utc::now());

    let IngestOutcome::Stored(id) = ctx.feed.handle_activity_recorded(&event).await.unwrap() else { panic!("the first delivery stores the entry") };
    assert_eq!(ctx.feed.handle_activity_recorded(&event).await.unwrap(), IngestOutcome::Duplicate, "a redelivery changes nothing");
    assert_eq!(ids(&ctx.my_log(&alice).await), vec![id.to_string()]);

    // Once the owner deleted the entry, a late redelivery does not bring it back.
    let _: DeleteActivityResponse = ctx.call(ACTIVITY, "DeleteActivity", Some(&alice), &DeleteActivityRequest { activity_id: id.to_string(), ..Default::default() }).await.unwrap();
    assert_eq!(ctx.feed.handle_activity_recorded(&event).await.unwrap(), IngestOutcome::Duplicate);
    assert!(ctx.my_log(&alice).await.is_empty());
}

#[tokio::test]
async fn unusable_entries_are_dropped_without_failing_the_consumer() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;

    let mut malformed = note(&alice);
    malformed.kind = "Not A Kind".to_owned();
    assert_eq!(ctx.deliver(&malformed, Utc::now()).await, IngestOutcome::Dropped);

    let mut nobody = note(&alice);
    nobody.user_id = Uuid::now_v7();
    assert_eq!(ctx.deliver(&nobody, Utc::now()).await, IngestOutcome::Dropped, "an entry of an unknown account is skipped");

    // The consumer entry point swallows both and keeps going.
    ctx.feed.handle_event(activity_event(&malformed, Utc::now())).await.unwrap();
    ctx.feed.handle_event(event("some.other.topic", None, serde_json::json!({}))).await.unwrap();
    assert!(ctx.my_log(&alice).await.is_empty());
}

#[tokio::test]
async fn joining_tokuly_is_the_first_log_entry() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let created = event("identity.user.created", Some(alice.id), serde_json::json!({ "user_id": alice.id, "handle": alice.handle, "invite_code": "" }));
    ctx.feed.handle_event(created.clone()).await.unwrap();
    ctx.feed.handle_event(created).await.unwrap();

    let log = ctx.my_log(&alice).await;
    assert_eq!(log.len(), 1, "one entry however often the event arrives");
    let joined = &log[0];
    assert_eq!(joined.kind, "account.joined");
    assert_eq!(joined.summary, "Tokulyに参加しました！");
    assert_eq!(joined.kind_label, "Tokulyに参加");
    assert_eq!(joined.audience, tokuly_proto::feed::ActivityAudience::ACTIVITY_AUDIENCE_PUBLIC);
    assert_eq!(joined.path, format!("/u/{}", alice.handle));
    assert_eq!(joined.actor.as_option().unwrap().id, alice.id.to_string());

    // It is public: the profile shows it to anyone, signed in or not.
    assert_eq!(kinds_of(&ctx.profile(None, alice.id).await.unwrap()), vec!["account.joined"]);
}

#[tokio::test]
async fn my_log_holds_everything_and_filters_by_kind_and_time() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let now = Utc::now();
    let private = ctx.record_at(entry(&alice, "geo.visit", ActivityAudience::Private).at(35.0, 139.0), now - Duration::days(3)).await;
    let bumped = ctx.record_at(bump(&alice, &bob, 35.1, 139.1), now - Duration::days(2)).await;
    let noted = ctx.record_at(note(&alice), now - Duration::days(1)).await;
    let region = ctx.record_at(entry(&alice, "geo.new_region", ActivityAudience::Public).data(serde_json::json!({ "region_name": "大阪府" })), now - Duration::hours(1)).await;
    // Somebody else's entry never shows up in my log.
    ctx.record(note(&bob), 5).await;

    let log = ctx.my_log(&alice).await;
    assert_eq!(ids(&log), [region, noted, bumped, private].map(|id| id.to_string()), "everything, newest first");

    // The structured fields come with the summary.
    let bump_entry = &log[2];
    assert_eq!(bump_entry.subject.as_option().unwrap().id, bob.id.to_string());
    assert_eq!(bump_entry.subject.as_option().unwrap().r#type, SubjectType::SUBJECT_TYPE_USER);
    assert_eq!(bump_entry.with_users.iter().map(|u| u.id.clone()).collect::<Vec<_>>(), vec![bob.id.to_string()]);
    assert_eq!(bump_entry.location.as_option().map(|l| (l.lat, l.lng)), Some((35.1, 139.1)));
    assert!(bump_entry.data_json.contains("bump_id"));
    assert_eq!(bump_entry.path, format!("/u/{}", bob.handle));
    assert_eq!(log[0].data_json, r#"{"region_name":"大阪府"}"#);
    assert!(log.iter().all(|activity| !activity.hidden_from_others));

    let list = |request: ListMyActivityRequest| {
        let (ctx, alice) = (&ctx, &alice);
        async move { ctx.call::<_, ListMyActivityResponse>(ACTIVITY, "ListMyActivity", Some(alice), &request).await }
    };
    // A kind prefix matches whole segments.
    let geo = list(ListMyActivityRequest { kind_prefix: "geo".into(), ..Default::default() }).await.unwrap();
    assert_eq!(ids(&geo.activities), [region, private].map(|id| id.to_string()));
    let exact = list(ListMyActivityRequest { kind_prefix: "geo.visit".into(), ..Default::default() }).await.unwrap();
    assert_eq!(ids(&exact.activities), vec![private.to_string()]);
    assert!(list(ListMyActivityRequest { kind_prefix: "ge".into(), ..Default::default() }).await.unwrap().activities.is_empty());

    // `from` is inclusive, `to` exclusive.
    let window = list(ListMyActivityRequest {
        from: tokuly_core::time::ts(now - Duration::days(2)),
        to: tokuly_core::time::ts(now - Duration::hours(1)),
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(ids(&window.activities), [noted, bumped].map(|id| id.to_string()));

    // Pages of two.
    let first = list(ListMyActivityRequest { page: page(2, "").into(), ..Default::default() }).await.unwrap();
    assert_eq!(ids(&first.activities), [region, noted].map(|id| id.to_string()));
    let info = first.page.as_option().unwrap();
    assert!(info.has_more);
    let second = list(ListMyActivityRequest { page: page(2, &info.next_cursor).into(), ..Default::default() }).await.unwrap();
    assert_eq!(ids(&second.activities), [bumped, private].map(|id| id.to_string()));
    assert!(!second.page.as_option().unwrap().has_more);

    // Validation.
    assert_eq!(list(ListMyActivityRequest { kind_prefix: "Geo!".into(), ..Default::default() }).await.unwrap_err().reason, "INVALID_KIND");
    let backwards = ListMyActivityRequest { from: tokuly_core::time::ts(now), to: tokuly_core::time::ts(now - Duration::days(1)), ..Default::default() };
    assert_eq!(list(backwards).await.unwrap_err().reason, "INVALID_RANGE");
    assert_eq!(list(ListMyActivityRequest { page: page(2, "garbage").into(), ..Default::default() }).await.unwrap_err().reason, "INVALID_CURSOR");
    let anonymous = ctx.call::<_, ListMyActivityResponse>(ACTIVITY, "ListMyActivity", None, &ListMyActivityRequest::default()).await.unwrap_err();
    assert_eq!((anonymous.code.as_str(), anonymous.reason.as_str()), ("unauthenticated", "SIGN_IN_REQUIRED"));
}

#[tokio::test]
async fn summaries_and_labels_follow_the_readers_language() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    let posted = ActivityEntry::new(alice.id, "social.bumped", ActivityAudience::Public, Localized::new("ボブさんとバンプしました！", "Bumped with Bob!"));
    ctx.record(posted, 1).await;

    let japanese = &ctx.my_log(&alice).await[0];
    assert_eq!((japanese.summary.as_str(), japanese.kind_label.as_str()), ("ボブさんとバンプしました！", "バンプ"));

    // Bob reads in English: the same entry, his language.
    ctx.set_locale(&bob, "en").await;
    let english = &ctx.friends_feed(&bob).await[0];
    let activity = english.activity.as_option().unwrap();
    assert_eq!((activity.summary.as_str(), activity.kind_label.as_str()), ("Bumped with Bob!", "Bumps"));
    assert_eq!(activity.kind, "social.bumped", "the kind stays machine-readable for client-side templates");

    // Anonymous readers get the language of the request (the test client asks for Japanese).
    assert_eq!(ctx.profile(None, alice.id).await.unwrap()[0].summary, "ボブさんとバンプしました！");
}

#[tokio::test]
async fn entries_can_be_deleted_one_by_one_or_by_range() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let now = Utc::now();
    let old_note = ctx.record_at(note(&alice), now - Duration::days(10)).await;
    let old_visit = ctx.record_at(entry(&alice, "geo.visit", ActivityAudience::Private), now - Duration::days(9)).await;
    let mid_note = ctx.record_at(note(&alice), now - Duration::days(5)).await;
    let new_note = ctx.record_at(note(&alice), now - Duration::days(1)).await;
    let bobs = ctx.record(note(&bob), 5).await;

    let delete = |user: &TestUser, id: String| {
        let (ctx, user) = (&ctx, user.clone());
        async move { ctx.call::<_, DeleteActivityResponse>(ACTIVITY, "DeleteActivity", Some(&user), &DeleteActivityRequest { activity_id: id, ..Default::default() }).await }
    };
    // Someone else's entry is "not found", and stays.
    assert_eq!(delete(&alice, bobs.to_string()).await.unwrap_err().reason, "ACTIVITY_NOT_FOUND");
    assert_eq!(ctx.my_log(&bob).await.len(), 1);
    assert_eq!(delete(&alice, Uuid::now_v7().to_string()).await.unwrap_err().reason, "ACTIVITY_NOT_FOUND");
    assert_eq!(delete(&alice, "nope".into()).await.unwrap_err().reason, "INVALID_ID");
    delete(&alice, mid_note.to_string()).await.unwrap();
    assert_eq!(delete(&alice, mid_note.to_string()).await.unwrap_err().reason, "ACTIVITY_NOT_FOUND", "already gone");

    let clear = |request: ClearActivityRequest| {
        let (ctx, alice) = (&ctx, &alice);
        async move { ctx.call::<_, ClearActivityResponse>(ACTIVITY, "ClearActivity", Some(alice), &request).await }
    };
    // Clearing everything by accident is not possible.
    assert_eq!(clear(ClearActivityRequest::default()).await.unwrap_err().reason, "RANGE_REQUIRED");
    let backwards = ClearActivityRequest { from: tokuly_core::time::ts(now), to: tokuly_core::time::ts(now - Duration::days(1)), ..Default::default() };
    assert_eq!(clear(backwards).await.unwrap_err().reason, "INVALID_RANGE");
    assert_eq!(clear(ClearActivityRequest { kind_prefix: "!".into(), everything: true, ..Default::default() }).await.unwrap_err().reason, "INVALID_KIND");

    // Only notes older than a week.
    let cleared = clear(ClearActivityRequest { to: tokuly_core::time::ts(now - Duration::days(7)), kind_prefix: "note".into(), ..Default::default() }).await.unwrap();
    assert_eq!(cleared.deleted_count, 1);
    assert_eq!(ids(&ctx.my_log(&alice).await), [new_note, old_visit].map(|id| id.to_string()));
    let _ = old_note;

    // The whole log, confirmed.
    let cleared = clear(ClearActivityRequest { everything: true, ..Default::default() }).await.unwrap();
    assert_eq!(cleared.deleted_count, 2);
    assert!(ctx.my_log(&alice).await.is_empty());
    assert_eq!(ctx.my_log(&bob).await.len(), 1, "other people's logs are untouched");
}

#[tokio::test]
async fn a_profile_shows_each_viewer_what_the_audience_allows() {
    let ctx = start().await;
    let (owner, friend, stranger) = (ctx.app.user("owner").await, ctx.app.user("friend").await, ctx.app.user("stranger").await);
    ctx.befriend(&owner, &friend).await;
    let private = ctx.record(entry(&owner, "geo.visit", ActivityAudience::Private), 30).await;
    let friends_only = ctx.record(note(&owner), 20).await;
    let public = ctx.record(entry(&owner, "video.published", ActivityAudience::Public), 10).await;

    let owner_id = owner.id;
    let seen_by = |viewer: Option<&TestUser>| {
        let (ctx, viewer) = (&ctx, viewer.cloned());
        async move { ctx.profile(viewer.as_ref(), owner_id).await.map(|activities| ids(&activities)) }
    };
    assert_eq!(seen_by(Some(&owner)).await.unwrap(), [public, friends_only, private].map(|id| id.to_string()), "the owner sees everything");
    assert_eq!(seen_by(Some(&friend)).await.unwrap(), [public, friends_only].map(|id| id.to_string()));
    assert_eq!(seen_by(Some(&stranger)).await.unwrap(), vec![public.to_string()]);
    assert_eq!(seen_by(None).await.unwrap(), vec![public.to_string()]);

    // Kind filter and paging work for visitors too.
    let videos: ListUserActivityResponse = ctx
        .call(ACTIVITY, "ListUserActivity", Some(&friend), &ListUserActivityRequest { user_id: owner.id.to_string(), kind_prefix: "video".into(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(ids(&videos.activities), vec![public.to_string()]);
    let first: ListUserActivityResponse =
        ctx.call(ACTIVITY, "ListUserActivity", Some(&friend), &ListUserActivityRequest { user_id: owner.id.to_string(), page: page(1, "").into(), ..Default::default() }).await.unwrap();
    assert_eq!(ids(&first.activities), vec![public.to_string()]);
    let cursor = first.page.as_option().unwrap().next_cursor.clone();
    let second: ListUserActivityResponse =
        ctx.call(ACTIVITY, "ListUserActivity", Some(&friend), &ListUserActivityRequest { user_id: owner.id.to_string(), page: page(1, &cursor).into(), ..Default::default() }).await.unwrap();
    assert_eq!(ids(&second.activities), vec![friends_only.to_string()]);
    assert!(!second.page.as_option().unwrap().has_more);

    // Unknown users, bad IDs, and anyone behind a block.
    assert_eq!(ctx.profile(Some(&friend), Uuid::now_v7()).await.unwrap_err().reason, "USER_NOT_FOUND");
    let bad_id = ctx.call::<_, ListUserActivityResponse>(ACTIVITY, "ListUserActivity", None, &ListUserActivityRequest { user_id: "nope".into(), ..Default::default() }).await.unwrap_err();
    assert_eq!(bad_id.reason, "INVALID_ID");
    ctx.block(&owner, &stranger).await;
    assert_eq!(seen_by(Some(&stranger)).await.unwrap_err().reason, "USER_NOT_FOUND", "the blocked person cannot look");
    assert_eq!(ctx.profile(Some(&owner), stranger.id).await.unwrap_err().reason, "USER_NOT_FOUND", "and neither can the blocker");

    // A banned or deleted owner has no profile list.
    sqlx::query("UPDATE users SET banned_at = now() WHERE id = $1").bind(owner.id).execute(&ctx.app.core.db).await.unwrap();
    assert_eq!(seen_by(Some(&friend)).await.unwrap_err().reason, "USER_NOT_FOUND");
    assert_eq!(seen_by(None).await.unwrap_err().reason, "USER_NOT_FOUND");
}

#[tokio::test]
async fn entries_that_name_someone_blocked_do_not_reach_the_blocker() {
    let ctx = start().await;
    let (alice, bob, mallory) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("mallory").await);
    ctx.befriend(&alice, &bob).await;
    ctx.befriend(&bob, &mallory).await;
    // Bob bumped Mallory (publicly) and posted a note.
    let mut bumped = bump(&bob, &mallory, 35.0, 139.0);
    bumped.audience = ActivityAudience::Public;
    let bumped = ctx.record(bumped, 20).await;
    let noted = ctx.record(note(&bob), 10).await;

    assert_eq!(ids(&ctx.profile(Some(&alice), bob.id).await.unwrap()), [noted, bumped].map(|id| id.to_string()));
    assert_eq!(ctx.friends_feed(&alice).await.len(), 2);

    // Alice blocks Mallory: Bob's entry carrying Mallory's name is gone for
    // Alice, on his profile and in her feed. Bob himself still has it.
    ctx.block(&alice, &mallory).await;
    assert_eq!(ids(&ctx.profile(Some(&alice), bob.id).await.unwrap()), vec![noted.to_string()]);
    let feed = ctx.friends_feed(&alice).await;
    assert_eq!(feed.iter().map(head_of).collect::<Vec<_>>(), vec![noted.to_string()]);
    assert_eq!(ctx.my_log(&bob).await.len(), 2);
    // It works in the other direction too: Mallory does not see entries naming Alice.
    let about_alice = ctx.record(bump(&bob, &alice, 35.0, 139.0), 5).await;
    assert!(!ctx.friends_feed(&mallory).await.iter().any(|card| head_of(card) == about_alice.to_string()));
}

#[tokio::test]
async fn the_friends_feed_shows_friends_only_and_collapses_bursts() {
    let ctx = start().await;
    let (me, anna, ben, stranger) = (ctx.app.user("me").await, ctx.app.user("anna").await, ctx.app.user("ben").await, ctx.app.user("stranger").await);
    ctx.befriend(&me, &anna).await;
    ctx.befriend(&ben, &me).await;

    // Anna: one note long ago, then three within the hour (a burst).
    let anna_old = ctx.record(note(&anna), 300).await;
    let anna_burst_1 = ctx.record(note(&anna), 100).await;
    let anna_burst_2 = ctx.record(note(&anna), 70).await;
    let anna_burst_3 = ctx.record(note(&anna), 30).await;
    // In between she did something of another kind: not part of the burst.
    let anna_video = ctx.record(entry(&anna, "video.published", ActivityAudience::Public), 50).await;
    // Ben bumped; his private entry and a stranger's public one are not for me.
    let ben_bump = ctx.record(bump(&ben, &anna, 35.0, 139.0), 10).await;
    ctx.record(entry(&ben, "geo.visit", ActivityAudience::Private), 5).await;
    ctx.record(entry(&stranger, "video.published", ActivityAudience::Public), 1).await;
    // My own entries are not in my friends' feed.
    ctx.record(note(&me), 2).await;

    let feed = ctx.friends_feed(&me).await;
    assert_eq!(feed.iter().map(head_of).collect::<Vec<_>>(), [ben_bump, anna_burst_3, anna_video, anna_old].map(|id| id.to_string()), "newest first, one card per burst");
    let burst = &feed[1];
    assert_eq!(burst.count, 3);
    assert_eq!(ids(&burst.more), [anna_burst_2, anna_burst_1].map(|id| id.to_string()), "the older entries of the burst, newest first");
    assert_eq!(burst.r#type, ActivityCardType::ACTIVITY_CARD_TYPE_NOTE);
    assert_eq!(feed[0].r#type, ActivityCardType::ACTIVITY_CARD_TYPE_BUMP);
    assert_eq!((feed[0].count, feed[0].more.len()), (1, 0));
    assert_eq!((feed[3].count, feed[3].more.len()), (1, 0), "an entry hours before the burst stands alone");
    assert_eq!(actor_of(&feed[0]), ben.id);

    // Kind filter.
    let notes: ListFriendsActivityResponse = ctx.call(ACTIVITY, "ListFriendsActivity", Some(&me), &ListFriendsActivityRequest { kind_prefix: "note".into(), ..Default::default() }).await.unwrap();
    assert_eq!(notes.cards.iter().map(head_of).collect::<Vec<_>>(), [anna_burst_3, anna_old].map(|id| id.to_string()));

    // Pages are pages of cards: nothing is repeated or skipped.
    let first: ListFriendsActivityResponse = ctx.call(ACTIVITY, "ListFriendsActivity", Some(&me), &ListFriendsActivityRequest { page: page(2, "").into(), ..Default::default() }).await.unwrap();
    assert_eq!(first.cards.iter().map(head_of).collect::<Vec<_>>(), [ben_bump, anna_burst_3].map(|id| id.to_string()));
    assert_eq!(first.cards[1].count, 3);
    let cursor = first.page.as_option().unwrap().next_cursor.clone();
    let second: ListFriendsActivityResponse = ctx.call(ACTIVITY, "ListFriendsActivity", Some(&me), &ListFriendsActivityRequest { page: page(2, &cursor).into(), ..Default::default() }).await.unwrap();
    assert_eq!(second.cards.iter().map(head_of).collect::<Vec<_>>(), [anna_video, anna_old].map(|id| id.to_string()));
    assert!(!second.page.as_option().unwrap().has_more);

    // Validation and authentication.
    let bad = ctx.call::<_, ListFriendsActivityResponse>(ACTIVITY, "ListFriendsActivity", Some(&me), &ListFriendsActivityRequest { kind_prefix: "No!".into(), ..Default::default() }).await;
    assert_eq!(bad.unwrap_err().reason, "INVALID_KIND");
    let bad = ctx.call::<_, ListFriendsActivityResponse>(ACTIVITY, "ListFriendsActivity", Some(&me), &ListFriendsActivityRequest { page: page(2, "garbage").into(), ..Default::default() }).await;
    assert_eq!(bad.unwrap_err().reason, "INVALID_CURSOR");
    let anonymous = ctx.call::<_, ListFriendsActivityResponse>(ACTIVITY, "ListFriendsActivity", None, &ListFriendsActivityRequest::default()).await;
    assert_eq!(anonymous.unwrap_err().code, "unauthenticated");

    // Someone without friends has an empty feed.
    assert!(ctx.friends_feed(&stranger).await.is_empty());
}

#[tokio::test]
async fn the_friends_feed_follows_the_graph_without_any_fan_out() {
    let ctx = start().await;
    let (me, anna, ben) = (ctx.app.user("me").await, ctx.app.user("anna").await, ctx.app.user("ben").await);
    ctx.befriend(&me, &anna).await;
    let anna_note = ctx.record(note(&anna), 20).await;
    let ben_note = ctx.record(note(&ben), 10).await;
    assert_eq!(ctx.friends_feed(&me).await.iter().map(head_of).collect::<Vec<_>>(), vec![anna_note.to_string()]);

    // A new friend's earlier entries appear; a former friend's disappear.
    ctx.befriend(&me, &ben).await;
    assert_eq!(ctx.friends_feed(&me).await.iter().map(head_of).collect::<Vec<_>>(), [ben_note, anna_note].map(|id| id.to_string()));
    ctx.unfriend(&me, &anna).await;
    assert_eq!(ctx.friends_feed(&me).await.iter().map(head_of).collect::<Vec<_>>(), vec![ben_note.to_string()]);

    // A banned friend's entries are not shown either.
    sqlx::query("UPDATE users SET banned_at = now() WHERE id = $1").bind(ben.id).execute(&ctx.app.core.db).await.unwrap();
    assert!(ctx.friends_feed(&me).await.is_empty());
}

#[tokio::test]
async fn hidden_kinds_disappear_for_everyone_but_the_owner() {
    let ctx = start().await;
    let (owner, friend) = (ctx.app.user("owner").await, ctx.app.user("friend").await);
    ctx.befriend(&owner, &friend).await;
    let region = ctx.record(entry(&owner, "geo.new_region", ActivityAudience::Public).at(34.7, 135.5), 30).await;
    let visit = ctx.record(entry(&owner, "geo.visit", ActivityAudience::Friends).at(34.7, 135.5), 20).await;
    let noted = ctx.record(note(&owner), 10).await;

    let set = |kind: &str, visible: bool| {
        let request = SetActivityKindVisibilityRequest { kind: kind.to_owned(), visible_to_others: visible, ..Default::default() };
        let (ctx, owner) = (&ctx, &owner);
        async move { ctx.call::<_, SetActivityKindVisibilityResponse>(ACTIVITY, "SetActivityKindVisibility", Some(owner), &request).await }
    };
    let preference = |kinds: &[ActivityKindPreference], kind: &str| kinds.iter().find(|p| p.kind == kind).cloned().unwrap_or_else(|| panic!("{kind} is listed"));

    // Before: everything visible, counts per kind, well-known kinds listed.
    let before: GetActivityPreferencesResponse = ctx.call(ACTIVITY, "GetActivityPreferences", Some(&owner), &GetActivityPreferencesRequest::default()).await.unwrap();
    assert!(before.kinds.iter().all(|p| p.visible_to_others));
    assert_eq!(preference(&before.kinds, "geo").entry_count, 2);
    assert_eq!(preference(&before.kinds, "geo.new_region").entry_count, 1);
    assert_eq!((preference(&before.kinds, "note.posted").entry_count, preference(&before.kinds, "note.posted").label.as_str()), (1, "ノート"));
    assert_eq!(preference(&before.kinds, "social.bumped").entry_count, 0, "well-known kinds are listed without entries");

    // Hide the whole map family.
    let after = set("geo", false).await.unwrap();
    assert!(!preference(&after.kinds, "geo").visible_to_others);
    assert!(!preference(&after.kinds, "geo.visit").visible_to_others, "covered by the family");
    assert!(preference(&after.kinds, "note.posted").visible_to_others);

    // Friends and strangers no longer see those entries anywhere; nothing was deleted.
    assert_eq!(ids(&ctx.profile(Some(&friend), owner.id).await.unwrap()), vec![noted.to_string()]);
    assert!(ctx.profile(None, owner.id).await.unwrap().is_empty(), "the public region entry is hidden too");
    assert_eq!(ctx.friends_feed(&friend).await.iter().map(head_of).collect::<Vec<_>>(), vec![noted.to_string()]);
    let own = ctx.my_log(&owner).await;
    assert_eq!(ids(&own), [noted, visit, region].map(|id| id.to_string()));
    assert_eq!(own.iter().map(|a| a.hidden_from_others).collect::<Vec<_>>(), vec![false, true, true], "the owner sees which entries are hidden");
    let own_profile = ctx.profile(Some(&owner), owner.id).await.unwrap();
    assert_eq!(own_profile.iter().filter(|a| a.hidden_from_others).count(), 2);

    // A kind covered by a hidden family cannot be shown on its own.
    let error = set("geo.new_region", true).await.unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("failed_precondition", "KIND_HIDDEN_BY_PREFIX"));
    // Hiding it again is a no-op; showing the family brings everything back.
    set("geo.visit", false).await.unwrap();
    let shown = set("geo", true).await.unwrap();
    assert!(shown.kinds.iter().all(|p| p.visible_to_others));
    assert_eq!(ids(&ctx.profile(Some(&friend), owner.id).await.unwrap()), [noted, visit, region].map(|id| id.to_string()));

    // Hiding a family replaces narrower entries below it.
    set("geo.visit", false).await.unwrap();
    set("geo", false).await.unwrap();
    let stored: Vec<String> = sqlx::query_scalar("SELECT kind FROM activity_hidden_kinds WHERE user_id = $1").bind(owner.id).fetch_all(&ctx.app.core.db).await.unwrap();
    assert_eq!(stored, vec!["geo"]);

    // Validation.
    assert_eq!(set("", false).await.unwrap_err().reason, "INVALID_KIND");
    assert_eq!(set("Geo Stuff", false).await.unwrap_err().reason, "INVALID_KIND");
    let anonymous = ctx.call::<_, GetActivityPreferencesResponse>(ACTIVITY, "GetActivityPreferences", None, &GetActivityPreferencesRequest::default()).await;
    assert_eq!(anonymous.unwrap_err().code, "unauthenticated");
}

#[tokio::test]
async fn only_so_many_kinds_can_be_hidden() {
    let ctx = start().await;
    let owner = ctx.app.user("owner").await;
    // Fill the list directly (the RPC is rate limited per minute).
    sqlx::query("INSERT INTO activity_hidden_kinds (user_id, kind) SELECT $1, 'custom.kind' || n FROM generate_series(1, 50) AS n").bind(owner.id).execute(&ctx.app.core.db).await.unwrap();
    let hide = |kind: &str| {
        let request = SetActivityKindVisibilityRequest { kind: kind.to_owned(), visible_to_others: false, ..Default::default() };
        let (ctx, owner) = (&ctx, &owner);
        async move { ctx.call::<_, SetActivityKindVisibilityResponse>(ACTIVITY, "SetActivityKindVisibility", Some(owner), &request).await }
    };
    assert_eq!(hide("one.more").await.unwrap_err().reason, "TOO_MANY_HIDDEN_KINDS");
    // One that is already covered is fine, and a family that replaces many makes room.
    hide("custom.kind7").await.unwrap();
    hide("custom").await.unwrap();
    hide("one.more").await.unwrap();
    let stored: Vec<String> = sqlx::query_scalar("SELECT kind FROM activity_hidden_kinds WHERE user_id = $1 ORDER BY kind").bind(owner.id).fetch_all(&ctx.app.core.db).await.unwrap();
    assert_eq!(stored, vec!["custom", "one.more"]);
}
