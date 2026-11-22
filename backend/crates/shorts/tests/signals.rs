//! Signals: views, loops and watch-through from playback reports; saves,
//! likes, comments and shares; milestones; creator analytics.
mod common;

use common::*;
use tokuly_core::subject::SubjectInfo;
use tokuly_proto::{
    common::{SubjectType, Visibility},
    shorts::*,
    video::NotificationLevel,
};
use tokuly_shorts::ShortsConfig;
use tokuly_testkit::TestUser;

const FOR_YOU: PlaybackSource = PlaybackSource::PLAYBACK_SOURCE_FOR_YOU;

async fn save(ctx: &Ctx, user: &TestUser, short_id: &str) -> Result<i64, tokuly_testkit::RpcError> {
    let request = SaveShortRequest { short_id: short_id.to_owned(), ..Default::default() };
    let response: SaveShortResponse = ctx.call(SHORTS, "SaveShort", Some(user), &request).await?;
    Ok(response.save_count)
}

async fn unsave(ctx: &Ctx, user: &TestUser, short_id: &str) -> Result<i64, tokuly_testkit::RpcError> {
    let request = UnsaveShortRequest { short_id: short_id.to_owned(), ..Default::default() };
    let response: UnsaveShortResponse = ctx.call(SHORTS, "UnsaveShort", Some(user), &request).await?;
    Ok(response.save_count)
}

async fn saved(ctx: &Ctx, user: &TestUser) -> Vec<String> {
    let response: ListSavedShortsResponse =
        ctx.call(SHORTS, "ListSavedShorts", Some(user), &ListSavedShortsRequest::default()).await.unwrap();
    response.shorts.into_iter().map(|short| short.caption).collect()
}

async fn liked(ctx: &Ctx, user: &TestUser) -> Vec<String> {
    let response: ListLikedShortsResponse =
        ctx.call(SHORTS, "ListLikedShorts", Some(user), &ListLikedShortsRequest::default()).await.unwrap();
    response.shorts.into_iter().map(|short| short.caption).collect()
}

async fn share(
    ctx: &Ctx,
    user: Option<&TestUser>,
    short_id: &str,
) -> Result<ShareShortResponse, tokuly_testkit::RpcError> {
    let request = ShareShortRequest {
        short_id: short_id.to_owned(),
        target: ShareTarget::SHARE_TARGET_COPY_LINK.into(),
        ..Default::default()
    };
    ctx.call(SHORTS, "ShareShort", user, &request).await
}

fn reaction(user: &TestUser, short_id: &str, liked: bool) -> serde_json::Value {
    let (kind, previous) = if liked { (1, 0) } else { (0, 1) };
    serde_json::json!({
        "user_id": user.id,
        "subject_type": SubjectType::SUBJECT_TYPE_SHORT as i32,
        "subject_id": short_id,
        "kind": kind,
        "previous_kind": previous,
    })
}

#[tokio::test]
async fn one_viewer_counts_one_view_and_one_sample_however_much_they_report() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let dave = ctx.app.user("dave").await;
    let channel = ctx.channel(&alice, "ch").await;
    let short = ctx.short(&alice, &channel, "12秒の動画").await;
    let _lock = playback_lock().await;
    let one = |viewer: Option<&TestUser>, watched_ms: i64, completions: i32| {
        let ctx = &ctx;
        let viewer = viewer.cloned();
        let short_id = short.id.clone();
        async move {
            let recorded = ctx.report(viewer.as_ref(), vec![stay(&short_id, FOR_YOU, watched_ms, completions)]).await;
            (recorded.accepted, recorded.views_counted)
        }
    };

    // Bob watches five seconds: a view, and his sample (5/12 watched).
    assert_eq!(one(Some(&bob), 5_000, 0).await, (1, 1));
    // Carol lets it play three times: a view, a sample of 100 %, two loops.
    assert_eq!(one(Some(&carol), 36_000, 3).await, (1, 1));
    // Dave swipes on at once: no view, but that is how he watched it.
    assert_eq!(one(Some(&dave), 300, 0).await, (1, 0));
    // He comes back and watches: now it is a view. His sample stays the first stay.
    assert_eq!(one(Some(&dave), 6_000, 0).await, (1, 1));
    // The owner's own plays count for nothing.
    assert_eq!(one(Some(&alice), 12_000, 1).await, (0, 0));
    // Visitors count per network, towards the number shown and nothing else.
    assert_eq!(one(None, 2_000, 0).await, (1, 1));
    assert_eq!(one(None, 12_000, 1).await, (1, 0));

    // Nothing has touched PostgreSQL yet.
    let buffered = ctx.row(&short.id).await;
    assert_eq!((buffered.view_count, buffered.watch_samples), (0, 0));
    ctx.flush().await;
    let row = ctx.row(&short.id).await;
    assert_eq!((row.view_count, row.loop_count, row.watch_samples), (4, 2, 3));
    let expected_ratio = (5.0 / 12.0 + 1.0 + 0.025) / 3.0;
    assert!((row.average_watch_ratio() - expected_ratio).abs() < 0.002, "{}", row.average_watch_ratio());
    // The visitor's view is in the count people see, not in what the ranking reads.
    assert_eq!(hourly(&ctx, &short.id).await, (3, 2, 0, 0, 0, 0));

    // Reporting more moves nothing: not the views, not the loops, not the ratio.
    for _ in 0..20 {
        assert_eq!(one(Some(&bob), 3_600_000, 50).await, (1, 0));
    }
    ctx.flush().await;
    let after = ctx.row(&short.id).await;
    assert_eq!((after.view_count, after.loop_count, after.watch_samples), (4, 2, 3));
    assert_eq!(after.watch_ratio_sum, row.watch_ratio_sum);
    assert_eq!(hourly(&ctx, &short.id).await, (3, 2, 0, 0, 0, 0));
    assert_eq!(ctx.get(Some(&bob), &short.id).await.unwrap().view_count, 4);
    // Every stay marked the short as seen, though.
    assert!(viewer_state(&ctx.get(Some(&dave), &short.id).await.unwrap()).seen);
}

#[tokio::test]
async fn a_view_counts_again_once_the_window_has_passed() {
    let ctx = start_with(ShortsConfig { view_dedupe_seconds: 1, ..test_config() }).await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let short = ctx.short(&alice, &channel, "また見た").await;
    let _lock = playback_lock().await;
    assert_eq!(ctx.watch(&bob, &short.id).await.views_counted, 1);
    assert_eq!(ctx.watch(&bob, &short.id).await.views_counted, 0);
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    assert_eq!(ctx.watch(&bob, &short.id).await.views_counted, 1);
    ctx.flush().await;
    let row = ctx.row(&short.id).await;
    assert_eq!((row.view_count, row.watch_samples), (2, 2));
}

#[tokio::test]
async fn view_milestones_tell_the_owner_once_each() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    let short = ctx.short(&alice, &channel, "のびる動画").await;
    let set_views = |views: i64| {
        let ctx = &ctx;
        let short_id = short.id.clone();
        async move {
            sqlx::query("UPDATE shorts SET view_count = $2 WHERE id = $1")
                .bind(id(&short_id))
                .bind(views)
                .execute(&ctx.app.core.db)
                .await
                .unwrap();
        }
    };
    let _lock = playback_lock().await;
    let mut viewers = Vec::new();
    for n in 0..4 {
        viewers.push(ctx.app.user(&format!("viewer{n}")).await);
    }

    set_views(998).await;
    ctx.watch(&viewers[0], &short.id).await;
    ctx.flush().await;
    assert!(ctx.notified(alice.id).await.is_empty(), "999 views are not a milestone");
    ctx.watch(&viewers[1], &short.id).await;
    ctx.watch(&viewers[2], &short.id).await;
    ctx.flush().await;
    let told = ctx.notifications(alice.id).await;
    assert_eq!(told.len(), 1);
    assert_eq!((told[0].0.as_str(), told[0].1["data"]["milestone"].as_i64()), ("shorts.milestone", Some(1000)));
    assert_eq!(told[0].1["title"]["ja"], "ショートが1,000回再生されたよ!");
    assert_eq!(told[0].1["collapse_key"], format!("shorts.milestone:{}", short.id));

    // A jump past the next line tells about that line, once.
    set_views(99_999).await;
    ctx.watch(&viewers[3], &short.id).await;
    ctx.flush().await;
    let told = ctx.notifications(alice.id).await;
    assert_eq!(told.len(), 2);
    assert_eq!(told[1].1["data"]["milestone"].as_i64(), Some(100_000));
    assert_eq!(told[1].1["collapse_key"], told[0].1["collapse_key"], "milestones of one short replace each other");
    assert_eq!(ctx.row(&short.id).await.milestone, 100_000);
}

#[tokio::test]
async fn saves_are_private_counted_and_limited() {
    let ctx = start_with(ShortsConfig { max_saves: 2, ..test_config() }).await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let channel = ctx.channel(&alice, "ch").await;
    let first = ctx.short(&alice, &channel, "一本目").await;
    let second = ctx.short(&alice, &channel, "二本目").await;
    let third = ctx.short(&alice, &channel, "三本目").await;

    assert_eq!(save(&ctx, &bob, &first.id).await.unwrap(), 1);
    assert_eq!(save(&ctx, &bob, &first.short_id).await.unwrap(), 1, "saving twice is one save");
    assert!(viewer_state(&ctx.get(Some(&bob), &first.id).await.unwrap()).saved);
    let for_carol = ctx.get(Some(&carol), &first.id).await.unwrap();
    assert!(!viewer_state(&for_carol).saved && for_carol.save_count == 1, "the count is public, the list is not");
    assert_eq!(save(&ctx, &bob, &second.id).await.unwrap(), 1);
    assert_eq!(saved(&ctx, &bob).await, ["二本目", "一本目"]);
    assert_eq!(save(&ctx, &bob, &third.id).await.unwrap_err().reason, "SAVE_LIMIT_REACHED");
    assert!(saved(&ctx, &carol).await.is_empty());

    // Taking one off makes room. Saving, unsaving and saving again is
    // still one person's interest in the short as far as its ranking goes.
    assert_eq!(unsave(&ctx, &bob, &first.id).await.unwrap(), 0);
    assert_eq!(unsave(&ctx, &bob, &first.id).await.unwrap(), 0, "nothing to take off, nothing wrong");
    assert_eq!(saved(&ctx, &bob).await, ["二本目"]);
    assert_eq!(save(&ctx, &bob, &first.id).await.unwrap(), 1);
    assert_eq!(hourly(&ctx, &first.id).await.5, 1);
    // The owner's own save is a save, and no signal.
    assert_eq!(save(&ctx, &alice, &third.id).await.unwrap(), 1);
    assert_eq!(hourly(&ctx, &third.id).await.5, 0);

    // A saved short that goes private drops out of the list, and its
    // numbers are no longer the saver's to learn.
    assert_eq!(save(&ctx, &carol, &second.id).await.unwrap(), 2);
    ctx.set_visibility(&alice, &second.id, Visibility::VISIBILITY_PRIVATE).await;
    assert_eq!(saved(&ctx, &bob).await, ["一本目"]);
    assert_eq!(unsave(&ctx, &bob, &second.short_id).await.unwrap(), 0, "one save is left; bob is not told");
    assert_eq!(ctx.row(&second.id).await.save_count, 1);
    // A short ID that names nothing answers the same way; only nonsense is refused.
    assert_eq!(unsave(&ctx, &bob, "abcDEF12345").await.unwrap(), 0);
    assert_eq!(unsave(&ctx, &bob, &uuid::Uuid::now_v7().to_string()).await.unwrap(), 0);
    assert_eq!(unsave(&ctx, &bob, "!!").await.unwrap_err().reason, "INVALID_ID");

    assert_eq!(save(&ctx, &bob, &second.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    let request = SaveShortRequest { short_id: first.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(SHORTS, "SaveShort", None, &request).await, "SIGN_IN_REQUIRED");
    assert_eq!(
        ctx.reason(SHORTS, "ListSavedShorts", None, &ListSavedShortsRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
}

#[tokio::test]
async fn likes_and_comments_follow_the_engagement_domain() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let channel = ctx.channel(&alice, "ch").await;
    let short = ctx.short(&alice, &channel, "いいねされる動画").await;
    let subjects = &ctx.app.core.subjects;
    let short_uuid = id(&short.id);

    ctx.like(&bob, &short.id).await;
    let seen = ctx.get(Some(&bob), &short.id).await.unwrap();
    assert!(viewer_state(&seen).liked && seen.like_count == 1);
    assert!(!viewer_state(&ctx.get(Some(&carol), &short.id).await.unwrap()).liked);
    assert_eq!(liked(&ctx, &bob).await, ["いいねされる動画"]);
    assert_eq!(hourly(&ctx, &short.id).await.2, 1);

    // The like taken back and given again is still one like for the ranking.
    ctx.set_like(&bob, &short.id, false).await;
    let seen = ctx.get(Some(&bob), &short.id).await.unwrap();
    assert!(!viewer_state(&seen).liked && seen.like_count == 0);
    assert!(liked(&ctx, &bob).await.is_empty());
    ctx.like(&bob, &short.id).await;
    assert_eq!(hourly(&ctx, &short.id).await.2, 1);
    assert_eq!(ctx.get(None, &short.id).await.unwrap().like_count, 1);

    // The same event twice changes nothing.
    let at = chrono::Utc::now();
    let event = |liked: bool, at: chrono::DateTime<chrono::Utc>| tokuly_core::events::Event {
        id: uuid::Uuid::now_v7(),
        topic: "engagement.reaction.changed".to_owned(),
        actor_id: Some(carol.id),
        occurred_at: at,
        data: reaction(&carol, &short.id, liked),
    };
    let like = event(true, at);
    ctx.shorts.handle_event(like.clone()).await.unwrap();
    ctx.shorts.handle_event(like.clone()).await.unwrap();
    assert_eq!(hourly(&ctx, &short.id).await.2, 2, "carol's like, once");
    assert_eq!(liked(&ctx, &carol).await.len(), 1);
    // Out of order: the like arrives after the event that took it back.
    ctx.shorts.handle_event(event(false, at + chrono::Duration::seconds(5))).await.unwrap();
    ctx.shorts.handle_event(like).await.unwrap();
    assert!(liked(&ctx, &carol).await.is_empty(), "the later change stands");
    ctx.shorts.handle_event(event(true, at + chrono::Duration::seconds(9))).await.unwrap();
    assert_eq!(liked(&ctx, &carol).await.len(), 1);
    // A reaction to something that is not a short, or to a short that is gone.
    let mut elsewhere = reaction(&carol, &uuid::Uuid::now_v7().to_string(), true);
    ctx.deliver_event("engagement.reaction.changed", Some(carol.id), elsewhere.clone()).await;
    elsewhere["subject_type"] = serde_json::json!(SubjectType::SUBJECT_TYPE_VIDEO as i32);
    ctx.deliver_event("engagement.reaction.changed", Some(carol.id), elsewhere).await;

    // Comments: the count follows the callback; the ranking counts commenters.
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_SHORT, short_uuid, "comment_count", 3).await.unwrap();
    assert_eq!(ctx.get(None, &short.id).await.unwrap().comment_count, 3);
    let comment = |author: &TestUser| {
        serde_json::json!({
            "comment_id": uuid::Uuid::now_v7(),
            "subject_type": SubjectType::SUBJECT_TYPE_SHORT as i32,
            "subject_id": short.id,
            "subject_owner_id": alice.id,
            "author_id": author.id,
            "channel_id": null,
            "parent_id": null,
        })
    };
    for author in [&bob, &bob, &bob, &alice] {
        ctx.deliver_event("engagement.comment.published", Some(author.id), comment(author)).await;
    }
    assert_eq!(hourly(&ctx, &short.id).await.3, 1, "three comments of one person, and the owner's own");
    // Counts never go below zero, and counters this domain does not keep are ignored.
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_SHORT, short_uuid, "comment_count", -10).await.unwrap();
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_SHORT, short_uuid, "bookmark_count", 4).await.unwrap();
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_SHORT, uuid::Uuid::now_v7(), "like_count", 1).await.unwrap();
    let seen = ctx.get(None, &short.id).await.unwrap();
    // (Carol's likes above were events only: the count follows the callback.)
    assert_eq!((seen.comment_count, seen.like_count, seen.save_count), (0, 1, 0));
    assert_eq!(
        ctx.reason(SHORTS, "ListLikedShorts", None, &ListLikedShortsRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
}

#[tokio::test]
async fn the_engagement_domain_sees_a_short_as_its_viewer_does() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let short = ctx.short(&alice, &channel, "コメントしてね #感想").await;
    let subjects = &ctx.app.core.subjects;
    let resolve = |reference: String, viewer: Option<uuid::Uuid>| async move {
        subjects.resolve(SubjectType::SUBJECT_TYPE_SHORT, &reference, viewer).await
    };

    let info: SubjectInfo = resolve(short.short_id.clone(), Some(bob.id)).await.unwrap();
    assert_eq!((info.id, info.owner_id), (id(&short.id), alice.id));
    assert_eq!(
        (info.title.as_str(), info.path.as_str()),
        ("コメントしてね #感想", format!("/shorts/{}", short.short_id).as_str())
    );
    assert!(info.interactions_enabled && info.image_url.is_some());
    assert!(resolve(short.id.clone(), None).await.is_ok());

    // Comments off: the engagement domain's one switch.
    let off = UpdateShortRequest { short_id: short.id.clone(), comments_enabled: Some(false), ..Default::default() };
    ctx.update(&alice, &off).await.unwrap();
    assert!(!resolve(short.id.clone(), Some(bob.id)).await.unwrap().interactions_enabled);

    // What a viewer cannot open cannot be commented on, liked or reported by them.
    ctx.set_visibility(&alice, &short.id, Visibility::VISIBILITY_FRIENDS).await;
    assert_eq!(resolve(short.id.clone(), Some(bob.id)).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");
    assert_eq!(resolve(short.id.clone(), None).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");
    assert!(resolve(short.id.clone(), Some(alice.id)).await.is_ok());
    assert_eq!(resolve("nonsense!".to_owned(), Some(bob.id)).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");
}

#[tokio::test]
async fn a_share_counts_once_per_sharer_and_always_returns_the_link() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let channel = ctx.channel(&alice, "ch").await;
    let short = ctx.short(&alice, &channel, "広めてね").await;

    let first = share(&ctx, Some(&bob), &short.id).await.unwrap();
    assert_eq!((first.url.as_str(), first.share_count), (short.share_url.as_str(), 1));
    // Sharing a hundred times is sharing once.
    for _ in 0..5 {
        let again = share(&ctx, Some(&bob), &short.short_id).await.unwrap();
        assert_eq!((again.url.as_str(), again.share_count), (short.share_url.as_str(), 1));
    }
    assert_eq!(share(&ctx, Some(&carol), &short.id).await.unwrap().share_count, 2);
    // A visitor gets the link; without an account their share counts for nothing.
    let visitor = share(&ctx, None, &short.id).await.unwrap();
    assert_eq!((visitor.url.as_str(), visitor.share_count), (short.share_url.as_str(), 2));
    assert_eq!(
        share(&ctx, Some(&alice), &short.id).await.unwrap().share_count,
        2,
        "the owner's shares are not a signal"
    );
    assert_eq!(hourly(&ctx, &short.id).await.4, 2);
    assert_eq!(ctx.get(None, &short.id).await.unwrap().share_count, 2);

    let bad =
        ShareShortRequest { short_id: short.id.clone(), target: buffa::EnumValue::from(99), ..Default::default() };
    assert_eq!(ctx.reason(SHORTS, "ShareShort", Some(&bob), &bad).await, "SHARE_TARGET_INVALID");
    ctx.set_visibility(&alice, &short.id, Visibility::VISIBILITY_PRIVATE).await;
    assert_eq!(share(&ctx, Some(&bob), &short.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    assert_eq!(share(&ctx, None, "nonsense!").await.unwrap_err().reason, "SHORT_NOT_FOUND");
}

/// The range from `from` to `to` days ago, as `day` names them.
fn days<P: buffa::ProtoBox<DateRange>>(
    day: &impl Fn(u64) -> String,
    from: u64,
    to: u64,
) -> buffa::MessageField<DateRange, P> {
    buffa::MessageField::some(DateRange { start_date: day(from), end_date: day(to), ..Default::default() })
}

/// Writes what a stay leaves in the analytics, dated `days_ago` days back.
async fn watched_on(
    ctx: &Ctx,
    short: &Short,
    owner: &TestUser,
    viewer: &TestUser,
    days_ago: i64,
    ratio: f64,
    loops: i64,
) {
    let at = chrono::Utc::now() - chrono::Duration::days(days_ago);
    let channel_id = short.channel.as_option().unwrap().id.clone();
    let mut event = tokuly_core::analytics::AnalyticsEvent::new("short.watch")
        .user(Some(viewer.id))
        .subject("short", id(&short.id))
        .owner(owner.id)
        .value(ratio * 12.0)
        .props(serde_json::json!({ "channel_id": channel_id, "viewer": viewer.id, "source": 1, "ratio": ratio, "loops": loops }));
    event.at = at;
    ctx.app.core.analytics.write(vec![event]).await.expect("analytics write");
}

#[tokio::test]
async fn creators_see_their_numbers() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let stranger = ctx.app.user("stranger").await;
    let channel = ctx.channel(&alice, "ch").await;
    let short = ctx.short(&alice, &channel, "分析される動画").await;
    let quiet = ctx.short(&alice, &channel, "ほとんど見られない動画").await;
    let mut viewers = Vec::new();
    for n in 1..=5 {
        viewers.push(ctx.app.user(&format!("viewer{n}")).await);
    }
    use PlaybackSource::*;
    {
        let _lock = playback_lock().await;
        // Five accounts and a visitor; one of the accounts swipes on at once.
        let stays = [
            (Some(&viewers[0]), PLAYBACK_SOURCE_FOR_YOU, 24_000, 2),
            (Some(&viewers[1]), PLAYBACK_SOURCE_FOR_YOU, 6_000, 0),
            (Some(&viewers[2]), PLAYBACK_SOURCE_HASHTAG, 3_000, 0),
            (Some(&viewers[3]), PLAYBACK_SOURCE_PROFILE, 12_000, 1),
            (Some(&viewers[4]), PLAYBACK_SOURCE_SHARE_LINK, 500, 0),
            (None, PLAYBACK_SOURCE_SHARE_LINK, 12_000, 1),
        ];
        for (viewer, source, watched_ms, completions) in stays {
            ctx.report(viewer, vec![stay(&short.id, source, watched_ms, completions)]).await;
        }
        ctx.report(Some(&viewers[0]), vec![stay(&quiet.id, PLAYBACK_SOURCE_FRIENDS, 1_500, 0)]).await;
        ctx.report(Some(&viewers[1]), vec![stay(&quiet.id, PLAYBACK_SOURCE_FRIENDS, 12_000, 1)]).await;
        ctx.flush().await;
    }
    ctx.like(&viewers[0], &short.id).await;
    save(&ctx, &viewers[1], &short.id).await.unwrap();
    share(&ctx, Some(&viewers[2]), &short.id).await.unwrap();
    ctx.app
        .core
        .subjects
        .counter_changed(SubjectType::SUBJECT_TYPE_SHORT, id(&short.id), "comment_count", 1)
        .await
        .unwrap();
    // Subscribing right after watching is credited to the short.
    ctx.subscribe(&viewers[3], &channel, NotificationLevel::NOTIFICATION_LEVEL_PERSONALIZED).await;
    // Someone who never watched a short of the channel is nobody's credit.
    ctx.subscribe(&stranger, &channel, NotificationLevel::NOTIFICATION_LEVEL_PERSONALIZED).await;
    assert!(ctx.app.flush_analytics().await > 0);

    // Days are the owner's days: accounts are in Asia/Tokyo unless they say otherwise.
    let today = chrono::Utc::now().with_timezone(&chrono_tz::Asia::Tokyo).date_naive();
    let day = |ago: u64| (today - chrono::Days::new(ago)).to_string();

    let of_short = |short_id: &str, from: u64, to: u64| {
        let ctx = &ctx;
        let alice = &alice;
        let request = GetShortAnalyticsRequest {
            short_id: short_id.to_owned(),
            range: days(&day, from, to),
            ..Default::default()
        };
        async move {
            ctx.call::<_, GetShortAnalyticsResponse>(ANALYTICS, "GetShortAnalytics", Some(alice), &request)
                .await
                .unwrap()
        }
    };

    // ── Counts are live ──
    let report = of_short(&short.short_id, 2, 0).await;
    let totals = report.totals.as_option().unwrap().clone();
    assert_eq!((totals.views, totals.unique_viewers), (5, 5));
    assert_eq!((totals.likes, totals.comments, totals.shares, totals.saves, totals.followers_gained), (1, 1, 1, 1, 1));
    assert_eq!(report.daily.len(), 3);
    assert_eq!(report.daily.iter().map(|day| day.views).collect::<Vec<_>>(), [0, 0, 5]);
    assert_eq!(
        (report.daily[2].date.as_str(), report.daily[2].likes, report.daily[2].followers_gained),
        (day(0).as_str(), 1, 1)
    );
    let sources: Vec<(PlaybackSource, i64)> =
        report.traffic_sources.iter().map(|s| (s.source.as_known().unwrap(), s.views)).collect();
    assert_eq!(
        sources,
        [
            (PLAYBACK_SOURCE_FOR_YOU, 2),
            (PLAYBACK_SOURCE_PROFILE, 1),
            (PLAYBACK_SOURCE_HASHTAG, 1),
            (PLAYBACK_SOURCE_SHARE_LINK, 1)
        ]
    );
    // ── How people watched is not: today is not over, so nothing of it shows,
    //    although five accounts watched ──
    assert_eq!((totals.average_watch_ratio, totals.loops, totals.plays, totals.watch_time_seconds), (0.0, 0, 0, 0.0));

    // ── Finished days, in blocks of one short and one day ──
    // Yesterday five accounts watched `short`; the day before, four did;
    // yesterday one account (the fifth viewer) watched `quiet`.
    for (viewer, ratio, loops) in [(0, 1.0, 1), (1, 0.5, 0), (2, 0.25, 0), (3, 1.0, 0), (4, 0.26, 0)] {
        watched_on(&ctx, &short, &alice, &viewers[viewer], 1, ratio, loops).await;
    }
    for viewer in 0..4 {
        watched_on(&ctx, &short, &alice, &viewers[viewer], 2, 0.1, 3).await;
    }
    watched_on(&ctx, &quiet, &alice, &viewers[4], 1, 0.03, 0).await;

    let report = of_short(&short.id, 3, 0).await;
    let totals = report.totals.as_option().unwrap();
    // (1 + 0.5 + 0.25 + 1 + 0.26) / 5 = 0.602, reported to two decimals.
    assert_eq!((totals.average_watch_ratio, totals.plays, totals.loops), (0.6, 5, 1));
    assert_eq!(totals.watch_time_seconds, 36.0, "whole seconds");
    let watching: Vec<(f64, i64)> = report.daily.iter().map(|day| (day.average_watch_ratio, day.plays)).collect();
    assert_eq!(watching, [(0.0, 0), (0.0, 0), (0.6, 5), (0.0, 0)], "the day four accounts watched says nothing");

    // Cutting the range differently cannot isolate anyone: with or without
    // the four-account day, with or without today, the answer is the same.
    for (from, to) in [(1, 1), (2, 1), (1, 0), (30, 0)] {
        let cut = of_short(&short.id, from, to).await;
        let cut = cut.totals.as_option().unwrap();
        assert_eq!(
            (cut.average_watch_ratio, cut.plays, cut.loops, cut.watch_time_seconds),
            (0.6, 5, 1, 36.0),
            "{from}..{to}"
        );
    }
    let only_small_day = of_short(&short.id, 2, 2).await;
    assert_eq!(only_small_day.totals.as_option().unwrap().plays, 0);
    // One viewer's short: views, and not a word about how they watched.
    let lone = of_short(&quiet.id, 3, 0).await;
    let lone = lone.totals.as_option().unwrap();
    assert_eq!((lone.views, lone.plays, lone.average_watch_ratio, lone.watch_time_seconds), (2, 0, 0.0, 0.0));

    // The channel is the sum of its shorts' blocks: subtracting one short's
    // report from the channel's leaves nothing of the lone viewer either.
    let request = GetChannelShortsAnalyticsRequest {
        channel_id: channel.summary.handle.clone(),
        range: days(&day, 3, 0),
        ..Default::default()
    };
    let report: GetChannelShortsAnalyticsResponse =
        ctx.call(ANALYTICS, "GetChannelShortsAnalytics", Some(&alice), &request).await.unwrap();
    let totals = report.totals.as_option().unwrap();
    assert_eq!((totals.views, totals.unique_viewers, report.short_count), (7, 5, 2));
    assert_eq!((totals.average_watch_ratio, totals.plays, totals.loops, totals.watch_time_seconds), (0.6, 5, 1, 36.0));
    assert_eq!(report.top_shorts.len(), 2);
    assert_eq!(
        (report.top_shorts[0].short.as_option().unwrap().id.as_str(), report.top_shorts[0].views),
        (short.id.as_str(), 5)
    );
    assert_eq!(report.top_shorts[1].views, 2);
    // The studio view of a short carries no watching figures at all.
    let studio =
        serde_json::to_value(ctx.get(Some(&alice), &short.id).await.unwrap().studio.as_option().unwrap()).unwrap();
    assert!(studio.get("averageWatchRatio").is_none() && studio.get("loopCount").is_none(), "{studio}");

    // Only the owner asks, and only for sensible ranges.
    let request =
        GetShortAnalyticsRequest { short_id: short.id.clone(), range: days(&day, 1, 0), ..Default::default() };
    assert_eq!(ctx.reason(ANALYTICS, "GetShortAnalytics", Some(&stranger), &request).await, "SHORT_NOT_FOUND");
    assert_eq!(ctx.reason(ANALYTICS, "GetShortAnalytics", None, &request).await, "SIGN_IN_REQUIRED");
    let theirs = GetChannelShortsAnalyticsRequest {
        channel_id: channel.summary.id.clone(),
        range: days(&day, 1, 0),
        ..Default::default()
    };
    assert_eq!(ctx.reason(ANALYTICS, "GetChannelShortsAnalytics", Some(&stranger), &theirs).await, "CHANNEL_NOT_FOUND");
    let no_range = GetShortAnalyticsRequest { short_id: short.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(ANALYTICS, "GetShortAnalytics", Some(&alice), &no_range).await, "DATE_RANGE_INVALID");
    let request =
        GetShortAnalyticsRequest { short_id: short.id.clone(), range: days(&day, 0, 1), ..Default::default() };
    assert_eq!(ctx.reason(ANALYTICS, "GetShortAnalytics", Some(&alice), &request).await, "DATE_RANGE_INVALID");
}

#[tokio::test]
async fn visitors_and_brand_new_accounts_do_not_move_a_ranking() {
    let ctx = start_with(ShortsConfig { signal_min_account_age_hours: 24, ..test_config() }).await;
    let alice = ctx.app.user("alice").await;
    let newcomer = ctx.app.user("newcomer").await;
    let regular = ctx.app.user("regular").await;
    sqlx::query("UPDATE users SET created_at = now() - interval '3 days' WHERE id = ANY($1)")
        .bind(vec![alice.id, regular.id])
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let channel = ctx.channel(&alice, "ch").await;
    let short = ctx.short(&alice, &channel, "育てたい動画").await;
    let comment = |author: &TestUser| {
        serde_json::json!({
            "comment_id": uuid::Uuid::now_v7(), "subject_type": SubjectType::SUBJECT_TYPE_SHORT as i32,
            "subject_id": short.id, "author_id": author.id,
        })
    };
    let _lock = playback_lock().await;

    // Everything a creator could do to their own short logged out, or with
    // an account made for the purpose.
    assert_eq!(ctx.report(None, vec![stay(&short.id, FOR_YOU, 36_000, 3)]).await.views_counted, 1);
    assert_eq!(ctx.watch(&newcomer, &short.id).await.views_counted, 1);
    assert_eq!(share(&ctx, None, &short.id).await.unwrap().share_count, 0);
    assert_eq!(share(&ctx, Some(&newcomer), &short.id).await.unwrap().share_count, 0);
    assert_eq!(save(&ctx, &newcomer, &short.id).await.unwrap(), 1, "it is saved for them all the same");
    ctx.like(&newcomer, &short.id).await;
    ctx.deliver_event("engagement.comment.published", Some(newcomer.id), comment(&newcomer)).await;
    ctx.flush().await;
    let row = ctx.row(&short.id).await;
    assert_eq!((row.view_count, row.watch_samples, row.loop_count, row.share_count), (2, 0, 0, 0), "views are shown");
    assert_eq!(hourly(&ctx, &short.id).await, (0, 0, 0, 0, 0, 0), "and nothing reaches the ranking");
    ctx.shorts.run_ranking().await.unwrap();
    assert!((ctx.row(&short.id).await.score - 1.0).abs() < 0.01, "the short ranks as if nobody had come");

    // An account that has been around counts in full.
    assert_eq!(ctx.report(Some(&regular), vec![stay(&short.id, FOR_YOU, 36_000, 3)]).await.views_counted, 1);
    assert_eq!(share(&ctx, Some(&regular), &short.id).await.unwrap().share_count, 1);
    save(&ctx, &regular, &short.id).await.unwrap();
    ctx.like(&regular, &short.id).await;
    ctx.deliver_event("engagement.comment.published", Some(regular.id), comment(&regular)).await;
    ctx.flush().await;
    assert_eq!(hourly(&ctx, &short.id).await, (1, 2, 1, 1, 1, 1));
    let row = ctx.row(&short.id).await;
    assert_eq!((row.view_count, row.watch_samples, row.loop_count), (3, 1, 2));
    // The newcomer still gets a feed, and what they watched is remembered for them.
    assert!(viewer_state(&ctx.get(Some(&newcomer), &short.id).await.unwrap()).seen);
}
