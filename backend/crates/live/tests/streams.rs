//! `LiveStreamService`: creating, editing, finding and watching streams.
mod common;

use common::*;
use tokuly_core::prelude::*;
use tokuly_live::{LiveConfig, access, audience, hooks::AuthRequest};
use tokuly_proto::{
    common::Visibility,
    live::*,
    video::{NotificationLevel, VideoCategory},
};
use tokuly_testkit::TestUser;
use tokuly_video::access::Viewer;

fn in_an_hour() -> Time {
    now() + chrono::Duration::hours(1)
}

#[tokio::test]
async fn a_stream_is_created_edited_and_deleted() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ありすch").await;
    let game = unique("マイクラ");
    let request = CreateStreamRequest {
        channel_id: channel.summary.id.clone(),
        title: "  初めてのライブ!  ".into(),
        description: "みんな来てね".into(),
        tags: vec!["#雑談".into(), "初見歓迎".into()],
        category: VideoCategory::VIDEO_CATEGORY_GAMING.into(),
        game: game.clone(),
        thumbnail_asset_id: ctx.thumbnail(&alice).await,
        scheduled_at: ts(in_an_hour()),
        chat: Some(ChatSettings { enabled: true, slow_mode_seconds: 5, ..Default::default() }).into(),
        ..Default::default()
    };
    let stream = ctx.create_stream(&alice, &request).await.unwrap();
    let summary = card(&stream);
    assert_eq!(summary.title, "初めてのライブ!");
    assert_eq!(summary.short_id.len(), 11);
    assert_eq!(summary.status, StreamStatus::STREAM_STATUS_UPCOMING);
    assert_eq!(summary.visibility, Visibility::VISIBILITY_PUBLIC);
    assert_eq!(summary.channel.as_option().unwrap().id, channel.summary.id);
    assert!(summary.thumbnail.is_set() && summary.scheduled_at.is_set() && summary.low_latency);
    assert_eq!(stream.tags, vec!["雑談", "初見歓迎"]);
    // The defaults: DVR, archive and gifts on; chat as asked, with the rest filled in.
    assert!(stream.dvr_enabled && stream.auto_archive && stream.gifts_enabled);
    let chat = stream.chat.as_option().unwrap();
    assert_eq!((chat.slow_mode_seconds, chat.audience), (5, ChatAudience::CHAT_AUDIENCE_EVERYONE.into()));
    assert_eq!(stream.realtime_topic, format!("live:{}", summary.id));
    assert!(!stream.playback.is_set(), "nothing to play before the stream is on air");
    let studio = stream.studio.as_option().expect("the owner sees the studio part");
    assert!(studio.auto_start && !studio.encoder_connected);
    assert!(stream.viewer.as_option().unwrap().is_owner);

    // Guests see the page (by UUID or short ID) without the owner's parts.
    let seen = ctx.get_stream(None, &summary.short_id).await.unwrap();
    assert_eq!(card(&seen).id, summary.id);
    assert!(!seen.studio.is_set() && !seen.viewer.is_set());
    // A public upcoming stream is searchable.
    let doc = ctx.search_docs(&summary.id).await.pop().unwrap().expect("a search document");
    assert_eq!((doc["kind"].as_str(), doc["title"].as_str()), (Some("live"), Some("初めてのライブ!")));
    assert_eq!(doc["path"], format!("/live/{}", summary.short_id));
    assert!(doc["tags"].as_array().unwrap().iter().any(|tag| tag == game.as_str()));

    // Editing: only what is sent changes; an empty thumbnail ID removes the picture.
    let update = UpdateStreamRequest {
        stream_id: summary.short_id.clone(),
        title: Some("タイトル変更".into()),
        visibility: Some(Visibility::VISIBILITY_UNLISTED.into()),
        thumbnail_asset_id: Some(String::new()),
        dvr_enabled: Some(false),
        clear_schedule: true,
        ..Default::default()
    };
    let updated: UpdateStreamResponse = ctx.call(STREAMS, "UpdateStream", Some(&alice), &update).await.unwrap();
    let updated = updated.stream.as_option().unwrap();
    assert_eq!(card(updated).title, "タイトル変更");
    assert_eq!(updated.description, "みんな来てね");
    assert!(!card(updated).thumbnail.is_set() && !card(updated).scheduled_at.is_set() && !updated.dvr_enabled);
    // No longer public: out of the search index.
    assert_eq!(ctx.search_docs(&summary.id).await.pop().unwrap(), None);

    // Nobody else edits or deletes it.
    let bob = ctx.app.user("bob").await;
    assert_eq!(ctx.reason(STREAMS, "UpdateStream", Some(&bob), &update).await, "STREAM_NOT_FOUND");
    let delete = DeleteStreamRequest { stream_id: summary.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(STREAMS, "DeleteStream", Some(&bob), &delete).await, "STREAM_NOT_FOUND");
    assert_eq!(ctx.reason(STREAMS, "DeleteStream", None, &delete).await, "SIGN_IN_REQUIRED");

    let _: DeleteStreamResponse = ctx.call(STREAMS, "DeleteStream", Some(&alice), &delete).await.unwrap();
    assert_eq!(ctx.get_stream(Some(&alice), &summary.id).await.unwrap_err().reason, "STREAM_NOT_FOUND");
    assert_eq!(ctx.get_stream(None, &summary.short_id).await.unwrap_err().reason, "STREAM_NOT_FOUND");
    let deleted = ctx.app.events_by("live.stream.deleted", alice.id).await;
    assert!(deleted.iter().any(|event| event.data["stream_id"] == summary.id.as_str()));
    // The row is kept for the retention period (what was said in it can still be reported).
    assert!(ctx.row(&stream).await.is_deleted());
}

#[tokio::test]
async fn creating_and_editing_streams_is_validated() {
    let ctx = start_with(LiveConfig { max_open_streams: 2, ..test_config() }).await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let base = CreateStreamRequest { title: "配信".into(), ..Default::default() };
    // No channel yet.
    assert_eq!(ctx.reason(STREAMS, "CreateStream", Some(&alice), &base).await, "CHANNEL_REQUIRED");
    assert_eq!(ctx.reason(STREAMS, "CreateStream", None, &base).await, "SIGN_IN_REQUIRED");
    let channel = ctx.channel(&alice, "ありすch").await;
    let on_channel = |fill: fn(&mut CreateStreamRequest)| {
        let mut request =
            CreateStreamRequest {
                channel_id: channel.summary.id.clone(), title: "配信".into(), ..Default::default()
            };
        fill(&mut request);
        request
    };
    // Someone else's channel reads as not found.
    assert_eq!(ctx.reason(STREAMS, "CreateStream", Some(&bob), &on_channel(|_| {})).await, "CHANNEL_NOT_FOUND");
    for (request, reason) in [
        (on_channel(|r| r.title = "  ".into()), "TITLE_INVALID"),
        (on_channel(|r| r.title = "あ".repeat(101)), "TITLE_INVALID"),
        (on_channel(|r| r.description = "あ".repeat(5001)), "DESCRIPTION_TOO_LONG"),
        (on_channel(|r| r.tags = (0..21).map(|i| format!("t{i}")).collect()), "TAGS_INVALID"),
        (on_channel(|r| r.category = buffa::EnumValue::from(99)), "CATEGORY_INVALID"),
        (on_channel(|r| r.visibility = buffa::EnumValue::from(99)), "VISIBILITY_INVALID"),
        (on_channel(|r| r.scheduled_at = ts(now() - chrono::Duration::hours(1))), "SCHEDULE_INVALID"),
        (on_channel(|r| r.scheduled_at = ts(now() + chrono::Duration::days(400))), "SCHEDULE_INVALID"),
        (
            on_channel(|r| r.chat = Some(ChatSettings { slow_mode_seconds: 900, ..Default::default() }).into()),
            "CHAT_SETTINGS_INVALID",
        ),
        (on_channel(|r| r.thumbnail_asset_id = Uuid::now_v7().to_string()), "ASSET_NOT_FOUND"),
        (on_channel(|r| r.thumbnail_asset_id = "nope".into()), "INVALID_ID"),
    ] {
        assert_eq!(ctx.reason(STREAMS, "CreateStream", Some(&alice), &request).await, reason);
    }
    // A picture uploaded for something else is refused.
    let mut wrong = on_channel(|_| {});
    wrong.thumbnail_asset_id = ctx
        .upload(
            &alice,
            tokuly_proto::media::AssetPurpose::ASSET_PURPOSE_AVATAR,
            tokuly_proto::media::AssetKind::ASSET_KIND_IMAGE,
        )
        .await;
    assert_eq!(ctx.reason(STREAMS, "CreateStream", Some(&alice), &wrong).await, "ASSET_WRONG_PURPOSE");

    // The default channel is used when none is named; the limit counts streams that are not over.
    let first = ctx.create_stream(&alice, &base).await.unwrap();
    assert_eq!(card(&first).channel.as_option().unwrap().id, channel.summary.id);
    let second = ctx.stream(&alice, &channel, "二つ目").await;
    assert_eq!(ctx.reason(STREAMS, "CreateStream", Some(&alice), &base).await, "STREAM_LIMIT_REACHED");
    ctx.end(&alice, &second).await;
    ctx.stream(&alice, &channel, "三つ目").await;

    // Edits go through the same checks; the schedule is fixed once a stream has started.
    let edit = |fill: fn(&mut UpdateStreamRequest)| {
        let mut request = UpdateStreamRequest { stream_id: card(&first).id.clone(), ..Default::default() };
        fill(&mut request);
        request
    };
    assert_eq!(
        ctx.reason(STREAMS, "UpdateStream", Some(&alice), &edit(|r| r.title = Some(String::new()))).await,
        "TITLE_INVALID"
    );
    assert_eq!(
        ctx.reason(
            STREAMS,
            "UpdateStream",
            Some(&alice),
            &edit(|r| r.visibility = Some(Visibility::VISIBILITY_UNSPECIFIED.into()))
        )
        .await,
        "VISIBILITY_INVALID"
    );
    ctx.go_live(&alice, &first).await;
    assert_eq!(
        ctx.reason(
            STREAMS,
            "UpdateStream",
            Some(&alice),
            &edit(|r| r.scheduled_at = ts(now() + chrono::Duration::hours(2)))
        )
        .await,
        "STREAM_ALREADY_STARTED"
    );
    assert_eq!(
        ctx.reason(STREAMS, "UpdateStream", Some(&alice), &edit(|r| r.auto_start = Some(false))).await,
        "STREAM_ALREADY_STARTED"
    );
    // The title of a stream on air can still change, and viewers are told.
    let mut topic = ctx.app.listen_topic(&first.realtime_topic).await;
    let _: UpdateStreamResponse =
        ctx.call(STREAMS, "UpdateStream", Some(&alice), &edit(|r| r.title = Some("改題".into()))).await.unwrap();
    let pushed = wait_for(&mut topic, |push| match push {
        Push::StreamUpdated(summary) => Some(summary),
        _ => None,
    })
    .await;
    assert_eq!(pushed.title, "改題");
}

/// Runs the single-row rules (`access::can_watch`, `access::sees_listed`)
/// and the SQL predicates behind the lists over every visibility and kind
/// of viewer, and requires them to agree with each other and with the API.
#[tokio::test]
async fn visibility_decides_who_sees_a_stream() {
    let ctx = start().await;
    let owner = ctx.app.user("owner").await;
    let friend = ctx.app.user("friend").await;
    let stranger = ctx.app.user("stranger").await;
    let blocked = ctx.app.user("blocked").await;
    ctx.befriend(&owner, &friend).await;
    ctx.block(&owner, &blocked).await;
    let channel = ctx.channel(&owner, "見え方ch").await;
    let visibilities = [
        Visibility::VISIBILITY_PUBLIC,
        Visibility::VISIBILITY_NO_FEED,
        Visibility::VISIBILITY_UNLISTED,
        Visibility::VISIBILITY_FRIENDS,
        Visibility::VISIBILITY_PRIVATE,
    ];
    let mut streams = Vec::new();
    for visibility in visibilities {
        let stream =
            ctx.on_air_with(&owner, &channel, &format!("{visibility:?}"), |r| r.visibility = visibility.into()).await;
        streams.push((visibility, stream));
    }
    let viewers: [(&str, Option<&TestUser>); 5] = [
        ("owner", Some(&owner)),
        ("friend", Some(&friend)),
        ("stranger", Some(&stranger)),
        ("blocked", Some(&blocked)),
        ("guest", None),
    ];
    for (name, user) in viewers {
        let viewer = Viewer::load(&ctx.app.core, user.map(|u| u.id)).await.unwrap();
        // The blocked user does not even find the channel.
        let listing: Result<ListChannelStreamsResponse, _> = ctx
            .call(
                STREAMS,
                "ListChannelStreams",
                user,
                &ListChannelStreamsRequest { channel_id: channel.summary.id.clone(), ..Default::default() },
            )
            .await;
        let listed: Vec<String> =
            listing.map(|response| response.streams.into_iter().map(|s| s.id).collect()).unwrap_or_default();
        for (visibility, stream) in &streams {
            let facts = ctx.row(stream).await.facts();
            let opens = ctx.get_stream(user, &card(stream).id).await.is_ok();
            assert_eq!(opens, access::can_watch(&viewer, &facts), "{name} opening a {visibility:?} stream");
            let expect_listed = if name == "owner" { true } else { access::sees_listed(&viewer, &facts) };
            assert_eq!(
                listed.contains(&card(stream).id),
                expect_listed,
                "{name} seeing a {visibility:?} stream on the channel page"
            );
        }
    }
    // The directory lists public streams only; the friend list adds friends-only ones.
    let live: ListLiveStreamsResponse =
        ctx.call(STREAMS, "ListLiveStreams", Some(&friend), &ListLiveStreamsRequest::default()).await.unwrap();
    let mine: Vec<&str> = live
        .streams
        .iter()
        .filter(|s| s.channel.as_option().is_some_and(|c| c.id == channel.summary.id))
        .map(|s| s.title.as_str())
        .collect();
    assert_eq!(mine, vec!["VISIBILITY_PUBLIC"]);
    let friends: ListFriendStreamsResponse =
        ctx.call(STREAMS, "ListFriendStreams", Some(&friend), &ListFriendStreamsRequest::default()).await.unwrap();
    let mut titles: Vec<&str> = friends.streams.iter().map(|s| s.title.as_str()).collect();
    titles.sort_unstable();
    assert_eq!(titles, vec!["VISIBILITY_FRIENDS", "VISIBILITY_PUBLIC"]);
    let none: ListFriendStreamsResponse =
        ctx.call(STREAMS, "ListFriendStreams", Some(&stranger), &ListFriendStreamsRequest::default()).await.unwrap();
    assert!(none.streams.is_empty());
    assert_eq!(
        ctx.reason(STREAMS, "ListFriendStreams", None, &ListFriendStreamsRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
}

#[tokio::test]
async fn the_directories_list_what_is_on_air_and_what_is_coming() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ディレクトリch").await;
    let game = unique("game");
    let with_game = |r: &mut CreateStreamRequest| {
        r.game = game.clone();
        r.category = VideoCategory::VIDEO_CATEGORY_GAMING.into();
    };
    let quiet = ctx.on_air_with(&alice, &channel, "静かな配信", with_game).await;
    let busy = ctx.on_air_with(&alice, &channel, "人気の配信", with_game).await;
    let other = ctx.on_air(&alice, &channel, "別ゲーム").await;
    for (stream, viewers) in [(&quiet, 3), (&busy, 40), (&other, 7)] {
        ctx.audience(stream, viewers).await;
    }
    // By game (case-insensitive): most viewers first.
    let by_game = ListLiveStreamsRequest { game: game.to_uppercase(), ..Default::default() };
    let listed: ListLiveStreamsResponse = ctx.call(STREAMS, "ListLiveStreams", None, &by_game).await.unwrap();
    assert_eq!(titles(&listed.streams), vec!["人気の配信", "静かな配信"]);
    assert_eq!(listed.streams[0].viewer_count, 40);
    // Paged.
    let first: ListLiveStreamsResponse = ctx
        .call(
            STREAMS,
            "ListLiveStreams",
            None,
            &ListLiveStreamsRequest { game: game.clone(), page: page(1, ""), ..Default::default() },
        )
        .await
        .unwrap();
    let info = first.page.as_option().unwrap();
    assert!(info.has_more && titles(&first.streams) == vec!["人気の配信"]);
    let second: ListLiveStreamsResponse = ctx
        .call(
            STREAMS,
            "ListLiveStreams",
            None,
            &ListLiveStreamsRequest { game: game.clone(), page: page(1, &info.next_cursor), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(titles(&second.streams), vec!["静かな配信"]);
    assert!(!second.page.as_option().unwrap().has_more);
    assert_eq!(
        ctx.reason(
            STREAMS,
            "ListLiveStreams",
            None,
            &ListLiveStreamsRequest { category: buffa::EnumValue::from(99), ..Default::default() }
        )
        .await,
        "CATEGORY_INVALID"
    );
    // A stream that ended leaves the directory.
    ctx.end(&alice, &busy).await;
    let listed: ListLiveStreamsResponse = ctx.call(STREAMS, "ListLiveStreams", None, &by_game).await.unwrap();
    assert_eq!(titles(&listed.streams), vec!["静かな配信"]);

    // Upcoming: scheduled streams, soonest first; unscheduled ones are not announced.
    let bob = ctx.app.user("bob").await;
    let bob_channel = ctx.channel(&bob, "予定ch").await;
    let later = ctx
        .stream_with(&bob, &bob_channel, "あさって", |r| r.scheduled_at = ts(now() + chrono::Duration::days(2)))
        .await;
    let sooner =
        ctx.stream_with(&bob, &bob_channel, "あした", |r| r.scheduled_at = ts(now() + chrono::Duration::days(1))).await;
    ctx.stream(&bob, &bob_channel, "そのうち").await;
    ctx.stream_with(&bob, &bob_channel, "ないしょ", |r| {
        r.scheduled_at = ts(now() + chrono::Duration::hours(3));
        r.visibility = Visibility::VISIBILITY_UNLISTED.into();
    })
    .await;
    let upcoming: ListUpcomingStreamsResponse =
        ctx.call(STREAMS, "ListUpcomingStreams", None, &ListUpcomingStreamsRequest::default()).await.unwrap();
    let of_bob: Vec<&str> = upcoming
        .streams
        .iter()
        .filter(|s| s.channel.as_option().is_some_and(|c| c.id == bob_channel.summary.id))
        .map(|s| s.title.as_str())
        .collect();
    assert_eq!(of_bob, vec!["あした", "あさって"]);
    // The channel page: everything for the owner, by status when asked.
    let scheduled = ListChannelStreamsRequest {
        channel_id: bob_channel.summary.id.clone(),
        status: StreamStatus::STREAM_STATUS_UPCOMING.into(),
        ..Default::default()
    };
    let own: ListChannelStreamsResponse =
        ctx.call(STREAMS, "ListChannelStreams", Some(&bob), &scheduled).await.unwrap();
    assert_eq!(own.streams.len(), 4);
    let public: ListChannelStreamsResponse = ctx.call(STREAMS, "ListChannelStreams", None, &scheduled).await.unwrap();
    assert_eq!(public.streams.len(), 3, "the unlisted stream is not on the page");
    // A stream cancelled before it started is the owner's business only.
    ctx.end(&bob, &later).await;
    let ended = ListChannelStreamsRequest {
        channel_id: bob_channel.summary.id.clone(),
        status: StreamStatus::STREAM_STATUS_ENDED.into(),
        ..Default::default()
    };
    let own: ListChannelStreamsResponse = ctx.call(STREAMS, "ListChannelStreams", Some(&bob), &ended).await.unwrap();
    assert_eq!(titles(&own.streams), vec!["あさって"]);
    let public: ListChannelStreamsResponse = ctx.call(STREAMS, "ListChannelStreams", None, &ended).await.unwrap();
    assert!(public.streams.is_empty());
    assert_eq!(
        ctx.reason(
            STREAMS,
            "ListChannelStreams",
            None,
            &ListChannelStreamsRequest { channel_id: "nobody-here".into(), ..Default::default() }
        )
        .await,
        "CHANNEL_NOT_FOUND"
    );
    let _ = sooner;
}

#[tokio::test]
async fn subscribers_see_the_streams_of_their_channels() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let channel = ctx.channel(&alice, "登録ch").await;
    let elsewhere = ctx.channel(&bob, "よそのch").await;
    ctx.subscribe(&carol, &channel, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    ctx.on_air(&alice, &channel, "配信中").await;
    ctx.stream_with(&alice, &channel, "予定", |r| r.scheduled_at = ts(in_an_hour())).await;
    ctx.stream(&alice, &channel, "未定").await;
    ctx.on_air(&bob, &elsewhere, "登録してない").await;
    let listed: ListSubscribedStreamsResponse = ctx
        .call(STREAMS, "ListSubscribedStreams", Some(&carol), &ListSubscribedStreamsRequest::default())
        .await
        .unwrap();
    // On air first, then what is scheduled; nothing from channels she does not follow.
    assert_eq!(titles(&listed.streams), vec!["配信中", "予定"]);
    let paged: ListSubscribedStreamsResponse = ctx
        .call(
            STREAMS,
            "ListSubscribedStreams",
            Some(&carol),
            &ListSubscribedStreamsRequest { page: page(1, ""), ..Default::default() },
        )
        .await
        .unwrap();
    let info = paged.page.as_option().unwrap();
    assert!(info.has_more && titles(&paged.streams) == vec!["配信中"]);
    let rest: ListSubscribedStreamsResponse = ctx
        .call(
            STREAMS,
            "ListSubscribedStreams",
            Some(&carol),
            &ListSubscribedStreamsRequest { page: page(1, &info.next_cursor), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(titles(&rest.streams), vec!["予定"]);
    let nothing: ListSubscribedStreamsResponse =
        ctx.call(STREAMS, "ListSubscribedStreams", Some(&bob), &ListSubscribedStreamsRequest::default()).await.unwrap();
    assert!(nothing.streams.is_empty());
    assert_eq!(
        ctx.reason(STREAMS, "ListSubscribedStreams", None, &ListSubscribedStreamsRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
}

#[tokio::test]
async fn without_a_media_server_streams_are_managed_but_cannot_go_on_air() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "メディアなしch").await;
    let stream = ctx.stream(&alice, &channel, "準備中").await;
    let by_id = GetIngestInfoRequest { stream_id: card(&stream).id.clone(), ..Default::default() };
    let error = ctx.call::<_, GetIngestInfoResponse>(STREAMS, "GetIngestInfo", Some(&alice), &by_id).await.unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("unavailable", "MEDIA_SERVER_NOT_CONFIGURED"));
    let start = StartStreamRequest { stream_id: card(&stream).id.clone(), ..Default::default() };
    let error = ctx.call::<_, StartStreamResponse>(STREAMS, "StartStream", Some(&alice), &start).await.unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("unavailable", "MEDIA_SERVER_NOT_CONFIGURED"));
    // Everything else works: the stream can be edited, cancelled and deleted.
    let ended = ctx.end(&alice, &stream).await;
    assert_eq!(card(&ended).status, StreamStatus::STREAM_STATUS_ENDED);
    assert_eq!(
        ended.archive_status,
        ArchiveStatus::ARCHIVE_STATUS_NONE,
        "a stream that never went on air has nothing to archive"
    );
    assert_eq!(ctx.reason(STREAMS, "GetIngestInfo", Some(&alice), &by_id).await, "STREAM_ENDED");
}

/// A media server that is configured (but, here, never asked).
fn with_media_server() -> LiveConfig {
    LiveConfig {
        mediamtx_api_url: "http://127.0.0.1:9".into(),
        hook_secret: "test-hook-secret-0123456789".into(),
        rtmp_url: "rtmp://live.example.test:1935".into(),
        srt_url: "srt://live.example.test:8890".into(),
        whip_url: "https://live.example.test/rtc".into(),
        whep_url: "https://live.example.test/rtc".into(),
        hls_url: "https://live.example.test/hls".into(),
        playback_url: "https://live.example.test/dvr".into(),
        ..test_config()
    }
}

fn publish_with(short_id: &str, key: &str) -> AuthRequest {
    AuthRequest {
        action: "publish".into(),
        path: format!("live/{short_id}"),
        protocol: "rtmp".into(),
        token: key.to_owned(),
        ..Default::default()
    }
}

#[tokio::test]
async fn the_owner_gets_ingest_urls_and_can_rotate_the_key() {
    let ctx = start_with(with_media_server()).await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "キーch").await;
    let stream = ctx.stream(&alice, &channel, "キーの配信").await;
    let short_id = card(&stream).short_id.clone();
    let request = GetIngestInfoRequest { stream_id: short_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(STREAMS, "GetIngestInfo", Some(&bob), &request).await, "STREAM_NOT_FOUND");
    assert_eq!(ctx.reason(STREAMS, "GetIngestInfo", None, &request).await, "SIGN_IN_REQUIRED");
    let info: GetIngestInfoResponse = ctx.call(STREAMS, "GetIngestInfo", Some(&alice), &request).await.unwrap();
    let ingest = info.ingest.as_option().unwrap();
    let key = ingest.stream_key.clone();
    assert!(key.len() >= 32 && !key.contains(&short_id));
    assert_eq!(ingest.stream_name, short_id);
    assert_eq!(ingest.rtmp_server_url, "rtmp://live.example.test:1935/live");
    assert_eq!(ingest.rtmp_stream_key, format!("{short_id}?token={key}"));
    assert_eq!(ingest.rtmp_url, format!("rtmp://live.example.test:1935/live/{short_id}?token={key}"));
    assert_eq!(
        ingest.srt_url,
        format!("srt://live.example.test:8890?streamid=publish:live/{short_id}:tokuly:{key}&pkt_size=1316")
    );
    assert_eq!(ingest.whip_url, format!("https://live.example.test/rtc/live/{short_id}/whip"));
    assert_eq!(ingest.whip_bearer_token, key);
    // The media server's question is answered with that key and nothing else.
    assert_eq!(ctx.live.authorize_media(&publish_with(&short_id, &key)).await.unwrap(), Ok(()));
    assert!(ctx.live.authorize_media(&publish_with(&short_id, "wrong")).await.unwrap().is_err());
    // The key is not what the page shows to anyone.
    let page_json = serde_json::to_string(&ctx.get_stream(Some(&alice), &short_id).await.unwrap()).unwrap();
    assert!(!page_json.contains(&key));

    let rotate = RotateStreamKeyRequest { stream_id: short_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(STREAMS, "RotateStreamKey", Some(&bob), &rotate).await, "STREAM_NOT_FOUND");
    let rotated: RotateStreamKeyResponse = ctx.call(STREAMS, "RotateStreamKey", Some(&alice), &rotate).await.unwrap();
    let new_key = rotated.ingest.as_option().unwrap().stream_key.clone();
    assert_ne!(new_key, key);
    assert!(ctx.live.authorize_media(&publish_with(&short_id, &key)).await.unwrap().is_err(), "the old key is dead");
    assert_eq!(ctx.live.authorize_media(&publish_with(&short_id, &new_key)).await.unwrap(), Ok(()));
    // An ended stream takes no encoder any more.
    ctx.end(&alice, &stream).await;
    assert_eq!(ctx.reason(STREAMS, "RotateStreamKey", Some(&alice), &rotate).await, "STREAM_ENDED");
    assert!(ctx.live.authorize_media(&publish_with(&short_id, &new_key)).await.unwrap().is_err());
}

#[tokio::test]
async fn reminders_are_set_before_a_stream_starts() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "リマインドch").await;
    let stream = ctx.stream_with(&alice, &channel, "あとで", |r| r.scheduled_at = ts(in_an_hour())).await;
    let set = |enabled| SetReminderRequest { stream_id: card(&stream).id.clone(), enabled, ..Default::default() };
    assert_eq!(ctx.reason(STREAMS, "SetReminder", None, &set(true)).await, "SIGN_IN_REQUIRED");
    let on: SetReminderResponse = ctx.call(STREAMS, "SetReminder", Some(&bob), &set(true)).await.unwrap();
    assert!(on.enabled);
    // Asking twice is fine.
    let _: SetReminderResponse = ctx.call(STREAMS, "SetReminder", Some(&bob), &set(true)).await.unwrap();
    assert!(ctx.get_stream(Some(&bob), &card(&stream).id).await.unwrap().viewer.as_option().unwrap().reminder_set);
    let off: SetReminderResponse = ctx.call(STREAMS, "SetReminder", Some(&bob), &set(false)).await.unwrap();
    assert!(!off.enabled);
    assert!(!ctx.get_stream(Some(&bob), &card(&stream).id).await.unwrap().viewer.as_option().unwrap().reminder_set);
    ctx.go_live(&alice, &stream).await;
    assert_eq!(ctx.reason(STREAMS, "SetReminder", Some(&bob), &set(true)).await, "STREAM_ALREADY_STARTED");
    // A private stream is not there to be reminded of.
    let hidden =
        ctx.stream_with(&alice, &channel, "ひみつ", |r| r.visibility = Visibility::VISIBILITY_PRIVATE.into()).await;
    let request = SetReminderRequest { stream_id: card(&hidden).id.clone(), enabled: true, ..Default::default() };
    assert_eq!(ctx.reason(STREAMS, "SetReminder", Some(&bob), &request).await, "STREAM_NOT_FOUND");
}

#[tokio::test]
async fn heartbeats_make_the_audience_the_viewer_count_and_the_viewer_list() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let dave = ctx.app.user("dave").await;
    let channel = ctx.channel(&alice, "視聴ch").await;
    ctx.subscribe(&carol, &channel, NotificationLevel::NOTIFICATION_LEVEL_PERSONALIZED).await;
    let stream = ctx.on_air(&alice, &channel, "見てね").await;
    let watch = |client: &str| ReportWatchingRequest {
        stream_id: card(&stream).id.clone(),
        client_id: client.to_owned(),
        ..Default::default()
    };
    let mut topic = ctx.app.listen_topic(&stream.realtime_topic).await;

    let report: ReportWatchingResponse = ctx.call(STREAMS, "ReportWatching", Some(&bob), &watch("")).await.unwrap();
    assert_eq!((report.status, report.report_interval_seconds), (StreamStatus::STREAM_STATUS_LIVE.into(), 15));
    let _: ReportWatchingResponse = ctx.call(STREAMS, "ReportWatching", Some(&carol), &watch("")).await.unwrap();
    // Two guests on two devices, one of them reporting twice.
    for client in ["device-a", "device-b", "device-a"] {
        let _: ReportWatchingResponse = ctx.call(STREAMS, "ReportWatching", None, &watch(client)).await.unwrap();
    }
    // The streamer's own player is the dashboard, not a viewer.
    let _: ReportWatchingResponse = ctx.call(STREAMS, "ReportWatching", Some(&alice), &watch("")).await.unwrap();
    assert!(audience::dashboard_open(&ctx.app.core, sid(&stream)).await.unwrap());
    assert_eq!(audience::heartbeat_count(&ctx.app.core, sid(&stream), 15).await.unwrap(), 4);
    assert_eq!(ctx.reason(STREAMS, "ReportWatching", None, &watch(&"x".repeat(65))).await, "CLIENT_ID_INVALID");

    // The next sample makes it the viewer count (the largest of the sources) and the peak, and tells the viewers.
    ctx.live.sample_viewers(&[(sid(&stream), 2)].into_iter().collect()).await.unwrap();
    let stats = wait_for(&mut topic, |push| match push {
        Push::Stats(stats) => Some(stats),
        _ => None,
    })
    .await;
    assert_eq!((stats.viewer_count, stats.peak_viewer_count), (4, 4));
    // More readers on the media server than heartbeats: those count.
    ctx.live.sample_viewers(&[(sid(&stream), 9)].into_iter().collect()).await.unwrap();
    // (Another test's sample may already have come by: the peak is what stays.)
    let seen = ctx.get_stream(None, &card(&stream).id).await.unwrap();
    assert_eq!(card(&seen).peak_viewer_count, 9);
    ctx.live.sample_viewers(&Default::default()).await.unwrap();
    let seen = ctx.get_stream(None, &card(&stream).id).await.unwrap();
    assert_eq!((card(&seen).viewer_count, card(&seen).peak_viewer_count), (4, 9), "the peak stays");

    // The viewer list: for the streamer and moderators, signed-in viewers with their standing.
    let list = ListViewersRequest { stream_id: card(&stream).id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(STREAMS, "ListViewers", Some(&dave), &list).await, "NOT_A_MODERATOR");
    assert_eq!(ctx.reason(STREAMS, "ListViewers", None, &list).await, "SIGN_IN_REQUIRED");
    let viewers: ListViewersResponse = ctx.call(STREAMS, "ListViewers", Some(&alice), &list).await.unwrap();
    let mut seen: Vec<(String, i32)> =
        viewers.viewers.iter().map(|v| (v.user.as_option().unwrap().id.clone(), v.role.to_i32())).collect();
    seen.sort();
    let mut expected = vec![
        (bob.id.to_string(), ChatRole::CHAT_ROLE_VIEWER as i32),
        (carol.id.to_string(), ChatRole::CHAT_ROLE_SUBSCRIBER as i32),
    ];
    expected.sort();
    assert_eq!(seen, expected);
    assert_eq!(viewers.viewer_count, 4);

    // Watching is measured: one view per viewer, and the time between reports.
    let _: ReportWatchingResponse = ctx.call(STREAMS, "ReportWatching", Some(&bob), &watch("")).await.unwrap();
    ctx.app.flush_analytics().await;
    let analytics: GetStreamAnalyticsResponse = ctx
        .call(
            ANALYTICS,
            "GetStreamAnalytics",
            Some(&alice),
            &GetStreamAnalyticsRequest { stream_id: card(&stream).id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(analytics.analytics.as_option().unwrap().unique_viewers, 4);

    // After the end nobody is watching.
    ctx.end(&alice, &stream).await;
    assert_eq!(audience::heartbeat_count(&ctx.app.core, sid(&stream), 15).await.unwrap(), 0);
    let report: ReportWatchingResponse = ctx.call(STREAMS, "ReportWatching", Some(&bob), &watch("")).await.unwrap();
    assert_eq!((report.status, report.viewer_count), (StreamStatus::STREAM_STATUS_ENDED.into(), 0));
}

#[tokio::test]
async fn one_address_cannot_inflate_an_audience_with_made_up_guests() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "水増しch").await;
    let stream = ctx.on_air(&alice, &channel, "水増し").await;
    let mut refused = 0;
    for i in 0..70 {
        let request = ReportWatchingRequest {
            stream_id: card(&stream).id.clone(),
            client_id: format!("fake-{i}"),
            ..Default::default()
        };
        if let Err(error) = ctx.call::<_, ReportWatchingResponse>(STREAMS, "ReportWatching", None, &request).await {
            assert_eq!(error.reason, "RATE_LIMITED");
            refused += 1;
        }
    }
    assert_eq!(refused, 10, "sixty new guests per address and ten minutes");
    assert_eq!(audience::heartbeat_count(&ctx.app.core, sid(&stream), 15).await.unwrap(), 60);
}

#[tokio::test]
async fn a_raid_sends_the_audience_to_another_stream() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let alice_channel = ctx.channel(&alice, "レイド元").await;
    let bob_channel = ctx.channel(&bob, "レイド先").await;
    let source = ctx.on_air(&alice, &alice_channel, "おわりの配信").await;
    let target = ctx.on_air(&bob, &bob_channel, "これからの配信").await;
    let waiting = ctx.stream(&bob, &bob_channel, "まだ").await;
    ctx.audience(&source, 12).await;
    let raid = |from: &Stream, to: &str| RaidStreamRequest {
        stream_id: card(from).id.clone(),
        target_stream_id: to.to_owned(),
        ..Default::default()
    };

    assert_eq!(
        ctx.reason(STREAMS, "RaidStream", Some(&bob), &raid(&source, &card(&target).id)).await,
        "STREAM_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(STREAMS, "RaidStream", Some(&alice), &raid(&source, &Uuid::now_v7().to_string())).await,
        "RAID_TARGET_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(STREAMS, "RaidStream", Some(&alice), &raid(&source, &card(&waiting).id)).await,
        "RAID_TARGET_NOT_LIVE"
    );
    let second = ctx.on_air(&alice, &alice_channel, "自分の別枠").await;
    assert_eq!(ctx.reason(STREAMS, "RaidStream", Some(&alice), &raid(&source, &card(&second).id)).await, "RAID_SELF");

    let mut source_topic = ctx.app.listen_topic(&source.realtime_topic).await;
    let mut target_topic = ctx.app.listen_topic(&target.realtime_topic).await;
    let done: RaidStreamResponse =
        ctx.call(STREAMS, "RaidStream", Some(&alice), &raid(&source, &card(&target).short_id)).await.unwrap();
    assert_eq!(done.viewer_count, 12);
    assert_eq!(done.target.as_option().unwrap().id, card(&target).id);
    let ended = done.stream.as_option().unwrap();
    assert_eq!(card(ended).status, StreamStatus::STREAM_STATUS_ENDED);
    assert_eq!(ended.raid_target_stream_id, card(&target).id);
    assert_eq!(ended.studio.as_option().unwrap().end_reason, EndReason::END_REASON_RAID);
    // The audience is told where to go...
    let pushed = wait_for(&mut source_topic, |push| match push {
        Push::Raid(raid) => Some(raid),
        _ => None,
    })
    .await;
    assert_eq!((pushed.viewer_count, pushed.target.as_option().unwrap().id.as_str()), (12, card(&target).id.as_str()));
    // ...and the target's chat hears who came.
    let notice = wait_for(&mut target_topic, |push| match push {
        Push::ChatMessage(message) => Some(message),
        _ => None,
    })
    .await;
    assert_eq!(notice.kind, ChatMessageKind::CHAT_MESSAGE_KIND_NOTICE);
    let raid_notice = notice.notice.as_option().unwrap();
    assert_eq!((raid_notice.kind, raid_notice.viewer_count), (ChatNoticeKind::CHAT_NOTICE_KIND_RAID.into(), 12));
    assert_eq!(raid_notice.channel.as_option().unwrap().id, alice_channel.summary.id);
    // A stream that is over cannot raid again.
    assert_eq!(
        ctx.reason(STREAMS, "RaidStream", Some(&alice), &raid(&source, &card(&target).id)).await,
        "STREAM_NOT_LIVE"
    );
}
