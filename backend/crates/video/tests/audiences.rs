//! The social graph's audiences on videos: followers, close friends,
//! circles, and the open videos of locked accounts.
mod common;

use common::*;
use tokuly_proto::{common::Visibility, video::*};
use tokuly_testkit::TestUser;

/// Search documents written for `id`, oldest first (`None` = removed).
async fn search_docs(ctx: &Ctx, id: &str) -> Vec<Option<serde_json::Value>> {
    ctx.app
        .events("search.changed")
        .await
        .into_iter()
        .filter(|event| event.data["id"] == id)
        .map(|event| Some(event.data["doc"].clone()).filter(|doc| !doc.is_null()))
        .collect()
}

async fn channel_titles(ctx: &Ctx, viewer: Option<&TestUser>, channel: &Channel) -> Vec<String> {
    let request = ListChannelVideosRequest { channel_id: channel.summary.id.clone(), ..Default::default() };
    let listed: ListChannelVideosResponse = ctx.call(CHANNELS, "ListChannelVideos", viewer, &request).await.unwrap();
    listed.videos.iter().map(|video| video.title.clone()).collect()
}

async fn batch_titles(ctx: &Ctx, viewer: Option<&TestUser>, ids: &[String]) -> Vec<String> {
    let request = BatchGetVideosRequest { video_ids: ids.to_vec(), ..Default::default() };
    let batch: BatchGetVideosResponse = ctx.call(VIDEOS, "BatchGetVideos", viewer, &request).await.unwrap();
    batch.videos.iter().map(|video| video.title.clone()).collect()
}

#[tokio::test]
async fn followers_close_friends_and_circles_watch_what_is_shared_with_them() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let follower = ctx.app.user("follower").await;
    let friend = ctx.app.user("friend").await;
    let close = ctx.app.user("close").await;
    let member = ctx.app.user("member").await;
    let stranger = ctx.app.user("stranger").await;
    ctx.follow(&follower, &alice).await;
    ctx.befriend(&alice, &friend).await;
    ctx.follow(&close, &alice).await;
    ctx.follow(&member, &alice).await;
    let close_friends = ctx.close_friends_circle(&alice).await;
    ctx.add_to_circle(&alice, &close_friends, &[&close]).await;
    let circle = ctx.circle(&alice, "部活", &[&member]).await;
    let channel = ctx.channel(&alice, "ch").await;

    use Visibility::*;
    let followers = ctx.published(&alice, &channel, "followers", VISIBILITY_FOLLOWERS).await;
    let close_only = ctx.published(&alice, &channel, "close", VISIBILITY_CLOSE_FRIENDS).await;
    let circled = ctx
        .published_with(&alice, &channel, "circle", VISIBILITY_CIRCLE, |request| request.circle_id = circle.clone())
        .await;
    let ids: Vec<String> = [&followers, &close_only, &circled].iter().map(|video| card(video).id.clone()).collect();

    let viewers: [(&str, Option<&TestUser>, [bool; 3]); 7] = [
        ("anonymous", None, [false, false, false]),
        ("stranger", Some(&stranger), [false, false, false]),
        ("follower", Some(&follower), [true, false, false]),
        ("friend", Some(&friend), [true, false, false]),
        ("close friend", Some(&close), [true, true, false]),
        ("circle member", Some(&member), [true, false, true]),
        ("owner", Some(&alice), [true, true, true]),
    ];
    for (name, viewer, expected) in viewers {
        let mut can_open = Vec::new();
        for (video_id, expected) in ids.iter().zip(expected) {
            let result = ctx.get_video(viewer, video_id).await;
            assert_eq!(result.is_ok(), expected, "{name} opening {video_id}");
            if let Ok(video) = result {
                can_open.push(card(&video).title.clone());
            }
        }
        // The SQL of the batch lookup and of the channel page agree with the single lookup.
        assert_eq!(batch_titles(&ctx, viewer, &ids).await, can_open, "{name}: batch");
        let mut listed = channel_titles(&ctx, viewer, &channel).await;
        listed.reverse();
        assert_eq!(listed, can_open, "{name}: channel page");
    }
    // Members never learn about the circle: the video reads as FOLLOWERS to them.
    let seen = ctx.get_video(Some(&member), &ids[2]).await.unwrap();
    assert_eq!(card(&seen).visibility, VISIBILITY_FOLLOWERS);
    let own = ctx.get_video(Some(&alice), &ids[2]).await.unwrap();
    assert_eq!(card(&own).visibility, VISIBILITY_CIRCLE);
    assert_eq!(own.studio.circle_id, circle);
    let close_seen = ctx.get_video(Some(&close), &ids[1]).await.unwrap();
    assert_eq!(card(&close_seen).visibility, VISIBILITY_CLOSE_FRIENDS, "the close-friends mark");

    // A circle must be one of the owner's.
    let other = ctx.circle(&stranger, "よそ", &[]).await;
    let request = CreateVideoRequest {
        channel_id: channel.summary.id.clone(),
        asset_id: ctx.video_upload(&alice).await,
        visibility: VISIBILITY_CIRCLE.into(),
        circle_id: other,
        ..Default::default()
    };
    assert_eq!(ctx.reason(VIDEOS, "CreateVideo", Some(&alice), &request).await, "CIRCLE_NOT_FOUND");
    let request = CreateVideoRequest { circle_id: String::new(), asset_id: ctx.video_upload(&alice).await, ..request };
    assert_eq!(ctx.reason(VIDEOS, "CreateVideo", Some(&alice), &request).await, "CIRCLE_REQUIRED");
    // Nothing shared with an audience is in search.
    for video_id in &ids {
        assert!(search_docs(&ctx, video_id).await.iter().all(Option::is_none), "{video_id} is not searchable");
    }
}

#[tokio::test]
async fn narrowing_a_video_names_who_may_still_watch_it() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let friend = ctx.app.user("friend").await;
    let follower = ctx.app.user("follower").await;
    ctx.befriend(&alice, &friend).await;
    ctx.follow(&follower, &alice).await;
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.public_video(&alice, &channel, "みんなへ").await;
    let video_id = card(&video).id.clone();
    let change = |visibility: Visibility| UpdateVideoRequest {
        video_id: video_id.clone(),
        visibility: Some(visibility.into()),
        ..Default::default()
    };
    for visibility in [Visibility::VISIBILITY_FOLLOWERS, Visibility::VISIBILITY_FRIENDS, Visibility::VISIBILITY_PUBLIC]
    {
        let _: UpdateVideoResponse = ctx.call(VIDEOS, "UpdateVideo", Some(&alice), &change(visibility)).await.unwrap();
    }
    let events = ctx.app.events_by("video.video.visibility_changed", alice.id).await;
    let seen: Vec<(i64, serde_json::Value)> = events
        .iter()
        .map(|event| (event.data["visibility"].as_i64().unwrap(), event.data["visible_to"].clone()))
        .collect();
    let mut tied = vec![friend.id.to_string(), follower.id.to_string()];
    tied.sort();
    assert_eq!(
        seen,
        vec![
            (6, serde_json::json!(tied)),
            (4, serde_json::json!([friend.id.to_string()])),
            (1, serde_json::Value::Null),
        ]
    );
    assert!(events.iter().all(|event| event.data["changed_at"].is_string()));
}

/// What a viewer can tell about a locked account's channel from outside.
async fn what_one_learns(ctx: &Ctx, viewer: Option<&TestUser>, channel: &Channel) -> serde_json::Value {
    let page = ctx.get_channel(viewer, &channel.summary.id).await.unwrap();
    let subscribers = page.summary.as_option().map(|summary| summary.subscriber_count).unwrap_or_default();
    let request = ListChannelPlaylistsRequest { channel_id: channel.summary.id.clone(), ..Default::default() };
    let playlists: ListChannelPlaylistsResponse =
        ctx.call(PLAYLISTS, "ListChannelPlaylists", viewer, &request).await.unwrap();
    serde_json::json!({
        "video_count": page.video_count,
        "view_count": page.view_count,
        "trailer": page.trailer_video_id,
        "subscribers": subscribers,
        "videos": channel_titles(ctx, viewer, channel).await,
        "playlists": playlists.playlists.len(),
    })
}

#[tokio::test]
async fn a_stranger_learns_nothing_from_a_locked_accounts_channel() {
    let ctx = start().await;
    let stranger = ctx.app.user("stranger").await;
    let follower = ctx.app.user("follower").await;
    // Two worlds: a locked account that published, and an empty channel.
    let (posted, silent) = (ctx.app.user("posted").await, ctx.app.user("silent").await);
    let mut channels = Vec::new();
    // The other world is an account that is not locked and has a channel
    // with nothing on it.
    for world in [&posted, &silent] {
        ctx.follow(&follower, world).await;
        if world.id == posted.id {
            ctx.set_locked(world, true).await;
        }
        channels.push(ctx.channel(world, "鍵チャンネル").await);
    }
    let channel = &channels[0];
    let public = ctx.public_video(&posted, channel, "鍵アカの動画").await;
    let unlisted = ctx.published(&posted, channel, "リンク限定", Visibility::VISIBILITY_UNLISTED).await;
    let video_id = card(&public).id.clone();
    ctx.rate(&posted, &video_id, Rating::RATING_LIKE).await;
    let request = UpdateChannelRequest {
        channel_id: channel.summary.id.clone(),
        trailer_video_id: Some(video_id.clone()),
        ..Default::default()
    };
    let _: UpdateChannelResponse = ctx.call(CHANNELS, "UpdateChannel", Some(&posted), &request).await.unwrap();
    let playlist = CreatePlaylistRequest {
        title: "鍵リスト".into(),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        channel_id: channel.summary.id.clone(),
        video_id: video_id.clone(),
        ..Default::default()
    };
    let created: CreatePlaylistResponse =
        ctx.call(PLAYLISTS, "CreatePlaylist", Some(&posted), &playlist).await.unwrap();
    // A stranger subscribed to both channels.
    for world in &channels {
        ctx.subscribe(&stranger, &world.summary.id, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    }
    assert_eq!(card(&public).visibility, Visibility::VISIBILITY_PUBLIC, "still shared publicly");

    for viewer in [Some(&stranger), None] {
        let seen = what_one_learns(&ctx, viewer, &channels[0]).await;
        let other = what_one_learns(&ctx, viewer, &channels[1]).await;
        assert_eq!(seen, other, "the two worlds look the same from outside");
        for video in [&public, &unlisted] {
            assert_eq!(ctx.get_video(viewer, &card(video).id).await.unwrap_err().reason, "VIDEO_NOT_FOUND");
        }
        let request = GetPlaylistRequest { playlist_id: created.playlist.id.clone(), ..Default::default() };
        let error = ctx.call::<_, GetPlaylistResponse>(PLAYLISTS, "GetPlaylist", viewer, &request).await.unwrap_err();
        assert_eq!(error.reason, "PLAYLIST_NOT_FOUND");
    }
    let subscriptions: GetSubscriptionsFeedResponse = ctx
        .call(FEEDS, "GetSubscriptionsFeed", Some(&stranger), &GetSubscriptionsFeedRequest::default())
        .await
        .unwrap();
    assert!(subscriptions.videos.iter().all(|video| video.id != video_id), "not in a stranger's subscriptions");
    assert!(search_docs(&ctx, &video_id).await.iter().all(Option::is_none), "nothing in search");
    // Followers see it all.
    let page = what_one_learns(&ctx, Some(&follower), channel).await;
    assert_eq!(page["videos"], serde_json::json!(["鍵アカの動画"]));
    assert_eq!(page["trailer"], card(&public).short_id);
    assert!(ctx.get_video(Some(&follower), &card(&unlisted).id).await.is_ok());
}

#[tokio::test]
async fn locking_an_account_applies_at_the_next_read_and_withdraws_what_was_open() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let follower = ctx.app.user("follower").await;
    let stranger = ctx.app.user("stranger").await;
    ctx.follow(&follower, &alice).await;
    let channel = ctx.channel(&alice, "ch").await;
    ctx.subscribe(&follower, &channel.summary.id, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    ctx.subscribe(&stranger, &channel.summary.id, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    let video = ctx.public_video(&alice, &channel, "公開動画").await;
    let video_id = card(&video).id.clone();
    assert!(ctx.get_video(Some(&stranger), &video_id).await.is_ok());

    ctx.set_locked(&alice, true).await;
    // Before this domain hears of the lock, reads ask the social domain.
    assert_eq!(ctx.get_video(Some(&stranger), &video_id).await.unwrap_err().reason, "VIDEO_NOT_FOUND");
    assert_eq!(ctx.get_video(None, &video_id).await.unwrap_err().reason, "VIDEO_NOT_FOUND");
    assert!(channel_titles(&ctx, Some(&stranger), &channel).await.is_empty());
    assert!(batch_titles(&ctx, None, std::slice::from_ref(&video_id)).await.is_empty());
    assert!(ctx.get_video(Some(&follower), &video_id).await.is_ok());

    ctx.deliver_lock_change(&alice).await;
    let events = ctx.app.events_by("video.video.visibility_changed", alice.id).await;
    let last = events.last().expect("the lock changes the video's reach");
    assert_eq!(last.data["video_id"], video_id);
    assert_eq!(last.data["visibility"], 6, "a locked account's public video reaches followers");
    assert_eq!(last.data["visible_to"], serde_json::json!([follower.id.to_string()]), "subscribers who follow");
    assert!(search_docs(&ctx, &video_id).await.last().unwrap().is_none(), "out of search");
    // Delivering it again changes nothing.
    ctx.deliver_lock_change(&alice).await;
    assert_eq!(ctx.app.events_by("video.video.visibility_changed", alice.id).await.len(), events.len());

    // Unlocking opens the video once this domain hears of it (being late
    // here hides something for a moment; it never shows too much).
    ctx.set_locked(&alice, false).await;
    ctx.deliver_lock_change(&alice).await;
    assert!(ctx.get_video(Some(&stranger), &video_id).await.is_ok(), "unlocked: open again");
    let events = ctx.app.events_by("video.video.visibility_changed", alice.id).await;
    let last = events.last().unwrap();
    assert_eq!((last.data["visibility"].as_i64(), last.data["visible_to"].is_null()), (Some(1), true));
    assert!(search_docs(&ctx, &video_id).await.last().unwrap().is_some(), "back in search");
}

#[tokio::test]
async fn passes_of_one_lock_running_at_once_change_each_video_once() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    for title in ["一本目", "二本目", "三本目"] {
        ctx.public_video(&alice, &channel, title).await;
    }
    ctx.set_locked(&alice, true).await;
    let pass = || ctx.video.sync_owner_lock(alice.id);
    let (a, b, c, d) = tokio::join!(pass(), pass(), pass(), pass());
    let changed: u64 = [a, b, c, d].into_iter().map(|pass| pass.expect("a pass")).sum();
    assert_eq!(changed, 3, "each video changes once, whichever pass gets it");
    assert_eq!(ctx.app.events_by("video.video.visibility_changed", alice.id).await.len(), 3);
}

#[tokio::test]
async fn a_lock_never_widens_a_video_that_was_announced_to_nobody() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    let public = ctx.public_video(&alice, &channel, "公開").await;
    let unlisted = ctx.published(&alice, &channel, "限定", Visibility::VISIBILITY_UNLISTED).await;
    let no_feed = ctx.published(&alice, &channel, "フィード外", Visibility::VISIBILITY_NO_FEED).await;
    ctx.set_locked(&alice, true).await;
    ctx.deliver_lock_change(&alice).await;
    let events = ctx.app.events_by("video.video.visibility_changed", alice.id).await;
    let visibility_of = |video: &Video| {
        events
            .iter()
            .find(|event| event.data["video_id"] == card(video).id)
            .map(|event| event.data["visibility"].clone())
    };
    // The feed shows the owner's entries to friends for 6 (followers); an
    // unlisted or no-feed video's entry stays the owner's own.
    assert_eq!(visibility_of(&public), Some(serde_json::json!(6)));
    assert_eq!(visibility_of(&unlisted), Some(serde_json::json!(3)));
    assert_eq!(visibility_of(&no_feed), Some(serde_json::json!(2)));
}
