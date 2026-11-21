//! `PlaybackService`: view counting, watch time, resume positions and the
//! watch history.
mod common;

use std::time::Duration;

use common::*;
use tokuly_proto::{common::Visibility, video::*};
use tokuly_testkit::TestUser;

/// Long enough for a view to count under the test configuration (0.2 s).
const LONG_ENOUGH: Duration = Duration::from_millis(300);

async fn history(ctx: &Ctx, user: &TestUser) -> ListWatchHistoryResponse {
    ctx.call(PLAYBACK, "ListWatchHistory", Some(user), &ListWatchHistoryRequest::default()).await.expect("watch history")
}

fn history_titles(response: &ListWatchHistoryResponse) -> Vec<&str> {
    response.items.iter().map(|item| item.video.title.as_str()).collect()
}

#[tokio::test]
async fn a_view_counts_after_the_minimum_watch_time_and_once_per_viewer() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.public_video(&alice, &channel, "見てね").await;
    let video_id = card(&video).id.clone();

    let started = ctx.start_playback(Some(&bob), &card(&video).short_id, TrafficSource::TRAFFIC_SOURCE_HOME).await;
    assert_eq!((started.view_count, started.report_interval_seconds), (0, 10));
    assert!((started.min_view_seconds - 0.2).abs() < 1e-9 && started.resume_position_seconds == 0.0);
    tokio::time::sleep(LONG_ENOUGH).await;
    let counted = ctx.report(Some(&bob), &started.session_id, 0.5, true, false).await;
    assert_eq!((counted.view_counted, counted.view_count), (true, 1));
    // The session keeps reporting; the view is not counted again.
    tokio::time::sleep(LONG_ENOUGH).await;
    assert_eq!(ctx.report(Some(&bob), &started.session_id, 1.0, true, false).await.view_count, 1);

    // The same viewer again (a reload, another tab): still one view.
    let again = ctx.watch(Some(&bob), &video_id, 1.0, TrafficSource::TRAFFIC_SOURCE_DIRECT).await;
    assert_eq!((again.view_counted, again.view_count), (true, 1));
    // Someone else: a second view. Anonymous viewers are told apart by address (one address here).
    assert_eq!(ctx.watch(Some(&carol), &video_id, 1.0, TrafficSource::TRAFFIC_SOURCE_SEARCH).await.view_count, 2);
    assert_eq!(ctx.watch(None, &video_id, 1.0, TrafficSource::TRAFFIC_SOURCE_EXTERNAL).await.view_count, 3);
    assert_eq!(ctx.watch(None, &video_id, 1.0, TrafficSource::TRAFFIC_SOURCE_EXTERNAL).await.view_count, 3);
    // The owner watching their own video is not an audience.
    assert_eq!(ctx.watch(Some(&alice), &video_id, 1.0, TrafficSource::TRAFFIC_SOURCE_CHANNEL).await.view_count, 3);

    assert_eq!(card(&ctx.get_video(None, &video_id).await.unwrap()).view_count, 3);
    assert_eq!(ctx.get_channel(None, &channel.summary.id).await.unwrap().view_count, 3);
    // Views feed the trending score.
    tokuly_video::feeds::refresh_trending(&ctx.app.core, 24.0).await.unwrap();
    let score: f64 = sqlx::query_scalar("SELECT trending_score FROM videos WHERE id = $1").bind(id(&video_id)).fetch_one(&ctx.app.core.db).await.unwrap();
    assert!(score > 2.5 && score <= 3.0, "three fresh views: {score}");
}

#[tokio::test]
async fn only_playing_time_counts() {
    // A longer minimum than the other tests use, so that a slow machine
    // cannot turn "too early" into "long enough".
    let ctx = start_with(tokuly_video::VideoConfig { min_view_seconds: 0.6, ..test_config() }).await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.public_video(&alice, &channel, "一時停止").await;

    let started = ctx.start_playback(Some(&bob), &card(&video).id, TrafficSource::TRAFFIC_SOURCE_UNSPECIFIED).await;
    assert!((started.min_view_seconds - 0.6).abs() < 1e-9);
    // Reporting right away: not enough watching yet. The player is paused at once, then left open.
    let early = ctx.report(Some(&bob), &started.session_id, 0.0, false, false).await;
    assert_eq!((early.view_counted, early.view_count), (false, 0));
    tokio::time::sleep(Duration::from_millis(800)).await;
    let paused = ctx.report(Some(&bob), &started.session_id, 0.0, true, false).await;
    assert_eq!((paused.view_counted, paused.view_count), (false, 0), "a paused player watches nothing");
    // Now it plays.
    tokio::time::sleep(Duration::from_millis(700)).await;
    let played = ctx.report(Some(&bob), &started.session_id, 0.7, true, false).await;
    assert_eq!((played.view_counted, played.view_count), (true, 1));
    let watched = history(&ctx, &bob).await.items[0].watched_seconds;
    assert!((0.6..1.4).contains(&watched), "only the playing stretch is watch time: {watched}");
}

#[tokio::test]
async fn sessions_belong_to_whoever_opened_them() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.public_video(&alice, &channel, "v").await;
    let private = ctx.published(&alice, &channel, "private", Visibility::VISIBILITY_PRIVATE).await;

    let started = ctx.start_playback(Some(&bob), &card(&video).id, TrafficSource::TRAFFIC_SOURCE_DIRECT).await;
    let report = |session_id: &str| ReportPlaybackRequest { session_id: session_id.to_owned(), position_seconds: 1.0, playing: true, ..Default::default() };
    assert_eq!(ctx.reason(PLAYBACK, "ReportPlayback", Some(&alice), &report(&started.session_id)).await, "PLAYBACK_SESSION_NOT_FOUND");
    assert_eq!(ctx.reason(PLAYBACK, "ReportPlayback", None, &report(&started.session_id)).await, "PLAYBACK_SESSION_NOT_FOUND");
    for unknown in ["", "nope", "aaaaaaaaaaaaaaaaaaaaaaaa", "has spaces in it"] {
        assert_eq!(ctx.reason(PLAYBACK, "ReportPlayback", Some(&bob), &report(unknown)).await, "PLAYBACK_SESSION_NOT_FOUND", "{unknown:?}");
    }
    // An anonymous session is a bearer token: whoever holds it reports.
    let anonymous = ctx.start_playback(None, &card(&video).id, TrafficSource::TRAFFIC_SOURCE_DIRECT).await;
    assert!(ctx.call::<_, ReportPlaybackResponse>(PLAYBACK, "ReportPlayback", None, &report(&anonymous.session_id)).await.is_ok());

    let hidden = StartPlaybackRequest { video_id: card(&private).id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(PLAYBACK, "StartPlayback", Some(&bob), &hidden).await, "VIDEO_NOT_FOUND");
    assert_eq!(ctx.reason(PLAYBACK, "StartPlayback", None, &hidden).await, "VIDEO_NOT_FOUND");
    let odd = StartPlaybackRequest { video_id: card(&video).id.clone(), source: buffa::EnumValue::from(99), ..Default::default() };
    assert_eq!(ctx.reason(PLAYBACK, "StartPlayback", Some(&bob), &odd).await, "SOURCE_INVALID");

    // A session of a video that was deleted meanwhile is over.
    let _: DeleteVideoResponse = ctx.call(VIDEOS, "DeleteVideo", Some(&alice), &DeleteVideoRequest { video_id: card(&video).id.clone(), ..Default::default() }).await.unwrap();
    assert_eq!(ctx.reason(PLAYBACK, "ReportPlayback", Some(&bob), &report(&started.session_id)).await, "PLAYBACK_SESSION_NOT_FOUND");
}

#[tokio::test]
async fn the_resume_position_follows_the_viewer() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.public_video(&alice, &channel, "つづきから").await;
    let video_id = card(&video).id.clone();

    // Left at second 5 of 12.
    ctx.watch(Some(&bob), &video_id, 5.0, TrafficSource::TRAFFIC_SOURCE_HOME).await;
    let seen = ctx.get_video(Some(&bob), &video_id).await.unwrap();
    assert_eq!((card(&seen).resume_position_seconds, card(&seen).watched), (5.0, false));
    let resumed = ctx.start_playback(Some(&bob), &video_id, TrafficSource::TRAFFIC_SOURCE_HISTORY).await;
    assert_eq!(resumed.resume_position_seconds, 5.0);
    // Positions belong to the viewer: others (and anonymous viewers) start from the beginning.
    assert_eq!(card(&ctx.get_video(Some(&alice), &video_id).await.unwrap()).resume_position_seconds, 0.0);
    assert_eq!(ctx.start_playback(None, &video_id, TrafficSource::TRAFFIC_SOURCE_DIRECT).await.resume_position_seconds, 0.0);

    // Watched to the end: next time it starts over, and cards show it as watched.
    tokio::time::sleep(LONG_ENOUGH).await;
    ctx.report(Some(&bob), &resumed.session_id, 12.0, false, true).await;
    let seen = ctx.get_video(Some(&bob), &video_id).await.unwrap();
    assert_eq!((card(&seen).resume_position_seconds, card(&seen).watched), (0.0, true));
    assert_eq!(ctx.start_playback(Some(&bob), &video_id, TrafficSource::TRAFFIC_SOURCE_DIRECT).await.resume_position_seconds, 0.0);
    // Nearly the end counts as the end, too.
    let other = ctx.public_video(&alice, &channel, "ほぼ最後まで").await;
    ctx.watch(Some(&bob), &card(&other).id, 11.6, TrafficSource::TRAFFIC_SOURCE_DIRECT).await;
    assert!(card(&ctx.get_video(Some(&bob), &card(&other).id).await.unwrap()).watched);
}

#[tokio::test]
async fn the_watch_history_can_be_listed_pruned_cleared_and_paused() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let mut videos = Vec::new();
    for title in ["one", "two", "three"] {
        let video = ctx.public_video(&alice, &channel, title).await;
        ctx.watch(Some(&bob), &card(&video).id, 3.0, TrafficSource::TRAFFIC_SOURCE_HOME).await;
        videos.push(video);
    }

    let listed = history(&ctx, &bob).await;
    assert_eq!(history_titles(&listed), vec!["three", "two", "one"], "most recently watched first");
    assert!(!listed.paused && listed.items.iter().all(|item| item.watched_seconds > 0.2 && item.last_watched_at.as_option().is_some()));
    assert_eq!(listed.items[0].video.resume_position_seconds, 3.0);
    // Watching something again moves it to the top.
    ctx.watch(Some(&bob), &card(&videos[0]).id, 4.0, TrafficSource::TRAFFIC_SOURCE_HISTORY).await;
    assert_eq!(history_titles(&history(&ctx, &bob).await), vec!["one", "three", "two"]);

    let first: ListWatchHistoryResponse = ctx.call(PLAYBACK, "ListWatchHistory", Some(&bob), &ListWatchHistoryRequest { page: page(2, ""), ..Default::default() }).await.unwrap();
    let info = first.page.as_option().unwrap();
    assert!(info.has_more && first.items.len() == 2);
    let rest: ListWatchHistoryResponse = ctx.call(PLAYBACK, "ListWatchHistory", Some(&bob), &ListWatchHistoryRequest { page: page(2, &info.next_cursor), ..Default::default() }).await.unwrap();
    assert_eq!(history_titles(&rest), vec!["two"]);
    assert!(history(&ctx, &alice).await.items.is_empty(), "histories are personal");

    // A video that became private drops out of the list (and comes back with it).
    let hide = |visibility: Visibility| UpdateVideoRequest { video_id: card(&videos[2]).id.clone(), visibility: Some(visibility.into()), ..Default::default() };
    let _: UpdateVideoResponse = ctx.call(VIDEOS, "UpdateVideo", Some(&alice), &hide(Visibility::VISIBILITY_PRIVATE)).await.unwrap();
    assert_eq!(history_titles(&history(&ctx, &bob).await), vec!["one", "two"]);
    let _: UpdateVideoResponse = ctx.call(VIDEOS, "UpdateVideo", Some(&alice), &hide(Visibility::VISIBILITY_PUBLIC)).await.unwrap();

    // Removing one entry forgets its resume position as well.
    let remove = RemoveWatchHistoryItemRequest { video_id: card(&videos[0]).short_id.clone(), ..Default::default() };
    let _: RemoveWatchHistoryItemResponse = ctx.call(PLAYBACK, "RemoveWatchHistoryItem", Some(&bob), &remove).await.unwrap();
    assert_eq!(history_titles(&history(&ctx, &bob).await), vec!["three", "two"]);
    assert_eq!(ctx.start_playback(Some(&bob), &card(&videos[0]).id, TrafficSource::TRAFFIC_SOURCE_DIRECT).await.resume_position_seconds, 0.0);
    let _: RemoveWatchHistoryItemResponse = ctx.call(PLAYBACK, "RemoveWatchHistoryItem", Some(&bob), &remove).await.unwrap();

    // While paused nothing is recorded and positions stay where they were.
    let paused: SetWatchHistoryPausedResponse = ctx.call(PLAYBACK, "SetWatchHistoryPaused", Some(&bob), &SetWatchHistoryPausedRequest { paused: true, ..Default::default() }).await.unwrap();
    assert!(paused.paused);
    let settings: GetWatchHistorySettingsResponse = ctx.call(PLAYBACK, "GetWatchHistorySettings", Some(&bob), &GetWatchHistorySettingsRequest::default()).await.unwrap();
    assert!(settings.paused);
    let _: ClearWatchHistoryResponse = ctx.call(PLAYBACK, "ClearWatchHistory", Some(&bob), &ClearWatchHistoryRequest::default()).await.unwrap();
    let secret = ctx.public_video(&alice, &channel, "ないしょ").await;
    let watched = ctx.watch(Some(&bob), &card(&secret).id, 6.0, TrafficSource::TRAFFIC_SOURCE_DIRECT).await;
    assert!(watched.view_counted, "views still count while the history is paused");
    let listed = history(&ctx, &bob).await;
    assert!(listed.items.is_empty() && listed.paused);
    assert_eq!(card(&ctx.get_video(Some(&bob), &card(&secret).id).await.unwrap()).resume_position_seconds, 0.0);
    // Recording again.
    let _: SetWatchHistoryPausedResponse = ctx.call(PLAYBACK, "SetWatchHistoryPaused", Some(&bob), &SetWatchHistoryPausedRequest { paused: false, ..Default::default() }).await.unwrap();
    ctx.watch(Some(&bob), &card(&secret).id, 7.0, TrafficSource::TRAFFIC_SOURCE_DIRECT).await;
    assert_eq!(history_titles(&history(&ctx, &bob).await), vec!["ないしょ"]);

    assert_eq!(ctx.reason(PLAYBACK, "ListWatchHistory", None, &ListWatchHistoryRequest::default()).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(PLAYBACK, "RemoveWatchHistoryItem", None, &remove).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(PLAYBACK, "ClearWatchHistory", None, &ClearWatchHistoryRequest::default()).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(PLAYBACK, "GetWatchHistorySettings", None, &GetWatchHistorySettingsRequest::default()).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(PLAYBACK, "SetWatchHistoryPaused", None, &SetWatchHistoryPausedRequest::default()).await, "SIGN_IN_REQUIRED");
}
