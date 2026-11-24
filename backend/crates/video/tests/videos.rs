//! `VideoService`: creating, editing, publishing, rating and subtitling
//! videos, and who may see them.
mod common;

use common::*;
use tokuly_core::storage::Bucket;
use tokuly_media::assets;
use tokuly_proto::{
    common::Visibility,
    media::{AssetKind, AssetPurpose},
    video::*,
};
use tokuly_testkit::TestUser;

fn chapter(start: f64, title: &str) -> Chapter {
    Chapter { start_seconds: start, title: title.into(), ..Default::default() }
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

async fn update(ctx: &Ctx, owner: &TestUser, request: &UpdateVideoRequest) -> Video {
    let response: UpdateVideoResponse =
        ctx.call(VIDEOS, "UpdateVideo", Some(owner), request).await.expect("update video");
    response.video.as_option().cloned().unwrap()
}

#[tokio::test]
async fn a_new_video_is_a_private_draft_only_its_owner_sees() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let upload = ctx.video_upload(&alice).await;

    // No channel named: the default one. No title: the file name.
    let request = CreateVideoRequest {
        asset_id: upload.clone(),
        tags: vec!["#Vlog".into(), "vlog".into(), "日常".into()],
        ..Default::default()
    };
    let video = ctx.create_video(&alice, &request).await.unwrap();
    let summary = card(&video);
    assert_eq!(summary.channel.id, channel.summary.id);
    assert_eq!((summary.title.as_str(), summary.short_id.len()), ("video", 11));
    assert_eq!(summary.status, VideoStatus::VIDEO_STATUS_DRAFT);
    assert_eq!(summary.processing, ProcessingState::PROCESSING_STATE_READY, "the upload was already processed");
    assert_eq!(summary.visibility, Visibility::VISIBILITY_PRIVATE);
    assert_eq!(summary.origin, VideoOrigin::VIDEO_ORIGIN_UPLOAD);
    assert!(summary.published_at.as_option().is_none() && summary.duration_seconds == 12.0);
    assert_eq!(
        summary.thumbnail.as_option().unwrap().asset_id,
        upload,
        "without a custom thumbnail the poster frame is used"
    );
    assert_eq!(video.tags, vec!["vlog", "日常"]);
    assert!(video.comments_enabled && video.playback.as_option().is_some_and(|p| !p.hls_url.is_empty()));
    let studio = video.studio.as_option().expect("the owner gets studio details");
    assert!(studio.notify_subscribers && !studio.has_custom_thumbnail && studio.asset_id == upload);
    assert!(video.viewer.as_option().unwrap().is_owner);
    assert!(assets::is_attached(&ctx.app.core, id(&upload)).await.unwrap());
    assert_eq!(ctx.app.events_by("video.video.created", alice.id).await.len(), 1);

    // Both ID forms resolve for the owner; nobody else learns the draft exists.
    for reference in [&summary.id, &summary.short_id] {
        assert!(ctx.get_video(Some(&alice), reference).await.is_ok());
        assert_eq!(ctx.get_video(Some(&bob), reference).await.unwrap_err().reason, "VIDEO_NOT_FOUND");
        assert_eq!(ctx.get_video(None, reference).await.unwrap_err().reason, "VIDEO_NOT_FOUND");
    }
    assert_eq!(ctx.get_video(None, "not an id!").await.unwrap_err().reason, "VIDEO_NOT_FOUND");
}

#[tokio::test]
async fn video_input_is_validated() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let nobody = ctx.app.user("nochannel").await;
    let channel = ctx.channel(&alice, "ch").await;
    let bobs_channel = ctx.channel(&bob, "bob").await;
    let bobs_upload = ctx.video_upload(&bob).await;
    let short = ctx.upload(&alice, AssetPurpose::ASSET_PURPOSE_SHORT, AssetKind::ASSET_KIND_VIDEO).await;
    let avatar = ctx.image_upload(&alice, AssetPurpose::ASSET_PURPOSE_AVATAR).await;
    let used = ctx.draft(&alice, &channel, "used").await.studio.as_option().unwrap().asset_id.clone();

    let cases: Vec<(&str, Box<dyn Fn(&mut CreateVideoRequest)>)> = vec![
        ("CHANNEL_NOT_FOUND", Box::new(|r| r.channel_id = bobs_channel.summary.id.clone())),
        ("ASSET_NOT_FOUND", Box::new(|r| r.asset_id = bobs_upload.clone())),
        ("ASSET_NOT_FOUND", Box::new(|r| r.asset_id = uuid::Uuid::now_v7().to_string())),
        ("INVALID_ID", Box::new(|r| r.asset_id = "upload".into())),
        ("ASSET_WRONG_PURPOSE", Box::new(|r| r.asset_id = short.clone())),
        ("ASSET_ALREADY_USED", Box::new(|r| r.asset_id = used.clone())),
        ("TITLE_INVALID", Box::new(|r| r.title = "あ".repeat(101))),
        ("DESCRIPTION_TOO_LONG", Box::new(|r| r.description = "d".repeat(5001))),
        ("TAGS_INVALID", Box::new(|r| r.tags = (0..21).map(|i| format!("t{i}")).collect())),
        ("TAGS_INVALID", Box::new(|r| r.tags = vec!["two words".into()])),
        ("GAME_INVALID", Box::new(|r| r.game = "g".repeat(101))),
        ("CATEGORY_INVALID", Box::new(|r| r.category = buffa::EnumValue::from(99))),
        ("VISIBILITY_INVALID", Box::new(|r| r.visibility = buffa::EnumValue::from(42))),
        ("CHAPTERS_INVALID", Box::new(|r| r.chapters = vec![chapter(3.0, "starts late")])),
        ("CHAPTERS_INVALID", Box::new(|r| r.chapters = vec![chapter(0.0, "a"), chapter(12.0, "at the end")])),
        ("ASSET_WRONG_PURPOSE", Box::new(|r| r.thumbnail_asset_id = avatar.clone())),
    ];
    for (reason, mutate) in cases {
        let mut request = CreateVideoRequest {
            channel_id: channel.summary.id.clone(),
            asset_id: ctx.video_upload(&alice).await,
            title: "ok".into(),
            ..Default::default()
        };
        mutate(&mut request);
        assert_eq!(ctx.reason(VIDEOS, "CreateVideo", Some(&alice), &request).await, reason);
    }
    let request = CreateVideoRequest { asset_id: ctx.video_upload(&nobody).await, ..Default::default() };
    assert_eq!(ctx.reason(VIDEOS, "CreateVideo", Some(&nobody), &request).await, "CHANNEL_REQUIRED");
    assert_eq!(ctx.reason(VIDEOS, "CreateVideo", None, &request).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn editing_changes_only_what_is_named() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let draft = ctx.draft_with(&alice, &channel, "before", |r| r.description = "keep me".into()).await;
    let video_id = card(&draft).id.clone();
    let poster = card(&draft).thumbnail.as_option().unwrap().asset_id.clone();
    let thumbnail = ctx.image_upload(&alice, AssetPurpose::ASSET_PURPOSE_THUMBNAIL).await;

    let request = UpdateVideoRequest {
        video_id: video_id.clone(),
        title: Some("  after  ".into()),
        tags: buffa::MessageField::some(TagList { tags: vec!["#Game".into(), "実況".into()], ..Default::default() }),
        category: Some(VideoCategory::VIDEO_CATEGORY_GAMING.into()),
        game: Some("Minecraft".into()),
        chapters: buffa::MessageField::some(ChapterList {
            chapters: vec![chapter(6.0, "本編"), chapter(0.0, "OP")],
            ..Default::default()
        }),
        age_restricted: Some(true),
        comments_enabled: Some(false),
        notify_subscribers: Some(false),
        thumbnail_asset_id: Some(thumbnail.clone()),
        ..Default::default()
    };
    let video = update(&ctx, &alice, &request).await;
    assert_eq!((card(&video).title.as_str(), video.description.as_str()), ("after", "keep me"));
    assert_eq!((video.tags.clone(), video.game.as_str()), (vec!["game".to_owned(), "実況".to_owned()], "Minecraft"));
    assert_eq!(card(&video).category, VideoCategory::VIDEO_CATEGORY_GAMING);
    assert_eq!(video.chapters.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), vec!["OP", "本編"]);
    assert!(card(&video).age_restricted && !video.comments_enabled);
    let studio = video.studio.as_option().unwrap();
    assert!(studio.has_custom_thumbnail && !studio.notify_subscribers);
    assert_eq!(card(&video).thumbnail.as_option().unwrap().asset_id, thumbnail);
    assert!(assets::is_attached(&ctx.app.core, id(&thumbnail)).await.unwrap());

    // An empty thumbnail goes back to the poster frame and releases the image.
    let video = update(
        &ctx,
        &alice,
        &UpdateVideoRequest {
            video_id: video_id.clone(),
            thumbnail_asset_id: Some(String::new()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(card(&video).thumbnail.as_option().unwrap().asset_id, poster);
    assert!(!video.studio.as_option().unwrap().has_custom_thumbnail);
    assert!(!assets::is_attached(&ctx.app.core, id(&thumbnail)).await.unwrap());
    assert_eq!(card(&video).title, "after", "nothing else moved");

    // `allow_link` only means something for friends-only videos.
    let request = UpdateVideoRequest {
        video_id: video_id.clone(),
        visibility: Some(Visibility::VISIBILITY_FRIENDS.into()),
        allow_link: Some(true),
        ..Default::default()
    };
    assert!(update(&ctx, &alice, &request).await.allow_link);
    let request = UpdateVideoRequest {
        video_id: video_id.clone(),
        visibility: Some(Visibility::VISIBILITY_PUBLIC.into()),
        ..Default::default()
    };
    assert!(!update(&ctx, &alice, &request).await.allow_link);

    let reasons: Vec<(&str, UpdateVideoRequest)> = vec![
        ("TITLE_INVALID", UpdateVideoRequest { title: Some(String::new()), ..Default::default() }),
        ("DESCRIPTION_TOO_LONG", UpdateVideoRequest { description: Some("d".repeat(5001)), ..Default::default() }),
        ("GAME_INVALID", UpdateVideoRequest { game: Some("g".repeat(101)), ..Default::default() }),
        (
            "VISIBILITY_INVALID",
            UpdateVideoRequest { visibility: Some(Visibility::VISIBILITY_UNSPECIFIED.into()), ..Default::default() },
        ),
        ("CATEGORY_INVALID", UpdateVideoRequest { category: Some(buffa::EnumValue::from(77)), ..Default::default() }),
        (
            "CHAPTERS_INVALID",
            UpdateVideoRequest {
                chapters: buffa::MessageField::some(ChapterList {
                    chapters: vec![chapter(0.0, "a"), chapter(2.0, "too close")],
                    ..Default::default()
                }),
                ..Default::default()
            },
        ),
        (
            "ASSET_NOT_FOUND",
            UpdateVideoRequest {
                thumbnail_asset_id: Some(ctx.image_upload(&bob, AssetPurpose::ASSET_PURPOSE_THUMBNAIL).await),
                ..Default::default()
            },
        ),
    ];
    for (reason, mut request) in reasons {
        request.video_id = video_id.clone();
        assert_eq!(ctx.reason(VIDEOS, "UpdateVideo", Some(&alice), &request).await, reason);
    }
    let foreign = UpdateVideoRequest { video_id: video_id.clone(), title: Some("mine".into()), ..Default::default() };
    assert_eq!(ctx.reason(VIDEOS, "UpdateVideo", Some(&bob), &foreign).await, "VIDEO_NOT_FOUND");
    assert_eq!(ctx.reason(VIDEOS, "UpdateVideo", None, &foreign).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn publishing_announces_once_and_unpublishing_takes_it_back() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let game = unique("game");
    let draft = ctx
        .draft_with(&alice, &channel, "公開するよ", |r| {
            r.visibility = Visibility::VISIBILITY_PUBLIC.into();
            r.game = game.clone();
            r.tags = vec!["tag".into()];
        })
        .await;
    let video_id = card(&draft).id.clone();
    assert!(search_docs(&ctx, &video_id).await.is_empty(), "drafts are not indexed");

    let published = ctx.publish(&alice, &video_id).await;
    assert_eq!(card(&published).status, VideoStatus::VIDEO_STATUS_PUBLISHED);
    let published_at = card(&published).published_at.as_option().cloned().expect("published_at");
    assert!(ctx.get_video(Some(&bob), &video_id).await.is_ok() && ctx.get_video(None, &video_id).await.is_ok());

    let events = ctx.app.events_by("video.video.published", alice.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].data["first_publication"].as_bool(), events[0].data["visibility"].as_i64()),
        (Some(true), Some(1))
    );
    // Friends hear about it in their feed (channels do not reveal their owner to everyone else).
    let activity = ctx.app.events_by("activity.recorded", alice.id).await;
    assert_eq!(activity.len(), 1);
    assert_eq!(
        (activity[0].data["kind"].as_str(), activity[0].data["audience"].as_str()),
        (Some("video.published"), Some("friends"))
    );
    assert_eq!(activity[0].data["path"], format!("/watch/{}", card(&published).short_id));
    let docs = search_docs(&ctx, &video_id).await;
    let doc = docs.last().unwrap().as_ref().expect("public videos are searchable");
    assert_eq!((doc["kind"].as_str(), doc["title"].as_str()), (Some("video"), Some("公開するよ")));
    assert_eq!(doc["tags"], serde_json::json!(["tag", game]));
    // The game title is now suggested to others.
    let suggested: SuggestGamesResponse = ctx
        .call(VIDEOS, "SuggestGames", None, &SuggestGamesRequest { query: game[..8].to_owned(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(suggested.games, vec![game.clone()]);

    // Publishing again changes nothing.
    ctx.publish(&alice, &video_id).await;
    assert_eq!(ctx.app.events_by("video.video.published", alice.id).await.len(), 1);

    // Back to draft: gone for everyone else and from search.
    let request = UnpublishVideoRequest { video_id: video_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(VIDEOS, "UnpublishVideo", Some(&bob), &request).await, "VIDEO_NOT_FOUND");
    let response: UnpublishVideoResponse = ctx.call(VIDEOS, "UnpublishVideo", Some(&alice), &request).await.unwrap();
    assert_eq!(card(response.video.as_option().unwrap()).status, VideoStatus::VIDEO_STATUS_DRAFT);
    assert_eq!(ctx.get_video(Some(&bob), &video_id).await.unwrap_err().reason, "VIDEO_NOT_FOUND");
    assert!(search_docs(&ctx, &video_id).await.last().unwrap().is_none());
    assert_eq!(ctx.app.events_by("video.video.unpublished", alice.id).await.len(), 1);

    // Publishing a second time is not a first publication: no second announcement.
    let again = ctx.publish(&alice, &video_id).await;
    assert_eq!(card(&again).published_at.as_option(), Some(&published_at), "the original publication time is kept");
    let events = ctx.app.events_by("video.video.published", alice.id).await;
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].data["first_publication"].as_bool(), Some(false));
    assert_eq!(ctx.app.events_by("activity.recorded", alice.id).await.len(), 1);

    let foreign = PublishVideoRequest { video_id: video_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(VIDEOS, "PublishVideo", Some(&bob), &foreign).await, "VIDEO_NOT_FOUND");
    assert_eq!(ctx.reason(VIDEOS, "PublishVideo", None, &foreign).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn a_private_video_is_announced_when_it_first_gets_an_audience() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    let private = ctx.published(&alice, &channel, "まだ秘密", Visibility::VISIBILITY_PRIVATE).await;
    let video_id = card(&private).id.clone();
    assert!(ctx.app.events_by("activity.recorded", alice.id).await.is_empty(), "nobody to tell yet");
    assert_eq!(
        ctx.app.events_by("video.video.published", alice.id).await[0].data["first_publication"].as_bool(),
        Some(false)
    );

    // PublishVideo on a live video only applies the visibility.
    let request = PublishVideoRequest {
        video_id: video_id.clone(),
        visibility: Some(Visibility::VISIBILITY_FRIENDS.into()),
        ..Default::default()
    };
    let response: PublishVideoResponse = ctx.call(VIDEOS, "PublishVideo", Some(&alice), &request).await.unwrap();
    assert_eq!(card(response.video.as_option().unwrap()).visibility, Visibility::VISIBILITY_FRIENDS);
    assert_eq!(ctx.app.events_by("activity.recorded", alice.id).await.len(), 1);
    let events = ctx.app.events_by("video.video.published", alice.id).await;
    assert_eq!(events.last().unwrap().data["first_publication"].as_bool(), Some(true));
    assert!(search_docs(&ctx, &video_id).await.is_empty(), "friends-only videos are not searchable");

    // Going public later indexes it but does not announce a second time.
    update(
        &ctx,
        &alice,
        &UpdateVideoRequest {
            video_id: video_id.clone(),
            visibility: Some(Visibility::VISIBILITY_PUBLIC.into()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(ctx.app.events_by("activity.recorded", alice.id).await.len(), 1);
    assert!(search_docs(&ctx, &video_id).await.last().unwrap().is_some());
    // And leaving the public takes it out of search again.
    update(
        &ctx,
        &alice,
        &UpdateVideoRequest {
            video_id: video_id.clone(),
            visibility: Some(Visibility::VISIBILITY_UNLISTED.into()),
            ..Default::default()
        },
    )
    .await;
    assert!(search_docs(&ctx, &video_id).await.last().unwrap().is_none());
}

#[tokio::test]
async fn scheduled_videos_go_live_when_due() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let draft =
        ctx.draft_with(&alice, &channel, "予約投稿", |r| r.visibility = Visibility::VISIBILITY_PUBLIC.into()).await;
    let video_id = card(&draft).id.clone();

    let at = |offset: chrono::Duration| {
        buffa::MessageField::some(tokuly_proto::Timestamp::from(chrono::Utc::now() + offset))
    };
    for bad in [chrono::Duration::hours(-2), chrono::Duration::days(400)] {
        let request = PublishVideoRequest { video_id: video_id.clone(), publish_at: at(bad), ..Default::default() };
        assert_eq!(ctx.reason(VIDEOS, "PublishVideo", Some(&alice), &request).await, "SCHEDULE_INVALID");
    }
    let request = PublishVideoRequest {
        video_id: video_id.clone(),
        publish_at: at(chrono::Duration::hours(3)),
        ..Default::default()
    };
    let response: PublishVideoResponse = ctx.call(VIDEOS, "PublishVideo", Some(&alice), &request).await.unwrap();
    let scheduled = card(response.video.as_option().unwrap()).clone();
    assert_eq!(scheduled.status, VideoStatus::VIDEO_STATUS_SCHEDULED);
    assert!(scheduled.scheduled_at.as_option().is_some() && scheduled.published_at.as_option().is_none());
    assert_eq!(
        ctx.get_video(Some(&bob), &video_id).await.unwrap_err().reason,
        "VIDEO_NOT_FOUND",
        "scheduled is not published"
    );

    // Not due yet: the worker leaves it alone.
    tokuly_video::videos::publish_due(&ctx.app.core).await.unwrap();
    assert_eq!(
        card(&ctx.get_video(Some(&alice), &video_id).await.unwrap()).status,
        VideoStatus::VIDEO_STATUS_SCHEDULED
    );

    // Time passes.
    sqlx::query("UPDATE videos SET scheduled_at = now() - interval '1 second' WHERE id = $1")
        .bind(id(&video_id))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let mut frames = ctx.app.listen_user(alice.id).await;
    assert!(tokuly_video::videos::publish_due(&ctx.app.core).await.unwrap() >= 1);
    let live = ctx.get_video(Some(&bob), &video_id).await.unwrap();
    assert_eq!(card(&live).status, VideoStatus::VIDEO_STATUS_PUBLISHED);
    assert!(card(&live).scheduled_at.as_option().is_none());
    assert_eq!(ctx.app.events_by("video.video.published", alice.id).await.len(), 1);
    // The owner's devices are told.
    match next_push(&mut frames).await {
        Push::VideoUpdated(summary) => assert_eq!(
            (summary.id.as_str(), summary.status),
            (video_id.as_str(), VideoStatus::VIDEO_STATUS_PUBLISHED.into())
        ),
        other => panic!("unexpected push {other:?}"),
    }

    // A schedule can be cancelled.
    let other = ctx.draft(&alice, &channel, "やっぱりやめた").await;
    let request = PublishVideoRequest {
        video_id: card(&other).id.clone(),
        publish_at: at(chrono::Duration::days(2)),
        ..Default::default()
    };
    let _: PublishVideoResponse = ctx.call(VIDEOS, "PublishVideo", Some(&alice), &request).await.unwrap();
    let response: UnpublishVideoResponse = ctx
        .call(
            VIDEOS,
            "UnpublishVideo",
            Some(&alice),
            &UnpublishVideoRequest { video_id: card(&other).id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let cancelled = card(response.video.as_option().unwrap());
    assert_eq!(cancelled.status, VideoStatus::VIDEO_STATUS_DRAFT);
    assert!(cancelled.scheduled_at.as_option().is_none());
}

#[tokio::test]
async fn visibility_decides_who_can_watch_by_link() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let friend = ctx.app.user("friend").await;
    let stranger = ctx.app.user("stranger").await;
    let blocked = ctx.app.user("blocked").await;
    ctx.befriend(&alice, &friend).await;
    ctx.block(&alice, &blocked).await;
    let channel = ctx.channel(&alice, "ch").await;

    use Visibility::*;
    let mut videos = Vec::new();
    for (visibility, allow_link) in [
        (VISIBILITY_PUBLIC, false),
        (VISIBILITY_NO_FEED, false),
        (VISIBILITY_UNLISTED, false),
        (VISIBILITY_FRIENDS, false),
        (VISIBILITY_FRIENDS, true),
        (VISIBILITY_PRIVATE, false),
    ] {
        let video = ctx
            .published_with(&alice, &channel, &format!("{visibility:?}/{allow_link}"), visibility, |r| {
                r.allow_link = allow_link
            })
            .await;
        videos.push((visibility, allow_link, card(&video).id.clone()));
    }
    let draft = ctx.draft_with(&alice, &channel, "draft", |r| r.visibility = VISIBILITY_PUBLIC.into()).await;

    // (viewer, the videos they can open)
    let open_to_all = [true, true, true, false, true, false];
    let viewers: [(&str, Option<&TestUser>, [bool; 6]); 5] = [
        ("anonymous", None, open_to_all),
        ("stranger", Some(&stranger), open_to_all),
        ("friend", Some(&friend), [true, true, true, true, true, false]),
        ("owner", Some(&alice), [true; 6]),
        ("blocked", Some(&blocked), [false; 6]),
    ];
    let all_ids: Vec<String> = videos.iter().map(|(_, _, id)| id.clone()).chain([card(&draft).id.clone()]).collect();
    for (name, viewer, expected) in viewers {
        let mut can_open = Vec::new();
        for ((visibility, allow_link, video_id), expected) in videos.iter().zip(expected) {
            let result = ctx.get_video(viewer, video_id).await;
            assert_eq!(result.is_ok(), expected, "{name} opening {visibility:?} (allow_link {allow_link})");
            if let Err(error) = result {
                assert_eq!(error.reason, "VIDEO_NOT_FOUND");
            } else {
                can_open.push(video_id.clone());
            }
        }
        // The batch lookup applies the same rules in SQL: the two must agree.
        let batch: BatchGetVideosResponse = ctx
            .call(
                VIDEOS,
                "BatchGetVideos",
                viewer,
                &BatchGetVideosRequest { video_ids: all_ids.clone(), ..Default::default() },
            )
            .await
            .unwrap();
        let mut batched: Vec<String> = batch.videos.iter().map(|video| video.id.clone()).collect();
        if name == "owner" {
            assert!(batched.contains(&card(&draft).id), "owners get their drafts");
            batched.retain(|id| *id != card(&draft).id);
        }
        assert_eq!(batched, can_open, "{name}: batch lookup and single lookup disagree");
    }
}

#[tokio::test]
async fn batch_lookup_keeps_the_request_order_and_skips_the_unknown() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    let first = ctx.public_video(&alice, &channel, "first").await;
    let second = ctx.public_video(&alice, &channel, "second").await;
    let request = BatchGetVideosRequest {
        video_ids: vec![
            card(&second).short_id.clone(),
            "garbage id!".into(),
            uuid::Uuid::now_v7().to_string(),
            card(&first).id.clone(),
        ],
        ..Default::default()
    };
    let response: BatchGetVideosResponse = ctx.call(VIDEOS, "BatchGetVideos", None, &request).await.unwrap();
    assert_eq!(titles(&response.videos), vec!["second", "first"]);
    let too_many = BatchGetVideosRequest {
        video_ids: (0..101).map(|_| uuid::Uuid::now_v7().to_string()).collect(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(VIDEOS, "BatchGetVideos", None, &too_many).await, "TOO_MANY_IDS");
}

#[tokio::test]
async fn the_studio_lists_every_video_of_the_owner() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let main = ctx.channel(&alice, "main").await;
    let sub = ctx.channel(&alice, "sub").await;
    ctx.draft(&alice, &main, "draft").await;
    ctx.public_video(&alice, &main, "public").await;
    ctx.published(&alice, &sub, "private", Visibility::VISIBILITY_PRIVATE).await;
    ctx.public_video(&bob, &ctx.channel(&bob, "bob").await, "not mine").await;

    let all: ListStudioVideosResponse =
        ctx.call(VIDEOS, "ListStudioVideos", Some(&alice), &ListStudioVideosRequest::default()).await.unwrap();
    assert_eq!(titles(&all.videos), vec!["private", "public", "draft"], "newest first, every state, only the caller's");
    let request = ListStudioVideosRequest { channel_id: main.summary.handle.clone(), ..Default::default() };
    let of_main: ListStudioVideosResponse = ctx.call(VIDEOS, "ListStudioVideos", Some(&alice), &request).await.unwrap();
    assert_eq!(titles(&of_main.videos), vec!["public", "draft"]);
    let request = ListStudioVideosRequest { status: VideoStatus::VIDEO_STATUS_DRAFT.into(), ..Default::default() };
    let drafts: ListStudioVideosResponse = ctx.call(VIDEOS, "ListStudioVideos", Some(&alice), &request).await.unwrap();
    assert_eq!(titles(&drafts.videos), vec!["draft"]);

    let first: ListStudioVideosResponse = ctx
        .call(
            VIDEOS,
            "ListStudioVideos",
            Some(&alice),
            &ListStudioVideosRequest { page: page(2, ""), ..Default::default() },
        )
        .await
        .unwrap();
    let info = first.page.as_option().unwrap();
    assert!(info.has_more);
    let rest: ListStudioVideosResponse = ctx
        .call(
            VIDEOS,
            "ListStudioVideos",
            Some(&alice),
            &ListStudioVideosRequest { page: page(2, &info.next_cursor), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(titles(&rest.videos), vec!["draft"]);

    let foreign = ListStudioVideosRequest { channel_id: main.summary.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(VIDEOS, "ListStudioVideos", Some(&bob), &foreign).await, "CHANNEL_NOT_FOUND");
    assert_eq!(
        ctx.reason(VIDEOS, "ListStudioVideos", None, &ListStudioVideosRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
}

#[tokio::test]
async fn ratings_count_once_per_user_and_dislikes_stay_with_the_owner() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.public_video(&alice, &channel, "評価してね").await;
    let video_id = card(&video).id.clone();

    let rated = ctx.rate(&bob, &video_id, Rating::RATING_LIKE).await;
    assert_eq!((rated.rating, rated.like_count), (Rating::RATING_LIKE.into(), 1));
    assert_eq!(ctx.rate(&bob, &video_id, Rating::RATING_LIKE).await.like_count, 1, "liking twice counts once");
    assert_eq!(ctx.rate(&carol, &card(&video).short_id, Rating::RATING_DISLIKE).await.like_count, 1);

    let seen = ctx.get_video(Some(&bob), &video_id).await.unwrap();
    assert_eq!(seen.viewer.as_option().unwrap().rating, Rating::RATING_LIKE);
    assert_eq!(card(&seen).like_count, 1);
    assert!(seen.studio.as_option().is_none(), "only the owner sees the dislike count");
    assert_eq!(ctx.get_video(Some(&alice), &video_id).await.unwrap().studio.as_option().unwrap().dislike_count, 1);
    assert_eq!(
        ctx.get_video(Some(&carol), &video_id).await.unwrap().viewer.as_option().unwrap().rating,
        Rating::RATING_DISLIKE
    );

    // Liked videos are the built-in LL playlist.
    let liked: ListPlaylistItemsResponse = ctx
        .call(
            PLAYLISTS,
            "ListPlaylistItems",
            Some(&bob),
            &ListPlaylistItemsRequest { playlist_id: "LL".into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(liked.items.len(), 1);

    // Changing one's mind moves the counters; taking the rating back clears them.
    assert_eq!(ctx.rate(&bob, &video_id, Rating::RATING_DISLIKE).await.like_count, 0);
    assert_eq!(ctx.get_video(Some(&alice), &video_id).await.unwrap().studio.as_option().unwrap().dislike_count, 2);
    let liked: ListPlaylistItemsResponse = ctx
        .call(
            PLAYLISTS,
            "ListPlaylistItems",
            Some(&bob),
            &ListPlaylistItemsRequest { playlist_id: "LL".into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(liked.items.is_empty());
    let cleared = ctx.rate(&bob, &video_id, Rating::RATING_NONE).await;
    assert_eq!((cleared.rating, cleared.like_count), (Rating::RATING_NONE.into(), 0));
    assert_eq!(ctx.get_video(Some(&alice), &video_id).await.unwrap().studio.as_option().unwrap().dislike_count, 1);

    let request =
        |rating: Rating| RateVideoRequest { video_id: video_id.clone(), rating: rating.into(), ..Default::default() };
    assert_eq!(
        ctx.reason(VIDEOS, "RateVideo", Some(&bob), &request(Rating::RATING_UNSPECIFIED)).await,
        "RATING_INVALID"
    );
    assert_eq!(ctx.reason(VIDEOS, "RateVideo", None, &request(Rating::RATING_LIKE)).await, "SIGN_IN_REQUIRED");
    let private = ctx.published(&alice, &channel, "private", Visibility::VISIBILITY_PRIVATE).await;
    let hidden = RateVideoRequest {
        video_id: card(&private).id.clone(),
        rating: Rating::RATING_LIKE.into(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(VIDEOS, "RateVideo", Some(&bob), &hidden).await, "VIDEO_NOT_FOUND");
}

#[tokio::test]
async fn age_restricted_videos_play_for_adults_only() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let adult = ctx.app.user("adult").await;
    let minor = ctx.app.user("minor").await;
    let undated = ctx.app.user("undated").await;
    let db = &ctx.app.core.db;
    sqlx::query("UPDATE users SET birthdate = current_date - interval '15 years' WHERE id = $1")
        .bind(minor.id)
        .execute(db)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET birthdate = NULL WHERE id = $1").bind(undated.id).execute(db).await.unwrap();
    let channel = ctx.channel(&alice, "ch").await;
    let video =
        ctx.published_with(&alice, &channel, "R18", Visibility::VISIBILITY_PUBLIC, |r| r.age_restricted = true).await;
    let video_id = card(&video).id.clone();

    let cases: [(&str, Option<&TestUser>, PlaybackBlock); 5] = [
        ("anonymous", None, PlaybackBlock::PLAYBACK_BLOCK_SIGN_IN_REQUIRED),
        ("minor", Some(&minor), PlaybackBlock::PLAYBACK_BLOCK_AGE_RESTRICTED),
        ("no birthdate", Some(&undated), PlaybackBlock::PLAYBACK_BLOCK_AGE_RESTRICTED),
        ("adult", Some(&adult), PlaybackBlock::PLAYBACK_BLOCK_UNSPECIFIED),
        ("owner", Some(&alice), PlaybackBlock::PLAYBACK_BLOCK_UNSPECIFIED),
    ];
    for (name, viewer, block) in cases {
        let seen = ctx.get_video(viewer, &video_id).await.unwrap();
        assert_eq!(seen.playback_block, block, "{name}");
        assert_eq!(
            seen.playback.as_option().is_some(),
            block == PlaybackBlock::PLAYBACK_BLOCK_UNSPECIFIED,
            "{name}: streams only when playable"
        );
        assert!(card(&seen).age_restricted && !card(&seen).title.is_empty(), "{name}: the page itself is shown");
        // Playback sessions follow the same rule.
        let request = StartPlaybackRequest { video_id: video_id.clone(), ..Default::default() };
        let started = ctx.call::<_, StartPlaybackResponse>(PLAYBACK, "StartPlayback", viewer, &request).await;
        match block {
            PlaybackBlock::PLAYBACK_BLOCK_UNSPECIFIED => assert!(started.is_ok(), "{name}"),
            _ => assert_eq!(started.unwrap_err().reason, "PLAYBACK_BLOCKED", "{name}"),
        }
    }
}

#[tokio::test]
async fn games_are_suggested_by_prefix_and_resemblance() {
    let ctx = start().await;
    let suggest =
        |query: &str, limit: i32| SuggestGamesRequest { query: query.to_owned(), limit, ..Default::default() };
    let response: SuggestGamesResponse = ctx.call(VIDEOS, "SuggestGames", None, &suggest("mine", 0)).await.unwrap();
    assert_eq!(response.games.first().map(String::as_str), Some("Minecraft"), "seeded titles match case-insensitively");
    let response: SuggestGamesResponse = ctx.call(VIDEOS, "SuggestGames", None, &suggest("スプラ", 5)).await.unwrap();
    assert_eq!(response.games, vec!["スプラトゥーン3"]);
    let response: SuggestGamesResponse = ctx.call(VIDEOS, "SuggestGames", None, &suggest("Mincraft", 3)).await.unwrap();
    assert!(response.games.contains(&"Minecraft".to_owned()), "typos still find the title: {:?}", response.games);
    let response: SuggestGamesResponse = ctx.call(VIDEOS, "SuggestGames", None, &suggest("", 4)).await.unwrap();
    assert_eq!(response.games.len(), 4, "an empty query lists the most used titles");
    assert_eq!(ctx.reason(VIDEOS, "SuggestGames", None, &suggest(&"q".repeat(101), 0)).await, "QUERY_TOO_LONG");
}

const VTT: &str = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\nこんにちは!\n\n00:00:04.000 --> 00:00:06.000\nまたね!\n";
const SRT: &str = "1\n00:00:01,000 --> 00:00:03,000\nHello!\n\n2\n00:00:04,000 --> 00:00:06,000\nBye!\n";

async fn stored_subtitle(ctx: &Ctx, track: &SubtitleTrack) -> String {
    let base = ctx.app.core.storage.media_url("");
    let key = track.url.strip_prefix(&base).unwrap_or_else(|| panic!("{} is not a media URL", track.url));
    String::from_utf8(ctx.app.core.storage.get(Bucket::Media, key).await.expect("the subtitle file is stored").to_vec())
        .unwrap()
}

#[tokio::test]
async fn subtitles_are_stored_as_webvtt_per_language() {
    let ctx = start().await;
    ctx.app.core.storage.ensure_buckets().await.unwrap();
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.public_video(&alice, &channel, "字幕つき").await;
    let video_id = card(&video).id.clone();
    let add = |language: &str, content: &str| AddSubtitleRequest {
        video_id: video_id.clone(),
        source: Some(add_subtitle_request::Source::Content(content.to_owned())),
        language: language.to_owned(),
        ..Default::default()
    };

    let request = AddSubtitleRequest { label: "日本語".into(), is_default: true, ..add("JA", VTT) };
    let response: AddSubtitleResponse = ctx.call(VIDEOS, "AddSubtitle", Some(&alice), &request).await.unwrap();
    let japanese = response.subtitle.as_option().cloned().unwrap();
    assert_eq!(
        (japanese.language.as_str(), japanese.label.as_str(), japanese.is_default, japanese.cue_count),
        ("ja", "日本語", true, 2)
    );
    assert_eq!(japanese.original_format, SubtitleFormat::SUBTITLE_FORMAT_VTT);
    assert_eq!(stored_subtitle(&ctx, &japanese).await, VTT);

    // SRT is converted; without a label the language tag is shown.
    let response: AddSubtitleResponse = ctx.call(VIDEOS, "AddSubtitle", Some(&alice), &add("en", SRT)).await.unwrap();
    let english = response.subtitle.as_option().cloned().unwrap();
    assert_eq!(
        (english.label.as_str(), english.is_default, english.original_format),
        ("en", false, SubtitleFormat::SUBTITLE_FORMAT_SRT.into())
    );
    assert_eq!(
        stored_subtitle(&ctx, &english).await,
        "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\nHello!\n\n00:00:04.000 --> 00:00:06.000\nBye!\n"
    );

    // An uploaded file works the same way.
    let upload = tokuly_media::testing::ready_asset(
        &ctx.app.core,
        alice.id,
        AssetPurpose::ASSET_PURPOSE_SUBTITLE,
        AssetKind::ASSET_KIND_FILE,
    )
    .await
    .unwrap();
    ctx.app
        .core
        .storage
        .put(Bucket::Private, &upload.original_key, SRT.as_bytes().to_vec(), "application/x-subrip")
        .await
        .unwrap();
    let request = AddSubtitleRequest {
        video_id: video_id.clone(),
        source: Some(add_subtitle_request::Source::AssetId(upload.id.to_string())),
        language: "pt_br".into(),
        ..Default::default()
    };
    let response: AddSubtitleResponse = ctx.call(VIDEOS, "AddSubtitle", Some(&alice), &request).await.unwrap();
    assert_eq!((response.subtitle.language.as_str(), response.subtitle.cue_count), ("pt-BR", 2));
    assert!(
        assets::find(&ctx.app.core, upload.id).await.unwrap().is_none(),
        "the upload was only a way to deliver the file"
    );

    // Anyone who may watch sees the tracks, default first; they come with the video too.
    let listed: ListSubtitlesResponse = ctx
        .call(VIDEOS, "ListSubtitles", None, &ListSubtitlesRequest { video_id: video_id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(listed.subtitles.iter().map(|t| t.language.as_str()).collect::<Vec<_>>(), vec!["ja", "en", "pt-BR"]);
    assert_eq!(ctx.get_video(Some(&bob), &video_id).await.unwrap().subtitles.len(), 3);

    let reasons = [
        ("SUBTITLE_LANGUAGE_EXISTS", add("ja", VTT)),
        ("SUBTITLE_LANGUAGE_INVALID", add("japanese!", VTT)),
        ("SUBTITLE_INVALID", add("fr", "just words")),
        ("SUBTITLE_INVALID", add("fr", "WEBVTT\n\n00:00:05.000 --> 00:00:01.000\nbackwards\n")),
        ("SUBTITLE_TOO_LARGE", add("fr", &"x".repeat(1024 * 1024 + 1))),
        ("SUBTITLE_LABEL_INVALID", AddSubtitleRequest { label: "l".repeat(41), ..add("fr", VTT) }),
        ("SUBTITLE_SOURCE_REQUIRED", AddSubtitleRequest { source: None, ..add("fr", VTT) }),
        (
            "ASSET_NOT_FOUND",
            AddSubtitleRequest {
                source: Some(add_subtitle_request::Source::AssetId(uuid::Uuid::now_v7().to_string())),
                ..add("fr", VTT)
            },
        ),
    ];
    for (reason, request) in reasons {
        assert_eq!(ctx.reason(VIDEOS, "AddSubtitle", Some(&alice), &request).await, reason);
    }
    assert_eq!(ctx.reason(VIDEOS, "AddSubtitle", Some(&bob), &add("fr", VTT)).await, "VIDEO_NOT_FOUND");
    assert_eq!(ctx.reason(VIDEOS, "AddSubtitle", None, &add("fr", VTT)).await, "SIGN_IN_REQUIRED");

    // Making another track the default moves the flag; content can be replaced.
    let request = UpdateSubtitleRequest {
        subtitle_id: english.id.clone(),
        label: Some("English".into()),
        is_default: Some(true),
        source: Some(update_subtitle_request::Source::Content(
            "WEBVTT\n\n00:00:02.000 --> 00:00:03.000\nReplaced\n".into(),
        )),
        ..Default::default()
    };
    let response: UpdateSubtitleResponse = ctx.call(VIDEOS, "UpdateSubtitle", Some(&alice), &request).await.unwrap();
    let updated = response.subtitle.as_option().cloned().unwrap();
    assert_eq!(
        (updated.label.as_str(), updated.is_default, updated.cue_count, updated.original_format),
        ("English", true, 1, SubtitleFormat::SUBTITLE_FORMAT_VTT.into())
    );
    assert_ne!(updated.url, english.url, "new content gets a new URL, so caches never serve the old file");
    assert!(stored_subtitle(&ctx, &updated).await.contains("Replaced"));
    let listed: ListSubtitlesResponse = ctx
        .call(VIDEOS, "ListSubtitles", None, &ListSubtitlesRequest { video_id: video_id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(
        listed.subtitles.iter().filter(|t| t.is_default).map(|t| t.language.as_str()).collect::<Vec<_>>(),
        vec!["en"]
    );
    // A video may also have no default track.
    let request =
        UpdateSubtitleRequest { subtitle_id: english.id.clone(), is_default: Some(false), ..Default::default() };
    let _: UpdateSubtitleResponse = ctx.call(VIDEOS, "UpdateSubtitle", Some(&alice), &request).await.unwrap();
    let listed: ListSubtitlesResponse = ctx
        .call(VIDEOS, "ListSubtitles", None, &ListSubtitlesRequest { video_id: video_id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert!(listed.subtitles.iter().all(|t| !t.is_default));

    let clash =
        UpdateSubtitleRequest { subtitle_id: english.id.clone(), language: Some("ja".into()), ..Default::default() };
    assert_eq!(ctx.reason(VIDEOS, "UpdateSubtitle", Some(&alice), &clash).await, "SUBTITLE_LANGUAGE_EXISTS");
    let rename =
        UpdateSubtitleRequest { subtitle_id: english.id.clone(), label: Some("x".into()), ..Default::default() };
    assert_eq!(ctx.reason(VIDEOS, "UpdateSubtitle", Some(&bob), &rename).await, "SUBTITLE_NOT_FOUND");

    // Deleting removes the track and its file.
    let delete = DeleteSubtitleRequest { subtitle_id: japanese.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(VIDEOS, "DeleteSubtitle", Some(&bob), &delete).await, "SUBTITLE_NOT_FOUND");
    let _: DeleteSubtitleResponse = ctx.call(VIDEOS, "DeleteSubtitle", Some(&alice), &delete).await.unwrap();
    assert_eq!(ctx.reason(VIDEOS, "DeleteSubtitle", Some(&alice), &delete).await, "SUBTITLE_NOT_FOUND");
    let base = ctx.app.core.storage.media_url("");
    assert!(
        ctx.app.core.storage.head(Bucket::Media, japanese.url.strip_prefix(&base).unwrap()).await.unwrap().is_none()
    );

    // Tracks of a video one may not watch are not listed.
    let private = ctx.published(&alice, &channel, "private", Visibility::VISIBILITY_PRIVATE).await;
    let request = ListSubtitlesRequest { video_id: card(&private).id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(VIDEOS, "ListSubtitles", Some(&bob), &request).await, "VIDEO_NOT_FOUND");
}

#[tokio::test]
async fn deleting_a_video_removes_everything_that_hangs_off_it() {
    let ctx = start().await;
    ctx.app.core.storage.ensure_buckets().await.unwrap();
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.public_video(&alice, &channel, "さよなら").await;
    let video_id = card(&video).id.clone();
    let upload = video.studio.as_option().unwrap().asset_id.clone();
    let request = AddSubtitleRequest {
        video_id: video_id.clone(),
        source: Some(add_subtitle_request::Source::Content(VTT.into())),
        language: "ja".into(),
        ..Default::default()
    };
    let response: AddSubtitleResponse = ctx.call(VIDEOS, "AddSubtitle", Some(&alice), &request).await.unwrap();
    let track = response.subtitle.as_option().cloned().unwrap();
    ctx.rate(&bob, &video_id, Rating::RATING_LIKE).await;
    let _: AddPlaylistItemResponse = ctx
        .call(
            PLAYLISTS,
            "AddPlaylistItem",
            Some(&bob),
            &AddPlaylistItemRequest { playlist_id: "WL".into(), video_id: video_id.clone(), ..Default::default() },
        )
        .await
        .unwrap();

    let delete = DeleteVideoRequest { video_id: video_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(VIDEOS, "DeleteVideo", Some(&bob), &delete).await, "VIDEO_NOT_FOUND");
    assert_eq!(ctx.reason(VIDEOS, "DeleteVideo", None, &delete).await, "SIGN_IN_REQUIRED");
    let _: DeleteVideoResponse = ctx.call(VIDEOS, "DeleteVideo", Some(&alice), &delete).await.unwrap();

    assert_eq!(ctx.get_video(Some(&alice), &video_id).await.unwrap_err().reason, "VIDEO_NOT_FOUND");
    assert!(assets::find(&ctx.app.core, id(&upload)).await.unwrap().is_none(), "the upload goes with the video");
    let base = ctx.app.core.storage.media_url("");
    assert!(
        ctx.app.core.storage.head(Bucket::Media, track.url.strip_prefix(&base).unwrap()).await.unwrap().is_none(),
        "so do its subtitle files"
    );
    assert!(search_docs(&ctx, &video_id).await.last().unwrap().is_none());
    let deleted = ctx.app.events_by("video.video.deleted", alice.id).await;
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].data["video_id"], video_id);
    // Playlists that held it count one less.
    let lists: ListMyPlaylistsResponse =
        ctx.call(PLAYLISTS, "ListMyPlaylists", Some(&bob), &ListMyPlaylistsRequest::default()).await.unwrap();
    assert!(lists.playlists.iter().all(|playlist| playlist.item_count == 0));
}

#[tokio::test]
async fn a_published_video_that_changes_visibility_says_so() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.published(&alice, &channel, "みんなへ", Visibility::VISIBILITY_FRIENDS).await;
    let video_id = card(&video).id.clone();
    let change = |visibility: Visibility| UpdateVideoRequest {
        video_id: video_id.clone(),
        visibility: Some(visibility.into()),
        ..Default::default()
    };
    update(&ctx, &alice, &change(Visibility::VISIBILITY_PRIVATE)).await;
    update(&ctx, &alice, &change(Visibility::VISIBILITY_PRIVATE)).await; // no change
    update(
        &ctx,
        &alice,
        &UpdateVideoRequest { video_id: video_id.clone(), title: Some("t".into()), ..Default::default() },
    )
    .await;
    let request = PublishVideoRequest {
        video_id: video_id.clone(),
        visibility: Some(Visibility::VISIBILITY_PUBLIC.into()),
        ..Default::default()
    };
    let _: PublishVideoResponse = ctx.call(VIDEOS, "PublishVideo", Some(&alice), &request).await.unwrap();
    let events = ctx.app.events_by("video.video.visibility_changed", alice.id).await;
    let visibilities: Vec<i64> = events.iter().map(|event| event.data["visibility"].as_i64().unwrap()).collect();
    assert_eq!(visibilities, vec![5, 1], "once per change of a published video's visibility");
    assert_eq!(events[0].data["video_id"], serde_json::json!(video_id));
}
