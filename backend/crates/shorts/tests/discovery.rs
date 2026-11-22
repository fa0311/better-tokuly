//! `ShortDiscoveryService`: hashtag pages, trending hashtags, sound pages
//! and the creator grid; and what a block does and does not give away.
mod common;

use common::*;
use tokuly_proto::{common::Visibility, shorts::*};
use tokuly_testkit::TestUser;

async fn hashtag(ctx: &Ctx, viewer: Option<&TestUser>, tag: &str) -> Result<Hashtag, tokuly_testkit::RpcError> {
    let request = GetHashtagRequest { tag: tag.to_owned(), ..Default::default() };
    let response: GetHashtagResponse = ctx.call(DISCOVERY, "GetHashtag", viewer, &request).await?;
    Ok(response.hashtag.as_option().cloned().expect("the hashtag"))
}

async fn tagged(
    ctx: &Ctx,
    viewer: Option<&TestUser>,
    tag: &str,
    sort: HashtagSort,
    limit: i32,
    cursor: &str,
) -> ListHashtagShortsResponse {
    let request = ListHashtagShortsRequest {
        tag: tag.to_owned(),
        sort: sort.into(),
        page: page(limit, cursor),
        ..Default::default()
    };
    ctx.call(DISCOVERY, "ListHashtagShorts", viewer, &request).await.expect("hashtag page")
}

async fn trending(ctx: &Ctx) -> Vec<Hashtag> {
    let request = ListTrendingHashtagsRequest { limit: 50, ..Default::default() };
    let response: ListTrendingHashtagsResponse =
        ctx.call(DISCOVERY, "ListTrendingHashtags", None, &request).await.unwrap();
    response.hashtags
}

/// Gives a short engagement in the current hour, as the flush would.
async fn engagement(ctx: &Ctx, short_id: &str, views: i64) {
    sqlx::query(
        "INSERT INTO short_hourly_stats (short_id, hour, views) VALUES ($1, date_trunc('hour', now()), $2)
         ON CONFLICT (short_id, hour) DO UPDATE SET views = short_hourly_stats.views + EXCLUDED.views",
    )
    .bind(id(short_id))
    .bind(views)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    sqlx::query("UPDATE shorts SET view_count = view_count + $2 WHERE id = $1")
        .bind(id(short_id))
        .bind(views)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn hashtag_pages_list_public_shorts_by_rank_or_by_age() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let a = ctx.channel(&alice, "a").await;
    let b = ctx.channel(&bob, "b").await;
    let c = ctx.channel(&carol, "c").await;
    let tag = unique("お祭り");
    let first = ctx.short(&alice, &a, &format!("一番目 #{tag}")).await;
    let second = ctx.short(&bob, &b, &format!("二番目 ＃{}", tag.to_uppercase())).await;
    ctx.short(&carol, &c, &format!("三番目 #{tag} #ほかのタグ")).await;
    ctx.short_with(&alice, &a, &format!("フレンド限定 #{tag}"), |request| {
        request.visibility = Visibility::VISIBILITY_FRIENDS.into()
    })
    .await;

    // Named with or without the sign, in any case and width.
    for name in [tag.clone(), format!("#{tag}"), format!("＃{}", tag.to_uppercase())] {
        let found = hashtag(&ctx, None, &name).await.unwrap();
        assert_eq!((found.tag.as_str(), found.short_count), (tag.as_str(), 3));
    }
    let recent = tagged(&ctx, None, &tag, HashtagSort::HASHTAG_SORT_RECENT, 0, "").await;
    assert_eq!(captions(&recent.shorts).iter().map(|c| &c[..9]).collect::<Vec<_>>(), ["三番目", "二番目", "一番目"]);

    // RECENT pages hold while shorts are posted.
    let page_one = tagged(&ctx, None, &tag, HashtagSort::HASHTAG_SORT_RECENT, 2, "").await;
    assert_eq!(page_one.shorts.len(), 2);
    let cursor = page_one.page.as_option().unwrap().next_cursor.clone();
    ctx.short(&bob, &b, &format!("四番目 #{tag}")).await;
    let page_two = tagged(&ctx, None, &tag, HashtagSort::HASHTAG_SORT_RECENT, 2, &cursor).await;
    assert_eq!(page_two.shorts.iter().map(|short| short.id.as_str()).collect::<Vec<_>>(), [first.id.as_str()]);
    assert!(!page_two.page.as_option().unwrap().has_more);

    // TOP follows the ranking: the short people watch comes first.
    engagement(&ctx, &second.id, 500).await;
    ctx.shorts.run_ranking().await.unwrap();
    let top = tagged(&ctx, None, &tag, HashtagSort::HASHTAG_SORT_TOP, 2, "").await;
    assert_eq!(top.shorts[0].id, second.id);
    let rest =
        tagged(&ctx, None, &tag, HashtagSort::HASHTAG_SORT_UNSPECIFIED, 2, &top.page.as_option().unwrap().next_cursor)
            .await;
    let mut all: Vec<String> = top.shorts.iter().chain(&rest.shorts).map(|short| short.id.clone()).collect();
    assert_eq!(all.len(), 4, "every short once across the pages");
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 4);
    assert_eq!(hashtag(&ctx, None, &tag).await.unwrap().view_count, 500);

    // Completion while typing a caption.
    let typed = SuggestHashtagsRequest { prefix: format!("#{}", &tag[..tag.len() - 4]), ..Default::default() };
    let suggested: SuggestHashtagsResponse =
        ctx.call(DISCOVERY, "SuggestHashtags", Some(&alice), &typed).await.unwrap();
    assert!(suggested.hashtags.iter().any(|found| found.tag == tag && found.short_count == 4));
    let nothing = SuggestHashtagsRequest { prefix: "% _".into(), ..Default::default() };
    let suggested: SuggestHashtagsResponse = ctx.call(DISCOVERY, "SuggestHashtags", None, &nothing).await.unwrap();
    assert!(suggested.hashtags.is_empty());

    // What is not a hashtag, and what nobody uses.
    assert_eq!(hashtag(&ctx, None, "two words").await.unwrap_err().reason, "HASHTAG_INVALID");
    assert_eq!(hashtag(&ctx, None, &unique("誰も使わない")).await.unwrap_err().reason, "HASHTAG_NOT_FOUND");
    assert!(tagged(&ctx, None, &unique("誰も使わない"), HashtagSort::HASHTAG_SORT_TOP, 0, "").await.shorts.is_empty());
    let bad = ListHashtagShortsRequest { tag: tag.clone(), page: page(2, "garbage"), ..Default::default() };
    assert_eq!(ctx.reason(DISCOVERY, "ListHashtagShorts", None, &bad).await, "INVALID_CURSOR");
    let bad = ListHashtagShortsRequest { tag: "two words".into(), ..Default::default() };
    assert_eq!(ctx.reason(DISCOVERY, "ListHashtagShorts", None, &bad).await, "HASHTAG_INVALID");
}

#[tokio::test]
async fn a_block_shows_in_no_count() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let vera = ctx.app.user("vera").await;
    let a = ctx.channel(&alice, "a").await;
    let b = ctx.channel(&bob, "b").await;
    let (shared, hers) = (unique("みんなの"), unique("ありすだけの"));
    let original = ctx.short(&alice, &a, &format!("元 #{shared} #{hers}")).await;
    engagement(&ctx, &original.id, 40).await;
    let sound_id = original.sound.as_option().unwrap().id.clone();
    let remix_of = |short_id: &str| RemixInput {
        short_id: short_id.to_owned(),
        kind: RemixKind::REMIX_KIND_REMIX.into(),
        ..Default::default()
    };
    let bobs = ctx.short(&bob, &b, &format!("ボブの #{shared}")).await;
    let alices_remix_of_bobs = ctx
        .short_with(&alice, &a, "ありすのリミックス", |request| {
            request.remix = buffa::MessageField::some(remix_of(&bobs.id))
        })
        .await;
    let bobs_use = ctx
        .short_with(&bob, &b, "ボブが音を使う", |request| {
            let alices_sound =
                SoundInput { sound: Some(sound_input::Sound::SoundId(sound_id.clone())), ..Default::default() };
            request.sound = buffa::MessageField::some(alices_sound);
            request.remix = buffa::MessageField::some(remix_of(&original.id));
        })
        .await;
    ctx.shorts.run_ranking().await.unwrap();

    let sound = |viewer: Option<&TestUser>| {
        let ctx = &ctx;
        let sound_id = sound_id.clone();
        let viewer = viewer.cloned();
        async move {
            let request = GetSoundRequest { sound_id, ..Default::default() };
            let response: GetSoundResponse = ctx.call(DISCOVERY, "GetSound", viewer.as_ref(), &request).await.unwrap();
            response.sound.as_option().cloned().unwrap()
        }
    };
    // Before: vera sees what everyone sees.
    assert_eq!(hashtag(&ctx, Some(&vera), &shared).await.unwrap().short_count, 2);
    assert_eq!(hashtag(&ctx, Some(&vera), &hers).await.unwrap().short_count, 1);
    assert_eq!(
        (sound(Some(&vera)).await.use_count, sound(Some(&vera)).await.origin_short_id.as_str()),
        (2, original.short_id.as_str())
    );
    assert_eq!(ctx.get(Some(&vera), &bobs.id).await.unwrap().remix_count, 1);

    // Alice blocks vera. For vera, alice's shorts are gone, and every number
    // agrees with the lists she can still get: nothing says "hidden from you".
    ctx.block(&alice, &vera).await;
    assert_eq!(ctx.get(Some(&vera), &original.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    let tag_for_vera = hashtag(&ctx, Some(&vera), &shared).await.unwrap();
    let page_for_vera = tagged(&ctx, Some(&vera), &shared, HashtagSort::HASHTAG_SORT_RECENT, 0, "").await;
    assert_eq!((tag_for_vera.short_count, page_for_vera.shorts.len()), (1, 1));
    assert_eq!(tag_for_vera.view_count, 0, "the views of the hidden short are not hers to add up");
    // A hashtag only alice used does not exist for vera, as if alice had deleted the short.
    assert_eq!(hashtag(&ctx, Some(&vera), &hers).await.unwrap_err().reason, "HASHTAG_NOT_FOUND");
    let typed = SuggestHashtagsRequest { prefix: hers.clone(), ..Default::default() };
    let suggested: SuggestHashtagsResponse = ctx.call(DISCOVERY, "SuggestHashtags", Some(&vera), &typed).await.unwrap();
    assert!(suggested.hashtags.is_empty());
    // The sound is still a page (bob's short uses it), without its origin.
    let for_vera = sound(Some(&vera)).await;
    assert_eq!((for_vera.use_count, for_vera.origin_short_id.as_str(), for_vera.reusable), (1, "", false));
    assert!(for_vera.channel.as_option().is_none() && for_vera.cover.as_option().is_none());
    let list = ListSoundShortsRequest { sound_id: sound_id.clone(), ..Default::default() };
    let using: ListSoundShortsResponse = ctx.call(DISCOVERY, "ListSoundShorts", Some(&vera), &list).await.unwrap();
    assert_eq!(captions(&using.shorts), ["ボブが音を使う"]);
    // On bob's shorts: the sound and the remix source without alice, the remix count without hers.
    let seen = ctx.get(Some(&vera), &bobs_use.id).await.unwrap();
    assert_eq!(seen.sound.as_option().unwrap().use_count, 1);
    assert!(seen.remix_of.as_option().unwrap().short_id.is_empty());
    assert_eq!(ctx.get(Some(&vera), &bobs.id).await.unwrap().remix_count, 0);
    let remixes = ListRemixesRequest { short_id: bobs.id.clone(), ..Default::default() };
    let listed: ListRemixesResponse = ctx.call(SHORTS, "ListRemixes", Some(&vera), &remixes).await.unwrap();
    assert!(listed.shorts.is_empty());

    // Everyone else is unaffected.
    assert_eq!(hashtag(&ctx, Some(&bob), &shared).await.unwrap().short_count, 2);
    assert_eq!(hashtag(&ctx, None, &shared).await.unwrap().view_count, 40);
    assert_eq!(sound(None).await.use_count, 2);
    assert_eq!(ctx.get(Some(&bob), &bobs.id).await.unwrap().remix_count, 1);

    // Had alice deleted her shorts instead, vera would see exactly the same.
    let blocked_view =
        (tag_for_vera.short_count, for_vera.use_count, for_vera.origin_short_id.clone(), for_vera.reusable);
    ctx.delete(&alice, &original.id).await;
    ctx.delete(&alice, &alices_remix_of_bobs.id).await;
    let after = sound(None).await;
    let deleted_view = (
        hashtag(&ctx, None, &shared).await.unwrap().short_count,
        after.use_count,
        after.origin_short_id.clone(),
        after.reusable,
    );
    assert_eq!(blocked_view, deleted_view);
    assert_eq!(hashtag(&ctx, None, &hers).await.unwrap_err().reason, "HASHTAG_NOT_FOUND");
}

#[tokio::test]
async fn nobody_trends_a_hashtag_alone() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let a = ctx.channel(&alice, "a").await;
    let b = ctx.channel(&bob, "b").await;
    let (together, alone) = (unique("流行"), unique("ひとりで"));
    let one = ctx.short(&alice, &a, &format!("#{together}")).await;
    let two = ctx.short(&bob, &b, &format!("#{together}")).await;
    let solo = ctx.short(&alice, &a, &format!("#{alone}")).await;
    let solo_again = ctx.short(&alice, &a, &format!("もう一本 #{alone}")).await;
    engagement(&ctx, &one.id, 30).await;
    engagement(&ctx, &two.id, 20).await;
    engagement(&ctx, &solo.id, 10_000).await;
    engagement(&ctx, &solo_again.id, 10_000).await;
    ctx.shorts.run_ranking().await.unwrap();

    let now_trending = trending(&ctx).await;
    let found = now_trending.iter().find(|tag| tag.tag == together).expect("two channels make a trend");
    assert!(found.trending_rank >= 1 && found.short_count == 2 && found.view_count == 50);
    assert!(now_trending.iter().all(|tag| tag.tag != alone), "one channel, however loud, does not");
    assert!(now_trending.windows(2).all(|pair| pair[0].trending_rank < pair[1].trending_rank), "strongest first");
    assert_eq!(hashtag(&ctx, None, &alone).await.unwrap().trending_rank, 0);
    assert_eq!(hashtag(&ctx, None, &together).await.unwrap().trending_rank, found.trending_rank);

    // The hashtag is searchable with its count...
    let documents = |tag: String| {
        let ctx = &ctx;
        async move {
            ctx.app
                .events("search.changed")
                .await
                .into_iter()
                .filter(|event| event.data["kind"] == "hashtag" && event.data["doc"]["title"] == format!("#{tag}"))
                .map(|event| event.data["doc"]["score"].as_f64().unwrap_or(-1.0))
                .collect::<Vec<f64>>()
        }
    };
    // (Other tests run the ranking worker too, so a document may have been
    // written while the hashtag had one short; the latest is what counts.)
    let written = documents(together.clone()).await;
    assert_eq!(written.last(), Some(&2.0), "{written:?}");
    // ...which follows its use, and is not rewritten while nothing changes.
    ctx.short(&alice, &a, &format!("三本目 #{together}")).await;
    ctx.shorts.run_ranking().await.unwrap();
    ctx.shorts.run_ranking().await.unwrap();
    let written = documents(together.clone()).await;
    assert_eq!(written.last(), Some(&3.0), "{written:?}");
    assert!(written.windows(2).all(|pair| pair[0] < pair[1]), "one document per count: {written:?}");

    // When the shorts go quiet the trend ends; when they are gone, so is the hashtag.
    sqlx::query("DELETE FROM short_hourly_stats WHERE short_id = ANY($1)")
        .bind(vec![id(&one.id), id(&two.id)])
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.shorts.run_ranking().await.unwrap();
    assert!(trending(&ctx).await.iter().all(|tag| tag.tag != together));
    assert_eq!(hashtag(&ctx, None, &together).await.unwrap().trending_rank, 0);
    ctx.delete(&alice, &solo.id).await;
    ctx.delete(&alice, &solo_again.id).await;
    ctx.shorts.run_ranking().await.unwrap();
    let removed = ctx
        .app
        .events("search.changed")
        .await
        .into_iter()
        .filter(|event| event.data["kind"] == "hashtag" && event.data["doc"].is_null())
        .count();
    assert!(removed >= 1, "a hashtag nothing public carries leaves the index");
}

#[tokio::test]
async fn the_grid_can_be_sorted_by_views() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let quiet = ctx.short(&alice, &channel, "あまり見られない").await;
    let popular = ctx.short(&alice, &channel, "よく見られる").await;
    let middling = ctx.short(&alice, &channel, "そこそこ").await;
    engagement(&ctx, &popular.id, 900).await;
    engagement(&ctx, &middling.id, 90).await;
    engagement(&ctx, &quiet.id, 9).await;

    let grid = |sort: ChannelShortSort, limit: i32, cursor: String| {
        let ctx = &ctx;
        let bob = &bob;
        let channel_id = format!("@{}", channel.summary.handle);
        async move {
            let request = ListChannelShortsRequest {
                channel_id,
                sort: sort.into(),
                page: page(limit, &cursor),
                ..Default::default()
            };
            ctx.call::<_, ListChannelShortsResponse>(DISCOVERY, "ListChannelShorts", Some(bob), &request).await
        }
    };
    let latest = grid(ChannelShortSort::CHANNEL_SHORT_SORT_UNSPECIFIED, 0, String::new()).await.unwrap();
    assert_eq!(captions(&latest.shorts), ["そこそこ", "よく見られる", "あまり見られない"]);
    let by_views = grid(ChannelShortSort::CHANNEL_SHORT_SORT_POPULAR, 2, String::new()).await.unwrap();
    assert_eq!(captions(&by_views.shorts), ["よく見られる", "そこそこ"]);
    let cursor = by_views.page.as_option().unwrap().next_cursor.clone();
    let rest = grid(ChannelShortSort::CHANNEL_SHORT_SORT_POPULAR, 2, cursor).await.unwrap();
    assert_eq!(captions(&rest.shorts), ["あまり見られない"]);
    assert_eq!(
        grid(ChannelShortSort::CHANNEL_SHORT_SORT_LATEST, 2, "garbage".into()).await.unwrap_err().reason,
        "INVALID_CURSOR"
    );
    let nowhere = ListChannelShortsRequest { channel_id: unique("nochannel"), ..Default::default() };
    assert_eq!(ctx.reason(DISCOVERY, "ListChannelShorts", None, &nowhere).await, "CHANNEL_NOT_FOUND");
}
