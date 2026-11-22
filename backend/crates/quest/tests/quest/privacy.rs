//! What a player (or the domain an event comes from) keeps hidden must not
//! show up in XP, counters, badges or streaks — all of which others can see.
use serde_json::json;
use tokuly_core::{events::Event, prelude::*};
use tokuly_testkit::TestUser;

use crate::common::*;

async fn check_in(ctx: &Ctx, user: &TestUser, audience: &str, with: &[&TestUser]) {
    let data = json!({
        "user_id": user.id, "kind": "geo.checkin", "audience": audience,
        "with_user_ids": with.iter().map(|friend| friend.id).collect::<Vec<_>>(),
        "summary": { "ja": "チェックイン!", "en": "Checked in" }, "path": "/map",
    });
    ctx.feed(event("activity.recorded", Some(user.id), data)).await;
}

#[tokio::test]
async fn what_is_recorded_as_private_earns_nothing() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;

    // In ghost mode a check-in stays between Alice and the friend she tagged.
    check_in(&ctx, &alice, "private", &[&bob]).await;
    ctx.flush(&[&alice]).await;
    assert_eq!(ctx.progress(&alice).await.level.as_option().unwrap().total_xp, 0);
    assert_eq!(ctx.counter(&alice, "check_ins").await, 0);
    assert_eq!(ctx.counter(&alice, "friends_reached").await, 0);
    let parts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM quest_streaks WHERE user_lo IN ($1, $2) AND user_hi IN ($1, $2)")
            .bind(alice.id)
            .bind(bob.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(
        parts, 0,
        "no streak part either: it would show Bob's other friends nothing, but Alice's leaderboard XP would"
    );

    // The same check-in shared with her friends counts.
    check_in(&ctx, &alice, "friends", &[&bob]).await;
    ctx.flush(&[&alice]).await;
    assert_eq!(ctx.source_today(&alice, "map").await, (10, false));
    assert_eq!(ctx.counter(&alice, "check_ins").await, 1);
    assert_eq!(ctx.counter(&alice, "friends_reached").await, 1);
}

#[tokio::test]
async fn what_hides_who_did_it_or_who_may_see_it_earns_nothing() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;

    // A comment on Bob's video written as Alice's channel: Bob must not be able to tell whose channel it is.
    let as_channel = json!({ "comment_id": Uuid::now_v7(), "subject_type": 5, "subject_id": Uuid::now_v7(), "subject_owner_id": bob.id, "author_id": alice.id, "channel_id": Uuid::now_v7(), "parent_id": null });
    ctx.feed(event("engagement.comment.published", Some(alice.id), as_channel)).await;
    // A story for chosen close friends, and an answer only the asker may read.
    ctx.feed(event("notes.story.created", Some(alice.id), json!({ "story_id": Uuid::now_v7(), "owner_id": alice.id, "audience": 2, "expires_at": "2030-01-01T00:00:00Z" }))).await;
    ctx.feed(event(
        "ask.answer.published",
        Some(alice.id),
        json!({ "question_id": Uuid::now_v7(), "owner_id": alice.id, "audience": 3, "first": true }),
    ))
    .await;
    // A note to herself.
    ctx.feed(event(
        "notes.note.created",
        Some(alice.id),
        json!({ "note_id": Uuid::now_v7(), "author_id": alice.id, "visibility": 5, "reply_to_id": null }),
    ))
    .await;
    ctx.flush(&[&alice]).await;

    assert_eq!(ctx.progress(&alice).await.level.as_option().unwrap().total_xp, 0);
    for metric in ["comments", "stories_posted", "questions_answered", "notes_posted", "friends_reached"] {
        assert_eq!(ctx.counter(&alice, metric).await, 0, "{metric} moved");
    }
    assert_eq!(ctx.streak(&bob, &alice).await.waiting_on, tokuly_proto::quest::StreakTurn::STREAK_TURN_UNSPECIFIED);
    let row: Option<(Vec<chrono::NaiveDate>, Vec<chrono::NaiveDate>)> =
        sqlx::query_as("SELECT lo_days, hi_days FROM quest_streaks WHERE user_lo IN ($1, $2) AND user_hi IN ($1, $2)")
            .bind(alice.id)
            .bind(bob.id)
            .fetch_optional(&ctx.app.core.db)
            .await
            .unwrap();
    assert!(row.is_none(), "the channel comment left a trace in the streak with Bob");

    // The same things done in the open count.
    let own_name = json!({ "comment_id": Uuid::now_v7(), "subject_type": 5, "subject_id": Uuid::now_v7(), "subject_owner_id": bob.id, "author_id": alice.id, "channel_id": null, "parent_id": null });
    ctx.feed(event("engagement.comment.published", Some(alice.id), own_name)).await;
    ctx.feed(event(
        "notes.story.created",
        Some(alice.id),
        json!({ "story_id": Uuid::now_v7(), "owner_id": alice.id, "audience": 1 }),
    ))
    .await;
    ctx.feed(event(
        "ask.answer.published",
        Some(alice.id),
        json!({ "question_id": Uuid::now_v7(), "owner_id": alice.id, "audience": 2, "first": true }),
    ))
    .await;
    ctx.flush(&[&alice]).await;
    assert_eq!(
        (
            ctx.counter(&alice, "comments").await,
            ctx.counter(&alice, "stories_posted").await,
            ctx.counter(&alice, "questions_answered").await
        ),
        (1, 1, 1)
    );
    assert_eq!(ctx.counter(&alice, "friends_reached").await, 1);
    assert_eq!(ctx.progress(&alice).await.level.as_option().unwrap().total_xp, 3 + 8 + 5);
}

/// A region entered for the first time, as geo records it: private while
/// the player shows their footprints to nobody.
fn new_region(user: &TestUser, audience: &str) -> Event {
    let data = json!({
        "user_id": user.id, "kind": "geo.new_region", "audience": audience,
        "summary": { "ja": "新しい地域!", "en": "A new region" }, "path": "/footprints",
        "data": { "region_id": Uuid::now_v7(), "kind": 1, "country_code": "JP" },
    });
    event("activity.recorded", Some(user.id), data)
}

#[tokio::test]
async fn where_somebody_has_been_earns_nothing_while_they_hide_their_footprints() {
    let ctx = start().await;
    let (hidden, open, friend) =
        (ctx.app.user("hidden").await, ctx.app.user("open").await, ctx.app.user("friend").await);
    ctx.befriend(&hidden, &friend).await;

    ctx.feed(new_region(&hidden, "private")).await;
    ctx.feed(new_region(&open, "friends")).await;
    ctx.flush(&[&hidden, &open]).await;

    // Nothing the friends of the hidden player could read their movements from.
    assert_eq!(ctx.progress(&hidden).await.level.as_option().unwrap().total_xp, 0);
    assert_eq!(ctx.counter(&hidden, "regions_explored").await, 0);
    assert_eq!(ctx.source_today(&open, "map").await, (30, false));
    assert_eq!(ctx.counter(&open, "regions_explored").await, 1);

    // What they choose to share still counts: a check-in shown to friends.
    check_in(&ctx, &hidden, "friends", &[&friend]).await;
    ctx.flush(&[&hidden]).await;
    assert_eq!(ctx.source_today(&hidden, "map").await, (10, false));
}

#[tokio::test]
async fn studying_counts_only_as_far_as_the_player_shows_it() {
    let ctx = start().await;
    let (shy, open) = (ctx.app.user("shy").await, ctx.app.user("open").await);
    let session = |user: &TestUser, manual: bool| {
        event(
            "school.study.session_ended",
            Some(user.id),
            json!({
                "user_id": user.id, "session_id": Uuid::now_v7(), "minutes": 50, "seconds": 3000, "subject": "math",
                "source": if manual { 3 } else { 1 }, "manual": manual, "auto_stopped": false,
            }),
        )
    };
    // The school domain's own event reaches everybody's consumers, also for
    // players who hide their studying and for time logged by hand. It is
    // not what counts.
    for user in [&shy, &open] {
        ctx.feed(session(user, false)).await;
        ctx.feed(session(user, true)).await;
    }
    // What counts is the entry school records for a timed session of a
    // player who shows their studying: only `open` gets one.
    let shown = json!({
        "user_id": open.id, "kind": "school.studied", "audience": "friends",
        "summary": { "ja": "50分勉強しました", "en": "Studied for 50 min" },
        "data": { "session_id": Uuid::now_v7(), "minutes": 50, "subject": "math", "in_room": false },
    });
    let entry = event("activity.recorded", Some(open.id), shown);
    ctx.feed(entry.clone()).await;
    // The same session's entry recorded again (a retry on school's side) is still one session.
    ctx.feed(event("activity.recorded", Some(open.id), entry.data.clone())).await;
    ctx.flush(&[&shy, &open]).await;

    assert_eq!(ctx.progress(&shy).await.level.as_option().unwrap().total_xp, 0);
    assert_eq!(ctx.counter(&shy, "study_minutes").await, 0);
    assert_eq!(ctx.source_today(&open, "study").await, (50, false));
    assert_eq!((ctx.counter(&open, "study_minutes").await, ctx.counter(&open, "study_sessions").await), (50, 1));
}
