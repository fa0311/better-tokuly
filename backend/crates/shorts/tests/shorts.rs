//! `ShortService`: creating, reading, editing and deleting shorts; covers,
//! pins, sounds, remixes, clips and mentions.
mod common;

use std::sync::Arc;

use async_trait::async_trait;
use common::*;
use tokuly_core::prelude::*;
use tokuly_media::assets;
use tokuly_proto::{
    common::{SubjectRef, SubjectType, Visibility},
    media::{AssetKind, AssetPurpose, AssetStatus},
    shorts::*,
};
use tokuly_shorts::{ClipDirectory, ClipMedia, access};
use tokuly_testkit::TestUser;
use tokuly_video::access::Viewer;

fn cover<P: buffa::ProtoBox<CoverInput>>(choice: cover_input::Cover) -> buffa::MessageField<CoverInput, P> {
    buffa::MessageField::some(CoverInput { cover: Some(choice), ..Default::default() })
}

fn sound<P: buffa::ProtoBox<SoundInput>>(choice: sound_input::Sound) -> buffa::MessageField<SoundInput, P> {
    buffa::MessageField::some(SoundInput { sound: Some(choice), ..Default::default() })
}

fn external<P: buffa::ProtoBox<SoundInput>>(title: &str, artist: &str) -> buffa::MessageField<SoundInput, P> {
    let track = ExternalTrack { title: title.to_owned(), artist: artist.to_owned(), ..Default::default() };
    sound(sound_input::Sound::External(Box::new(track)))
}

fn remix<P: buffa::ProtoBox<RemixInput>>(short_id: &str, kind: RemixKind) -> buffa::MessageField<RemixInput, P> {
    buffa::MessageField::some(RemixInput { short_id: short_id.to_owned(), kind: kind.into(), ..Default::default() })
}

fn subject<P: buffa::ProtoBox<SubjectRef>>(subject_type: SubjectType, id: &str) -> buffa::MessageField<SubjectRef, P> {
    buffa::MessageField::some(SubjectRef { r#type: subject_type.into(), id: id.to_owned(), ..Default::default() })
}

async fn hashtag_count(ctx: &Ctx, tag: &str) -> i64 {
    let request = GetHashtagRequest { tag: tag.to_owned(), ..Default::default() };
    match ctx.call::<_, GetHashtagResponse>(DISCOVERY, "GetHashtag", None, &request).await {
        Ok(response) => response.hashtag.as_option().map(|hashtag| hashtag.short_count).unwrap_or(0),
        Err(error) => {
            assert_eq!(error.reason, "HASHTAG_NOT_FOUND");
            0
        }
    }
}

async fn get_sound(ctx: &Ctx, viewer: Option<&TestUser>, sound_id: &str) -> Result<Sound, tokuly_testkit::RpcError> {
    let request = GetSoundRequest { sound_id: sound_id.to_owned(), ..Default::default() };
    let response: GetSoundResponse = ctx.call(DISCOVERY, "GetSound", viewer, &request).await?;
    Ok(response.sound.as_option().cloned().expect("the sound"))
}

/// The search documents written for a short, oldest first: `Some(public)`
/// for a document, `None` for a removal.
async fn search_changes(ctx: &Ctx, short_id: &str) -> Vec<Option<bool>> {
    ctx.app
        .events("search.changed")
        .await
        .into_iter()
        .filter(|event| event.data["kind"] == "short" && event.data["id"] == short_id)
        .map(|event| event.data["doc"].get("public").and_then(|public| public.as_bool()))
        .collect()
}

#[tokio::test]
async fn a_short_from_a_processed_upload_is_published_at_once() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ありすのチャンネル").await;
    let tag = unique("dance");
    let asset_id = ctx.upload(&alice).await;
    let request = CreateShortRequest {
        // No channel named: the default channel.
        asset_id: asset_id.clone(),
        caption: format!("  今日のダンス! #{tag} ＃{} @{} @nobody_here_x  ", tag.to_uppercase(), bob.handle),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        ..Default::default()
    };
    let short = ctx.create(&alice, &request).await.unwrap();

    assert_eq!(short.status, ShortStatus::SHORT_STATUS_PUBLISHED);
    assert_eq!(short.short_id.len(), 11);
    assert_eq!(short.channel.as_option().unwrap().id, channel.summary.id);
    assert!(short.caption.starts_with("今日のダンス!") && short.caption.ends_with("@nobody_here_x"));
    assert_eq!(short.hashtags, [tag.clone()], "hashtags are normalized and deduplicated");
    assert_eq!(short.mentions.len(), 1, "only handles that name an account are mentions");
    assert_eq!(
        (short.mentions[0].handle.as_str(), short.mentions[0].user_id.as_str()),
        (bob.handle.as_str(), bob.id.to_string().as_str())
    );
    assert_eq!((short.duration_seconds, short.width, short.height), (12.0, 1280, 720));
    let playback = short.playback.as_option().expect("streams");
    assert!(playback.hls_url.ends_with("master.m3u8"));
    assert_eq!(short.cover_kind, CoverKind::COVER_KIND_POSTER);
    assert_eq!(short.cover.as_option().expect("the poster frame").asset_id, asset_id);
    assert!(short.comments_enabled && short.allow_remix && short.allow_duet);
    assert!(short.published_at.as_option().is_some());
    assert_eq!(short.share_url, format!("http://localhost:8080/shorts/{}", short.short_id));
    assert!(viewer_state(&short).is_owner);
    let studio = short.studio.as_option().expect("the owner sees studio details");
    assert_eq!(studio.asset_id, asset_id);
    assert!(studio.notify_subscribers && !studio.cover_pending);
    // Every short with audio has an original sound others can use.
    let sound = short.sound.as_option().expect("an original sound");
    assert_eq!(
        (sound.kind, sound.origin_short_id.as_str(), sound.use_count),
        (SoundKind::SOUND_KIND_ORIGINAL.into(), short.short_id.as_str(), 1)
    );
    assert_eq!(sound.channel.as_option().unwrap().id, channel.summary.id);

    // Anyone can open it, by either ID, without the owner's details.
    for reference in [&short.id, &short.short_id] {
        let seen = ctx.get(None, reference).await.unwrap();
        assert_eq!(seen.id, short.id);
        assert!(seen.viewer.as_option().is_none() && seen.studio.as_option().is_none());
    }
    let seen = ctx.get(Some(&bob), &short.short_id).await.unwrap();
    assert!(!viewer_state(&seen).is_owner && seen.studio.as_option().is_none());

    // What follows from going public.
    let events = ctx.app.events_by("shorts.short.", alice.id).await;
    let topics: Vec<&str> = events.iter().map(|event| event.topic.as_str()).collect();
    assert_eq!(topics, ["shorts.short.created", "shorts.short.published"]);
    assert_eq!(events[1].data["first_publication"], true);
    assert_eq!(
        (events[1].data["id"].as_str(), events[1].data["short_id"].as_str()),
        (Some(short.id.as_str()), Some(short.short_id.as_str()))
    );
    let activity = ctx.app.events_by("activity.recorded", alice.id).await;
    assert_eq!(activity.len(), 1);
    assert_eq!(
        (activity[0].data["kind"].as_str(), activity[0].data["audience"].as_str()),
        (Some("short.published"), Some("friends"))
    );
    assert_eq!(activity[0].data["path"], format!("/shorts/{}", short.short_id));
    assert_eq!(search_changes(&ctx, &short.id).await, [Some(true)]);
    assert_eq!(hashtag_count(&ctx, &tag).await, 1);
    // The mentioned account is told, in the channel's name.
    let told = ctx.notifications(bob.id).await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].0, "shorts.mention");
    assert_eq!(told[0].1["title"]["ja"], "ありすのチャンネル");
    assert!(told[0].1["actor_id"].is_null(), "a channel does not reveal its account");
    // The upload is held by the short.
    assert!(assets::is_attached(&ctx.app.core, id(&asset_id)).await.unwrap());
}

#[tokio::test]
async fn an_unspecified_visibility_means_friends_only() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    ctx.channel(&alice, "ch").await;
    let request = CreateShortRequest { asset_id: ctx.upload(&alice).await, ..Default::default() };
    let short = ctx.create(&alice, &request).await.unwrap();
    assert_eq!(short.visibility, Visibility::VISIBILITY_FRIENDS);
    assert_eq!(ctx.get(Some(&bob), &short.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    assert_eq!(ctx.get(None, &short.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    ctx.befriend(&alice, &bob).await;
    assert_eq!(ctx.get(Some(&bob), &short.id).await.unwrap().id, short.id);
    // Nothing friends-only reaches the search index.
    assert!(search_changes(&ctx, &short.id).await.is_empty());
}

#[tokio::test]
async fn creating_a_short_validates_what_it_is_given() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let loner = ctx.app.user("loner").await;
    let channel = ctx.channel(&alice, "ch").await;
    let bobs = ctx.channel(&bob, "bob").await;
    let base = CreateShortRequest {
        channel_id: channel.summary.id.clone(),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        ..Default::default()
    };
    let with = |asset_id: String| CreateShortRequest { asset_id, ..base.clone() };
    let reason = |request: CreateShortRequest| {
        let ctx = &ctx;
        let alice = &alice;
        async move { ctx.create(alice, &request).await.expect_err("should be refused").reason }
    };

    // Who may post where.
    let upload = ctx.upload(&alice).await;
    let anonymous = ctx.call::<_, CreateShortResponse>(SHORTS, "CreateShort", None, &with(upload.clone())).await;
    assert_eq!(anonymous.unwrap_err().reason, "SIGN_IN_REQUIRED");
    let no_channel = CreateShortRequest { asset_id: ctx.upload(&loner).await, ..Default::default() };
    assert_eq!(ctx.create(&loner, &no_channel).await.unwrap_err().reason, "CHANNEL_REQUIRED");
    assert_eq!(
        reason(CreateShortRequest { channel_id: bobs.summary.id.clone(), ..with(upload.clone()) }).await,
        "CHANNEL_NOT_FOUND"
    );

    // The upload.
    assert_eq!(reason(with("nonsense".into())).await, "INVALID_ID");
    assert_eq!(reason(with(Uuid::now_v7().to_string())).await, "ASSET_NOT_FOUND");
    assert_eq!(reason(with(ctx.upload(&bob).await)).await, "ASSET_NOT_FOUND", "someone else's upload");
    let for_video = ctx.asset(&alice, AssetPurpose::ASSET_PURPOSE_VIDEO, AssetKind::ASSET_KIND_VIDEO).await;
    assert_eq!(reason(with(for_video)).await, "ASSET_WRONG_PURPOSE");
    let image = ctx.asset(&alice, AssetPurpose::ASSET_PURPOSE_SHORT, AssetKind::ASSET_KIND_IMAGE).await;
    assert_eq!(reason(with(image)).await, "ASSET_WRONG_PURPOSE");
    let failed = ctx.upload(&alice).await;
    ctx.set_asset_status(&failed, AssetStatus::ASSET_STATUS_FAILED, "CORRUPT_MEDIA").await;
    assert_eq!(reason(with(failed)).await, "ASSET_FAILED");
    let unfinished = ctx.upload(&alice).await;
    ctx.set_asset_status(&unfinished, AssetStatus::ASSET_STATUS_UPLOADING, "").await;
    assert_eq!(reason(with(unfinished)).await, "ASSET_NOT_READY");
    let long = ctx.upload(&alice).await;
    ctx.reshape(&long, 181.0, 1080, 1920, true).await;
    assert_eq!(reason(with(long)).await, "SHORT_TOO_LONG");
    let wide = ctx.upload(&alice).await;
    ctx.reshape(&wide, 10.0, 2560, 1080, true).await;
    assert_eq!(reason(with(wide)).await, "SHORT_NOT_VERTICAL");

    // The rest of the request.
    assert_eq!(
        reason(CreateShortRequest { caption: "あ".repeat(501), ..with(upload.clone()) }).await,
        "CAPTION_TOO_LONG"
    );
    assert_eq!(
        reason(CreateShortRequest { visibility: buffa::EnumValue::from(42), ..with(upload.clone()) }).await,
        "VISIBILITY_INVALID"
    );
    assert_eq!(
        reason(CreateShortRequest { cover: cover(cover_input::Cover::OffsetMs(-1)), ..with(upload.clone()) }).await,
        "COVER_OFFSET_INVALID"
    );
    assert_eq!(
        reason(CreateShortRequest { cover: cover(cover_input::Cover::OffsetMs(999_000)), ..with(upload.clone()) })
            .await,
        "COVER_OFFSET_INVALID"
    );
    let not_an_image = ctx.asset(&alice, AssetPurpose::ASSET_PURPOSE_THUMBNAIL, AssetKind::ASSET_KIND_VIDEO).await;
    assert_eq!(
        reason(CreateShortRequest { cover: cover(cover_input::Cover::AssetId(not_an_image)), ..with(upload.clone()) })
            .await,
        "ASSET_NOT_IMAGE"
    );
    let bobs_image = ctx.asset(&bob, AssetPurpose::ASSET_PURPOSE_THUMBNAIL, AssetKind::ASSET_KIND_IMAGE).await;
    assert_eq!(
        reason(CreateShortRequest { cover: cover(cover_input::Cover::AssetId(bobs_image)), ..with(upload.clone()) })
            .await,
        "ASSET_NOT_FOUND"
    );
    let unknown_sound = sound(sound_input::Sound::SoundId(Uuid::now_v7().to_string()));
    assert_eq!(reason(CreateShortRequest { sound: unknown_sound, ..with(upload.clone()) }).await, "SOUND_NOT_FOUND");
    assert_eq!(
        reason(CreateShortRequest { sound: external("  ", "誰か"), ..with(upload.clone()) }).await,
        "SOUND_TITLE_INVALID"
    );
    assert_eq!(
        reason(CreateShortRequest { sound: external(&"長".repeat(101), ""), ..with(upload.clone()) }).await,
        "SOUND_TITLE_INVALID"
    );
    let named_too_long = sound(sound_input::Sound::OriginalTitle("名".repeat(101)));
    assert_eq!(
        reason(CreateShortRequest { sound: named_too_long, ..with(upload.clone()) }).await,
        "SOUND_TITLE_INVALID"
    );
    let from_a_note = subject(SubjectType::SUBJECT_TYPE_NOTE, &Uuid::now_v7().to_string());
    assert_eq!(reason(CreateShortRequest { source: from_a_note, ..with(upload.clone()) }).await, "SOURCE_INVALID");
    let from_nothing = subject(SubjectType::SUBJECT_TYPE_VIDEO, &Uuid::now_v7().to_string());
    assert_eq!(reason(CreateShortRequest { source: from_nothing, ..with(upload.clone()) }).await, "SOURCE_NOT_FOUND");
    // No live domain is serving clips here.
    let from_a_clip = subject(SubjectType::SUBJECT_TYPE_CLIP, &Uuid::now_v7().to_string());
    assert_eq!(reason(CreateShortRequest { source: from_a_clip, ..with(upload.clone()) }).await, "SOURCE_NOT_FOUND");
    let of_nothing = remix(&Uuid::now_v7().to_string(), RemixKind::REMIX_KIND_REMIX);
    assert_eq!(reason(CreateShortRequest { remix: of_nothing, ..with(upload.clone()) }).await, "REMIX_NOT_FOUND");

    // After all that the upload is still good for exactly one short.
    let short = ctx.create(&alice, &with(upload.clone())).await.unwrap();
    assert_eq!(reason(with(upload)).await, "ASSET_ALREADY_USED");
    let unsaid = remix(&short.id, RemixKind::REMIX_KIND_UNSPECIFIED);
    assert_eq!(
        reason(CreateShortRequest { remix: unsaid, ..with(ctx.upload(&alice).await) }).await,
        "REMIX_KIND_INVALID"
    );
}

#[tokio::test]
async fn a_draft_goes_live_when_its_upload_is_processed() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let asset_id = ctx.upload(&alice).await;
    ctx.set_asset_status(&asset_id, AssetStatus::ASSET_STATUS_PROCESSING, "").await;
    let tag = unique("draft");
    let draft = ctx
        .short_with(&alice, &channel, &format!("まだ処理中 #{tag}"), |request| request.asset_id = asset_id.clone())
        .await;

    assert_eq!(draft.status, ShortStatus::SHORT_STATUS_PROCESSING);
    assert!(draft.playback.as_option().is_none() && draft.published_at.as_option().is_none());
    // A draft exists for its owner only.
    assert_eq!(ctx.get(Some(&bob), &draft.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    assert_eq!(ctx.get(Some(&alice), &draft.id).await.unwrap().status, ShortStatus::SHORT_STATUS_PROCESSING);
    assert_eq!(hashtag_count(&ctx, &tag).await, 0);
    let topics: Vec<String> = ctx.app.events_by("shorts.short.", alice.id).await.into_iter().map(|e| e.topic).collect();
    assert_eq!(topics, ["shorts.short.created"]);

    // The media domain finishes: its event publishes the short.
    ctx.set_asset_status(&asset_id, AssetStatus::ASSET_STATUS_READY, "").await;
    let ready = serde_json::json!({ "asset_id": asset_id, "owner_id": alice.id, "purpose": 13, "kind": 2 });
    ctx.deliver_event("media.asset.ready", Some(alice.id), ready.clone()).await;
    let live = ctx.get(Some(&bob), &draft.id).await.expect("the short went live");
    assert_eq!(live.status, ShortStatus::SHORT_STATUS_PUBLISHED);
    assert!(live.playback.as_option().is_some() && live.cover.as_option().is_some());
    assert_eq!(hashtag_count(&ctx, &tag).await, 1);
    // Delivered again, nothing happens twice.
    ctx.deliver_event("media.asset.ready", Some(alice.id), ready).await;
    let published = ctx.app.events_by("shorts.short.published", alice.id).await;
    assert_eq!(published.len(), 1);
    assert_eq!(ctx.app.events_by("activity.recorded", alice.id).await.len(), 1);
}

#[tokio::test]
async fn a_draft_fails_when_its_upload_cannot_become_a_short() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let processing = |asset_id: String| {
        let ctx = &ctx;
        async move {
            ctx.set_asset_status(&asset_id, AssetStatus::ASSET_STATUS_PROCESSING, "").await;
            asset_id
        }
    };

    // The media domain gives up on the file.
    let broken = processing(ctx.upload(&alice).await).await;
    let draft = ctx.short_with(&alice, &channel, "壊れたファイル", |request| request.asset_id = broken.clone()).await;
    ctx.set_asset_status(&broken, AssetStatus::ASSET_STATUS_FAILED, "CORRUPT_MEDIA").await;
    let failed = serde_json::json!({ "asset_id": broken, "owner_id": alice.id, "purpose": 13, "kind": 2, "error": "CORRUPT_MEDIA" });
    ctx.deliver_event("media.asset.failed", Some(alice.id), failed).await;
    let seen = ctx.get(Some(&alice), &draft.id).await.unwrap();
    assert_eq!(seen.status, ShortStatus::SHORT_STATUS_FAILED);
    assert_eq!(seen.studio.as_option().unwrap().processing_error, "CORRUPT_MEDIA");
    assert_eq!(ctx.get(Some(&bob), &draft.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");

    // The file is fine but turns out to be four minutes long.
    let long = processing(ctx.upload(&alice).await).await;
    let too_long = ctx.short_with(&alice, &channel, "長すぎた", |request| request.asset_id = long.clone()).await;
    ctx.reshape(&long, 240.0, 1080, 1920, true).await;
    ctx.set_asset_status(&long, AssetStatus::ASSET_STATUS_READY, "").await;
    // No event this time: the worker that catches missed events finds it.
    sqlx::query("UPDATE shorts SET created_at = now() - interval '5 minutes' WHERE id = $1")
        .bind(id(&too_long.id))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(ctx.shorts.reconcile().await.unwrap() >= 1);
    let seen = ctx.get(Some(&alice), &too_long.id).await.unwrap();
    assert_eq!(seen.status, ShortStatus::SHORT_STATUS_FAILED);
    assert_eq!(seen.studio.as_option().unwrap().processing_error, "SHORT_TOO_LONG");

    // The owner is told about each, and nothing was announced.
    let told = ctx.notifications(alice.id).await;
    assert_eq!(told.iter().map(|(kind, _)| kind.as_str()).collect::<Vec<_>>(), ["shorts.processing_failed"; 2]);
    assert_eq!(
        (told[0].1["data"]["error"].as_str(), told[1].1["data"]["error"].as_str()),
        (Some("CORRUPT_MEDIA"), Some("SHORT_TOO_LONG"))
    );
    assert!(ctx.app.events_by("shorts.short.published", alice.id).await.is_empty());
    // A failed short can still be deleted, which removes the upload.
    ctx.delete(&alice, &draft.id).await;
    assert!(assets::find(&ctx.app.core, id(&broken)).await.unwrap().is_none());
}

#[tokio::test]
async fn visibility_decides_who_sees_a_short() {
    let ctx = start().await;
    let owner = ctx.app.user("owner").await;
    let friend = ctx.app.user("friend").await;
    let stranger = ctx.app.user("stranger").await;
    let blocked = ctx.app.user("blocked").await;
    ctx.befriend(&owner, &friend).await;
    ctx.block(&owner, &blocked).await;
    let channel = ctx.channel(&owner, "ch").await;
    let tag = unique("vis");

    let visibilities = [
        Visibility::VISIBILITY_PUBLIC,
        Visibility::VISIBILITY_NO_FEED,
        Visibility::VISIBILITY_UNLISTED,
        Visibility::VISIBILITY_FRIENDS,
        Visibility::VISIBILITY_PRIVATE,
    ];
    let mut shorts = Vec::new();
    for visibility in visibilities {
        let short = ctx
            .short_with(&owner, &channel, &format!("{visibility:?} #{tag}"), |request| {
                request.visibility = visibility.into()
            })
            .await;
        shorts.push((visibility, short));
    }

    let viewers: [(&str, Option<&TestUser>); 5] = [
        ("owner", Some(&owner)),
        ("friend", Some(&friend)),
        ("stranger", Some(&stranger)),
        ("blocked", Some(&blocked)),
        ("anonymous", None),
    ];
    for (name, user) in viewers {
        let viewer = Viewer::load(&ctx.app.core, user.map(|user| user.id)).await.unwrap();
        let grid = ListChannelShortsRequest {
            channel_id: channel.summary.id.clone(),
            page: page(50, ""),
            ..Default::default()
        };
        let listed: Vec<String> =
            match ctx.call::<_, ListChannelShortsResponse>(DISCOVERY, "ListChannelShorts", user, &grid).await {
                Ok(response) => response.shorts.into_iter().map(|short| short.id).collect(),
                // The channel of someone in a block with the viewer does not exist for them.
                Err(error) => {
                    assert_eq!((name, error.reason.as_str()), ("blocked", "CHANNEL_NOT_FOUND"));
                    Vec::new()
                }
            };
        let by_tag = ListHashtagShortsRequest { tag: tag.clone(), page: page(50, ""), ..Default::default() };
        let tagged: ListHashtagShortsResponse = ctx.call(DISCOVERY, "ListHashtagShorts", user, &by_tag).await.unwrap();
        let tagged: Vec<String> = tagged.shorts.into_iter().map(|short| short.id).collect();
        for (visibility, short) in &shorts {
            let facts = ctx.row(&short.id).await.facts();
            // The rules as code and as SQL agree, for every kind of viewer.
            let opened = ctx.get(user, &short.id).await.is_ok();
            assert_eq!(opened, access::can_watch(&viewer, &facts), "{name} opening a {visibility:?} short");
            assert_eq!(
                listed.contains(&short.id),
                access::sees_listed(&viewer, &facts),
                "{name} seeing a {visibility:?} short on the grid"
            );
            assert_eq!(
                tagged.contains(&short.id),
                access::sees_public(&viewer, &facts),
                "{name} seeing a {visibility:?} short on a hashtag page"
            );
        }
    }
    // Spot checks, so the agreement is not an agreement on nonsense.
    let friends_only = &shorts[3].1.id;
    assert!(ctx.get(Some(&friend), friends_only).await.is_ok());
    assert_eq!(ctx.get(Some(&stranger), friends_only).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    assert_eq!(ctx.get(Some(&blocked), &shorts[0].1.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    assert!(ctx.get(None, &shorts[2].1.id).await.is_ok(), "unlisted shorts open by link");
    assert_eq!(hashtag_count(&ctx, &tag).await, 1, "only the public short counts for the hashtag");
}

#[tokio::test]
async fn editing_a_short_moves_everything_derived_from_it() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let channel = ctx.channel(&alice, "ch").await;
    let (old_tag, new_tag) = (unique("old"), unique("new"));
    let short = ctx.short(&alice, &channel, &format!("最初の文 #{old_tag} @{}", bob.handle)).await;
    assert_eq!((hashtag_count(&ctx, &old_tag).await, ctx.notified(bob.id).await.len()), (1, 1));

    // Only the owner edits.
    let hijack =
        UpdateShortRequest { short_id: short.id.clone(), caption: Some("乗っ取り".into()), ..Default::default() };
    assert_eq!(ctx.update(&bob, &hijack).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    let too_long =
        UpdateShortRequest { short_id: short.id.clone(), caption: Some("あ".repeat(501)), ..Default::default() };
    assert_eq!(ctx.update(&alice, &too_long).await.unwrap_err().reason, "CAPTION_TOO_LONG");
    let bad = UpdateShortRequest {
        short_id: short.id.clone(),
        visibility: Some(Visibility::VISIBILITY_UNSPECIFIED.into()),
        ..Default::default()
    };
    assert_eq!(ctx.update(&alice, &bad).await.unwrap_err().reason, "VISIBILITY_INVALID");

    // A new caption: hashtags and mentions follow it.
    let rewrite = UpdateShortRequest {
        short_id: short.short_id.clone(),
        caption: Some(format!("書き直し #{new_tag} @{} @{}", bob.handle, carol.handle)),
        comments_enabled: Some(false),
        allow_duet: Some(false),
        ..Default::default()
    };
    let edited = ctx.update(&alice, &rewrite).await.unwrap();
    assert_eq!(edited.hashtags, [new_tag.clone()]);
    assert_eq!(edited.mentions.len(), 2);
    assert!(!edited.comments_enabled && !edited.allow_duet && edited.allow_remix);
    assert_eq!((hashtag_count(&ctx, &old_tag).await, hashtag_count(&ctx, &new_tag).await), (0, 1));
    assert_eq!(ctx.notified(bob.id).await.len(), 1, "bob was told the first time");
    assert_eq!(ctx.notified(carol.id).await, ["shorts.mention"]);

    // Private: gone from the hashtag page and the search index, not announced again later.
    ctx.set_visibility(&alice, &short.id, Visibility::VISIBILITY_PRIVATE).await;
    assert_eq!(hashtag_count(&ctx, &new_tag).await, 0);
    assert_eq!(ctx.get(Some(&bob), &short.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    let changes = search_changes(&ctx, &short.id).await;
    assert_eq!(changes.last(), Some(&None), "{changes:?}");
    ctx.set_visibility(&alice, &short.id, Visibility::VISIBILITY_PUBLIC).await;
    assert_eq!(hashtag_count(&ctx, &new_tag).await, 1);
    assert_eq!(search_changes(&ctx, &short.id).await.last(), Some(&Some(true)));
    assert_eq!(ctx.app.events_by("activity.recorded", alice.id).await.len(), 1, "a short is announced once");
    let announcements: Vec<bool> = ctx
        .app
        .events_by("shorts.short.published", alice.id)
        .await
        .iter()
        .map(|event| event.data["first_publication"] == true)
        .collect();
    assert_eq!(announcements, [true]);
}

#[tokio::test]
async fn a_private_short_is_announced_when_it_first_reaches_an_audience() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let caption = format!("こっそり @{}", bob.handle);
    let short = ctx
        .short_with(&alice, &channel, &caption, |request| request.visibility = Visibility::VISIBILITY_PRIVATE.into())
        .await;
    assert_eq!(short.status, ShortStatus::SHORT_STATUS_PUBLISHED);
    assert!(ctx.app.events_by("activity.recorded", alice.id).await.is_empty());
    assert!(ctx.notified(bob.id).await.is_empty(), "nobody is told about a short they cannot open");
    let events = ctx.app.events_by("shorts.short.published", alice.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["first_publication"], false);

    ctx.set_visibility(&alice, &short.id, Visibility::VISIBILITY_PUBLIC).await;
    assert_eq!(ctx.app.events_by("activity.recorded", alice.id).await.len(), 1);
    assert_eq!(ctx.notified(bob.id).await, ["shorts.mention"]);
    let events = ctx.app.events_by("shorts.short.published", alice.id).await;
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].data["first_publication"], true);
}

#[tokio::test]
async fn mentions_are_told_once_they_can_see_the_short() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let friend = ctx.app.user("friend").await;
    let stranger = ctx.app.user("stranger").await;
    let blocked = ctx.app.user("blocked").await;
    ctx.befriend(&alice, &friend).await;
    ctx.block(&blocked, &alice).await;
    let channel = ctx.channel(&alice, "ch").await;
    let caption =
        format!("@{} @{} @{} @{} みんなありがとう", friend.handle, stranger.handle, blocked.handle, alice.handle);
    let short = ctx
        .short_with(&alice, &channel, &caption, |request| request.visibility = Visibility::VISIBILITY_FRIENDS.into())
        .await;

    // Someone in a block with the owner is not a mention at all.
    let mentioned: Vec<&str> = short.mentions.iter().map(|mention| mention.handle.as_str()).collect();
    assert_eq!(mentioned, [friend.handle.as_str(), stranger.handle.as_str(), alice.handle.as_str()]);
    assert_eq!(ctx.notified(friend.id).await, ["shorts.mention"]);
    assert!(ctx.notified(stranger.id).await.is_empty(), "a friends-only short is not theirs to open");
    assert!(ctx.notified(blocked.id).await.is_empty() && ctx.notified(alice.id).await.is_empty());

    ctx.set_visibility(&alice, &short.id, Visibility::VISIBILITY_UNLISTED).await;
    assert_eq!(ctx.notified(stranger.id).await, ["shorts.mention"]);
    assert_eq!(ctx.notified(friend.id).await.len(), 1, "nobody is told twice");
    let told = ctx.notifications(stranger.id).await;
    assert_eq!(told[0].1["path"], format!("/shorts/{}", short.short_id));
    assert_eq!(told[0].1["data"]["short_id"], short.short_id);
}

#[tokio::test]
async fn covers_are_the_poster_an_uploaded_image_or_a_frame() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    let image = ctx.asset(&alice, AssetPurpose::ASSET_PURPOSE_THUMBNAIL, AssetKind::ASSET_KIND_IMAGE).await;
    let short = ctx
        .short_with(&alice, &channel, "カバー付き", |request| {
            request.cover = cover(cover_input::Cover::AssetId(image.clone()))
        })
        .await;
    assert_eq!(short.cover_kind, CoverKind::COVER_KIND_IMAGE);
    assert_eq!(short.cover.as_option().unwrap().asset_id, image);
    assert!(assets::is_attached(&ctx.app.core, id(&image)).await.unwrap());
    let video_asset = short.studio.as_option().unwrap().asset_id.clone();

    // Back to the poster frame: the image is let go.
    let to_poster = UpdateShortRequest {
        short_id: short.id.clone(),
        cover: cover(cover_input::Cover::Poster(true)),
        ..Default::default()
    };
    let edited = ctx.update(&alice, &to_poster).await.unwrap();
    assert_eq!(edited.cover_kind, CoverKind::COVER_KIND_POSTER);
    assert_eq!(edited.cover.as_option().unwrap().asset_id, video_asset);
    assert!(!assets::is_attached(&ctx.app.core, id(&image)).await.unwrap());
    assert!(assets::find(&ctx.app.core, id(&image)).await.unwrap().is_some(), "the creator's upload is theirs to keep");

    // A frame is asked for: the poster stands in until it is cut.
    let to_frame = UpdateShortRequest {
        short_id: short.id.clone(),
        cover: cover(cover_input::Cover::OffsetMs(3500)),
        ..Default::default()
    };
    let edited = ctx.update(&alice, &to_frame).await.unwrap();
    assert_eq!(edited.cover_kind, CoverKind::COVER_KIND_FRAME);
    let studio = edited.studio.as_option().unwrap();
    assert!(studio.cover_pending);
    assert_eq!(studio.cover_offset_ms, 3500);
    assert_eq!(edited.cover.as_option().unwrap().asset_id, video_asset);

    // And an image again; an edit that names no cover keeps it.
    let to_image = UpdateShortRequest {
        short_id: short.id.clone(),
        cover: cover(cover_input::Cover::AssetId(image.clone())),
        ..Default::default()
    };
    assert_eq!(ctx.update(&alice, &to_image).await.unwrap().cover.as_option().unwrap().asset_id, image);
    let caption_only =
        UpdateShortRequest { short_id: short.id.clone(), caption: Some("文だけ".into()), ..Default::default() };
    let edited = ctx.update(&alice, &caption_only).await.unwrap();
    assert_eq!(
        (edited.cover_kind, edited.cover.as_option().unwrap().asset_id.as_str()),
        (CoverKind::COVER_KIND_IMAGE.into(), image.as_str())
    );
    assert!(!edited.studio.as_option().unwrap().cover_pending);

    // A cover image deleted by force falls back to the poster.
    assets::delete(&ctx.app.core, id(&image), None).await.unwrap();
    ctx.deliver_event(
        "media.asset.deleted",
        None,
        serde_json::json!({ "asset_id": image, "owner_id": alice.id, "purpose": 14, "kind": 1 }),
    )
    .await;
    let seen = ctx.get(Some(&alice), &short.id).await.unwrap();
    assert_eq!(
        (seen.cover_kind, seen.cover.as_option().unwrap().asset_id.as_str()),
        (CoverKind::COVER_KIND_POSTER.into(), video_asset.as_str())
    );
}

#[tokio::test]
async fn deleting_a_short_releases_what_it_held() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let tag = unique("bye");
    let short = ctx.short(&alice, &channel, &format!("消える #{tag}")).await;
    let asset_id = short.studio.as_option().unwrap().asset_id.clone();
    let sound_id = short.sound.as_option().unwrap().id.clone();

    let request = DeleteShortRequest { short_id: short.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(SHORTS, "DeleteShort", Some(&bob), &request).await, "SHORT_NOT_FOUND");
    assert_eq!(ctx.reason(SHORTS, "DeleteShort", None, &request).await, "SIGN_IN_REQUIRED");
    ctx.delete(&alice, &short.short_id).await;

    assert_eq!(ctx.get(Some(&alice), &short.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    assert_eq!(ctx.reason(SHORTS, "DeleteShort", Some(&alice), &request).await, "SHORT_NOT_FOUND");
    assert!(assets::find(&ctx.app.core, id(&asset_id)).await.unwrap().is_none(), "the upload goes with its short");
    assert_eq!(hashtag_count(&ctx, &tag).await, 0);
    assert_eq!(get_sound(&ctx, None, &sound_id).await.unwrap_err().reason, "SOUND_NOT_FOUND");
    assert_eq!(search_changes(&ctx, &short.id).await.last(), Some(&None));
    let deleted = ctx.app.events_by("shorts.short.deleted", alice.id).await;
    assert_eq!(deleted.len(), 1);
    assert_eq!(
        (deleted[0].data["id"].as_str(), deleted[0].data["reason"].as_str()),
        (Some(short.id.as_str()), Some("owner"))
    );
    assert!(ctx.app.subject_deleted(tokuly_proto::common::SubjectType::SUBJECT_TYPE_SHORT, &short.id).await);
}

#[tokio::test]
async fn a_channel_pins_at_most_three_shorts() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let mut ids = Vec::new();
    for n in 1..=5 {
        ids.push(ctx.short(&alice, &channel, &format!("{n}本目")).await.id);
    }
    let pin = |short_id: &str| PinShortRequest { short_id: short_id.to_owned(), ..Default::default() };
    assert_eq!(ctx.reason(SHORTS, "PinShort", Some(&bob), &pin(&ids[0])).await, "SHORT_NOT_FOUND");
    for short_id in [&ids[0], &ids[2], &ids[1]] {
        let pinned: PinShortResponse = ctx.call(SHORTS, "PinShort", Some(&alice), &pin(short_id)).await.unwrap();
        assert!(pinned.short.as_option().unwrap().pinned);
    }
    assert_eq!(ctx.reason(SHORTS, "PinShort", Some(&alice), &pin(&ids[3])).await, "PIN_LIMIT_REACHED");
    // Pinning a pinned short is not a fourth pin.
    let again: PinShortResponse = ctx.call(SHORTS, "PinShort", Some(&alice), &pin(&ids[0])).await.unwrap();
    assert!(again.short.as_option().unwrap().pinned);

    // The grid opens with the pins, the newest pin first, then the rest by age.
    let grid = ListChannelShortsRequest {
        channel_id: channel.summary.handle.clone(),
        page: page(4, ""),
        ..Default::default()
    };
    let first: ListChannelShortsResponse = ctx.call(DISCOVERY, "ListChannelShorts", Some(&bob), &grid).await.unwrap();
    assert_eq!(captions(&first.shorts), ["2本目", "3本目", "1本目", "5本目"]);
    assert_eq!(first.shorts.iter().map(|short| short.pinned).collect::<Vec<_>>(), [true, true, true, false]);
    let next = ListChannelShortsRequest { page: page(4, &first.page.as_option().unwrap().next_cursor), ..grid.clone() };
    let second: ListChannelShortsResponse = ctx.call(DISCOVERY, "ListChannelShorts", Some(&bob), &next).await.unwrap();
    assert_eq!(captions(&second.shorts), ["4本目"]);
    assert!(!second.page.as_option().unwrap().has_more);

    // Unpinning frees a place.
    let unpin = UnpinShortRequest { short_id: ids[2].clone(), ..Default::default() };
    assert_eq!(ctx.reason(SHORTS, "UnpinShort", Some(&bob), &unpin).await, "SHORT_NOT_FOUND");
    let unpinned: UnpinShortResponse = ctx.call(SHORTS, "UnpinShort", Some(&alice), &unpin).await.unwrap();
    assert!(!unpinned.short.as_option().unwrap().pinned);
    let _: PinShortResponse = ctx.call(SHORTS, "PinShort", Some(&alice), &pin(&ids[3])).await.unwrap();

    // Drafts cannot be pinned.
    let processing = ctx.upload(&alice).await;
    ctx.set_asset_status(&processing, AssetStatus::ASSET_STATUS_PROCESSING, "").await;
    let draft = ctx.short_with(&alice, &channel, "下書き", |request| request.asset_id = processing.clone()).await;
    assert_eq!(ctx.reason(SHORTS, "PinShort", Some(&alice), &pin(&draft.id)).await, "SHORT_NOT_PUBLISHED");
}

#[tokio::test]
async fn sounds_are_shared_while_their_short_allows_it() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let a = ctx.channel(&alice, "ありす").await;
    let b = ctx.channel(&bob, "ぼぶ").await;
    let c = ctx.channel(&carol, "きゃろる").await;
    let name = unique("ありすの鼻歌");
    let original = ctx
        .short_with(&alice, &a, "元の音", |request| {
            request.sound = sound(sound_input::Sound::OriginalTitle(format!("  {name} ")))
        })
        .await;
    let its_sound = original.sound.as_option().unwrap().clone();
    assert_eq!((its_sound.title.as_str(), its_sound.reusable, its_sound.use_count), (name.as_str(), true, 1));

    // Bob sets a short to Alice's sound.
    let use_it = sound(sound_input::Sound::SoundId(its_sound.id.clone()));
    let second = ctx.short_with(&bob, &b, "使ってみた", |request| request.sound = use_it.clone()).await;
    assert_eq!(second.sound.as_option().unwrap().id, its_sound.id);
    let page_of = get_sound(&ctx, Some(&carol), &its_sound.id).await.unwrap();
    assert_eq!((page_of.use_count, page_of.origin_short_id.as_str()), (2, original.short_id.as_str()));
    assert_eq!(page_of.channel.as_option().unwrap().id, a.summary.id);
    let list = ListSoundShortsRequest { sound_id: its_sound.id.clone(), ..Default::default() };
    let using: ListSoundShortsResponse = ctx.call(DISCOVERY, "ListSoundShorts", None, &list).await.unwrap();
    assert_eq!(captions(&using.shorts), ["使ってみた", "元の音"]);
    // It can be found by name when picking a sound.
    let search = SuggestSoundsRequest { query: name[..name.len() - 3].to_owned(), ..Default::default() };
    let found: SuggestSoundsResponse = ctx.call(DISCOVERY, "SuggestSounds", Some(&carol), &search).await.unwrap();
    assert_eq!(found.sounds.iter().map(|sound| sound.id.as_str()).collect::<Vec<_>>(), [its_sound.id.as_str()]);

    // Alice stops allowing remixes: no new shorts on her sound, except her own.
    let closed = UpdateShortRequest { short_id: original.id.clone(), allow_remix: Some(false), ..Default::default() };
    ctx.update(&alice, &closed).await.unwrap();
    let refused = CreateShortRequest {
        channel_id: c.summary.id.clone(),
        asset_id: ctx.upload(&carol).await,
        sound: use_it.clone(),
        ..Default::default()
    };
    assert_eq!(ctx.create(&carol, &refused).await.unwrap_err().reason, "SOUND_NOT_REUSABLE");
    assert!(!get_sound(&ctx, Some(&carol), &its_sound.id).await.unwrap().reusable);
    let own = ctx.short_with(&alice, &a, "自分でもう一度", |request| request.sound = use_it.clone()).await;
    assert_eq!(own.sound.as_option().unwrap().use_count, 3);
    let found: SuggestSoundsResponse = ctx.call(DISCOVERY, "SuggestSounds", Some(&carol), &search).await.unwrap();
    assert!(found.sounds.is_empty(), "only sounds the caller may use are suggested");

    // The original goes: the sound stays a page for the shorts using it.
    ctx.delete(&alice, &original.id).await;
    let orphan = get_sound(&ctx, Some(&carol), &its_sound.id).await.unwrap();
    assert!(orphan.origin_short_id.is_empty() && orphan.channel.as_option().is_none() && !orphan.reusable);
    assert_eq!((orphan.use_count, orphan.title.as_str()), (2, name.as_str()));
    // Once nothing uses it, it is gone.
    ctx.delete(&alice, &own.id).await;
    ctx.delete(&bob, &second.id).await;
    assert_eq!(get_sound(&ctx, None, &its_sound.id).await.unwrap_err().reason, "SOUND_NOT_FOUND");
    assert_eq!(get_sound(&ctx, None, "not-a-sound").await.unwrap_err().reason, "SOUND_NOT_FOUND");
}

#[tokio::test]
async fn external_tracks_are_one_sound_however_they_are_written() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let a = ctx.channel(&alice, "a").await;
    let b = ctx.channel(&bob, "b").await;
    let title = unique("夜に駆ける");
    let first = ctx.short_with(&alice, &a, "一つ目", |request| request.sound = external(&title, "YOASOBI")).await;
    let second = ctx
        .short_with(&bob, &b, "二つ目", |request| {
            request.sound = external(&format!("  {} ", title.to_uppercase()), "ｙｏａｓｏｂｉ")
        })
        .await;
    let track = second.sound.as_option().unwrap();
    assert_eq!(track.id, first.sound.as_option().unwrap().id);
    // The first short to name the track decides how it is written.
    assert_eq!(
        (track.kind, track.title.as_str(), track.artist.as_str()),
        (SoundKind::SOUND_KIND_EXTERNAL.into(), title.as_str(), "YOASOBI")
    );
    assert_eq!((track.use_count, track.reusable), (2, true));
    assert!(track.origin_short_id.is_empty());
    // Another artist is another track.
    let cover_version =
        ctx.short_with(&alice, &a, "カバー", |request| request.sound = external(&title, "だれか")).await;
    assert_ne!(cover_version.sound.as_option().unwrap().id, track.id);

    // A private short does not count as a use.
    ctx.set_visibility(&bob, &second.id, Visibility::VISIBILITY_PRIVATE).await;
    assert_eq!(get_sound(&ctx, None, &track.id).await.unwrap().use_count, 1);
    let list = ListSoundShortsRequest { sound_id: track.id.clone(), ..Default::default() };
    let using: ListSoundShortsResponse = ctx.call(DISCOVERY, "ListSoundShorts", None, &list).await.unwrap();
    assert_eq!(captions(&using.shorts), ["一つ目"]);
}

#[tokio::test]
async fn a_silent_video_has_no_sound_of_its_own() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    let silent = ctx.upload(&alice).await;
    ctx.reshape(&silent, 8.0, 1080, 1920, false).await;
    let short = ctx.short_with(&alice, &channel, "無音", |request| request.asset_id = silent.clone()).await;
    assert!(short.sound.as_option().is_none());
    // A track named by the creator stays: the attribution is theirs to make.
    let also_silent = ctx.upload(&alice).await;
    ctx.reshape(&also_silent, 8.0, 1080, 1920, false).await;
    let title = unique("あとから乗せた曲");
    let short = ctx
        .short_with(&alice, &channel, "曲付き", |request| {
            request.asset_id = also_silent.clone();
            request.sound = external(&title, "");
        })
        .await;
    assert_eq!(short.sound.as_option().unwrap().title, title);
}

#[tokio::test]
async fn remixes_and_duets_follow_the_originals_permissions() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let a = ctx.channel(&alice, "ありす").await;
    let b = ctx.channel(&bob, "ぼぶ").await;
    let original = ctx.short_with(&alice, &a, "元ネタ", |request| request.allow_duet = Some(false)).await;

    let remixed = ctx
        .short_with(&bob, &b, "リミックス", |request| {
            request.remix = remix(&original.short_id, RemixKind::REMIX_KIND_REMIX)
        })
        .await;
    let source = remixed.remix_of.as_option().expect("a remix names its original");
    assert_eq!(
        (source.kind, source.short_id.as_str()),
        (RemixKind::REMIX_KIND_REMIX.into(), original.short_id.as_str())
    );
    assert_eq!(source.channel.as_option().unwrap().id, a.summary.id);
    assert_eq!(ctx.get(None, &original.id).await.unwrap().remix_count, 1);

    // Duets were switched off, for everyone but the creator.
    let duet = CreateShortRequest {
        channel_id: b.summary.id.clone(),
        asset_id: ctx.upload(&bob).await,
        remix: remix(&original.id, RemixKind::REMIX_KIND_DUET),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        ..Default::default()
    };
    assert_eq!(ctx.create(&bob, &duet).await.unwrap_err().reason, "REMIX_NOT_ALLOWED");
    let own_duet = ctx
        .short_with(&alice, &a, "ひとりデュエット", |request| {
            request.remix = remix(&original.id, RemixKind::REMIX_KIND_DUET)
        })
        .await;
    assert_eq!(own_duet.remix_of.as_option().unwrap().kind, RemixKind::REMIX_KIND_DUET);

    // The list of remixes shows the public ones, newest first.
    let hidden = ctx
        .short_with(&bob, &b, "非公開リミックス", |request| {
            request.remix = remix(&original.id, RemixKind::REMIX_KIND_REMIX);
            request.visibility = Visibility::VISIBILITY_PRIVATE.into();
        })
        .await;
    let list = ListRemixesRequest { short_id: original.short_id.clone(), ..Default::default() };
    let remixes: ListRemixesResponse = ctx.call(SHORTS, "ListRemixes", None, &list).await.unwrap();
    assert_eq!(captions(&remixes.shorts), ["ひとりデュエット", "リミックス"]);
    assert_eq!(ctx.get(None, &original.id).await.unwrap().remix_count, 2);
    assert!(ctx.get(Some(&bob), &hidden.id).await.unwrap().remix_of.as_option().is_some());

    // A short the creator cannot see cannot be remixed, and reads as missing.
    ctx.set_visibility(&alice, &original.id, Visibility::VISIBILITY_PRIVATE).await;
    let of_private = CreateShortRequest {
        remix: remix(&original.id, RemixKind::REMIX_KIND_REMIX),
        asset_id: ctx.upload(&bob).await,
        ..duet.clone()
    };
    assert_eq!(ctx.create(&bob, &of_private).await.unwrap_err().reason, "REMIX_NOT_FOUND");
    assert_eq!(ctx.reason(SHORTS, "ListRemixes", Some(&bob), &list).await, "SHORT_NOT_FOUND");
    // Existing remixes keep saying what they are, without a way to the original.
    let orphaned = ctx.get(Some(&bob), &remixed.id).await.unwrap();
    let source = orphaned.remix_of.as_option().unwrap();
    assert!(source.short_id.is_empty() && source.channel.as_option().is_none());
    assert_eq!(source.kind, RemixKind::REMIX_KIND_REMIX);
    ctx.delete(&alice, &original.id).await;
    assert_eq!(
        ctx.get(Some(&bob), &remixed.id).await.unwrap().remix_of.as_option().unwrap().kind,
        RemixKind::REMIX_KIND_REMIX
    );
}

/// Stands in for the live domain: clips with their creator and media.
struct Clips(Vec<(Uuid, Uuid, Uuid)>);

#[async_trait]
impl ClipDirectory for Clips {
    async fn clip_of(&self, user_id: Uuid, clip_id: &str) -> AppResult<Option<ClipMedia>> {
        Ok(self
            .0
            .iter()
            .find(|(clip, creator, _)| clip.to_string() == clip_id && *creator == user_id)
            .map(|(clip_id, _, asset_id)| ClipMedia { clip_id: *clip_id, asset_id: *asset_id }))
    }
}

#[tokio::test]
async fn a_short_from_a_clip_needs_the_clip_to_be_the_callers() {
    let ctx = start().await;
    let streamer = ctx.app.user("streamer").await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let a = ctx.channel(&alice, "a").await;
    let b = ctx.channel(&bob, "b").await;
    // Clip media belongs to whoever streamed; the clip to whoever cut it.
    let clip_media = ctx.asset(&streamer, AssetPurpose::ASSET_PURPOSE_LIVE_ARCHIVE, AssetKind::ASSET_KIND_VIDEO).await;
    let other_media = ctx.asset(&streamer, AssetPurpose::ASSET_PURPOSE_LIVE_ARCHIVE, AssetKind::ASSET_KIND_VIDEO).await;
    let unprocessed = ctx.asset(&streamer, AssetPurpose::ASSET_PURPOSE_LIVE_ARCHIVE, AssetKind::ASSET_KIND_VIDEO).await;
    ctx.set_asset_status(&unprocessed, AssetStatus::ASSET_STATUS_PROCESSING, "").await;
    let (clip, raw_clip) = (Uuid::now_v7(), Uuid::now_v7());
    assets::attach(&ctx.app.core.db, id(&clip_media), &format!("live:clip:{clip}")).await.unwrap();
    ctx.shorts.set_clip_directory(Arc::new(Clips(vec![
        (clip, alice.id, id(&clip_media)),
        (raw_clip, alice.id, id(&unprocessed)),
    ])));

    let from_clip = |channel: &tokuly_proto::video::Channel, asset_id: &str, clip_id: Uuid| CreateShortRequest {
        channel_id: channel.summary.id.clone(),
        asset_id: asset_id.to_owned(),
        source: subject(SubjectType::SUBJECT_TYPE_CLIP, &clip_id.to_string()),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        ..Default::default()
    };
    // Not the clip's creator; not the clip's media; no clip named at all.
    assert_eq!(ctx.create(&bob, &from_clip(&b, &clip_media, clip)).await.unwrap_err().reason, "ASSET_NOT_FOUND");
    assert_eq!(ctx.create(&alice, &from_clip(&a, &other_media, clip)).await.unwrap_err().reason, "ASSET_NOT_FOUND");
    let unnamed = CreateShortRequest { source: None.into(), ..from_clip(&a, &clip_media, clip) };
    assert_eq!(ctx.create(&alice, &unnamed).await.unwrap_err().reason, "ASSET_NOT_FOUND");
    assert_eq!(ctx.create(&alice, &from_clip(&a, &unprocessed, raw_clip)).await.unwrap_err().reason, "ASSET_NOT_READY");

    let short = ctx.create(&alice, &from_clip(&a, &clip_media, clip)).await.unwrap();
    assert_eq!(short.status, ShortStatus::SHORT_STATUS_PUBLISHED);
    let source = short.source.as_option().expect("the clip is credited");
    assert_eq!((source.r#type, source.id.as_str()), (SubjectType::SUBJECT_TYPE_CLIP.into(), clip.to_string().as_str()));
    assert!(short.playback.as_option().is_some());
    // One short per clip.
    assert_eq!(ctx.create(&alice, &from_clip(&a, &clip_media, clip)).await.unwrap_err().reason, "ASSET_ALREADY_USED");

    // The media is the clip's: deleting the short leaves it alone.
    ctx.delete(&alice, &short.id).await;
    let media = assets::find(&ctx.app.core, id(&clip_media)).await.unwrap().expect("the clip's media survives");
    assert!(media.is_attached(), "the clip still holds it");
}

#[tokio::test]
async fn without_the_live_domain_only_the_callers_own_media_is_accepted() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let a = ctx.channel(&alice, "a").await;
    let clip = subject(SubjectType::SUBJECT_TYPE_CLIP, &Uuid::now_v7().to_string());
    let bobs = ctx.asset(&bob, AssetPurpose::ASSET_PURPOSE_LIVE_ARCHIVE, AssetKind::ASSET_KIND_VIDEO).await;
    let request = CreateShortRequest {
        channel_id: a.summary.id.clone(),
        asset_id: bobs,
        source: clip.clone(),
        ..Default::default()
    };
    assert_eq!(ctx.create(&alice, &request).await.unwrap_err().reason, "ASSET_NOT_FOUND");
    // Her own media, but no clip anyone can vouch for.
    let own = ctx.asset(&alice, AssetPurpose::ASSET_PURPOSE_LIVE_ARCHIVE, AssetKind::ASSET_KIND_VIDEO).await;
    let request = CreateShortRequest { asset_id: own.clone(), ..request };
    assert_eq!(ctx.create(&alice, &request).await.unwrap_err().reason, "SOURCE_NOT_FOUND");
    let no_source = CreateShortRequest { source: None.into(), ..request };
    assert_eq!(ctx.create(&alice, &no_source).await.unwrap_err().reason, "ASSET_WRONG_PURPOSE");

    // An ordinary upload may say which video it was cut from.
    let long_form = ctx.asset(&alice, AssetPurpose::ASSET_PURPOSE_VIDEO, AssetKind::ASSET_KIND_VIDEO).await;
    let video_id = tokuly_video::videos::create_from_asset(
        &ctx.app.core,
        tokuly_video::videos::NewVideo::new(id(&a.summary.id), id(&long_form), "長い動画"),
    )
    .await
    .unwrap();
    let cut = ctx
        .short_with(&alice, &a, "切り抜き", |request| {
            request.source = subject(SubjectType::SUBJECT_TYPE_VIDEO, &video_id.to_string())
        })
        .await;
    let source = cut.source.as_option().unwrap();
    assert_eq!(
        (source.r#type, source.id.as_str()),
        (SubjectType::SUBJECT_TYPE_VIDEO.into(), video_id.to_string().as_str())
    );
    // Someone else's private video is not something to point at.
    let b = ctx.channel(&bob, "b").await;
    let request = CreateShortRequest {
        channel_id: b.summary.id.clone(),
        asset_id: ctx.upload(&bob).await,
        source: subject(SubjectType::SUBJECT_TYPE_VIDEO, &video_id.to_string()),
        ..Default::default()
    };
    assert_eq!(ctx.create(&bob, &request).await.unwrap_err().reason, "SOURCE_NOT_FOUND");
}

#[tokio::test]
async fn the_studio_list_shows_every_state_to_the_owner_only() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let main = ctx.channel(&alice, "main").await;
    let side = ctx.channel(&alice, "side").await;
    ctx.short(&alice, &main, "公開").await;
    ctx.short_with(&alice, &main, "非公開", |request| request.visibility = Visibility::VISIBILITY_PRIVATE.into()).await;
    let processing = ctx.upload(&alice).await;
    ctx.set_asset_status(&processing, AssetStatus::ASSET_STATUS_PROCESSING, "").await;
    ctx.short_with(&alice, &side, "処理中", |request| request.asset_id = processing.clone()).await;
    let bobs = ctx.channel(&bob, "bob").await;
    ctx.short(&bob, &bobs, "ボブの").await;

    let list = |request: ListMyShortsRequest| {
        let ctx = &ctx;
        let alice = &alice;
        async move { ctx.call::<_, ListMyShortsResponse>(SHORTS, "ListMyShorts", Some(alice), &request).await }
    };
    let all = list(ListMyShortsRequest::default()).await.unwrap();
    assert_eq!(captions(&all.shorts), ["処理中", "非公開", "公開"]);
    assert!(all.shorts.iter().all(|short| short.studio.as_option().is_some()));
    let on_side =
        list(ListMyShortsRequest { channel_id: side.summary.handle.clone(), ..Default::default() }).await.unwrap();
    assert_eq!(captions(&on_side.shorts), ["処理中"]);
    let live = list(ListMyShortsRequest { status: ShortStatus::SHORT_STATUS_PUBLISHED.into(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(captions(&live.shorts), ["非公開", "公開"]);
    // Paging.
    let first = list(ListMyShortsRequest { page: page(2, ""), ..Default::default() }).await.unwrap();
    assert_eq!(captions(&first.shorts), ["処理中", "非公開"]);
    let cursor = first.page.as_option().unwrap().next_cursor.clone();
    let rest = list(ListMyShortsRequest { page: page(2, &cursor), ..Default::default() }).await.unwrap();
    assert_eq!(captions(&rest.shorts), ["公開"]);
    assert_eq!(
        list(ListMyShortsRequest { page: page(2, "garbage"), ..Default::default() }).await.unwrap_err().reason,
        "INVALID_CURSOR"
    );
    // Someone else's channel is not a filter one can use.
    let theirs = ListMyShortsRequest { channel_id: bobs.summary.id.clone(), ..Default::default() };
    assert_eq!(list(theirs).await.unwrap_err().reason, "CHANNEL_NOT_FOUND");
    assert_eq!(ctx.reason(SHORTS, "ListMyShorts", None, &ListMyShortsRequest::default()).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn batch_get_skips_what_the_caller_may_not_see() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let open = ctx.short(&alice, &channel, "見える").await;
    let closed = ctx
        .short_with(&alice, &channel, "見えない", |request| {
            request.visibility = Visibility::VISIBILITY_PRIVATE.into()
        })
        .await;
    let request = BatchGetShortsRequest {
        short_ids: vec![
            closed.id.clone(),
            "nonsense!".into(),
            open.short_id.clone(),
            Uuid::now_v7().to_string(),
            open.id.clone(),
        ],
        ..Default::default()
    };
    let seen: BatchGetShortsResponse = ctx.call(SHORTS, "BatchGetShorts", Some(&bob), &request).await.unwrap();
    assert_eq!(captions(&seen.shorts), ["見える", "見える"]);
    let own: BatchGetShortsResponse = ctx.call(SHORTS, "BatchGetShorts", Some(&alice), &request).await.unwrap();
    assert_eq!(captions(&own.shorts), ["見えない", "見える", "見える"]);
    let too_many = BatchGetShortsRequest { short_ids: vec![open.id.clone(); 51], ..Default::default() };
    assert_eq!(ctx.reason(SHORTS, "BatchGetShorts", None, &too_many).await, "TOO_MANY_SHORTS");
}
