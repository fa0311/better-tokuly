//! Requests that really run at the same time.
//!
//! Two things are held to account here:
//!
//! - **No request needs two database connections.** The test pool has five
//!   connections and gives up on acquiring one after ten seconds. A handler
//!   that held a transaction (or a lock) while asking the pool for another
//!   connection would deadlock as soon as five of them ran together, and
//!   every request of every user would wait out that timeout. Each test
//!   below fires more calls than the pool has connections, from one
//!   account, and requires them all back well inside the timeout.
//! - **Every "check, then write" holds under a race**: one upload is one
//!   short, three pins are three pins, a save is counted once, a view is
//!   counted once, an event delivered twice counts once.
mod common;

use std::time::Duration;

use common::*;
use futures::future::join_all;
use tokuly_proto::{common::SubjectType, shorts::*, video::NotificationLevel};
use tokuly_shorts::{ShortsConfig, events::AnnounceJob};
use tokuly_testkit::RpcError;

/// More than the test pool's five connections.
const CROWD: usize = 12;
/// Well inside the pool's ten second acquire timeout: a request that waits
/// for a second connection does not make it.
const PROMPTLY: Duration = Duration::from_secs(8);

/// Awaits all of `calls` together and fails if they do not come back promptly.
async fn together<T>(what: &str, calls: Vec<impl std::future::Future<Output = T>>) -> Vec<T> {
    tokio::time::timeout(PROMPTLY, join_all(calls))
        .await
        .unwrap_or_else(|_| panic!("{what}: concurrent requests starved each other of database connections"))
}

fn reasons<T>(results: &[Result<T, RpcError>]) -> Vec<&str> {
    let mut reasons: Vec<&str> =
        results.iter().filter_map(|result| result.as_ref().err()).map(|e| e.reason.as_str()).collect();
    reasons.sort_unstable();
    reasons
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_account_cannot_stall_the_api_by_creating_and_editing_at_once() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    ctx.befriend(&alice, &bob).await;
    let tag = unique("crowd");

    // Creating: each call publishes its short (hashtags, search document,
    // announcement, mention) inside one transaction.
    let mut uploads = Vec::new();
    for _ in 0..CROWD {
        uploads.push(ctx.upload(&alice).await);
    }
    let requests: Vec<CreateShortRequest> = uploads
        .iter()
        .map(|asset_id| CreateShortRequest {
            channel_id: channel.summary.id.clone(),
            asset_id: asset_id.clone(),
            caption: format!("同時に投稿 #{tag} @{}", bob.handle),
            visibility: tokuly_proto::common::Visibility::VISIBILITY_FRIENDS.into(),
            ..Default::default()
        })
        .collect();
    let created = together("CreateShort", requests.iter().map(|request| ctx.create(&alice, request)).collect()).await;
    let shorts: Vec<Short> = created.into_iter().map(|result| result.expect("every short is created")).collect();
    assert!(shorts.iter().all(|short| short.status == ShortStatus::SHORT_STATUS_PUBLISHED));

    // Editing one short from many requests: they queue on its row.
    let target = shorts[0].id.clone();
    let edits: Vec<UpdateShortRequest> = (0..CROWD)
        .map(|n| UpdateShortRequest {
            short_id: target.clone(),
            caption: Some(format!("編集{n} #{tag} #{tag}{n} @{}", bob.handle)),
            visibility: Some(tokuly_proto::common::Visibility::VISIBILITY_PUBLIC.into()),
            ..Default::default()
        })
        .collect();
    let edited = together("UpdateShort", edits.iter().map(|request| ctx.update(&alice, request)).collect()).await;
    assert!(edited.iter().all(Result::is_ok), "{:?}", reasons(&edited));
    // Whichever edit came last, what is derived from the short agrees with it.
    let row = ctx.row(&target).await;
    let counted: Vec<String> = sqlx::query_scalar(
        "SELECT h.tag FROM short_hashtag_uses u JOIN short_hashtags h ON h.id = u.hashtag_id WHERE u.short_id = $1 ORDER BY h.tag",
    )
    .bind(row.id)
    .fetch_all(&ctx.app.core.db)
    .await
    .unwrap();
    let mut expected = row.hashtags.clone();
    expected.sort();
    assert_eq!(counted, expected);
    let stray: i64 = sqlx::query_scalar("SELECT count(*) FROM short_hashtags WHERE tag LIKE $1 AND use_count <> (SELECT count(*) FROM short_hashtag_uses u WHERE u.hashtag_id = short_hashtags.id)")
        .bind(format!("{tag}%"))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(stray, 0, "every hashtag count equals its rows");
    // Bob was mentioned in thirteen captions of twelve shorts: one notification per short.
    let told = ctx.notifications(bob.id).await;
    assert_eq!(told.iter().filter(|(kind, _)| kind == "shorts.mention").count(), CROWD);

    // Deleting them all at once, each one twice.
    let deletes: Vec<DeleteShortRequest> = shorts
        .iter()
        .chain(&shorts)
        .map(|short| DeleteShortRequest { short_id: short.id.clone(), ..Default::default() })
        .collect();
    let deleted = together(
        "DeleteShort",
        deletes
            .iter()
            .map(|request| ctx.call::<_, DeleteShortResponse>(SHORTS, "DeleteShort", Some(&alice), request))
            .collect(),
    )
    .await;
    assert_eq!(deleted.iter().filter(|result| result.is_ok()).count(), CROWD, "each short is deleted exactly once");
    assert_eq!(reasons(&deleted), vec!["SHORT_NOT_FOUND"; CROWD]);
    assert_eq!(ctx.app.events_by("shorts.short.deleted", alice.id).await.len(), CROWD);
    let left: i64 =
        sqlx::query_scalar("SELECT COALESCE(sum(use_count), 0)::bigint FROM short_hashtags WHERE tag LIKE $1")
            .bind(format!("{tag}%"))
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(left, 0, "no hashtag still counts a deleted short, none went below zero");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upload_becomes_one_short_however_many_requests_race_for_it() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    let request = CreateShortRequest {
        channel_id: channel.summary.id.clone(),
        asset_id: ctx.upload(&alice).await,
        caption: "一本だけ".into(),
        ..Default::default()
    };
    let results = together("CreateShort", (0..CROWD).map(|_| ctx.create(&alice, &request)).collect()).await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(reasons(&results), vec!["ASSET_ALREADY_USED"; CROWD - 1]);
    let mine: ListMyShortsResponse =
        ctx.call(SHORTS, "ListMyShorts", Some(&alice), &ListMyShortsRequest::default()).await.unwrap();
    assert_eq!(mine.shorts.len(), 1);
    assert_eq!(ctx.app.events_by("shorts.short.published", alice.id).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_draft_goes_live_once_when_its_upload_is_reported_ready_many_times() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    let asset_id = ctx.upload(&alice).await;
    ctx.set_asset_status(&asset_id, tokuly_proto::media::AssetStatus::ASSET_STATUS_PROCESSING, "").await;
    let tag = unique("once");
    let draft = ctx
        .short_with(&alice, &channel, &format!("一度だけ公開 #{tag}"), |request| {
            request.asset_id = asset_id.clone()
        })
        .await;
    ctx.set_asset_status(&asset_id, tokuly_proto::media::AssetStatus::ASSET_STATUS_READY, "").await;

    let ready = serde_json::json!({ "asset_id": asset_id, "owner_id": alice.id, "purpose": 13, "kind": 2 });
    together(
        "media.asset.ready",
        (0..CROWD).map(|_| ctx.deliver_event("media.asset.ready", Some(alice.id), ready.clone())).collect(),
    )
    .await;
    assert_eq!(ctx.get(None, &draft.id).await.unwrap().status, ShortStatus::SHORT_STATUS_PUBLISHED);
    assert_eq!(ctx.app.events_by("shorts.short.published", alice.id).await.len(), 1);
    assert_eq!(ctx.app.events_by("activity.recorded", alice.id).await.len(), 1);
    let count: i64 = sqlx::query_scalar("SELECT use_count FROM short_hashtags WHERE tag = $1")
        .bind(&tag)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_pins_are_three_pins_under_a_race() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    let mut requests = Vec::new();
    for n in 0..CROWD {
        let short = ctx.short(&alice, &channel, &format!("{n}本目")).await;
        requests.push(PinShortRequest { short_id: short.id, ..Default::default() });
    }
    let results = together(
        "PinShort",
        requests
            .iter()
            .map(|request| ctx.call::<_, PinShortResponse>(SHORTS, "PinShort", Some(&alice), request))
            .collect(),
    )
    .await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 3);
    assert_eq!(reasons(&results), vec!["PIN_LIMIT_REACHED"; CROWD - 3]);
    let pinned: i64 = sqlx::query_scalar("SELECT count(*) FROM shorts WHERE channel_id = $1 AND pinned_at IS NOT NULL")
        .bind(id(&channel.summary.id))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(pinned, 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn saves_count_once_and_keep_their_limit_under_a_race() {
    let ctx = start_with(ShortsConfig { max_saves: 3, ..test_config() }).await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let short = ctx.short(&alice, &channel, "何度も保存").await;

    // The same short, from many requests of one user: one save.
    let request = SaveShortRequest { short_id: short.id.clone(), ..Default::default() };
    let results = together(
        "SaveShort",
        (0..CROWD).map(|_| ctx.call::<_, SaveShortResponse>(SHORTS, "SaveShort", Some(&bob), &request)).collect(),
    )
    .await;
    assert!(results.iter().all(Result::is_ok), "{:?}", reasons(&results));
    assert_eq!(ctx.row(&short.id).await.save_count, 1);
    assert_eq!(hourly(&ctx, &short.id).await.5, 1);
    // Unsaving from many requests takes it off once.
    let request = UnsaveShortRequest { short_id: short.id.clone(), ..Default::default() };
    let results = together(
        "UnsaveShort",
        (0..CROWD).map(|_| ctx.call::<_, UnsaveShortResponse>(SHORTS, "UnsaveShort", Some(&bob), &request)).collect(),
    )
    .await;
    assert!(results.iter().all(Result::is_ok));
    assert_eq!(ctx.row(&short.id).await.save_count, 0);

    // Many different shorts at once: the limit of three holds.
    let mut requests = Vec::new();
    for n in 0..CROWD {
        let short = ctx.short(&alice, &channel, &format!("保存候補{n}")).await;
        requests.push(SaveShortRequest { short_id: short.id, ..Default::default() });
    }
    let results = together(
        "SaveShort",
        requests
            .iter()
            .map(|request| ctx.call::<_, SaveShortResponse>(SHORTS, "SaveShort", Some(&bob), request))
            .collect(),
    )
    .await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 3);
    assert_eq!(reasons(&results), vec!["SAVE_LIMIT_REACHED"; CROWD - 3]);
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM short_saves WHERE user_id = $1")
        .bind(bob.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(kept, 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stay_reported_many_times_at_once_is_one_view() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let short = ctx.short(&alice, &channel, "同時に報告").await;
    let _lock = playback_lock().await;

    let reports = || vec![stay(&short.id, PlaybackSource::PLAYBACK_SOURCE_FOR_YOU, 36_000, 3)];
    let results = together("ReportPlayback", (0..CROWD).map(|_| ctx.report(Some(&bob), reports())).collect()).await;
    assert_eq!(results.iter().map(|recorded| recorded.views_counted).sum::<i32>(), 1);
    // Visitors race for their marker the same way (their view is shown, not ranked).
    let results = together("ReportPlayback", (0..CROWD).map(|_| ctx.report(None, reports())).collect()).await;
    assert_eq!(results.iter().map(|recorded| recorded.views_counted).sum::<i32>(), 1);
    // Flushing from several workers at once writes each counter once.
    together("flush", (0..4).map(|_| ctx.shorts.flush_all_signals()).collect()).await.into_iter().for_each(|flushed| {
        flushed.expect("flush");
    });
    let row = ctx.row(&short.id).await;
    assert_eq!((row.view_count, row.watch_samples, row.loop_count), (2, 1, 2));
    assert_eq!(hourly(&ctx, &short.id).await, (1, 2, 0, 0, 0, 0));

    // A share from many requests at once is one share.
    let request = ShareShortRequest { short_id: short.id.clone(), ..Default::default() };
    let results = together(
        "ShareShort",
        (0..CROWD).map(|_| ctx.call::<_, ShareShortResponse>(SHORTS, "ShareShort", Some(&bob), &request)).collect(),
    )
    .await;
    assert!(results.iter().all(Result::is_ok));
    assert_eq!(ctx.row(&short.id).await.share_count, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_like_delivered_twice_at_once_is_one_like() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let short = ctx.short(&alice, &channel, "同時にいいね").await;
    let event = tokuly_core::events::Event {
        id: uuid::Uuid::now_v7(),
        topic: "engagement.reaction.changed".to_owned(),
        actor_id: Some(bob.id),
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({
            "user_id": bob.id, "subject_type": SubjectType::SUBJECT_TYPE_SHORT as i32, "subject_id": short.id,
            "kind": 1, "previous_kind": 0,
        }),
    };
    let results =
        together("reaction event", (0..CROWD).map(|_| ctx.shorts.handle_event(event.clone())).collect()).await;
    assert!(results.iter().all(Result::is_ok));
    let (rows, liked): (i64, bool) = sqlx::query_as(
        "SELECT count(*), COALESCE(bool_and(liked), false) FROM short_likes WHERE user_id = $1 AND short_id = $2",
    )
    .bind(bob.id)
    .bind(id(&short.id))
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!((rows, liked), (1, true));
    assert_eq!(hourly(&ctx, &short.id).await.2, 1, "one like in the ranking");

    // The same for comments of one person arriving together.
    let comment = serde_json::json!({
        "comment_id": uuid::Uuid::now_v7(), "subject_type": SubjectType::SUBJECT_TYPE_SHORT as i32,
        "subject_id": short.id, "author_id": bob.id,
    });
    together(
        "comment event",
        (0..CROWD).map(|_| ctx.deliver_event("engagement.comment.published", Some(bob.id), comment.clone())).collect(),
    )
    .await;
    assert_eq!(hourly(&ctx, &short.id).await.3, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_feed_requests_and_reports_keep_the_seen_list_whole() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let vera = ctx.app.user("vera").await;
    let channel = ctx.channel(&alice, "ch").await;
    ctx.subscribe(&vera, &channel, NotificationLevel::NOTIFICATION_LEVEL_NONE).await;
    let mut mine = Vec::new();
    for n in 0..8 {
        mine.push(ctx.short(&alice, &channel, &format!("並行{n}")).await.id);
    }
    // Two devices open the feed at the same moment (several times over).
    let batches = together("GetForYouFeed", (0..8).map(|_| ctx.for_you(Some(&vera), 30, "")).collect()).await;
    let mut shown: Vec<&str> = Vec::new();
    for batch in &batches {
        let mut ids: Vec<&str> = batch.items.iter().map(|item| item_short(item).id.as_str()).collect();
        let total = ids.len();
        shown.extend(&ids);
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "no short twice in a page");
    }
    assert!(mine.iter().all(|id| shown.contains(&id.as_str())), "every followed short was shown");

    // Both devices report what they played, in overlapping halves, at once.
    let _lock = playback_lock().await;
    let halves: Vec<Vec<PlaybackReport>> = (0..CROWD)
        .map(|n| {
            let part = if n % 2 == 0 { &mine[..5] } else { &mine[3..] };
            part.iter().map(|short_id| stay(short_id, PlaybackSource::PLAYBACK_SOURCE_FOR_YOU, 4_000, 0)).collect()
        })
        .collect();
    let recorded =
        together("ReportPlayback", halves.into_iter().map(|reports| ctx.report(Some(&vera), reports)).collect()).await;
    assert_eq!(recorded.iter().map(|recorded| recorded.views_counted).sum::<i32>(), 8, "one view per short");
    let seen = tokuly_shorts::signals::seen_ids(&ctx.app.core, vera.id).await.unwrap();
    assert!(mine.iter().all(|short_id| seen.contains(&id(short_id))));

    // Whatever the next batch holds, none of them is in it.
    let batches = together("GetForYouFeed", (0..4).map(|_| ctx.for_you(Some(&vera), 30, "")).collect()).await;
    for batch in &batches {
        assert!(batch.items.iter().all(|item| item.repeat || !mine.contains(&item_short(item).id)));
    }
    ctx.flush().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn announcing_from_several_workers_tells_each_subscriber_once() {
    let ctx = start_with(ShortsConfig { announce_batch_size: 2, announcements_per_day: 1, ..test_config() }).await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    for n in 0..7 {
        let fan = ctx.app.user(&format!("fan{n}")).await;
        ctx.subscribe(&fan, &channel, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    }
    let first = ctx.short(&alice, &channel, "一本目").await;
    let second = ctx.short(&alice, &channel, "二本目").await;
    let told = |short_id: String| {
        let ctx = &ctx;
        async move {
            ctx.app
                .events("notify.requested")
                .await
                .into_iter()
                .filter(|event| event.data["kind"] == "shorts.published" && event.data["data"]["id"] == short_id)
                .map(|event| event.data["user_id"].as_str().unwrap().to_owned())
                .collect::<Vec<String>>()
        }
    };

    // The job for one short, delivered to many workers at once.
    let job = AnnounceJob { short_id: id(&first.id), after: None };
    let results = together("shorts.announce", (0..CROWD).map(|_| ctx.shorts.announce(job.clone())).collect()).await;
    assert_eq!(results.into_iter().map(|notified| notified.expect("announce")).sum::<u64>(), 7);
    let mut recipients = told(first.id.clone()).await;
    assert_eq!(recipients.len(), 7);
    recipients.sort();
    recipients.dedup();
    assert_eq!(recipients.len(), 7, "seven subscribers, each told once");

    // Two shorts racing for the day's one announcement: one gets it.
    let fresh = ctx.channel(&alice, "another").await;
    let fan = ctx.app.user("fan").await;
    ctx.subscribe(&fan, &fresh, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    let (a, b) = (ctx.short(&alice, &fresh, "あ").await, ctx.short(&alice, &fresh, "い").await);
    let jobs = [AnnounceJob { short_id: id(&a.id), after: None }, AnnounceJob { short_id: id(&b.id), after: None }];
    let results = together(
        "shorts.announce",
        jobs.iter().cycle().take(CROWD).map(|job| ctx.shorts.announce(job.clone())).collect(),
    )
    .await;
    assert_eq!(results.into_iter().map(|notified| notified.expect("announce")).sum::<u64>(), 1);
    assert_eq!(told(a.id.clone()).await.len() + told(b.id.clone()).await.len(), 1);
    // And the channel that already announced today stays quiet.
    assert_eq!(ctx.shorts.announce(AnnounceJob { short_id: id(&second.id), after: None }).await.unwrap(), 0);
}
