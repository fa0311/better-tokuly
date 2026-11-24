//! `PlaylistService`: playlists and the built-in watch later / liked lists.
mod common;

use common::*;
use tokuly_proto::{common::Visibility, video::*};
use tokuly_testkit::{RpcError, TestUser};
use tokuly_video::VideoConfig;

async fn create(ctx: &Ctx, owner: &TestUser, title: &str, visibility: Visibility) -> Playlist {
    let request =
        CreatePlaylistRequest { title: title.to_owned(), visibility: visibility.into(), ..Default::default() };
    let response: CreatePlaylistResponse =
        ctx.call(PLAYLISTS, "CreatePlaylist", Some(owner), &request).await.expect("create playlist");
    response.playlist.as_option().cloned().unwrap()
}

async fn get(ctx: &Ctx, viewer: Option<&TestUser>, playlist_id: &str) -> Result<Playlist, RpcError> {
    let response: GetPlaylistResponse = ctx
        .call(
            PLAYLISTS,
            "GetPlaylist",
            viewer,
            &GetPlaylistRequest { playlist_id: playlist_id.to_owned(), ..Default::default() },
        )
        .await?;
    Ok(response.playlist.as_option().cloned().unwrap())
}

async fn add(ctx: &Ctx, owner: &TestUser, playlist_id: &str, video: &Video) -> Playlist {
    let request = AddPlaylistItemRequest {
        playlist_id: playlist_id.to_owned(),
        video_id: card(video).id.clone(),
        ..Default::default()
    };
    let response: AddPlaylistItemResponse =
        ctx.call(PLAYLISTS, "AddPlaylistItem", Some(owner), &request).await.expect("add playlist item");
    response.playlist.as_option().cloned().unwrap()
}

async fn items(ctx: &Ctx, viewer: Option<&TestUser>, playlist_id: &str) -> Vec<String> {
    let request = ListPlaylistItemsRequest { playlist_id: playlist_id.to_owned(), ..Default::default() };
    let response: ListPlaylistItemsResponse =
        ctx.call(PLAYLISTS, "ListPlaylistItems", viewer, &request).await.expect("list playlist items");
    response.items.iter().map(|item| item.video.title.clone()).collect()
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
async fn playlists_are_created_private_unless_told_otherwise() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.public_video(&alice, &channel, "最初の一本").await;
    let hidden =
        ctx.published(&bob, &ctx.channel(&bob, "bob").await, "bob private", Visibility::VISIBILITY_PRIVATE).await;

    let request = CreatePlaylistRequest {
        title: "  あとで見るやつ  ".into(),
        description: "メモ".into(),
        video_id: card(&video).short_id.clone(),
        ..Default::default()
    };
    let response: CreatePlaylistResponse = ctx.call(PLAYLISTS, "CreatePlaylist", Some(&alice), &request).await.unwrap();
    let playlist = response.playlist.as_option().unwrap();
    assert_eq!(
        (playlist.title.as_str(), playlist.description.as_str(), playlist.short_id.len()),
        ("あとで見るやつ", "メモ", 11)
    );
    assert_eq!(
        (playlist.visibility, playlist.kind),
        (Visibility::VISIBILITY_PRIVATE.into(), PlaylistKind::PLAYLIST_KIND_CUSTOM.into())
    );
    assert!(playlist.is_owner && playlist.item_count == 1 && playlist.channel.as_option().is_none());
    assert_eq!(playlist.owner.as_option().unwrap().id, alice.id.to_string());
    assert_eq!(
        playlist.thumbnail.as_option().unwrap().url,
        card(&video).thumbnail.as_option().unwrap().url,
        "the first video's thumbnail"
    );
    assert!(search_docs(&ctx, &playlist.id).await.is_empty(), "private playlists are not indexed");

    let base = CreatePlaylistRequest { title: "ok".into(), ..Default::default() };
    let reasons = [
        ("PLAYLIST_TITLE_INVALID", CreatePlaylistRequest { title: "  ".into(), ..base.clone() }),
        ("PLAYLIST_TITLE_INVALID", CreatePlaylistRequest { title: "t".repeat(101), ..base.clone() }),
        ("PLAYLIST_DESCRIPTION_TOO_LONG", CreatePlaylistRequest { description: "d".repeat(1001), ..base.clone() }),
        (
            "VISIBILITY_INVALID",
            CreatePlaylistRequest { visibility: Visibility::VISIBILITY_FRIENDS.into(), ..base.clone() },
        ),
        (
            "VISIBILITY_INVALID",
            CreatePlaylistRequest { visibility: Visibility::VISIBILITY_NO_FEED.into(), ..base.clone() },
        ),
        (
            "CHANNEL_NOT_FOUND",
            CreatePlaylistRequest { channel_id: ctx.channel(&bob, "other").await.summary.id.clone(), ..base.clone() },
        ),
        ("VIDEO_NOT_FOUND", CreatePlaylistRequest { video_id: card(&hidden).id.clone(), ..base.clone() }),
    ];
    for (reason, request) in reasons {
        assert_eq!(ctx.reason(PLAYLISTS, "CreatePlaylist", Some(&alice), &request).await, reason);
    }
    assert_eq!(ctx.reason(PLAYLISTS, "CreatePlaylist", None, &base).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn limits_apply_to_playlists_and_their_items() {
    let ctx = start_with(VideoConfig { max_playlists: 2, max_playlist_items: 2, ..test_config() }).await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    let first = create(&ctx, &alice, "one", Visibility::VISIBILITY_PRIVATE).await;
    create(&ctx, &alice, "two", Visibility::VISIBILITY_PRIVATE).await;
    let request = CreatePlaylistRequest { title: "three".into(), ..Default::default() };
    assert_eq!(ctx.reason(PLAYLISTS, "CreatePlaylist", Some(&alice), &request).await, "PLAYLIST_LIMIT_REACHED");
    // The built-in lists do not count against the limit.
    let lists: ListMyPlaylistsResponse =
        ctx.call(PLAYLISTS, "ListMyPlaylists", Some(&alice), &ListMyPlaylistsRequest::default()).await.unwrap();
    assert_eq!(lists.playlists.len(), 4);

    let videos = [
        ctx.public_video(&alice, &channel, "a").await,
        ctx.public_video(&alice, &channel, "b").await,
        ctx.public_video(&alice, &channel, "c").await,
    ];
    add(&ctx, &alice, &first.id, &videos[0]).await;
    add(&ctx, &alice, &first.id, &videos[1]).await;
    let full = AddPlaylistItemRequest {
        playlist_id: first.id.clone(),
        video_id: card(&videos[2]).id.clone(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(PLAYLISTS, "AddPlaylistItem", Some(&alice), &full).await, "PLAYLIST_FULL");
    // Adding what is already there is not an addition.
    assert_eq!(add(&ctx, &alice, &first.id, &videos[0]).await.item_count, 2);
}

#[tokio::test]
async fn visibility_decides_who_can_open_a_playlist() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let blocked = ctx.app.user("blocked").await;
    ctx.block(&alice, &blocked).await;
    let channel = ctx.channel(&alice, "ch").await;

    let private = create(&ctx, &alice, "private", Visibility::VISIBILITY_PRIVATE).await;
    let unlisted = create(&ctx, &alice, "unlisted", Visibility::VISIBILITY_UNLISTED).await;
    let request = CreatePlaylistRequest {
        title: "public".into(),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        channel_id: channel.summary.handle.clone(),
        ..Default::default()
    };
    let response: CreatePlaylistResponse = ctx.call(PLAYLISTS, "CreatePlaylist", Some(&alice), &request).await.unwrap();
    let public = response.playlist.as_option().cloned().unwrap();
    assert_eq!(public.channel.as_option().unwrap().id, channel.summary.id);

    for (playlist, open) in [(&private, false), (&unlisted, true), (&public, true)] {
        for reference in [&playlist.id, &playlist.short_id] {
            assert!(get(&ctx, Some(&alice), reference).await.is_ok());
            assert_eq!(get(&ctx, Some(&bob), reference).await.is_ok(), open, "{} for a stranger", playlist.title);
            assert_eq!(get(&ctx, None, reference).await.is_ok(), open, "{} anonymously", playlist.title);
            assert_eq!(get(&ctx, Some(&blocked), reference).await.unwrap_err().reason, "PLAYLIST_NOT_FOUND");
        }
    }
    assert_eq!(get(&ctx, Some(&bob), &private.id).await.unwrap_err().reason, "PLAYLIST_NOT_FOUND");
    assert_eq!(get(&ctx, None, "no such list!").await.unwrap_err().reason, "PLAYLIST_NOT_FOUND");

    // A playlist on a channel does not say which account made it; a personal one does.
    let seen = get(&ctx, Some(&bob), &public.id).await.unwrap();
    assert!(seen.owner.as_option().is_none() && !seen.is_owner);
    assert_eq!(get(&ctx, Some(&alice), &public.id).await.unwrap().owner.as_option().unwrap().id, alice.id.to_string());
    assert_eq!(get(&ctx, Some(&bob), &unlisted.id).await.unwrap().owner.as_option().unwrap().id, alice.id.to_string());

    // Only public playlists are listed on the channel and indexed.
    let listing = ListChannelPlaylistsRequest { channel_id: channel.summary.id.clone(), ..Default::default() };
    let listed: ListChannelPlaylistsResponse =
        ctx.call(PLAYLISTS, "ListChannelPlaylists", None, &listing).await.unwrap();
    assert_eq!(listed.playlists.iter().map(|p| p.title.as_str()).collect::<Vec<_>>(), vec!["public"]);
    let doc = search_docs(&ctx, &public.id).await.last().cloned().unwrap().expect("public playlists are searchable");
    assert_eq!(
        (doc["kind"].as_str(), doc["path"].as_str()),
        (Some("playlist"), Some(format!("/playlist/{}", public.short_id).as_str()))
    );
    assert!(search_docs(&ctx, &unlisted.id).await.is_empty());
    let missing = ListChannelPlaylistsRequest { channel_id: unique("nochannel"), ..Default::default() };
    assert_eq!(ctx.reason(PLAYLISTS, "ListChannelPlaylists", None, &missing).await, "CHANNEL_NOT_FOUND");
}

#[tokio::test]
async fn items_keep_their_order_and_can_be_moved() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let playlist = create(&ctx, &alice, "順番", Visibility::VISIBILITY_UNLISTED).await;
    let mut videos = Vec::new();
    for title in ["a", "b", "c", "d"] {
        let video = ctx.public_video(&alice, &channel, title).await;
        add(&ctx, &alice, &playlist.short_id, &video).await;
        videos.push(video);
    }
    assert_eq!(items(&ctx, Some(&bob), &playlist.id).await, vec!["a", "b", "c", "d"], "in the order they were added");

    let move_item = |video: &Video, after: Option<&Video>| MovePlaylistItemRequest {
        playlist_id: playlist.id.clone(),
        video_id: card(video).id.clone(),
        after_video_id: after.map(|video| card(video).id.clone()).unwrap_or_default(),
        ..Default::default()
    };
    let _: MovePlaylistItemResponse =
        ctx.call(PLAYLISTS, "MovePlaylistItem", Some(&alice), &move_item(&videos[3], None)).await.unwrap();
    assert_eq!(items(&ctx, None, &playlist.id).await, vec!["d", "a", "b", "c"], "an empty anchor moves to the front");
    let _: MovePlaylistItemResponse =
        ctx.call(PLAYLISTS, "MovePlaylistItem", Some(&alice), &move_item(&videos[0], Some(&videos[2]))).await.unwrap();
    assert_eq!(items(&ctx, None, &playlist.id).await, vec!["d", "b", "c", "a"]);
    let _: MovePlaylistItemResponse =
        ctx.call(PLAYLISTS, "MovePlaylistItem", Some(&alice), &move_item(&videos[2], Some(&videos[3]))).await.unwrap();
    assert_eq!(items(&ctx, None, &playlist.id).await, vec!["d", "c", "b", "a"]);
    let _: MovePlaylistItemResponse =
        ctx.call(PLAYLISTS, "MovePlaylistItem", Some(&alice), &move_item(&videos[1], Some(&videos[1]))).await.unwrap();
    assert_eq!(items(&ctx, None, &playlist.id).await, vec!["d", "c", "b", "a"], "after itself: stays");

    // Pages follow the order without gaps.
    let first: ListPlaylistItemsResponse = ctx
        .call(
            PLAYLISTS,
            "ListPlaylistItems",
            None,
            &ListPlaylistItemsRequest { playlist_id: playlist.id.clone(), page: page(3, ""), ..Default::default() },
        )
        .await
        .unwrap();
    let info = first.page.as_option().unwrap();
    assert!(info.has_more && first.items.iter().all(|item| item.added_at.as_option().is_some()));
    let rest: ListPlaylistItemsResponse = ctx
        .call(
            PLAYLISTS,
            "ListPlaylistItems",
            None,
            &ListPlaylistItemsRequest {
                playlist_id: playlist.id.clone(),
                page: page(3, &info.next_cursor),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(rest.items.iter().map(|item| item.video.title.as_str()).collect::<Vec<_>>(), vec!["a"]);

    let outsider = ctx.public_video(&alice, &channel, "not in the list").await;
    assert_eq!(
        ctx.reason(PLAYLISTS, "MovePlaylistItem", Some(&alice), &move_item(&outsider, None)).await,
        "PLAYLIST_ITEM_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(PLAYLISTS, "MovePlaylistItem", Some(&alice), &move_item(&videos[0], Some(&outsider))).await,
        "PLAYLIST_ITEM_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(PLAYLISTS, "MovePlaylistItem", Some(&bob), &move_item(&videos[0], None)).await,
        "PLAYLIST_NOT_FOUND"
    );

    // Removing: by the owner only; removing what is not there is fine.
    let remove = |video: &Video| RemovePlaylistItemRequest {
        playlist_id: playlist.id.clone(),
        video_id: card(video).short_id.clone(),
        ..Default::default()
    };
    assert_eq!(
        ctx.reason(PLAYLISTS, "RemovePlaylistItem", Some(&bob), &remove(&videos[0])).await,
        "PLAYLIST_NOT_FOUND"
    );
    let response: RemovePlaylistItemResponse =
        ctx.call(PLAYLISTS, "RemovePlaylistItem", Some(&alice), &remove(&videos[3])).await.unwrap();
    assert_eq!(response.playlist.item_count, 3);
    let response: RemovePlaylistItemResponse =
        ctx.call(PLAYLISTS, "RemovePlaylistItem", Some(&alice), &remove(&outsider)).await.unwrap();
    assert_eq!(response.playlist.item_count, 3);
    assert_eq!(items(&ctx, None, &playlist.id).await, vec!["c", "b", "a"]);

    // Videos the viewer may not watch are left out of the listing (but still counted).
    let hide = UpdateVideoRequest {
        video_id: card(&videos[2]).id.clone(),
        visibility: Some(Visibility::VISIBILITY_PRIVATE.into()),
        ..Default::default()
    };
    let _: UpdateVideoResponse = ctx.call(VIDEOS, "UpdateVideo", Some(&alice), &hide).await.unwrap();
    assert_eq!(items(&ctx, Some(&bob), &playlist.id).await, vec!["b", "a"]);
    assert_eq!(items(&ctx, Some(&alice), &playlist.id).await, vec!["c", "b", "a"]);
    let seen = get(&ctx, Some(&bob), &playlist.id).await.unwrap();
    assert_eq!(seen.item_count, 3);
    assert_eq!(
        seen.thumbnail.as_option().unwrap().url,
        card(&videos[1]).thumbnail.as_option().unwrap().url,
        "the thumbnail skips what the viewer cannot see"
    );

    // Others cannot add; nobody can add what they cannot watch.
    let foreign = AddPlaylistItemRequest {
        playlist_id: playlist.id.clone(),
        video_id: card(&videos[0]).id.clone(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(PLAYLISTS, "AddPlaylistItem", Some(&bob), &foreign).await, "PLAYLIST_NOT_FOUND");
    assert_eq!(ctx.reason(PLAYLISTS, "AddPlaylistItem", None, &foreign).await, "SIGN_IN_REQUIRED");
    let bobs = create(&ctx, &bob, "bob's", Visibility::VISIBILITY_PRIVATE).await;
    let unwatchable = AddPlaylistItemRequest {
        playlist_id: bobs.id.clone(),
        video_id: card(&videos[2]).id.clone(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(PLAYLISTS, "AddPlaylistItem", Some(&bob), &unwatchable).await, "VIDEO_NOT_FOUND");
    let private_list = ListPlaylistItemsRequest { playlist_id: bobs.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(PLAYLISTS, "ListPlaylistItems", Some(&alice), &private_list).await, "PLAYLIST_NOT_FOUND");
}

#[tokio::test]
async fn watch_later_and_liked_videos_are_built_in() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let first = ctx.public_video(&alice, &channel, "first").await;
    let second = ctx.public_video(&alice, &channel, "second").await;

    // Watch later: newest first.
    let saved = add(&ctx, &bob, "WL", &first).await;
    assert_eq!(
        (saved.kind, saved.visibility, saved.item_count),
        (PlaylistKind::PLAYLIST_KIND_WATCH_LATER.into(), Visibility::VISIBILITY_PRIVATE.into(), 1)
    );
    add(&ctx, &bob, "WL", &second).await;
    assert_eq!(items(&ctx, Some(&bob), "WL").await, vec!["second", "first"]);
    assert!(ctx.get_video(Some(&bob), &card(&first).id).await.unwrap().viewer.as_option().unwrap().in_watch_later);
    let remove =
        RemovePlaylistItemRequest { playlist_id: "WL".into(), video_id: card(&first).id.clone(), ..Default::default() };
    let _: RemovePlaylistItemResponse = ctx.call(PLAYLISTS, "RemovePlaylistItem", Some(&bob), &remove).await.unwrap();
    assert!(!ctx.get_video(Some(&bob), &card(&first).id).await.unwrap().viewer.as_option().unwrap().in_watch_later);

    // Liked videos follow the ratings and cannot be edited by hand.
    ctx.rate(&bob, &card(&first).id, Rating::RATING_LIKE).await;
    ctx.rate(&bob, &card(&second).id, Rating::RATING_LIKE).await;
    assert_eq!(items(&ctx, Some(&bob), "LL").await, vec!["second", "first"]);
    let by_hand =
        AddPlaylistItemRequest { playlist_id: "LL".into(), video_id: card(&first).id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(PLAYLISTS, "AddPlaylistItem", Some(&bob), &by_hand).await, "PLAYLIST_READ_ONLY");
    let by_hand =
        RemovePlaylistItemRequest { playlist_id: "LL".into(), video_id: card(&first).id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(PLAYLISTS, "RemovePlaylistItem", Some(&bob), &by_hand).await, "PLAYLIST_READ_ONLY");
    let reorder =
        MovePlaylistItemRequest { playlist_id: "WL".into(), video_id: card(&second).id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(PLAYLISTS, "MovePlaylistItem", Some(&bob), &reorder).await, "PLAYLIST_READ_ONLY");

    // "Save to..." dialog: built-in lists first, with whether each holds the video.
    let custom = create(&ctx, &bob, "custom", Visibility::VISIBILITY_PRIVATE).await;
    add(&ctx, &bob, &custom.id, &second).await;
    let request = ListMyPlaylistsRequest { contains_video_id: card(&second).short_id.clone(), ..Default::default() };
    let lists: ListMyPlaylistsResponse = ctx.call(PLAYLISTS, "ListMyPlaylists", Some(&bob), &request).await.unwrap();
    let overview: Vec<(PlaylistKind, bool)> =
        lists.playlists.iter().map(|p| (p.kind.as_known().unwrap(), p.contains_video)).collect();
    assert_eq!(
        overview,
        vec![
            (PlaylistKind::PLAYLIST_KIND_WATCH_LATER, true),
            (PlaylistKind::PLAYLIST_KIND_LIKED, true),
            (PlaylistKind::PLAYLIST_KIND_CUSTOM, true)
        ]
    );
    let request = ListMyPlaylistsRequest { contains_video_id: card(&first).id.clone(), ..Default::default() };
    let lists: ListMyPlaylistsResponse = ctx.call(PLAYLISTS, "ListMyPlaylists", Some(&bob), &request).await.unwrap();
    assert_eq!(lists.playlists.iter().map(|p| p.contains_video).collect::<Vec<_>>(), vec![false, true, false]);
    assert_eq!(
        ctx.reason(PLAYLISTS, "ListMyPlaylists", None, &ListMyPlaylistsRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );

    // Built-in lists cannot be renamed, shared or deleted, and are nobody else's business.
    let watch_later = lists.playlists[0].clone();
    let rename = UpdatePlaylistRequest { playlist_id: "WL".into(), title: Some("mine".into()), ..Default::default() };
    assert_eq!(ctx.reason(PLAYLISTS, "UpdatePlaylist", Some(&bob), &rename).await, "PLAYLIST_BUILT_IN");
    assert_eq!(
        ctx.reason(
            PLAYLISTS,
            "DeletePlaylist",
            Some(&bob),
            &DeletePlaylistRequest { playlist_id: "LL".into(), ..Default::default() }
        )
        .await,
        "PLAYLIST_BUILT_IN"
    );
    assert_eq!(get(&ctx, Some(&alice), &watch_later.id).await.unwrap_err().reason, "PLAYLIST_NOT_FOUND");
    assert_eq!(get(&ctx, None, &watch_later.short_id).await.unwrap_err().reason, "PLAYLIST_NOT_FOUND");
    assert_eq!(get(&ctx, None, "WL").await.unwrap_err().reason, "SIGN_IN_REQUIRED");
    // Everyone has their own.
    assert!(items(&ctx, Some(&alice), "WL").await.is_empty());
}

#[tokio::test]
async fn playlists_can_be_edited_and_deleted_by_their_owner() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let playlist = create(&ctx, &alice, "before", Visibility::VISIBILITY_PRIVATE).await;
    add(&ctx, &alice, &playlist.id, &ctx.public_video(&alice, &channel, "v").await).await;

    let request = UpdatePlaylistRequest {
        playlist_id: playlist.short_id.clone(),
        title: Some("after".into()),
        visibility: Some(Visibility::VISIBILITY_PUBLIC.into()),
        channel_id: Some(channel.summary.id.clone()),
        ..Default::default()
    };
    let response: UpdatePlaylistResponse = ctx.call(PLAYLISTS, "UpdatePlaylist", Some(&alice), &request).await.unwrap();
    let updated = response.playlist.as_option().unwrap();
    assert_eq!((updated.title.as_str(), updated.visibility), ("after", Visibility::VISIBILITY_PUBLIC.into()));
    assert_eq!(updated.channel.as_option().unwrap().id, channel.summary.id);
    let doc = search_docs(&ctx, &playlist.id).await.last().cloned().unwrap().expect("now public and searchable");
    assert_eq!(doc["title"], "after");
    assert!(doc["image_url"].as_str().is_some_and(|url| !url.is_empty()));

    // An empty channel makes it personal again; the description was never touched.
    let request = UpdatePlaylistRequest {
        playlist_id: playlist.id.clone(),
        channel_id: Some(String::new()),
        visibility: Some(Visibility::VISIBILITY_UNLISTED.into()),
        ..Default::default()
    };
    let response: UpdatePlaylistResponse = ctx.call(PLAYLISTS, "UpdatePlaylist", Some(&alice), &request).await.unwrap();
    assert!(response.playlist.channel.as_option().is_none());
    assert!(search_docs(&ctx, &playlist.id).await.last().unwrap().is_none(), "unlisted playlists leave the index");

    let reasons = [
        ("PLAYLIST_TITLE_INVALID", UpdatePlaylistRequest { title: Some(String::new()), ..Default::default() }),
        (
            "PLAYLIST_DESCRIPTION_TOO_LONG",
            UpdatePlaylistRequest { description: Some("d".repeat(1001)), ..Default::default() },
        ),
        (
            "VISIBILITY_INVALID",
            UpdatePlaylistRequest { visibility: Some(Visibility::VISIBILITY_FRIENDS.into()), ..Default::default() },
        ),
        (
            "VISIBILITY_INVALID",
            UpdatePlaylistRequest { visibility: Some(Visibility::VISIBILITY_UNSPECIFIED.into()), ..Default::default() },
        ),
        (
            "CHANNEL_NOT_FOUND",
            UpdatePlaylistRequest {
                channel_id: Some(ctx.channel(&bob, "bob").await.summary.id.clone()),
                ..Default::default()
            },
        ),
    ];
    for (reason, mut request) in reasons {
        request.playlist_id = playlist.id.clone();
        assert_eq!(ctx.reason(PLAYLISTS, "UpdatePlaylist", Some(&alice), &request).await, reason);
    }
    let foreign =
        UpdatePlaylistRequest { playlist_id: playlist.id.clone(), title: Some("mine".into()), ..Default::default() };
    assert_eq!(ctx.reason(PLAYLISTS, "UpdatePlaylist", Some(&bob), &foreign).await, "PLAYLIST_NOT_FOUND");
    assert_eq!(ctx.reason(PLAYLISTS, "UpdatePlaylist", None, &foreign).await, "SIGN_IN_REQUIRED");

    let delete = DeletePlaylistRequest { playlist_id: playlist.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(PLAYLISTS, "DeletePlaylist", Some(&bob), &delete).await, "PLAYLIST_NOT_FOUND");
    let _: DeletePlaylistResponse = ctx.call(PLAYLISTS, "DeletePlaylist", Some(&alice), &delete).await.unwrap();
    assert_eq!(get(&ctx, Some(&alice), &playlist.id).await.unwrap_err().reason, "PLAYLIST_NOT_FOUND");
    let deleted = ctx.app.events_by("video.playlist.deleted", alice.id).await;
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].data["playlist_id"], playlist.id);
}

#[tokio::test]
async fn a_playlists_search_picture_never_shows_a_video_that_left_the_public() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let channel = ctx.channel(&alice, "ch").await;
    let image_of = |video: &Video| card(video).thumbnail.as_option().unwrap().url.clone();
    let picture = async |playlist: &Playlist| {
        let docs = search_docs(&ctx, &playlist.id).await;
        let doc = docs.last().cloned().flatten().expect("a public playlist is searchable");
        doc["image_url"].as_str().map(str::to_owned)
    };

    // Made private.
    let secret = ctx.public_video(&alice, &channel, "soon private").await;
    let list = create(&ctx, &bob, "mix", Visibility::VISIBILITY_PUBLIC).await;
    add(&ctx, &bob, &list.id, &secret).await;
    assert_eq!(picture(&list).await, Some(image_of(&secret)));
    let edit = UpdateVideoRequest {
        video_id: card(&secret).id.clone(),
        visibility: Some(Visibility::VISIBILITY_PRIVATE.into()),
        ..Default::default()
    };
    ctx.call::<_, UpdateVideoResponse>(VIDEOS, "UpdateVideo", Some(&alice), &edit).await.unwrap();
    assert_eq!(picture(&list).await, None);

    // Deleted.
    let doomed = ctx.public_video(&alice, &channel, "soon gone").await;
    let list = create(&ctx, &bob, "mix 2", Visibility::VISIBILITY_PUBLIC).await;
    add(&ctx, &bob, &list.id, &doomed).await;
    assert_eq!(picture(&list).await, Some(image_of(&doomed)));
    let request = DeleteVideoRequest { video_id: card(&doomed).id.clone(), ..Default::default() };
    ctx.call::<_, DeleteVideoResponse>(VIDEOS, "DeleteVideo", Some(&alice), &request).await.unwrap();
    assert_eq!(picture(&list).await, None);

    // Its owner suspended.
    let banned = ctx.public_video(&alice, &channel, "owner banned").await;
    let list = create(&ctx, &bob, "mix 3", Visibility::VISIBILITY_PUBLIC).await;
    add(&ctx, &bob, &list.id, &banned).await;
    assert_eq!(picture(&list).await, Some(image_of(&banned)));
    ctx.deliver_event("identity.user.banned", None, serde_json::json!({ "user_id": alice.id })).await;
    assert_eq!(picture(&list).await, None);

    // Its custom thumbnail taken down: the video and the list show the poster frame.
    let carol = ctx.app.user("carol").await;
    let channel = ctx.channel(&carol, "ch").await;
    let custom = ctx.image_upload(&carol, tokuly_proto::media::AssetPurpose::ASSET_PURPOSE_THUMBNAIL).await;
    let pictured = ctx
        .published_with(&carol, &channel, "custom picture", Visibility::VISIBILITY_PUBLIC, |r| {
            r.thumbnail_asset_id = custom.clone()
        })
        .await;
    let list = create(&ctx, &bob, "mix 4", Visibility::VISIBILITY_PUBLIC).await;
    add(&ctx, &bob, &list.id, &pictured).await;
    let custom_url = image_of(&pictured);
    assert_eq!(picture(&list).await, Some(custom_url.clone()));
    ctx.deliver_event(
        "media.asset.deleted",
        None,
        serde_json::json!({ "asset_id": custom, "owner_id": carol.id, "purpose": 0, "kind": 0 }),
    )
    .await;
    let video_doc = search_docs(&ctx, &card(&pictured).id).await.last().cloned().flatten().unwrap();
    assert_ne!(video_doc["image_url"].as_str(), Some(custom_url.as_str()));
    assert_ne!(picture(&list).await, Some(custom_url));
}
