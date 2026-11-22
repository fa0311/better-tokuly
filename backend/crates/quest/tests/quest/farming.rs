//! Only XP has a daily cap. The counters behind missions and badges must
//! not move when one action is repeated, undone and redone, or done on the
//! player's own content.
use serde_json::json;
use tokuly_core::{analytics::AnalyticsEvent, prelude::*};
use tokuly_proto::quest::*;
use tokuly_testkit::TestUser;

use crate::common::*;

/// `user` likes the note `note` of `author`, as the notes domain tracks it:
/// a new analytics event each time, also after an unlike.
async fn like(ctx: &Ctx, user: &TestUser, note: Uuid, author: Uuid) {
    ctx.track(AnalyticsEvent::new("note.like").user(Some(user.id)).subject("note", note).owner(author)).await;
}

/// `user` sets their reaction on `subject` of `owner` from `previous` to
/// `kind` (`ReactionKind`: 0 none, 1 like), as engagement announces it.
async fn react(ctx: &Ctx, user: &TestUser, subject: Uuid, owner: Uuid, kind: i32, previous: i32) {
    let data = json!({
        "user_id": user.id, "subject_type": 3, "subject_id": subject, "subject_owner_id": owner,
        "kind": kind, "emoji": null, "previous_kind": previous,
    });
    ctx.feed(event("engagement.reaction.changed", Some(user.id), data)).await;
}

#[tokio::test]
async fn liking_unliking_and_liking_again_is_one_like() {
    let ctx = start().await;
    let (alice, author) = (ctx.app.user("alice").await, ctx.app.user("author").await);
    let note = Uuid::now_v7();
    for _ in 0..5 {
        like(&ctx, &alice, note, author.id).await;
    }
    let video = Uuid::now_v7();
    for _ in 0..5 {
        react(&ctx, &alice, video, author.id, 1, 0).await;
        react(&ctx, &alice, video, author.id, 0, 1).await;
    }
    ctx.flush(&[&alice, &author]).await;

    // One note and one video liked: two likes given, two received.
    assert_eq!(ctx.counter(&alice, "likes_given").await, 2);
    assert_eq!(ctx.counter(&author, "likes_received").await, 2);
    assert_eq!(ctx.source_today(&alice, "reactions").await, (2, false));

    // Different notes are different likes.
    ctx.like_notes(&alice, 3).await;
    ctx.flush(&[&alice]).await;
    assert_eq!(ctx.counter(&alice, "likes_given").await, 5);
}

#[tokio::test]
async fn a_like_mission_cannot_be_finished_on_one_note_or_on_ones_own() {
    let ctx = start().await;
    let (alice, author) = (ctx.app.user("alice").await, ctx.app.user("author").await);
    ctx.known_set(&alice).await;
    let before = ctx.counter(&alice, "likes_given").await;

    // Her own note, as often as she likes; and one note of somebody else, again and again.
    let (own, other) = (Uuid::now_v7(), Uuid::now_v7());
    for _ in 0..4 {
        like(&ctx, &alice, own, alice.id).await;
        react(&ctx, &alice, own, alice.id, 1, 0).await;
        react(&ctx, &alice, own, alice.id, 0, 1).await;
        like(&ctx, &alice, other, author.id).await;
    }
    ctx.flush(&[&alice]).await;

    assert_eq!(ctx.counter(&alice, "likes_given").await - before, 1);
    assert_eq!(ctx.counter(&alice, "likes_received").await, 0, "her own likes are not likes received");
    let missions = ctx.missions(&alice).await;
    let likes = missions.daily.iter().find(|mission| mission.key == "likes_3").expect("the known set has likes_3");
    assert_eq!((likes.progress, likes.target), (1, 3));
    assert_eq!(likes.state.as_known(), Some(MissionState::MISSION_STATE_ACTIVE));
}

#[tokio::test]
async fn unfriending_and_making_up_is_not_a_new_friend() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    for _ in 0..4 {
        ctx.befriend(&alice, &bob).await;
        ctx.friend_accepted(&alice, &bob).await;
        ctx.unfriend(&alice, &bob).await;
    }
    // The other side announcing the same friendship is the same friendship.
    ctx.friend_accepted(&bob, &alice).await;
    ctx.flush(&[&alice, &bob]).await;
    assert_eq!((ctx.counter(&alice, "friends_added").await, ctx.counter(&bob, "friends_added").await), (1, 1));
    assert_eq!(ctx.source_today(&alice, "friends").await, (20, false));

    // A friend she never had counts.
    ctx.befriend(&alice, &carol).await;
    ctx.friend_accepted(&carol, &alice).await;
    ctx.flush(&[&alice]).await;
    assert_eq!(ctx.counter(&alice, "friends_added").await, 2);
}

#[tokio::test]
async fn what_can_be_undone_counts_once_per_thing() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    let (pack, video, shared, post) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    for _ in 0..3 {
        // A pack added, removed and added again.
        ctx.feed(event("stamp.pack.installed", Some(alice.id), json!({ "pack_id": pack, "user_id": alice.id }))).await;
        // A public video published, taken down and published again (were the producer to say "first" twice).
        let published = json!({ "video_id": video, "owner_id": alice.id, "visibility": 1, "first_publication": true });
        ctx.feed(event("video.video.published", Some(alice.id), published)).await;
        // The same thing shared again.
        let share = json!({ "user_id": alice.id, "subject_type": 3, "subject_id": shared, "subject_owner_id": bob.id, "target": "copy" });
        ctx.feed(event("engagement.share.recorded", Some(alice.id), share)).await;
        // A comment written, deleted and written again under the same post — and under her own.
        for (subject, owner) in [(post, bob.id), (Uuid::now_v7(), alice.id)] {
            let comment = json!({ "comment_id": Uuid::now_v7(), "author_id": alice.id, "subject_type": 3, "subject_id": subject, "subject_owner_id": owner, "channel_id": null });
            ctx.feed(event("engagement.comment.published", Some(alice.id), comment)).await;
        }
        // Poking the same friend, and a plan made, called off and made again.
        ctx.map_reaction(&alice, &bob).await;
        ctx.feed(event(
            "plans.plan.created",
            Some(alice.id),
            json!({ "plan_id": Uuid::now_v7(), "host_id": alice.id, "invitee_ids": [bob.id] }),
        ))
        .await;
        // Two accounts bumping phones over and over.
        ctx.feed(event(
            "social.bump.created",
            Some(alice.id),
            json!({ "bump_id": Uuid::now_v7(), "user_ids": [alice.id, bob.id], "friends": true }),
        ))
        .await;
    }
    ctx.flush(&[&alice, &bob]).await;

    for metric in
        ["packs_installed", "videos_published", "shares", "comments", "map_reactions", "plans_created", "bumps"]
    {
        assert_eq!(ctx.counter(&alice, metric).await, 1, "{metric}");
    }
    assert_eq!(ctx.counter(&bob, "bumps").await, 1);
    // And the XP is what one of each is worth, not three.
    assert_eq!(ctx.source_today(&alice, "stamps").await.0, 5);
    assert_eq!(ctx.source_today(&alice, "creators").await.0, 40);
    assert_eq!(ctx.source_today(&alice, "reactions").await.0, 2 + 3);
    assert_eq!(ctx.source_today(&alice, "together").await.0, 10);
    assert_eq!(ctx.source_today(&alice, "friends").await.0, 3 + 15);
}

#[tokio::test]
async fn the_event_that_was_counted_still_counts_when_it_is_delivered_again() {
    let ctx = start().await;
    let (alice, author) = (ctx.app.user("alice").await, ctx.app.user("author").await);
    let note = Uuid::now_v7();
    let tracked = AnalyticsEvent::new("note.like").user(Some(alice.id)).subject("note", note).owner(author.id);
    // Delivered at once to several consumers, and again later.
    let together: Vec<_> = (0..8).map(|_| ctx.quest.handle_analytics(tracked.clone())).collect();
    for result in futures::future::join_all(together).await {
        result.unwrap();
    }
    ctx.track(tracked).await;
    // Another like of the same note, arriving together with nothing to tell them apart but their IDs.
    let others: Vec<_> = (0..8)
        .map(|_| {
            ctx.quest.handle_analytics(
                AnalyticsEvent::new("note.like").user(Some(alice.id)).subject("note", note).owner(author.id),
            )
        })
        .collect();
    for result in futures::future::join_all(others).await {
        result.unwrap();
    }
    ctx.flush(&[&alice, &author]).await;
    assert_eq!(ctx.counter(&alice, "likes_given").await, 1);
    assert_eq!(ctx.counter(&author, "likes_received").await, 1);

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM quest_once WHERE user_id = $1")
        .bind(alice.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn what_is_honest_to_repeat_still_counts_every_time() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    ctx.send_stamps(&alice, 4).await;
    for _ in 0..3 {
        ctx.post_note(&alice).await;
    }
    ctx.flush(&[&alice]).await;
    assert_eq!((ctx.counter(&alice, "stamps_sent").await, ctx.counter(&alice, "notes_posted").await), (4, 3));
}

#[tokio::test]
async fn a_deleted_player_takes_the_keys_that_name_them_along() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.befriend(&alice, &bob).await;
    ctx.friend_accepted(&alice, &bob).await;
    like(&ctx, &bob, Uuid::now_v7(), alice.id).await;
    let about_bob = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM quest_once WHERE user_id = $1 OR about = $1 OR key LIKE '%' || $1::text || '%'",
        )
        .bind(bob.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap()
    };
    assert!(about_bob().await >= 4);
    ctx.feed(event("identity.user.deleted", Some(bob.id), json!({ "user_id": bob.id }))).await;
    assert_eq!(about_bob().await, 0);
}
