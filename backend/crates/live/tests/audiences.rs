//! The social graph's audiences on live streams: followers, close friends,
//! circles, and the open streams of locked accounts.
mod common;

use common::*;
use tokuly_live::{access, hooks::AuthRequest};
use tokuly_proto::{common::Visibility, live::*, video::Channel};
use tokuly_testkit::TestUser;
use tokuly_video::access::Viewer;

async fn channel_streams(ctx: &Ctx, viewer: Option<&TestUser>, channel: &Channel) -> Vec<String> {
    let request = ListChannelStreamsRequest { channel_id: channel.summary.id.clone(), ..Default::default() };
    let listed: ListChannelStreamsResponse = ctx.call(STREAMS, "ListChannelStreams", viewer, &request).await.unwrap();
    listed.streams.into_iter().map(|stream| stream.title).collect()
}

async fn live_now(ctx: &Ctx, viewer: Option<&TestUser>, channel: &Channel) -> Vec<String> {
    let listed: ListLiveStreamsResponse =
        ctx.call(STREAMS, "ListLiveStreams", viewer, &ListLiveStreamsRequest::default()).await.unwrap();
    listed
        .streams
        .into_iter()
        .filter(|stream| stream.channel.id == channel.summary.id)
        .map(|stream| stream.title)
        .collect()
}

fn read(stream: &Stream) -> AuthRequest {
    AuthRequest {
        action: "read".into(),
        path: format!("live/{}", card(stream).short_id),
        protocol: "hls".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn followers_close_friends_and_circles_watch_what_is_shared_with_them() {
    let ctx = start().await;
    let owner = ctx.app.user("owner").await;
    let follower = ctx.app.user("follower").await;
    let friend = ctx.app.user("friend").await;
    let close = ctx.app.user("close").await;
    let member = ctx.app.user("member").await;
    let stranger = ctx.app.user("stranger").await;
    ctx.follow(&follower, &owner).await;
    ctx.befriend(&owner, &friend).await;
    ctx.follow(&close, &owner).await;
    ctx.follow(&member, &owner).await;
    let close_friends = ctx.close_friends_circle(&owner).await;
    ctx.add_to_circle(&owner, &close_friends, &[&close]).await;
    let circle = ctx.circle(&owner, "部活", &[&member]).await;
    let channel = ctx.channel(&owner, "ch").await;

    use Visibility::*;
    let mut streams = Vec::new();
    for visibility in [VISIBILITY_FOLLOWERS, VISIBILITY_CLOSE_FRIENDS, VISIBILITY_CIRCLE] {
        let stream = ctx
            .stream_with(&owner, &channel, &format!("{visibility:?}"), |request| {
                request.visibility = visibility.into();
                if visibility == VISIBILITY_CIRCLE {
                    request.circle_id = circle.clone();
                }
            })
            .await;
        streams.push(stream);
    }
    let viewers: [(&str, Option<&TestUser>, [bool; 3]); 6] = [
        ("anonymous", None, [false, false, false]),
        ("stranger", Some(&stranger), [false, false, false]),
        ("follower", Some(&follower), [true, false, false]),
        ("friend", Some(&friend), [true, false, false]),
        ("close friend", Some(&close), [true, true, false]),
        ("circle member", Some(&member), [true, false, true]),
    ];
    for (name, user, expected) in viewers {
        let viewer = Viewer::load(&ctx.app.core, user.map(|user| user.id)).await.unwrap();
        let listed = channel_streams(&ctx, user, &channel).await;
        for (stream, expected) in streams.iter().zip(expected) {
            let facts = ctx.row(stream).await.facts();
            assert_eq!(
                ctx.get_stream(user, &card(stream).id).await.is_ok(),
                expected,
                "{name}: {}",
                card(stream).title
            );
            assert_eq!(access::can_watch(&viewer, &facts), expected, "{name}: the rule in code");
            assert_eq!(listed.contains(&card(stream).title), expected, "{name}: the channel page agrees");
        }
    }
    let seen = ctx.get_stream(Some(&member), &card(&streams[2]).id).await.unwrap();
    assert_eq!(card(&seen).visibility, buffa::EnumValue::from(VISIBILITY_FOLLOWERS), "the circle stays unnamed");
    let own = ctx.get_stream(Some(&owner), &card(&streams[2]).id).await.unwrap();
    assert_eq!(
        (card(&own).visibility, own.studio.circle_id.clone()),
        (buffa::EnumValue::from(VISIBILITY_CIRCLE), circle)
    );

    // Narrowing a stream names who may still watch it.
    let request = UpdateStreamRequest {
        stream_id: card(&streams[0]).id.clone(),
        visibility: Some(VISIBILITY_FRIENDS.into()),
        ..Default::default()
    };
    let _: UpdateStreamResponse = ctx.call(STREAMS, "UpdateStream", Some(&owner), &request).await.unwrap();
    let last = ctx.app.events_by("live.stream.visibility_changed", owner.id).await.pop().expect("announced");
    assert_eq!(last.data["stream_id"], card(&streams[0]).id);
    assert_eq!(last.data["visibility"], 4);
    assert_eq!(last.data["visible_to"], serde_json::json!([friend.id.to_string()]));
}

/// What a viewer can tell about a locked account's channel from outside.
async fn what_one_learns(ctx: &Ctx, viewer: Option<&TestUser>, channel: &Channel) -> serde_json::Value {
    let upcoming: ListUpcomingStreamsResponse =
        ctx.call(STREAMS, "ListUpcomingStreams", viewer, &ListUpcomingStreamsRequest::default()).await.unwrap();
    let upcoming: Vec<String> = upcoming
        .streams
        .into_iter()
        .filter(|stream| stream.channel.id == channel.summary.id)
        .map(|stream| stream.title)
        .collect();
    serde_json::json!({
        "channel": channel_streams(ctx, viewer, channel).await,
        "live": live_now(ctx, viewer, channel).await,
        "upcoming": upcoming,
    })
}

#[tokio::test]
async fn a_stranger_learns_nothing_from_a_locked_accounts_streams() {
    let ctx = start().await;
    let stranger = ctx.app.user("stranger").await;
    let follower = ctx.app.user("follower").await;
    // Two worlds that differ only in what a locked account streams publicly.
    let (posted, silent) = (ctx.app.user("posted").await, ctx.app.user("silent").await);
    let mut channels = Vec::new();
    for world in [&posted, &silent] {
        ctx.follow(&follower, world).await;
        ctx.set_locked(world, true).await;
        channels.push(ctx.channel(world, "鍵チャンネル").await);
    }
    let on_air = ctx.on_air(&posted, &channels[0], "鍵アカの配信").await;
    let later = ctx
        .stream_with(&posted, &channels[0], "予定", |request| {
            request.scheduled_at = tokuly_core::time::ts(chrono::Utc::now() + chrono::Duration::hours(2))
        })
        .await;
    assert_eq!(card(&on_air).visibility, buffa::EnumValue::from(Visibility::VISIBILITY_PUBLIC), "still public");

    for viewer in [Some(&stranger), None] {
        let seen = what_one_learns(&ctx, viewer, &channels[0]).await;
        let other = what_one_learns(&ctx, viewer, &channels[1]).await;
        assert_eq!(seen, other, "the two worlds look the same from outside");
        for stream in [&on_air, &later] {
            assert_eq!(ctx.get_stream(viewer, &card(stream).id).await.unwrap_err().reason, "STREAM_NOT_FOUND");
        }
    }
    assert!(ctx.search_docs(&card(&on_air).id).await.iter().all(Option::is_none), "nothing in search");
    assert!(ctx.live.authorize_media(&read(&on_air)).await.unwrap().is_err(), "no playing without a token");
    // Followers watch, with a token.
    let watched = ctx.get_stream(Some(&follower), &card(&on_air).id).await.unwrap();
    assert_eq!(channel_streams(&ctx, Some(&follower), &channels[0]).await.len(), 2);
    if let Some(playback) = watched.playback.as_option() {
        assert!(!playback.token.is_empty(), "a locked account's stream needs a token");
    }
}

#[tokio::test]
async fn locking_an_account_applies_at_the_next_read_and_takes_streams_off_the_directories() {
    let ctx = start().await;
    let owner = ctx.app.user("owner").await;
    let follower = ctx.app.user("follower").await;
    let stranger = ctx.app.user("stranger").await;
    ctx.follow(&follower, &owner).await;
    let channel = ctx.channel(&owner, "ch").await;
    let stream = ctx.on_air(&owner, &channel, "公開配信").await;
    assert_eq!(live_now(&ctx, None, &channel).await, vec!["公開配信".to_owned()]);
    assert_eq!(ctx.live.authorize_media(&read(&stream)).await.unwrap(), Ok(()));

    ctx.set_locked(&owner, true).await;
    // Before this domain hears of the lock, reads ask the social domain.
    assert_eq!(ctx.get_stream(Some(&stranger), &card(&stream).id).await.unwrap_err().reason, "STREAM_NOT_FOUND");
    assert!(live_now(&ctx, Some(&stranger), &channel).await.is_empty());
    assert!(ctx.live.authorize_media(&read(&stream)).await.unwrap().is_err(), "the media server asks too");
    assert!(ctx.get_stream(Some(&follower), &card(&stream).id).await.is_ok());

    ctx.deliver_lock_change(&owner).await;
    assert!(ctx.row(&stream).await.owner_locked);
    assert!(live_now(&ctx, Some(&follower), &channel).await.is_empty(), "off the directories for everyone");
    assert!(ctx.search_docs(&card(&stream).id).await.last().unwrap().is_none(), "out of search");
    let events = ctx.app.events_by("live.stream.visibility_changed", owner.id).await;
    let last = events.last().expect("the lock changes the stream's reach");
    assert_eq!(
        (last.data["stream_id"].as_str(), last.data["visibility"].as_i64()),
        (Some(card(&stream).id.as_str()), Some(6))
    );
    assert_eq!(last.data["visible_to"], serde_json::json!([follower.id.to_string()]));
    ctx.deliver_lock_change(&owner).await;
    assert_eq!(ctx.app.events_by("live.stream.visibility_changed", owner.id).await.len(), events.len(), "idempotent");

    ctx.set_locked(&owner, false).await;
    ctx.deliver_lock_change(&owner).await;
    assert!(ctx.get_stream(Some(&stranger), &card(&stream).id).await.is_ok(), "unlocked: open again");
    assert_eq!(ctx.live.authorize_media(&read(&stream)).await.unwrap(), Ok(()));
    let last = ctx.app.events_by("live.stream.visibility_changed", owner.id).await.pop().unwrap();
    assert!(last.data["visible_to"].is_null());
}
