//! `ChannelService`: channels, their pages and subscriptions.
mod common;

use common::*;
use tokuly_media::assets;
use tokuly_proto::{common::Visibility, media::AssetPurpose, video::*};

fn link(title: &str, url: &str) -> ChannelLink {
    ChannelLink { title: title.into(), url: url.into(), ..Default::default() }
}

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

#[tokio::test]
async fn creating_a_channel_stores_its_page_and_announces_it() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let icon = ctx.image_upload(&alice, AssetPurpose::ASSET_PURPOSE_CHANNEL_ICON).await;
    let banner = ctx.image_upload(&alice, AssetPurpose::ASSET_PURPOSE_CHANNEL_BANNER).await;
    let handle = unique("toku");
    let request = CreateChannelRequest {
        handle: format!("@{}", handle.to_uppercase()),
        name: "  とくぞうちゃんねる  ".into(),
        description: "ゲーム実況と雑談!".into(),
        icon_asset_id: icon.clone(),
        banner_asset_id: banner.clone(),
        links: vec![link("X", "https://x.com/tokuzou")],
        ..Default::default()
    };
    let channel = ctx.create_channel(&alice, &request).await.unwrap();
    let summary = channel.summary.as_option().unwrap();
    assert_eq!((summary.handle.as_str(), summary.name.as_str()), (handle.as_str(), "とくぞうちゃんねる"));
    assert_eq!(summary.icon.as_option().unwrap().asset_id, icon);
    assert_eq!(channel.banner.as_option().unwrap().asset_id, banner);
    assert_eq!((channel.description.as_str(), channel.links.len(), summary.subscriber_count), ("ゲーム実況と雑談!", 1, 0));
    let state = channel.viewer.as_option().unwrap();
    assert!(state.is_owner && state.is_default && !state.subscribed, "the first channel is the default");

    // The uploads are held, so the media janitor leaves them alone.
    assert!(assets::is_attached(&ctx.app.core, id(&icon)).await.unwrap());
    assert!(assets::is_attached(&ctx.app.core, id(&banner)).await.unwrap());

    let created = ctx.app.events_by("video.channel.created", alice.id).await;
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].data["handle"], handle);
    let docs = search_docs(&ctx, &summary.id).await;
    let doc = docs.last().unwrap().as_ref().expect("the channel is searchable");
    assert_eq!((doc["kind"].as_str(), doc["title"].as_str(), doc["path"].as_str()), (Some("channel"), Some("とくぞうちゃんねる"), Some(format!("/channel/{handle}").as_str())));

    // A second channel is not the default.
    let second = ctx.channel(&alice, "サブ").await;
    assert!(!second.viewer.as_option().unwrap().is_default);
}

#[tokio::test]
async fn channel_input_is_validated() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let taken = ctx.channel(&alice, "first").await.summary.as_option().unwrap().handle.clone();
    let bobs_icon = ctx.image_upload(&bob, AssetPurpose::ASSET_PURPOSE_CHANNEL_ICON).await;
    let avatar = ctx.image_upload(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR).await;

    let cases: Vec<(&str, Box<dyn Fn(&mut CreateChannelRequest)>)> = vec![
        ("HANDLE_INVALID", Box::new(|r| r.handle = "a b".into())),
        ("HANDLE_INVALID", Box::new(|r| r.handle = "x".into())),
        ("HANDLE_RESERVED", Box::new(|r| r.handle = "official".into())),
        ("HANDLE_TAKEN", Box::new(|r| r.handle = taken.to_uppercase())),
        ("CHANNEL_NAME_INVALID", Box::new(|r| r.name = "  ".into())),
        ("CHANNEL_NAME_INVALID", Box::new(|r| r.name = "あ".repeat(51))),
        ("CHANNEL_DESCRIPTION_TOO_LONG", Box::new(|r| r.description = "d".repeat(1001))),
        ("CHANNEL_LINKS_INVALID", Box::new(|r| r.links = vec![link("site", "javascript:alert(1)")])),
        ("CHANNEL_LINKS_INVALID", Box::new(|r| r.links = (0..6).map(|i| link("a", &format!("https://example.com/{i}"))).collect())),
        ("ASSET_NOT_FOUND", Box::new(|r| r.icon_asset_id = bobs_icon.clone())),
        ("ASSET_WRONG_PURPOSE", Box::new(|r| r.icon_asset_id = avatar.clone())),
        ("INVALID_ID", Box::new(|r| r.banner_asset_id = "nope".into())),
    ];
    for (reason, mutate) in cases {
        let mut request = CreateChannelRequest { handle: unique("ok"), name: "ok".into(), ..Default::default() };
        mutate(&mut request);
        assert_eq!(ctx.reason(CHANNELS, "CreateChannel", Some(&alice), &request).await, reason);
    }
    let request = CreateChannelRequest { handle: unique("anon"), name: "anon".into(), ..Default::default() };
    assert_eq!(ctx.reason(CHANNELS, "CreateChannel", None, &request).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn an_account_owns_at_most_five_channels() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    for index in 0..5 {
        ctx.channel(&alice, &format!("ch{index}")).await;
    }
    let request = CreateChannelRequest { handle: unique("six"), name: "six".into(), ..Default::default() };
    assert_eq!(ctx.reason(CHANNELS, "CreateChannel", Some(&alice), &request).await, "CHANNEL_LIMIT_REACHED");
    let mine: ListMyChannelsResponse = ctx.call(CHANNELS, "ListMyChannels", Some(&alice), &ListMyChannelsRequest::default()).await.unwrap();
    assert_eq!((mine.channels.len(), mine.max_channels), (5, 5));
    assert!(mine.channels[0].viewer.as_option().unwrap().is_default, "the default channel comes first");
    assert_eq!(mine.channels.iter().filter(|channel| channel.viewer.as_option().unwrap().is_default).count(), 1);
}

#[tokio::test]
async fn updating_a_channel_changes_only_what_is_named() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let icon = ctx.image_upload(&alice, AssetPurpose::ASSET_PURPOSE_CHANNEL_ICON).await;
    let request = CreateChannelRequest {
        handle: unique("up"),
        name: "before".into(),
        description: "keep me".into(),
        icon_asset_id: icon.clone(),
        links: vec![link("a", "https://example.com/a")],
        ..Default::default()
    };
    let channel = ctx.create_channel(&alice, &request).await.unwrap();
    let channel_id = channel.summary.id.clone();

    let new_handle = unique("renamed");
    let update = UpdateChannelRequest {
        channel_id: channel_id.clone(),
        name: Some("after".into()),
        handle: Some(new_handle.clone()),
        links: buffa::MessageField::some(ChannelLinkList { links: vec![link("b", "https://example.com/b"), link("c", "https://example.com/c")], ..Default::default() }),
        ..Default::default()
    };
    let response: UpdateChannelResponse = ctx.call(CHANNELS, "UpdateChannel", Some(&alice), &update).await.unwrap();
    let updated = response.channel.as_option().unwrap();
    assert_eq!((updated.summary.name.as_str(), updated.summary.handle.as_str()), ("after", new_handle.as_str()));
    assert_eq!(updated.description, "keep me");
    assert_eq!(updated.links.iter().map(|l| l.title.as_str()).collect::<Vec<_>>(), vec!["b", "c"]);
    assert_eq!(updated.summary.icon.as_option().unwrap().asset_id, icon, "an unset icon stays");
    assert_eq!(ctx.app.events_by("video.channel.updated", alice.id).await.len(), 1);
    // The old handle is free again; the new one resolves.
    assert!(ctx.get_channel(None, &new_handle).await.is_ok());
    assert_eq!(ctx.get_channel(None, &request.handle).await.unwrap_err().reason, "CHANNEL_NOT_FOUND");

    // An empty asset ID removes the picture and releases the upload; an empty link list clears the links.
    let clear = UpdateChannelRequest {
        channel_id: channel_id.clone(),
        icon_asset_id: Some(String::new()),
        links: buffa::MessageField::some(ChannelLinkList::default()),
        ..Default::default()
    };
    let response: UpdateChannelResponse = ctx.call(CHANNELS, "UpdateChannel", Some(&alice), &clear).await.unwrap();
    let cleared = response.channel.as_option().unwrap();
    assert!(cleared.summary.icon.as_option().is_none() && cleared.links.is_empty());
    assert!(!assets::is_attached(&ctx.app.core, id(&icon)).await.unwrap());

    // The trailer has to be a public video of this very channel.
    let draft = ctx.draft(&alice, &channel, "下書き").await;
    let trailer = |video_id: &str| UpdateChannelRequest { channel_id: channel_id.clone(), trailer_video_id: Some(video_id.to_owned()), ..Default::default() };
    assert_eq!(ctx.reason(CHANNELS, "UpdateChannel", Some(&alice), &trailer(&card(&draft).id)).await, "TRAILER_NOT_FOUND");
    let public = ctx.public_video(&alice, &channel, "紹介動画").await;
    let response: UpdateChannelResponse = ctx.call(CHANNELS, "UpdateChannel", Some(&alice), &trailer(&card(&public).short_id)).await.unwrap();
    assert_eq!(response.channel.trailer_video_id, card(&public).short_id);
    let response: UpdateChannelResponse = ctx.call(CHANNELS, "UpdateChannel", Some(&alice), &trailer("")).await.unwrap();
    assert_eq!(response.channel.trailer_video_id, "");

    // Validation applies to updates too, and other people's channels are not found.
    let bad = UpdateChannelRequest { channel_id: channel_id.clone(), name: Some(String::new()), ..Default::default() };
    assert_eq!(ctx.reason(CHANNELS, "UpdateChannel", Some(&alice), &bad).await, "CHANNEL_NAME_INVALID");
    let other = ctx.channel(&bob, "bob").await;
    let steal = UpdateChannelRequest { channel_id: channel_id.clone(), handle: Some(other.summary.handle.clone()), ..Default::default() };
    assert_eq!(ctx.reason(CHANNELS, "UpdateChannel", Some(&alice), &steal).await, "HANDLE_TAKEN");
    let foreign = UpdateChannelRequest { channel_id: channel_id.clone(), name: Some("mine now".into()), ..Default::default() };
    assert_eq!(ctx.reason(CHANNELS, "UpdateChannel", Some(&bob), &foreign).await, "CHANNEL_NOT_FOUND");
    assert_eq!(ctx.reason(CHANNELS, "UpdateChannel", None, &foreign).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn the_default_channel_can_be_switched_and_survives_deletion() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let first = ctx.channel(&alice, "first").await;
    let second = ctx.channel(&alice, "second").await;
    let third = ctx.channel(&alice, "third").await;

    let request = SetDefaultChannelRequest { channel_id: second.summary.id.clone(), ..Default::default() };
    let response: SetDefaultChannelResponse = ctx.call(CHANNELS, "SetDefaultChannel", Some(&alice), &request).await.unwrap();
    assert!(response.channel.viewer.as_option().unwrap().is_default);
    assert_eq!(tokuly_video::channels::default_for(&ctx.app.core, alice.id).await.unwrap().unwrap().id, id(&second.summary.id));
    assert_eq!(ctx.reason(CHANNELS, "SetDefaultChannel", Some(&bob), &request).await, "CHANNEL_NOT_FOUND");

    // Deleting the default hands the role to the oldest channel left, and takes the content with it.
    let video = ctx.public_video(&alice, &second, "消える動画").await;
    let upload = video.studio.as_option().unwrap().asset_id.clone();
    let delete = DeleteChannelRequest { channel_id: second.summary.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(CHANNELS, "DeleteChannel", Some(&bob), &delete).await, "CHANNEL_NOT_FOUND");
    let _: DeleteChannelResponse = ctx.call(CHANNELS, "DeleteChannel", Some(&alice), &delete).await.unwrap();
    assert_eq!(ctx.get_channel(Some(&alice), &second.summary.id).await.unwrap_err().reason, "CHANNEL_NOT_FOUND");
    assert_eq!(ctx.get_video(Some(&alice), &card(&video).id).await.unwrap_err().reason, "VIDEO_NOT_FOUND");
    assert!(assets::find(&ctx.app.core, id(&upload)).await.unwrap().is_none(), "the upload is deleted with the video");
    let mine: ListMyChannelsResponse = ctx.call(CHANNELS, "ListMyChannels", Some(&alice), &ListMyChannelsRequest::default()).await.unwrap();
    assert_eq!(mine.channels.iter().map(|c| c.summary.id.as_str()).collect::<Vec<_>>(), vec![first.summary.id.as_str(), third.summary.id.as_str()]);
    assert!(mine.channels[0].viewer.as_option().unwrap().is_default);
    assert_eq!(ctx.app.events_by("video.channel.deleted", alice.id).await.len(), 1);
    assert_eq!(ctx.app.events_by("video.video.deleted", alice.id).await.len(), 1);
    assert!(search_docs(&ctx, &second.summary.id).await.last().unwrap().is_none(), "the channel left the search index");
}

#[tokio::test]
async fn handles_can_be_checked_before_they_are_taken() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let taken = ctx.channel(&alice, "taken").await.summary.handle.clone();
    let check = |handle: &str| CheckChannelHandleRequest { handle: handle.to_owned(), ..Default::default() };
    let free = unique("free");
    let response: CheckChannelHandleResponse = ctx.call(CHANNELS, "CheckChannelHandle", None, &check(&format!("@{}", free.to_uppercase()))).await.unwrap();
    assert_eq!((response.available, response.normalized.as_str(), response.reason.as_str()), (true, free.as_str(), ""));
    for (handle, reason) in [(taken.as_str(), "HANDLE_TAKEN"), ("no", "HANDLE_INVALID"), ("admin", "HANDLE_RESERVED")] {
        let response: CheckChannelHandleResponse = ctx.call(CHANNELS, "CheckChannelHandle", Some(&alice), &check(handle)).await.unwrap();
        assert_eq!((response.available, response.reason.as_str()), (false, reason), "{handle}");
    }
}

#[tokio::test]
async fn channels_are_found_by_id_or_handle_unless_hidden() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "見つけてね").await;
    let handle = channel.summary.handle.clone();
    for reference in [channel.summary.id.clone(), handle.clone(), format!("@{handle}"), handle.to_uppercase()] {
        let found = ctx.get_channel(None, &reference).await.unwrap_or_else(|e| panic!("{reference}: {e}"));
        assert_eq!(found.summary.id, channel.summary.id);
        assert!(found.viewer.as_option().is_none(), "anonymous callers have no viewer state");
    }
    let seen = ctx.get_channel(Some(&bob), &handle).await.unwrap();
    let state = seen.viewer.as_option().unwrap();
    assert!(!state.is_owner && !state.subscribed && !state.is_default);
    for missing in [uuid::Uuid::now_v7().to_string(), unique("ghost"), "not a handle!".to_owned(), String::new()] {
        assert_eq!(ctx.get_channel(None, &missing).await.unwrap_err().reason, "CHANNEL_NOT_FOUND", "{missing}");
    }

    // A block in either direction hides the channel; the owner still sees it.
    ctx.block(&bob, &alice).await;
    assert_eq!(ctx.get_channel(Some(&bob), &handle).await.unwrap_err().reason, "CHANNEL_NOT_FOUND");
    assert!(ctx.get_channel(Some(&alice), &handle).await.is_ok());
    let carol = ctx.app.user("carol").await;
    ctx.block(&alice, &carol).await;
    assert_eq!(ctx.get_channel(Some(&carol), &handle).await.unwrap_err().reason, "CHANNEL_NOT_FOUND");
}

#[tokio::test]
async fn the_channel_page_lists_what_the_viewer_may_see() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let friend = ctx.app.user("friend").await;
    let stranger = ctx.app.user("stranger").await;
    ctx.befriend(&alice, &friend).await;
    let channel = ctx.channel(&alice, "page").await;
    let channel_id = channel.summary.id.clone();

    let public_old = ctx.public_video(&alice, &channel, "公開1").await;
    let no_feed = ctx.published(&alice, &channel, "フィード外", Visibility::VISIBILITY_NO_FEED).await;
    ctx.published(&alice, &channel, "リンク限定", Visibility::VISIBILITY_UNLISTED).await;
    ctx.published(&alice, &channel, "フレンド限定", Visibility::VISIBILITY_FRIENDS).await;
    ctx.published(&alice, &channel, "非公開", Visibility::VISIBILITY_PRIVATE).await;
    ctx.draft_with(&alice, &channel, "下書き", |r| r.visibility = Visibility::VISIBILITY_PUBLIC.into()).await;
    let public_new = ctx.public_video(&alice, &channel, "公開2").await;

    let list = |sort: ChannelVideoSort| ListChannelVideosRequest { channel_id: channel_id.clone(), sort: sort.into(), ..Default::default() };
    let newest = ChannelVideoSort::CHANNEL_VIDEO_SORT_UNSPECIFIED;
    let listed: ListChannelVideosResponse = ctx.call(CHANNELS, "ListChannelVideos", Some(&stranger), &list(newest)).await.unwrap();
    assert_eq!(titles(&listed.videos), vec!["公開2", "フィード外", "公開1"], "newest first; unlisted, friends-only, private and drafts are not listed");
    let listed: ListChannelVideosResponse = ctx.call(CHANNELS, "ListChannelVideos", None, &list(newest)).await.unwrap();
    assert_eq!(titles(&listed.videos), vec!["公開2", "フィード外", "公開1"]);
    let listed: ListChannelVideosResponse = ctx.call(CHANNELS, "ListChannelVideos", Some(&friend), &list(newest)).await.unwrap();
    assert_eq!(titles(&listed.videos), vec!["公開2", "フレンド限定", "フィード外", "公開1"], "friends also see friends-only videos");
    let listed: ListChannelVideosResponse = ctx.call(CHANNELS, "ListChannelVideos", Some(&alice), &list(newest)).await.unwrap();
    assert_eq!(titles(&listed.videos), vec!["公開2", "フレンド限定", "フィード外", "公開1"], "the owner sees their page like a friend does");

    let listed: ListChannelVideosResponse = ctx.call(CHANNELS, "ListChannelVideos", None, &list(ChannelVideoSort::CHANNEL_VIDEO_SORT_OLDEST)).await.unwrap();
    assert_eq!(titles(&listed.videos), vec!["公開1", "フィード外", "公開2"]);

    // Views decide the popular order.
    ctx.watch(Some(&stranger), &card(&no_feed).id, 3.0, TrafficSource::TRAFFIC_SOURCE_CHANNEL).await;
    let listed: ListChannelVideosResponse = ctx.call(CHANNELS, "ListChannelVideos", None, &list(ChannelVideoSort::CHANNEL_VIDEO_SORT_POPULAR)).await.unwrap();
    assert_eq!(titles(&listed.videos)[0], "フィード外");

    // Pages follow each other without gaps or repeats, in every order.
    for sort in [ChannelVideoSort::CHANNEL_VIDEO_SORT_NEWEST, ChannelVideoSort::CHANNEL_VIDEO_SORT_OLDEST, ChannelVideoSort::CHANNEL_VIDEO_SORT_POPULAR] {
        let whole: ListChannelVideosResponse = ctx.call(CHANNELS, "ListChannelVideos", None, &list(sort)).await.unwrap();
        let mut paged = Vec::new();
        let mut cursor = String::new();
        loop {
            let request = ListChannelVideosRequest { page: page(2, &cursor), ..list(sort) };
            let response: ListChannelVideosResponse = ctx.call(CHANNELS, "ListChannelVideos", None, &request).await.unwrap();
            paged.extend(response.videos.iter().map(|video| video.title.clone()));
            let info = response.page.as_option().unwrap();
            if !info.has_more {
                break;
            }
            cursor = info.next_cursor.clone();
        }
        assert_eq!(paged, titles(&whole.videos), "{sort:?}");
    }

    // The header counts what everyone can see listed.
    let header = ctx.get_channel(None, &channel_id).await.unwrap();
    assert_eq!((header.video_count, header.view_count), (3, 1));

    // The home tab: trailer, latest, popular; no archives or playlists yet.
    let trailer = UpdateChannelRequest { channel_id: channel_id.clone(), trailer_video_id: Some(card(&public_old).id.clone()), ..Default::default() };
    let _: UpdateChannelResponse = ctx.call(CHANNELS, "UpdateChannel", Some(&alice), &trailer).await.unwrap();
    let home: GetChannelPageResponse = ctx.call(CHANNELS, "GetChannelPage", None, &GetChannelPageRequest { channel_id: channel_id.clone(), ..Default::default() }).await.unwrap();
    let kinds: Vec<_> = home.sections.iter().map(|section| section.kind.as_known().unwrap()).collect();
    assert_eq!(kinds, vec![ChannelSectionKind::CHANNEL_SECTION_KIND_TRAILER, ChannelSectionKind::CHANNEL_SECTION_KIND_LATEST, ChannelSectionKind::CHANNEL_SECTION_KIND_POPULAR]);
    assert_eq!(titles(&home.sections[0].videos), vec!["公開1"]);
    assert_eq!(titles(&home.sections[1].videos), vec!["公開2", "フィード外", "公開1"]);
    assert_eq!(home.channel.trailer_video_id, card(&public_old).short_id);

    // A public playlist with something in it gets its own section.
    let playlist = CreatePlaylistRequest {
        title: "おすすめ".into(),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        channel_id: channel_id.clone(),
        video_id: card(&public_new).id.clone(),
        ..Default::default()
    };
    let _: CreatePlaylistResponse = ctx.call(PLAYLISTS, "CreatePlaylist", Some(&alice), &playlist).await.unwrap();
    let home: GetChannelPageResponse = ctx.call(CHANNELS, "GetChannelPage", None, &GetChannelPageRequest { channel_id: channel_id.clone(), ..Default::default() }).await.unwrap();
    let playlists = home.sections.iter().find(|section| section.kind == ChannelSectionKind::CHANNEL_SECTION_KIND_PLAYLISTS).expect("a playlists section");
    assert_eq!(playlists.playlists[0].title, "おすすめ");

    let missing = GetChannelPageRequest { channel_id: unique("nochannel"), ..Default::default() };
    assert_eq!(ctx.reason(CHANNELS, "GetChannelPage", None, &missing).await, "CHANNEL_NOT_FOUND");
    let missing = ListChannelVideosRequest { channel_id: unique("nochannel"), ..Default::default() };
    assert_eq!(ctx.reason(CHANNELS, "ListChannelVideos", None, &missing).await, "CHANNEL_NOT_FOUND");
}

#[tokio::test]
async fn subscribing_counts_notifies_and_can_be_tuned() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "登録してね").await;
    let channel_id = channel.summary.id.clone();

    let subscribed = ctx.subscribe(&bob, &channel_id, NotificationLevel::NOTIFICATION_LEVEL_UNSPECIFIED).await;
    let state = subscribed.viewer.as_option().unwrap();
    assert!(state.subscribed && state.subscribed_at.as_option().is_some());
    assert_eq!(state.notification_level, NotificationLevel::NOTIFICATION_LEVEL_PERSONALIZED, "the default level");
    assert_eq!(subscribed.summary.subscriber_count, 1);
    assert!(tokuly_video::channels::is_subscribed(&ctx.app.core, bob.id, id(&channel_id)).await.unwrap());

    let events = ctx.app.events_by("video.channel.subscribed", bob.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["owner_id"], alice.id.to_string());
    let notices = ctx.app.events_by("notify.requested", bob.id).await;
    assert_eq!(notices.len(), 1);
    assert_eq!((notices[0].data["kind"].as_str(), notices[0].data["user_id"].as_str()), (Some("video.subscribed"), Some(alice.id.to_string().as_str())));
    assert_eq!(notices[0].data["collapse_key"], format!("video.subscribed:{channel_id}"));

    // Subscribing again only changes the level: no second count, event or notification.
    let again = ctx.subscribe(&bob, &channel_id, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    assert_eq!(again.viewer.as_option().unwrap().notification_level, NotificationLevel::NOTIFICATION_LEVEL_ALL);
    assert_eq!(again.summary.subscriber_count, 1);
    let unchanged = ctx.subscribe(&bob, &channel_id, NotificationLevel::NOTIFICATION_LEVEL_UNSPECIFIED).await;
    assert_eq!(unchanged.viewer.as_option().unwrap().notification_level, NotificationLevel::NOTIFICATION_LEVEL_ALL, "unspecified keeps the level");
    assert_eq!(ctx.app.events_by("video.channel.subscribed", bob.id).await.len(), 1);
    assert_eq!(ctx.app.events_by("notify.requested", bob.id).await.len(), 1);

    use tokuly_video::channels::{NotifyLevel, subscriber_ids};
    assert_eq!(subscriber_ids(&ctx.app.core, id(&channel_id), NotifyLevel::All).await.unwrap(), vec![bob.id]);
    ctx.subscribe(&bob, &channel_id, NotificationLevel::NOTIFICATION_LEVEL_NONE).await;
    assert!(subscriber_ids(&ctx.app.core, id(&channel_id), NotifyLevel::Personalized).await.unwrap().is_empty());
    assert_eq!(subscriber_ids(&ctx.app.core, id(&channel_id), NotifyLevel::None).await.unwrap(), vec![bob.id]);

    // What cannot be done.
    let request = SubscribeRequest { channel_id: channel_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(CHANNELS, "Subscribe", Some(&alice), &request).await, "CANNOT_SUBSCRIBE_TO_OWN_CHANNEL");
    assert_eq!(ctx.reason(CHANNELS, "Subscribe", None, &request).await, "SIGN_IN_REQUIRED");
    let missing = SubscribeRequest { channel_id: unique("nochannel"), ..Default::default() };
    assert_eq!(ctx.reason(CHANNELS, "Subscribe", Some(&bob), &missing).await, "CHANNEL_NOT_FOUND");
    let carol = ctx.app.user("carol").await;
    ctx.block(&alice, &carol).await;
    assert_eq!(ctx.reason(CHANNELS, "Subscribe", Some(&carol), &request).await, "CHANNEL_NOT_FOUND", "blocked users cannot subscribe");

    // Unsubscribing takes the count back; doing it twice is fine.
    let unsubscribe = UnsubscribeRequest { channel_id: channel_id.clone(), ..Default::default() };
    let response: UnsubscribeResponse = ctx.call(CHANNELS, "Unsubscribe", Some(&bob), &unsubscribe).await.unwrap();
    assert_eq!(response.channel.summary.subscriber_count, 0);
    assert!(!response.channel.viewer.as_option().unwrap().subscribed);
    let _: UnsubscribeResponse = ctx.call(CHANNELS, "Unsubscribe", Some(&bob), &unsubscribe).await.unwrap();
    assert_eq!(ctx.get_channel(None, &channel_id).await.unwrap().summary.subscriber_count, 0);
    assert_eq!(ctx.app.events_by("video.channel.unsubscribed", bob.id).await.len(), 1);
    assert_eq!(ctx.reason(CHANNELS, "Unsubscribe", None, &unsubscribe).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn subscriptions_and_subscribers_are_listed_newest_first() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let first = ctx.channel(&alice, "first").await;
    let second = ctx.channel(&alice, "second").await;
    let third = ctx.channel(&carol, "third").await;
    for channel in [&first, &second, &third] {
        ctx.subscribe(&bob, &channel.summary.id, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    }
    ctx.subscribe(&carol, &first.summary.id, NotificationLevel::NOTIFICATION_LEVEL_UNSPECIFIED).await;

    let mine: ListSubscriptionsResponse = ctx.call(CHANNELS, "ListSubscriptions", Some(&bob), &ListSubscriptionsRequest::default()).await.unwrap();
    assert_eq!(mine.subscriptions.iter().map(|s| s.channel.name.as_str()).collect::<Vec<_>>(), vec!["third", "second", "first"]);
    assert_eq!(mine.subscriptions[0].notification_level, NotificationLevel::NOTIFICATION_LEVEL_ALL);
    let first_page: ListSubscriptionsResponse = ctx.call(CHANNELS, "ListSubscriptions", Some(&bob), &ListSubscriptionsRequest { page: page(2, ""), ..Default::default() }).await.unwrap();
    let cursor = first_page.page.as_option().unwrap().next_cursor.clone();
    let rest: ListSubscriptionsResponse = ctx.call(CHANNELS, "ListSubscriptions", Some(&bob), &ListSubscriptionsRequest { page: page(2, &cursor), ..Default::default() }).await.unwrap();
    assert_eq!((first_page.subscriptions.len(), rest.subscriptions.len()), (2, 1));
    assert_eq!(rest.subscriptions[0].channel.name, "first");
    assert!(!rest.page.as_option().unwrap().has_more);
    assert_eq!(ctx.reason(CHANNELS, "ListSubscriptions", None, &ListSubscriptionsRequest::default()).await, "SIGN_IN_REQUIRED");

    // A channel whose owner the subscriber has since blocked drops out of the list.
    ctx.block(&bob, &carol).await;
    let mine: ListSubscriptionsResponse = ctx.call(CHANNELS, "ListSubscriptions", Some(&bob), &ListSubscriptionsRequest::default()).await.unwrap();
    assert_eq!(mine.subscriptions.len(), 2);

    // Only the owner sees who subscribes.
    let request = ListSubscribersRequest { channel_id: first.summary.id.clone(), ..Default::default() };
    let subscribers: ListSubscribersResponse = ctx.call(CHANNELS, "ListSubscribers", Some(&alice), &request).await.unwrap();
    assert_eq!(subscribers.subscribers.iter().map(|s| s.user.id.clone()).collect::<Vec<_>>(), vec![carol.id.to_string(), bob.id.to_string()]);
    assert_eq!(subscribers.subscriber_count, 2);
    assert_eq!(ctx.reason(CHANNELS, "ListSubscribers", Some(&bob), &request).await, "CHANNEL_NOT_FOUND");
    assert_eq!(ctx.reason(CHANNELS, "ListSubscribers", None, &request).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn recommended_channels_have_videos_and_are_new_to_the_viewer() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let viewer = ctx.app.user("viewer").await;
    let fan = ctx.app.user("fan").await;
    let popular = ctx.channel(&alice, "popular").await;
    let quiet = ctx.channel(&alice, "quiet").await;
    let empty = ctx.channel(&bob, "empty").await;
    let followed = ctx.channel(&bob, "followed").await;
    let own = ctx.channel(&viewer, "own").await;
    for channel in [&popular, &quiet, &followed] {
        ctx.public_video(if channel.summary.id == followed.summary.id { &bob } else { &alice }, channel, "動画").await;
    }
    ctx.public_video(&viewer, &own, "自分の動画").await;
    for subscriber in [&fan, &bob] {
        ctx.subscribe(subscriber, &popular.summary.id, NotificationLevel::NOTIFICATION_LEVEL_UNSPECIFIED).await;
    }
    ctx.subscribe(&viewer, &followed.summary.id, NotificationLevel::NOTIFICATION_LEVEL_UNSPECIFIED).await;

    // Other tests share the database: look only at this test's channels.
    let mine = [&popular, &quiet, &empty, &followed, &own].map(|channel| channel.summary.id.clone());
    let recommended = |response: ListRecommendedChannelsResponse| -> Vec<String> {
        response.channels.into_iter().filter(|channel| mine.contains(&channel.id)).map(|channel| channel.name).collect()
    };
    let request = ListRecommendedChannelsRequest { limit: 50, ..Default::default() };
    let response: ListRecommendedChannelsResponse = ctx.call(CHANNELS, "ListRecommendedChannels", Some(&viewer), &request).await.unwrap();
    assert_eq!(recommended(response), vec!["popular", "quiet"], "most subscribers first; no empty, followed or own channels");
    let response: ListRecommendedChannelsResponse = ctx.call(CHANNELS, "ListRecommendedChannels", None, &request).await.unwrap();
    let anonymous = recommended(response);
    assert_eq!(anonymous[0], "popular");
    assert!(anonymous.contains(&"followed".to_owned()) && anonymous.contains(&"own".to_owned()) && !anonymous.contains(&"empty".to_owned()));

    // "Don't recommend this channel" and blocks are honoured.
    let hide = MarkNotInterestedRequest { target: Some(mark_not_interested_request::Target::ChannelId(popular.summary.id.clone())), ..Default::default() };
    let _: MarkNotInterestedResponse = ctx.call(FEEDS, "MarkNotInterested", Some(&viewer), &hide).await.unwrap();
    let response: ListRecommendedChannelsResponse = ctx.call(CHANNELS, "ListRecommendedChannels", Some(&viewer), &request).await.unwrap();
    assert_eq!(recommended(response), vec!["quiet"]);
    ctx.block(&viewer, &alice).await;
    let response: ListRecommendedChannelsResponse = ctx.call(CHANNELS, "ListRecommendedChannels", Some(&viewer), &request).await.unwrap();
    assert!(recommended(response).is_empty());
}
