//! The social graph's audiences on shorts: followers, close friends,
//! circles, and the open shorts of locked accounts.
mod common;

use common::*;
use tokuly_proto::{common::Visibility, shorts::*, video::Channel};
use tokuly_shorts::access;
use tokuly_testkit::TestUser;
use tokuly_video::access::Viewer;

async fn grid(ctx: &Ctx, viewer: Option<&TestUser>, channel: &Channel) -> Vec<String> {
    let request =
        ListChannelShortsRequest { channel_id: channel.summary.id.clone(), page: page(50, ""), ..Default::default() };
    let listed: ListChannelShortsResponse =
        ctx.call(DISCOVERY, "ListChannelShorts", viewer, &request).await.expect("channel grid");
    listed.shorts.into_iter().map(|short| short.caption).collect()
}

async fn hashtag(ctx: &Ctx, viewer: Option<&TestUser>, tag: &str) -> serde_json::Value {
    let request = GetHashtagRequest { tag: tag.to_owned(), ..Default::default() };
    let count = match ctx.call::<_, GetHashtagResponse>(DISCOVERY, "GetHashtag", viewer, &request).await {
        Ok(response) => response.hashtag.short_count.to_string(),
        Err(error) => error.reason,
    };
    let request = ListHashtagShortsRequest { tag: tag.to_owned(), page: page(50, ""), ..Default::default() };
    let listed = match ctx.call::<_, ListHashtagShortsResponse>(DISCOVERY, "ListHashtagShorts", viewer, &request).await
    {
        Ok(response) => response.shorts.len().to_string(),
        Err(error) => error.reason,
    };
    serde_json::json!([count, listed])
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
async fn followers_close_friends_and_circles_see_what_is_shared_with_them() {
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
    let mut shorts = Vec::new();
    for visibility in [VISIBILITY_FOLLOWERS, VISIBILITY_CLOSE_FRIENDS, VISIBILITY_CIRCLE] {
        let short = ctx
            .short_with(&owner, &channel, &format!("{visibility:?}"), |request| {
                request.visibility = visibility.into();
                if visibility == VISIBILITY_CIRCLE {
                    request.circle_id = circle.clone();
                }
            })
            .await;
        shorts.push(short);
    }
    let viewers: [(&str, Option<&TestUser>, [bool; 3]); 7] = [
        ("anonymous", None, [false, false, false]),
        ("stranger", Some(&stranger), [false, false, false]),
        ("follower", Some(&follower), [true, false, false]),
        ("friend", Some(&friend), [true, false, false]),
        ("close friend", Some(&close), [true, true, false]),
        ("circle member", Some(&member), [true, false, true]),
        ("owner", Some(&owner), [true, true, true]),
    ];
    for (name, user, expected) in viewers {
        let viewer = Viewer::load(&ctx.app.core, user.map(|user| user.id)).await.unwrap();
        let listed = grid(&ctx, user, &channel).await;
        for (short, expected) in shorts.iter().zip(expected) {
            let facts = ctx.row(&short.id).await.facts();
            assert_eq!(ctx.get(user, &short.id).await.is_ok(), expected, "{name} opening {}", short.caption);
            assert_eq!(access::can_watch(&viewer, &facts), expected, "{name}: the rule in code");
            assert_eq!(listed.contains(&short.caption), expected, "{name}: the grid agrees");
        }
    }
    let seen = ctx.get(Some(&member), &shorts[2].id).await.unwrap();
    assert_eq!(seen.visibility, buffa::EnumValue::from(VISIBILITY_FOLLOWERS), "members never learn about the circle");
    let own = ctx.get(Some(&owner), &shorts[2].id).await.unwrap();
    assert_eq!((own.visibility, own.studio.circle_id.clone()), (buffa::EnumValue::from(VISIBILITY_CIRCLE), circle));

    // Narrowing a published short names who may still watch it.
    ctx.set_visibility(&owner, &shorts[0].id, VISIBILITY_FRIENDS).await;
    let events = ctx.app.events_by("shorts.short.visibility_changed", owner.id).await;
    let last = events.last().expect("the narrowing is announced");
    assert_eq!(last.data["id"], shorts[0].id);
    assert_eq!(last.data["visibility"], 4);
    assert_eq!(last.data["visible_to"], serde_json::json!([friend.id.to_string()]));
    assert!(last.data["changed_at"].is_string());
}

/// What a viewer can tell about a locked account's channel from outside.
async fn what_one_learns(ctx: &Ctx, viewer: Option<&TestUser>, channel: &Channel, tag: &str) -> serde_json::Value {
    serde_json::json!({ "grid": grid(ctx, viewer, channel).await, "hashtag": hashtag(ctx, viewer, tag).await })
}

#[tokio::test]
async fn a_stranger_learns_nothing_from_a_locked_accounts_shorts() {
    let ctx = start().await;
    let stranger = ctx.app.user("stranger").await;
    let follower = ctx.app.user("follower").await;
    // Two worlds that differ only in what a locked account posted publicly.
    let (posted, silent) = (ctx.app.user("posted").await, ctx.app.user("silent").await);
    let mut channels = Vec::new();
    for world in [&posted, &silent] {
        ctx.follow(&follower, world).await;
        ctx.set_locked(world, true).await;
        channels.push(ctx.channel(world, "鍵チャンネル").await);
    }
    let (posted_tag, silent_tag) = (unique("locked"), unique("locked"));
    let public = ctx.short(&posted, &channels[0], &format!("鍵アカのショート #{posted_tag}")).await;
    let unlisted = ctx
        .short_with(&posted, &channels[0], &format!("リンク限定 #{posted_tag}"), |request| {
            request.visibility = Visibility::VISIBILITY_UNLISTED.into()
        })
        .await;
    assert_eq!(public.visibility, buffa::EnumValue::from(Visibility::VISIBILITY_PUBLIC), "still shared publicly");

    for viewer in [Some(&stranger), None] {
        let seen = what_one_learns(&ctx, viewer, &channels[0], &posted_tag).await;
        let other = what_one_learns(&ctx, viewer, &channels[1], &silent_tag).await;
        assert_eq!(seen, other, "the two worlds look the same from outside");
        for short in [&public, &unlisted] {
            assert_eq!(ctx.get(viewer, &short.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
        }
    }
    assert!(search_docs(&ctx, &public.id).await.iter().all(Option::is_none), "nothing in search");
    assert!(!ctx.row(&public.id).await.listed, "not counted anywhere");
    // Followers see them.
    assert_eq!(grid(&ctx, Some(&follower), &channels[0]).await.len(), 1, "the public one on the grid");
    assert!(ctx.get(Some(&follower), &unlisted.id).await.is_ok());
}

#[tokio::test]
async fn locking_an_account_applies_at_the_next_read_and_unlists_its_shorts() {
    let ctx = start().await;
    let owner = ctx.app.user("owner").await;
    let follower = ctx.app.user("follower").await;
    let stranger = ctx.app.user("stranger").await;
    ctx.follow(&follower, &owner).await;
    let channel = ctx.channel(&owner, "ch").await;
    let tag = unique("lock");
    let short = ctx.short(&owner, &channel, &format!("公開 #{tag}")).await;
    assert!(ctx.get(Some(&stranger), &short.id).await.is_ok());
    assert_eq!(hashtag(&ctx, None, &tag).await, serde_json::json!(["1", "1"]));

    ctx.set_locked(&owner, true).await;
    // Before this domain hears of the lock, reads ask the social domain.
    assert_eq!(ctx.get(Some(&stranger), &short.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    assert!(grid(&ctx, None, &channel).await.is_empty());
    assert_eq!(hashtag(&ctx, Some(&stranger), &tag).await[1], "0", "the page leaves it out at once");
    assert!(ctx.get(Some(&follower), &short.id).await.is_ok());

    ctx.deliver_lock_change(&owner).await;
    assert!(!ctx.row(&short.id).await.listed);
    assert_eq!(hashtag(&ctx, None, &tag).await[0], "HASHTAG_NOT_FOUND", "the hashtag no longer counts it");
    assert!(search_docs(&ctx, &short.id).await.last().unwrap().is_none(), "out of search");
    let events = ctx.app.events_by("shorts.short.visibility_changed", owner.id).await;
    let last = events.last().expect("the lock changes the short's reach");
    assert_eq!((last.data["id"].as_str(), last.data["visibility"].as_i64()), (Some(short.id.as_str()), Some(6)));
    assert_eq!(last.data["visible_to"], serde_json::json!([follower.id.to_string()]));
    ctx.deliver_lock_change(&owner).await;
    assert_eq!(ctx.app.events_by("shorts.short.visibility_changed", owner.id).await.len(), events.len(), "idempotent");

    ctx.set_locked(&owner, false).await;
    ctx.deliver_lock_change(&owner).await;
    assert!(ctx.get(Some(&stranger), &short.id).await.is_ok(), "unlocked: open again");
    assert_eq!(hashtag(&ctx, None, &tag).await, serde_json::json!(["1", "1"]));
    let last = ctx.app.events_by("shorts.short.visibility_changed", owner.id).await.pop().unwrap();
    assert!(last.data["visible_to"].is_null());
}

#[tokio::test]
async fn passes_of_one_lock_running_at_once_change_each_short_once() {
    let ctx = start().await;
    let owner = ctx.app.user("owner").await;
    let channel = ctx.channel(&owner, "ch").await;
    for caption in ["一本目", "二本目", "三本目"] {
        ctx.short(&owner, &channel, caption).await;
    }
    ctx.set_locked(&owner, true).await;
    let passes = futures::future::join_all((0..4).map(|_| ctx.shorts.sync_owner_lock(owner.id))).await;
    let changed: u64 = passes.into_iter().map(|pass| pass.expect("a pass")).sum();
    assert_eq!(changed, 3, "each short changes once, whichever pass gets it");
    assert_eq!(ctx.app.events_by("shorts.short.visibility_changed", owner.id).await.len(), 3);
}
