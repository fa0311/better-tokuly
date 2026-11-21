//! What happens outside of requests: reactions to other domains' events,
//! the subscriber announcement job, subject resolvers, the data export and
//! the Rust API other domains call.
mod common;

use common::*;
use tokuly_core::prelude::*;
use tokuly_media::assets;
use tokuly_proto::{
    common::{SubjectType, Visibility},
    media::AssetPurpose,
    video::*,
};
use tokuly_testkit::TestUser;
use tokuly_video::{VideoConfig, channels, events::AnnounceJob, games, videos};

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

/// `video.published` notifications requested for `user`, as `(inbox, push)`.
async fn announcements(ctx: &Ctx, user: &TestUser) -> Vec<(bool, bool)> {
    ctx.app
        .events("notify.requested")
        .await
        .into_iter()
        .filter(|event| event.data["user_id"] == user.id.to_string() && event.data["kind"] == "video.published")
        .map(|event| (event.data["inbox"].as_bool().unwrap(), event.data["push"].as_bool().unwrap()))
        .collect()
}

async fn user_deleted(ctx: &Ctx, user: &TestUser) {
    ctx.deliver_event("identity.user.deleted", Some(user.id), serde_json::json!({ "user_id": user.id })).await;
}

#[tokio::test]
async fn subscribers_are_told_about_a_new_video_according_to_their_level() {
    let ctx = start_with(VideoConfig { announce_batch_size: 2, ..test_config() }).await;
    let alice = ctx.app.user("alice").await;
    let everything = ctx.app.user("everything").await;
    let engaged = ctx.app.user("engaged").await;
    let casual = ctx.app.user("casual").await;
    let silent = ctx.app.user("silent").await;
    let outsider = ctx.app.user("outsider").await;
    let channel = ctx.channel(&alice, "お知らせch").await;
    let channel_id = channel.summary.id.clone();
    let earlier = ctx.public_video(&alice, &channel, "前の動画").await;
    ctx.subscribe(&everything, &channel_id, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    ctx.subscribe(&engaged, &channel_id, NotificationLevel::NOTIFICATION_LEVEL_PERSONALIZED).await;
    ctx.subscribe(&casual, &channel_id, NotificationLevel::NOTIFICATION_LEVEL_PERSONALIZED).await;
    ctx.subscribe(&silent, &channel_id, NotificationLevel::NOTIFICATION_LEVEL_NONE).await;
    // "engaged" keeps watching the channel.
    ctx.watch(Some(&engaged), &card(&earlier).id, 3.0, TrafficSource::TRAFFIC_SOURCE_SUBSCRIPTIONS).await;

    let video = ctx.public_video(&alice, &channel, "新作だよ").await;
    let video_id = id(&card(&video).id);
    // The consumer of `video.video.published` queues the first batch...
    assert!(ctx.deliver("video.video.published", alice.id).await >= 1);
    // ...and each job handles one batch (two subscribers here) and queues the next.
    let mut subscribers =
        channels::subscriber_ids(&ctx.app.core, id(&channel_id), channels::NotifyLevel::Personalized).await.unwrap();
    subscribers.sort_unstable();
    assert_eq!(subscribers.len(), 3);
    assert_eq!(ctx.video.announce(AnnounceJob { video_id, after: None }).await.unwrap(), 2);
    assert_eq!(ctx.video.announce(AnnounceJob { video_id, after: Some(subscribers[1]) }).await.unwrap(), 1);
    assert_eq!(ctx.video.announce(AnnounceJob { video_id, after: Some(subscribers[2]) }).await.unwrap(), 0);

    // ALL: inbox and push. PERSONALIZED: inbox, and a push only for those who keep watching. NONE: nothing.
    assert_eq!(announcements(&ctx, &everything).await, vec![(true, true)]);
    assert_eq!(announcements(&ctx, &engaged).await, vec![(true, true)]);
    assert_eq!(announcements(&ctx, &casual).await, vec![(true, false)]);
    assert!(announcements(&ctx, &silent).await.is_empty() && announcements(&ctx, &outsider).await.is_empty());
    let request = ctx
        .app
        .events("notify.requested")
        .await
        .into_iter()
        .find(|e| e.data["user_id"] == everything.id.to_string() && e.data["kind"] == "video.published")
        .unwrap();
    assert_eq!(request.data["title"]["ja"], "お知らせch");
    assert!(request.data["body"]["ja"].as_str().unwrap().contains("新作だよ"));
    assert_eq!(request.data["path"], format!("/watch/{}", card(&video).short_id));
    assert_eq!(request.data["collapse_key"], format!("video.published:{video_id}"));
    assert!(request.data["actor_id"].is_null(), "the channel speaks, not the account behind it");

    // Videos that opt out, are not public, or are gone announce nothing.
    let quiet = ctx
        .published_with(&alice, &channel, "こっそり", Visibility::VISIBILITY_PUBLIC, |r| {
            r.notify_subscribers = Some(false)
        })
        .await;
    assert_eq!(ctx.video.announce(AnnounceJob { video_id: id(&card(&quiet).id), after: None }).await.unwrap(), 0);
    let unlisted = ctx.published(&alice, &channel, "リンク限定", Visibility::VISIBILITY_UNLISTED).await;
    assert_eq!(ctx.video.announce(AnnounceJob { video_id: id(&card(&unlisted).id), after: None }).await.unwrap(), 0);
    assert_eq!(ctx.video.announce(AnnounceJob { video_id: Uuid::now_v7(), after: None }).await.unwrap(), 0);

    // A friends-only video is announced to subscribers who are friends.
    ctx.befriend(&alice, &casual).await;
    let friends_only = ctx.published(&alice, &channel, "フレンド限定", Visibility::VISIBILITY_FRIENDS).await;
    let mut told = 0;
    for after in [None, Some(subscribers[1])] {
        told += ctx.video.announce(AnnounceJob { video_id: id(&card(&friends_only).id), after }).await.unwrap();
    }
    assert_eq!(told, 1);
    assert_eq!(announcements(&ctx, &casual).await.len(), 2);
    assert_eq!(announcements(&ctx, &everything).await.len(), 1);
}

#[tokio::test]
async fn a_deleted_account_takes_its_channels_and_traces_with_it() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let alices = ctx.channel(&alice, "alice").await;
    let bobs = ctx.channel(&bob, "bob").await;
    let alices_video = ctx.public_video(&alice, &alices, "alice's").await;
    let bobs_video = ctx.public_video(&bob, &bobs, "bob's").await;
    let upload = alices_video.studio.as_option().unwrap().asset_id.clone();

    // Alice leaves traces on bob's content, and bob on hers.
    ctx.subscribe(&alice, &bobs.summary.id, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    ctx.subscribe(&bob, &alices.summary.id, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    ctx.rate(&alice, &card(&bobs_video).id, Rating::RATING_LIKE).await;
    ctx.rate(&carol, &card(&bobs_video).id, Rating::RATING_LIKE).await;
    ctx.watch(Some(&alice), &card(&bobs_video).id, 3.0, TrafficSource::TRAFFIC_SOURCE_HOME).await;
    let playlist = CreatePlaylistRequest {
        title: "alice's list".into(),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        video_id: card(&bobs_video).id.clone(),
        ..Default::default()
    };
    let playlist: CreatePlaylistResponse =
        ctx.call(PLAYLISTS, "CreatePlaylist", Some(&alice), &playlist).await.unwrap();
    let _: AddPlaylistItemResponse = ctx
        .call(
            PLAYLISTS,
            "AddPlaylistItem",
            Some(&bob),
            &AddPlaylistItemRequest {
                playlist_id: "WL".into(),
                video_id: card(&alices_video).id.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // Alice hosts a party on bob's video, with carol in it.
    let party: CreatePartyResponse = ctx
        .call(
            PARTIES,
            "CreateParty",
            Some(&alice),
            &CreatePartyRequest { video_id: card(&bobs_video).id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let party_id = party.session.party.id.clone();
    let _: JoinPartyResponse = ctx
        .call(
            PARTIES,
            "JoinParty",
            Some(&carol),
            &JoinPartyRequest { code: party.session.party.code.clone(), ..Default::default() },
        )
        .await
        .unwrap();

    user_deleted(&ctx, &alice).await;

    // Her channel and video are gone, with the upload and the search documents.
    assert_eq!(ctx.get_channel(None, &alices.summary.id).await.unwrap_err().reason, "CHANNEL_NOT_FOUND");
    assert_eq!(ctx.get_video(None, &card(&alices_video).id).await.unwrap_err().reason, "VIDEO_NOT_FOUND");
    assert!(assets::find(&ctx.app.core, id(&upload)).await.unwrap().is_none());
    assert!(search_docs(&ctx, &alices.summary.id).await.last().unwrap().is_none());
    assert!(search_docs(&ctx, &playlist.playlist.id).await.last().unwrap().is_none());
    let get = GetPlaylistRequest { playlist_id: playlist.playlist.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(PLAYLISTS, "GetPlaylist", None, &get).await, "PLAYLIST_NOT_FOUND");
    // Bob's counters lose her share; carol's like stays.
    assert_eq!(ctx.get_channel(None, &bobs.summary.id).await.unwrap().summary.subscriber_count, 0);
    assert_eq!(card(&ctx.get_video(None, &card(&bobs_video).id).await.unwrap()).like_count, 1);
    // Bob's own lists no longer contain her video.
    let mine: ListSubscriptionsResponse =
        ctx.call(CHANNELS, "ListSubscriptions", Some(&bob), &ListSubscriptionsRequest::default()).await.unwrap();
    assert!(mine.subscriptions.is_empty());
    let lists: ListMyPlaylistsResponse =
        ctx.call(PLAYLISTS, "ListMyPlaylists", Some(&bob), &ListMyPlaylistsRequest::default()).await.unwrap();
    assert!(lists.playlists.iter().all(|list| list.item_count == 0));
    // The party goes on with carol as host.
    let seen: GetPartyResponse =
        ctx.call(PARTIES, "GetParty", Some(&carol), &GetPartyRequest { party_id, ..Default::default() }).await.unwrap();
    assert_eq!((seen.session.members.len(), seen.session.self_.role), (1, PartyRole::PARTY_ROLE_HOST.into()));
    let left: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM watch_history WHERE user_id = $1) + (SELECT count(*) FROM video_reactions WHERE user_id = $1)
              + (SELECT count(*) FROM channel_subscriptions WHERE user_id = $1) + (SELECT count(*) FROM playlists WHERE owner_id = $1)
              + (SELECT count(*) FROM channels WHERE owner_id = $1) + (SELECT count(*) FROM videos WHERE owner_id = $1)",
    )
    .bind(alice.id)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(left, 0, "nothing of the account is left in the video tables");

    // The event may arrive twice.
    user_deleted(&ctx, &alice).await;
    assert_eq!(ctx.get_channel(None, &bobs.summary.id).await.unwrap().summary.subscriber_count, 0);
    assert_eq!(card(&ctx.get_video(None, &card(&bobs_video).id).await.unwrap()).like_count, 1);
}

#[tokio::test]
async fn a_suspended_account_disappears_until_it_is_reinstated() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.public_video(&alice, &channel, "v").await;
    let (channel_id, video_id) = (channel.summary.id.clone(), card(&video).id.clone());
    let playlist = CreatePlaylistRequest {
        title: "list".into(),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        channel_id: channel_id.clone(),
        video_id: video_id.clone(),
        ..Default::default()
    };
    let playlist: CreatePlaylistResponse =
        ctx.call(PLAYLISTS, "CreatePlaylist", Some(&alice), &playlist).await.unwrap();
    let playlist_id = playlist.playlist.id.clone();

    // What identity does when a moderator suspends the account: the flag, then the event.
    sqlx::query("UPDATE users SET banned_at = now() WHERE id = $1")
        .bind(alice.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.deliver_event("identity.user.banned", None, serde_json::json!({ "user_id": alice.id })).await;
    assert_eq!(ctx.get_channel(Some(&bob), &channel_id).await.unwrap_err().reason, "CHANNEL_NOT_FOUND");
    assert_eq!(ctx.get_video(None, &video_id).await.unwrap_err().reason, "VIDEO_NOT_FOUND");
    let get = GetPlaylistRequest { playlist_id: playlist_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(PLAYLISTS, "GetPlaylist", Some(&bob), &get).await, "PLAYLIST_NOT_FOUND");
    let batch: BatchGetVideosResponse = ctx
        .call(
            VIDEOS,
            "BatchGetVideos",
            Some(&bob),
            &BatchGetVideosRequest { video_ids: vec![video_id.clone()], ..Default::default() },
        )
        .await
        .unwrap();
    assert!(batch.videos.is_empty());
    let home: GetHomeFeedResponse = ctx
        .call(FEEDS, "GetHomeFeed", Some(&bob), &GetHomeFeedRequest { page: page(50, ""), ..Default::default() })
        .await
        .unwrap();
    assert!(home.videos.iter().all(|shown| shown.id != video_id));
    for document in [&channel_id, &video_id, &playlist_id] {
        assert!(search_docs(&ctx, document).await.last().unwrap().is_none(), "search forgets suspended accounts");
    }
    // A video created during the suspension is hidden from the start.
    let during = videos::create_from_asset(
        &ctx.app.core,
        videos::NewVideo {
            visibility: Visibility::VISIBILITY_PUBLIC,
            publish: true,
            ..videos::NewVideo::new(id(&channel_id), id(&ctx.video_upload(&alice).await), "during")
        },
    )
    .await
    .unwrap();
    assert_eq!(ctx.get_video(Some(&bob), &during.to_string()).await.unwrap_err().reason, "VIDEO_NOT_FOUND");
    assert!(search_docs(&ctx, &during.to_string()).await.is_empty());

    sqlx::query("UPDATE users SET banned_at = NULL WHERE id = $1")
        .bind(alice.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.deliver_event("identity.user.unbanned", None, serde_json::json!({ "user_id": alice.id })).await;
    assert!(ctx.get_channel(Some(&bob), &channel_id).await.is_ok());
    assert!(ctx.get_video(None, &video_id).await.is_ok() && ctx.get_video(None, &during.to_string()).await.is_ok());
    assert!(ctx.call::<_, GetPlaylistResponse>(PLAYLISTS, "GetPlaylist", Some(&bob), &get).await.is_ok());
    for document in [&channel_id, &video_id, &playlist_id, &during.to_string()] {
        assert!(search_docs(&ctx, document).await.last().unwrap().is_some(), "and remembers them again");
    }
}

#[tokio::test]
async fn a_block_ends_subscriptions_in_both_directions() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let alices = ctx.channel(&alice, "alice").await;
    let bobs = ctx.channel(&bob, "bob").await;
    ctx.subscribe(&bob, &alices.summary.id, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    ctx.subscribe(&carol, &alices.summary.id, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    ctx.subscribe(&alice, &bobs.summary.id, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;

    ctx.block(&alice, &bob).await;
    assert_eq!(ctx.deliver("social.user.blocked", alice.id).await, 1);
    assert!(!channels::is_subscribed(&ctx.app.core, bob.id, id(&alices.summary.id)).await.unwrap());
    assert!(!channels::is_subscribed(&ctx.app.core, alice.id, id(&bobs.summary.id)).await.unwrap());
    assert!(
        channels::is_subscribed(&ctx.app.core, carol.id, id(&alices.summary.id)).await.unwrap(),
        "others are untouched"
    );
    assert_eq!(ctx.get_channel(Some(&carol), &alices.summary.id).await.unwrap().summary.subscriber_count, 1);
    assert_eq!(ctx.get_channel(Some(&carol), &bobs.summary.id).await.unwrap().summary.subscriber_count, 0);
    assert_eq!(ctx.app.events_by("video.channel.unsubscribed", bob.id).await.len(), 1);
    // Redelivery finds nothing left to do.
    ctx.deliver("social.user.blocked", alice.id).await;
    assert_eq!(ctx.get_channel(Some(&carol), &alices.summary.id).await.unwrap().summary.subscriber_count, 1);
}

#[tokio::test]
async fn takedowns_remove_the_content_and_its_upload() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let alice = ctx.app.user("alice").await;
    let moderator = ctx.app.user("moderator").await;
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.public_video(&alice, &channel, "規約違反").await;
    let keeper = ctx.public_video(&alice, &channel, "問題なし").await;
    let upload = id(&video.studio.as_option().unwrap().asset_id);
    // Something else uses the same upload (a clip, say): a takedown removes the file regardless.
    assets::attach(&core.db, upload, "live:clip:test").await.unwrap();
    let removed = |subject_type: SubjectType, subject_id: &str| serde_json::json!({ "subject_type": subject_type as i32, "subject_id": subject_id, "reason": "spam", "moderator_id": moderator.id });

    ctx.deliver_event(
        "admin.content.removed",
        Some(moderator.id),
        removed(SubjectType::SUBJECT_TYPE_VIDEO, &card(&video).id),
    )
    .await;
    assert_eq!(ctx.get_video(Some(&alice), &card(&video).id).await.unwrap_err().reason, "VIDEO_NOT_FOUND");
    assert!(assets::find(core, upload).await.unwrap().is_none());
    assert!(search_docs(&ctx, &card(&video).id).await.last().unwrap().is_none());
    assert!(ctx.get_video(None, &card(&keeper).id).await.is_ok());
    // Unknown, already removed and foreign subjects are all "done".
    ctx.deliver_event(
        "admin.content.removed",
        Some(moderator.id),
        removed(SubjectType::SUBJECT_TYPE_VIDEO, &card(&video).id),
    )
    .await;
    ctx.deliver_event(
        "admin.content.removed",
        Some(moderator.id),
        removed(SubjectType::SUBJECT_TYPE_NOTE, &card(&keeper).id),
    )
    .await;
    assert!(ctx.get_video(None, &card(&keeper).id).await.is_ok());

    let playlist = CreatePlaylistRequest {
        title: "list".into(),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        ..Default::default()
    };
    let playlist: CreatePlaylistResponse =
        ctx.call(PLAYLISTS, "CreatePlaylist", Some(&alice), &playlist).await.unwrap();
    ctx.deliver_event(
        "admin.content.removed",
        Some(moderator.id),
        removed(SubjectType::SUBJECT_TYPE_PLAYLIST, &playlist.playlist.id),
    )
    .await;
    let get = GetPlaylistRequest { playlist_id: playlist.playlist.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(PLAYLISTS, "GetPlaylist", Some(&alice), &get).await, "PLAYLIST_NOT_FOUND");

    ctx.deliver_event(
        "admin.content.removed",
        Some(moderator.id),
        removed(SubjectType::SUBJECT_TYPE_CHANNEL, &channel.summary.id),
    )
    .await;
    assert_eq!(ctx.get_channel(Some(&alice), &channel.summary.id).await.unwrap_err().reason, "CHANNEL_NOT_FOUND");
    assert_eq!(ctx.get_video(Some(&alice), &card(&keeper).id).await.unwrap_err().reason, "VIDEO_NOT_FOUND");
}

#[tokio::test]
async fn deleted_uploads_take_what_showed_them_along() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let alice = ctx.app.user("alice").await;
    let icon = ctx.image_upload(&alice, AssetPurpose::ASSET_PURPOSE_CHANNEL_ICON).await;
    let request = CreateChannelRequest {
        handle: unique("ch"),
        name: "ch".into(),
        icon_asset_id: icon.clone(),
        ..Default::default()
    };
    let channel = ctx.create_channel(&alice, &request).await.unwrap();
    let thumbnail = ctx.image_upload(&alice, AssetPurpose::ASSET_PURPOSE_THUMBNAIL).await;
    let video = ctx
        .published_with(&alice, &channel, "v", Visibility::VISIBILITY_PUBLIC, |r| {
            r.thumbnail_asset_id = thumbnail.clone()
        })
        .await;
    let upload = video.studio.as_option().unwrap().asset_id.clone();
    assert_eq!(card(&video).thumbnail.as_option().unwrap().asset_id, thumbnail);

    // Held uploads cannot be deleted by their owner; a domain acting on its own authority can.
    assert_eq!(assets::delete(core, id(&thumbnail), Some(alice.id)).await.unwrap_err().reason, "ASSET_IN_USE");
    for asset in [&thumbnail, &icon] {
        assert!(assets::delete(core, id(asset), None).await.unwrap());
    }
    assert_eq!(ctx.deliver("media.asset.deleted", alice.id).await, 2);
    let seen = ctx.get_video(Some(&alice), &card(&video).id).await.unwrap();
    assert_eq!(card(&seen).thumbnail.as_option().unwrap().asset_id, upload, "back to the poster frame");
    assert!(!seen.studio.as_option().unwrap().has_custom_thumbnail);
    assert!(ctx.get_channel(None, &channel.summary.id).await.unwrap().summary.icon.as_option().is_none());

    // Without its upload there is no video.
    assert!(assets::delete(core, id(&upload), None).await.unwrap());
    ctx.deliver("media.asset.deleted", alice.id).await;
    assert_eq!(ctx.get_video(Some(&alice), &card(&video).id).await.unwrap_err().reason, "VIDEO_NOT_FOUND");
    assert_eq!(
        ctx.app
            .events("video.video.deleted")
            .await
            .iter()
            .filter(|event| event.data["video_id"] == card(&video).id)
            .count(),
        1
    );
}

#[tokio::test]
async fn generic_domains_resolve_channels_videos_and_playlists() {
    let ctx = start().await;
    let subjects = &ctx.app.core.subjects;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let blocked = ctx.app.user("blocked").await;
    ctx.block(&alice, &blocked).await;
    let channel = ctx.channel(&alice, "コメントしてね").await;
    let video = ctx.public_video(&alice, &channel, "コメント歓迎").await;
    let closed = ctx
        .published_with(&alice, &channel, "コメント不可", Visibility::VISIBILITY_PUBLIC, |r| {
            r.comments_enabled = Some(false)
        })
        .await;
    let private = ctx.published(&alice, &channel, "private", Visibility::VISIBILITY_PRIVATE).await;
    let draft = ctx.draft(&alice, &channel, "draft").await;

    // Videos: by either ID, as far as the viewer may watch them.
    for reference in [&card(&video).id, &card(&video).short_id] {
        let info = subjects.resolve(SubjectType::SUBJECT_TYPE_VIDEO, reference, Some(bob.id)).await.unwrap();
        assert_eq!((info.id, info.owner_id, info.title.as_str()), (id(&card(&video).id), alice.id, "コメント歓迎"));
        assert_eq!(info.path, format!("/watch/{}", card(&video).short_id));
        assert!(info.interactions_enabled && info.image_url.is_some());
    }
    assert!(subjects.resolve(SubjectType::SUBJECT_TYPE_VIDEO, &card(&video).id, None).await.is_ok());
    assert!(
        !subjects
            .resolve(SubjectType::SUBJECT_TYPE_VIDEO, &card(&closed).id, Some(bob.id))
            .await
            .unwrap()
            .interactions_enabled
    );
    assert!(
        !subjects
            .resolve(SubjectType::SUBJECT_TYPE_VIDEO, &card(&draft).id, Some(alice.id))
            .await
            .unwrap()
            .interactions_enabled,
        "drafts take no comments"
    );
    for (reference, viewer) in [
        (&card(&private).id, Some(bob.id)),
        (&card(&video).id, Some(blocked.id)),
        (&"garbage!".to_owned(), Some(bob.id)),
    ] {
        assert_eq!(
            subjects.resolve(SubjectType::SUBJECT_TYPE_VIDEO, reference, viewer).await.unwrap_err().reason,
            "SUBJECT_NOT_FOUND"
        );
    }
    // The comment counter belongs to the engagement domain; likes of videos have their own RPC.
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_VIDEO, id(&card(&video).id), "comment_count", 3).await.unwrap();
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_VIDEO, id(&card(&video).id), "comment_count", -1).await.unwrap();
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_VIDEO, id(&card(&video).id), "like_count", 50).await.unwrap();
    let seen = ctx.get_video(None, &card(&video).id).await.unwrap();
    assert_eq!((card(&seen).comment_count, card(&seen).like_count), (2, 0));

    // Channels: by ID or handle.
    for reference in [&channel.summary.id, &channel.summary.handle] {
        let info = subjects.resolve(SubjectType::SUBJECT_TYPE_CHANNEL, reference, Some(bob.id)).await.unwrap();
        assert_eq!(
            (info.id, info.owner_id, info.path.as_str()),
            (id(&channel.summary.id), alice.id, format!("/channel/{}", channel.summary.handle).as_str())
        );
    }
    assert!(subjects.resolve(SubjectType::SUBJECT_TYPE_CHANNEL, &channel.summary.id, Some(blocked.id)).await.is_err());

    // Playlists: custom ones the viewer may open; the built-in lists are not subjects.
    let request = CreatePlaylistRequest {
        title: "みんなの".into(),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        ..Default::default()
    };
    let public: CreatePlaylistResponse = ctx.call(PLAYLISTS, "CreatePlaylist", Some(&alice), &request).await.unwrap();
    let request = CreatePlaylistRequest { title: "わたしの".into(), ..Default::default() };
    let hidden: CreatePlaylistResponse = ctx.call(PLAYLISTS, "CreatePlaylist", Some(&alice), &request).await.unwrap();
    let info =
        subjects.resolve(SubjectType::SUBJECT_TYPE_PLAYLIST, &public.playlist.short_id, Some(bob.id)).await.unwrap();
    assert_eq!(
        (info.title.as_str(), info.path.as_str(), info.interactions_enabled),
        ("みんなの", format!("/playlist/{}", public.playlist.short_id).as_str(), true)
    );
    assert!(subjects.resolve(SubjectType::SUBJECT_TYPE_PLAYLIST, &hidden.playlist.id, Some(bob.id)).await.is_err());
    assert!(
        !subjects
            .resolve(SubjectType::SUBJECT_TYPE_PLAYLIST, &hidden.playlist.id, Some(alice.id))
            .await
            .unwrap()
            .interactions_enabled
    );
    let lists: ListMyPlaylistsResponse =
        ctx.call(PLAYLISTS, "ListMyPlaylists", Some(&alice), &ListMyPlaylistsRequest::default()).await.unwrap();
    assert!(
        subjects.resolve(SubjectType::SUBJECT_TYPE_PLAYLIST, &lists.playlists[0].id, Some(alice.id)).await.is_err()
    );
    subjects
        .counter_changed(SubjectType::SUBJECT_TYPE_PLAYLIST, id(&public.playlist.id), "like_count", 2)
        .await
        .unwrap();
    let get = GetPlaylistRequest { playlist_id: public.playlist.id.clone(), ..Default::default() };
    let seen: GetPlaylistResponse = ctx.call(PLAYLISTS, "GetPlaylist", None, &get).await.unwrap();
    assert_eq!(seen.playlist.like_count, 2);
}

#[tokio::test]
async fn other_domains_use_channels_through_the_rust_api() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    assert!(channels::default_for(core, alice.id).await.unwrap().is_none(), "no channel yet");
    let first = ctx.channel(&alice, "first").await;
    let second = ctx.channel(&alice, "second").await;
    let (first_id, second_id) = (id(&first.summary.id), id(&second.summary.id));

    let default = channels::default_for(core, alice.id).await.unwrap().unwrap();
    assert_eq!(
        (default.id, default.owner_id, default.handle.as_str(), default.name.as_str()),
        (first_id, alice.id, first.summary.handle.as_str(), "first")
    );
    assert_eq!(channels::require_owned(core, alice.id, second_id).await.unwrap().name, "second");
    assert_eq!(channels::require_owned(core, bob.id, second_id).await.unwrap_err().reason, "CHANNEL_NOT_FOUND");
    assert_eq!(channels::require_owned(core, alice.id, Uuid::now_v7()).await.unwrap_err().reason, "CHANNEL_NOT_FOUND");

    ctx.subscribe(&bob, &first.summary.id, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    let summaries = channels::summaries(core, &[first_id, second_id, Uuid::now_v7()]).await.unwrap();
    assert_eq!(summaries.len(), 2);
    assert_eq!(
        (summaries[&first_id].name.as_str(), summaries[&first_id].subscriber_count, summaries[&first_id].verified),
        ("first", 1, false)
    );
    assert!(channels::summaries(core, &[]).await.unwrap().is_empty());
    assert!(
        channels::is_subscribed(core, bob.id, first_id).await.unwrap()
            && !channels::is_subscribed(core, bob.id, second_id).await.unwrap()
    );

    // The verified badge: the channel's own, or that of the account behind it.
    channels::set_verified(core, second_id, true).await.unwrap();
    assert!(channels::summaries(core, &[second_id]).await.unwrap()[&second_id].verified);
    sqlx::query("UPDATE users SET verified = true WHERE id = $1").bind(alice.id).execute(&core.db).await.unwrap();
    assert!(ctx.get_channel(None, &first.summary.id).await.unwrap().summary.verified);

    // Game titles used elsewhere (a live stream, say) join the suggestions.
    let title = format!("Game {}", unique(""));
    games::record_use(core, &title).await.unwrap();
    games::record_use(core, &title.to_uppercase()).await.unwrap();
    assert_eq!(
        games::suggest(core, &title[..12], 5).await.unwrap(),
        vec![title.clone()],
        "one entry, whatever the case"
    );
}

#[tokio::test]
async fn the_data_export_has_a_video_section() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "export me").await;
    let video = ctx.public_video(&alice, &channel, "my video").await;
    let bobs = ctx.public_video(&bob, &ctx.channel(&bob, "bob").await, "bob's").await;
    ctx.rate(&alice, &card(&bobs).id, Rating::RATING_LIKE).await;
    ctx.watch(Some(&alice), &card(&bobs).id, 2.0, TrafficSource::TRAFFIC_SOURCE_HOME).await;
    ctx.subscribe(&alice, &card(&bobs).channel.id, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    let _: MarkNotInterestedResponse = ctx
        .call(
            FEEDS,
            "MarkNotInterested",
            Some(&alice),
            &MarkNotInterestedRequest {
                target: Some(mark_not_interested_request::Target::VideoId(card(&bobs).id.clone())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let _: SetWatchHistoryPausedResponse = ctx
        .call(
            PLAYBACK,
            "SetWatchHistoryPaused",
            Some(&alice),
            &SetWatchHistoryPausedRequest { paused: true, ..Default::default() },
        )
        .await
        .unwrap();

    let exporter = ctx
        .app
        .core
        .exporters
        .all()
        .into_iter()
        .find(|exporter| exporter.name() == "video")
        .expect("a video exporter is registered");
    let export = exporter.export(alice.id).await.unwrap();
    assert_eq!(export["channels"][0]["name"], "export me");
    assert_eq!(
        (export["videos"][0]["title"].as_str(), export["videos"][0]["short_id"].as_str()),
        (Some("my video"), Some(card(&video).short_id.as_str()))
    );
    assert_eq!(export["subscriptions"][0]["channel_id"], card(&bobs).channel.id);
    assert_eq!(
        (export["ratings"][0]["rating"].as_str(), export["ratings"][0]["video_id"].as_str()),
        (Some("like"), Some(card(&bobs).id.as_str()))
    );
    assert_eq!(export["watch_history"][0]["video_id"], card(&bobs).id);
    assert_eq!(export["not_interested"][0]["video_id"], card(&bobs).id);
    assert_eq!(export["settings"][0]["watch_history_paused"], true);
    // The liked-videos list holds bob's video.
    let liked = export["playlists"].as_array().unwrap().iter().find(|list| list["kind"] == 3).expect("the liked list");
    assert_eq!(liked["video_ids"][0], card(&bobs).id);
    // Someone else's export has none of it.
    let other = exporter.export(bob.id).await.unwrap();
    assert!(other["ratings"].as_array().unwrap().is_empty() && other["videos"].as_array().unwrap().len() == 1);
}
