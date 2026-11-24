//! `StudioAnalyticsService`: creator analytics computed from the ClickHouse
//! `events` table. Tests produce real activity through the RPCs, flush the
//! tracked events into ClickHouse and read the reports back.
mod common;

use std::time::Duration;

use common::*;
use tokuly_proto::video::*;
use tokuly_testkit::TestUser;

/// Yesterday to tomorrow in Tokyo: always contains "now", whatever the hour.
fn around_today() -> Option<DateRange> {
    let today = chrono::Utc::now().with_timezone(&chrono_tz::Asia::Tokyo).date_naive();
    Some(DateRange {
        start_date: today.pred_opt().unwrap().to_string(),
        end_date: today.succ_opt().unwrap().to_string(),
        time_zone: "Asia/Tokyo".into(),
        ..Default::default()
    })
}

/// A session that plays `steps` stretches of 2.4 s (a fifth of the 12 s
/// fixture each), reporting after every one.
async fn watch_fifths(ctx: &Ctx, viewer: Option<&TestUser>, video_id: &str, source: TrafficSource, steps: u32) {
    let started = ctx.start_playback(viewer, video_id, source).await;
    for step in 1..=steps {
        tokio::time::sleep(Duration::from_millis(300)).await;
        ctx.report(viewer, &started.session_id, 2.4 * f64::from(step), true, false).await;
    }
}

#[tokio::test]
async fn channel_and_video_reports_add_up() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let channel = ctx.channel(&alice, "ch").await;
    let channel_id = channel.summary.id.clone();
    let first = ctx.public_video(&alice, &channel, "first").await;
    let second = ctx.public_video(&alice, &channel, "second").await;
    let (first_id, second_id) = (card(&first).id.clone(), card(&second).id.clone());

    // first: bob watches two fifths (from search), carol one fifth (from home). second: bob one fifth.
    watch_fifths(&ctx, Some(&bob), &first_id, TrafficSource::TRAFFIC_SOURCE_SEARCH, 2).await;
    watch_fifths(&ctx, Some(&carol), &first_id, TrafficSource::TRAFFIC_SOURCE_HOME, 1).await;
    watch_fifths(&ctx, Some(&bob), &second_id, TrafficSource::TRAFFIC_SOURCE_HOME, 1).await;
    // The owner checking their own video is not audience.
    watch_fifths(&ctx, Some(&alice), &first_id, TrafficSource::TRAFFIC_SOURCE_CHANNEL, 1).await;
    // Ratings and subscriptions.
    ctx.rate(&bob, &first_id, Rating::RATING_LIKE).await;
    ctx.rate(&carol, &first_id, Rating::RATING_DISLIKE).await;
    ctx.rate(&bob, &second_id, Rating::RATING_LIKE).await;
    let subscribe = SubscribeRequest {
        channel_id: channel_id.clone(),
        source_video_id: card(&first).short_id.clone(),
        ..Default::default()
    };
    let _: SubscribeResponse = ctx.call(CHANNELS, "Subscribe", Some(&bob), &subscribe).await.unwrap();
    ctx.subscribe(&carol, &channel_id, NotificationLevel::NOTIFICATION_LEVEL_UNSPECIFIED).await;
    let _: UnsubscribeResponse = ctx
        .call(
            CHANNELS,
            "Unsubscribe",
            Some(&carol),
            &UnsubscribeRequest { channel_id: channel_id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(ctx.app.flush_analytics().await > 0);

    let request = GetChannelAnalyticsRequest {
        channel_id: channel_id.clone(),
        range: around_today().into(),
        ..Default::default()
    };
    let report: GetChannelAnalyticsResponse =
        ctx.call(ANALYTICS, "GetChannelAnalytics", Some(&alice), &request).await.unwrap();
    let totals = report.totals.as_option().unwrap();
    assert_eq!((totals.views, totals.unique_viewers), (3, 2), "three counted views by two people");
    assert_eq!((totals.likes, totals.dislikes, totals.subscribers_gained, totals.subscribers_lost), (2, 1, 2, 1));
    assert!(
        totals.watch_time_seconds > 1.0 && totals.watch_time_seconds < 6.0,
        "four stretches of about 0.3 s: {}",
        totals.watch_time_seconds
    );
    assert!((totals.average_view_duration_seconds - totals.watch_time_seconds / 3.0).abs() < 1e-9);
    assert_eq!(report.subscriber_count, 1);

    // One entry per day of the range, adding up to the totals.
    assert_eq!(report.daily.len(), 3);
    assert_eq!(report.daily.iter().map(|day| day.views).sum::<i64>(), 3);
    assert_eq!(report.daily.iter().map(|day| day.subscribers_gained).sum::<i64>(), 2);
    assert!(
        (report.daily.iter().map(|day| day.watch_time_seconds).sum::<f64>() - totals.watch_time_seconds).abs() < 1e-6
    );
    assert!(report.daily.windows(2).all(|pair| pair[0].date < pair[1].date));

    let sources: Vec<(TrafficSource, i64)> =
        report.traffic_sources.iter().map(|s| (s.source.as_known().unwrap(), s.views)).collect();
    assert_eq!(sources, vec![(TrafficSource::TRAFFIC_SOURCE_HOME, 2), (TrafficSource::TRAFFIC_SOURCE_SEARCH, 1)]);
    assert!(report.traffic_sources.iter().all(|source| source.watch_time_seconds > 0.0));
    let top: Vec<(&str, i64)> =
        report.top_videos.iter().map(|stats| (stats.video.title.as_str(), stats.views)).collect();
    assert_eq!(top, vec![("first", 2), ("second", 1)]);

    // The same for one video, with its audience retention.
    let request = GetVideoAnalyticsRequest {
        video_id: card(&first).short_id.clone(),
        range: around_today().into(),
        ..Default::default()
    };
    let report: GetVideoAnalyticsResponse =
        ctx.call(ANALYTICS, "GetVideoAnalytics", Some(&alice), &request).await.unwrap();
    let totals = report.totals.as_option().unwrap();
    assert_eq!((totals.views, totals.unique_viewers, totals.likes, totals.dislikes), (2, 2, 1, 1));
    assert_eq!((totals.subscribers_gained, totals.subscribers_lost), (1, 0), "bob subscribed from this video's page");
    assert_eq!(report.daily.iter().map(|day| day.unique_viewers).max(), Some(2));
    assert_eq!(report.traffic_sources.len(), 2);
    // Both sessions watched the first fifth, one went on through the second, nobody saw the rest.
    assert_eq!(report.retention.len(), 100);
    assert_eq!((report.retention[0], report.retention[19]), (1.0, 1.0));
    assert_eq!((report.retention[20], report.retention[39]), (0.5, 0.5));
    assert_eq!((report.retention[40], report.retention[99]), (0.0, 0.0));
    // A video nobody watched has no curve.
    let unwatched = ctx.public_video(&alice, &channel, "unwatched").await;
    let request = GetVideoAnalyticsRequest {
        video_id: card(&unwatched).id.clone(),
        range: around_today().into(),
        ..Default::default()
    };
    let report: GetVideoAnalyticsResponse =
        ctx.call(ANALYTICS, "GetVideoAnalytics", Some(&alice), &request).await.unwrap();
    assert!(report.retention.is_empty() && report.totals.views == 0 && report.daily.len() == 3);

    // A range before any of this happened is empty.
    let last_year =
        Some(DateRange { start_date: "2025-01-01".into(), end_date: "2025-01-31".into(), ..Default::default() });
    let request =
        GetChannelAnalyticsRequest { channel_id: channel_id.clone(), range: last_year.into(), ..Default::default() };
    let report: GetChannelAnalyticsResponse =
        ctx.call(ANALYTICS, "GetChannelAnalytics", Some(&alice), &request).await.unwrap();
    assert_eq!((report.totals.views, report.daily.len(), report.top_videos.len()), (0, 31, 0));
    assert_eq!(report.daily[0].date, "2025-01-01");
}

#[tokio::test]
async fn realtime_counters_cover_the_last_two_days_and_the_last_hour() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let channel = ctx.channel(&alice, "ch").await;
    let busy = ctx.public_video(&alice, &channel, "busy").await;
    let calm = ctx.public_video(&alice, &channel, "calm").await;
    for viewer in [&bob, &carol] {
        ctx.watch(Some(viewer), &card(&busy).id, 1.0, TrafficSource::TRAFFIC_SOURCE_HOME).await;
    }
    ctx.watch(None, &card(&busy).id, 1.0, TrafficSource::TRAFFIC_SOURCE_EXTERNAL).await;
    ctx.watch(Some(&bob), &card(&calm).id, 1.0, TrafficSource::TRAFFIC_SOURCE_HOME).await;
    ctx.app.flush_analytics().await;

    let target = |target: get_realtime_analytics_request::Target| GetRealtimeAnalyticsRequest {
        target: Some(target),
        ..Default::default()
    };
    let by_channel = target(get_realtime_analytics_request::Target::ChannelId(channel.summary.handle.clone()));
    let report: GetRealtimeAnalyticsResponse =
        ctx.call(ANALYTICS, "GetRealtimeAnalytics", Some(&alice), &by_channel).await.unwrap();
    assert_eq!((report.views_per_hour.len(), report.views_per_minute.len()), (48, 60));
    assert_eq!((report.views_last_48_hours, report.views_last_60_minutes), (4, 4));
    // The views just happened: they sit at the recent end of both series.
    assert_eq!(report.views_per_hour[46..].iter().sum::<i64>(), 4);
    assert_eq!(report.views_per_minute[55..].iter().sum::<i64>(), 4);
    let top: Vec<(&str, i64)> =
        report.top_videos.iter().map(|stats| (stats.video.title.as_str(), stats.views)).collect();
    assert_eq!(top, vec![("busy", 3), ("calm", 1)]);

    let by_video = target(get_realtime_analytics_request::Target::VideoId(card(&calm).id.clone()));
    let report: GetRealtimeAnalyticsResponse =
        ctx.call(ANALYTICS, "GetRealtimeAnalytics", Some(&alice), &by_video).await.unwrap();
    assert_eq!((report.views_last_48_hours, report.views_last_60_minutes, report.top_videos.len()), (1, 1, 0));

    assert_eq!(
        ctx.reason(ANALYTICS, "GetRealtimeAnalytics", Some(&alice), &GetRealtimeAnalyticsRequest::default()).await,
        "TARGET_REQUIRED"
    );
    assert_eq!(ctx.reason(ANALYTICS, "GetRealtimeAnalytics", Some(&bob), &by_channel).await, "CHANNEL_NOT_FOUND");
    assert_eq!(ctx.reason(ANALYTICS, "GetRealtimeAnalytics", Some(&bob), &by_video).await, "VIDEO_NOT_FOUND");
    assert_eq!(ctx.reason(ANALYTICS, "GetRealtimeAnalytics", None, &by_channel).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn analytics_are_for_the_owner_and_need_a_sane_range() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.public_video(&alice, &channel, "v").await;
    let of_channel = |range: Option<DateRange>| GetChannelAnalyticsRequest {
        channel_id: channel.summary.id.clone(),
        range: range.into(),
        ..Default::default()
    };
    let of_video = |range: Option<DateRange>| GetVideoAnalyticsRequest {
        video_id: card(&video).id.clone(),
        range: range.into(),
        ..Default::default()
    };

    // Even for public content, the numbers are the creator's.
    assert_eq!(
        ctx.reason(ANALYTICS, "GetChannelAnalytics", Some(&bob), &of_channel(around_today())).await,
        "CHANNEL_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(ANALYTICS, "GetVideoAnalytics", Some(&bob), &of_video(around_today())).await,
        "VIDEO_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(ANALYTICS, "GetChannelAnalytics", None, &of_channel(around_today())).await,
        "SIGN_IN_REQUIRED"
    );
    assert_eq!(ctx.reason(ANALYTICS, "GetVideoAnalytics", None, &of_video(around_today())).await, "SIGN_IN_REQUIRED");

    let range = |start: &str, end: &str, zone: &str| {
        Some(DateRange { start_date: start.into(), end_date: end.into(), time_zone: zone.into(), ..Default::default() })
    };
    for (bad, reason) in [
        (None, "DATE_RANGE_INVALID"),
        (range("2026-11-10", "2026-11-01", ""), "DATE_RANGE_INVALID"),
        (range("2026-11-10", "someday", ""), "DATE_RANGE_INVALID"),
        (range("2024-01-01", "2026-01-01", ""), "DATE_RANGE_INVALID"),
        (range("2026-11-01", "2026-11-10", "Mars/Olympus"), "TIME_ZONE_INVALID"),
    ] {
        assert_eq!(ctx.reason(ANALYTICS, "GetChannelAnalytics", Some(&alice), &of_channel(bad.clone())).await, reason);
        assert_eq!(ctx.reason(ANALYTICS, "GetVideoAnalytics", Some(&alice), &of_video(bad)).await, reason);
    }
    // A channel with no activity reports zeros, not an error.
    let report: GetChannelAnalyticsResponse = ctx
        .call(ANALYTICS, "GetChannelAnalytics", Some(&alice), &of_channel(range("2026-11-01", "2026-11-07", "UTC")))
        .await
        .unwrap();
    assert_eq!((report.totals.views, report.daily.len(), report.traffic_sources.len()), (0, 7, 0));
}

#[tokio::test]
async fn an_audience_of_one_is_not_broken_down() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let channel = ctx.channel(&alice, "ch").await;
    // Shared with one friend: whatever the curve shows is what bob did.
    let video = ctx.published(&alice, &channel, "for bob", tokuly_proto::common::Visibility::VISIBILITY_UNLISTED).await;
    watch_fifths(&ctx, Some(&bob), &card(&video).id, TrafficSource::TRAFFIC_SOURCE_DIRECT, 2).await;
    assert!(ctx.app.flush_analytics().await > 0);

    let request = GetVideoAnalyticsRequest {
        video_id: card(&video).id.clone(),
        range: around_today().into(),
        ..Default::default()
    };
    let report: GetVideoAnalyticsResponse =
        ctx.call(ANALYTICS, "GetVideoAnalytics", Some(&alice), &request).await.unwrap();
    let totals = report.totals.as_option().unwrap();
    assert_eq!((totals.views, totals.unique_viewers), (1, 1), "the totals stay");
    assert!(report.retention.is_empty(), "no retention curve of a single viewer");
    assert!(report.traffic_sources.is_empty(), "no word on where a single viewer came from");

    let request = GetChannelAnalyticsRequest {
        channel_id: channel.summary.id.clone(),
        range: around_today().into(),
        ..Default::default()
    };
    let report: GetChannelAnalyticsResponse =
        ctx.call(ANALYTICS, "GetChannelAnalytics", Some(&alice), &request).await.unwrap();
    assert_eq!(report.totals.as_option().unwrap().views, 1);
    assert!(report.traffic_sources.is_empty());
}
