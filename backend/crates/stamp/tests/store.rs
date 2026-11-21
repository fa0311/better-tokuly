//! The store: shelves, ranking, search, tags, creator pages, and who gets to
//! see which pack.
mod common;

use common::*;
use tokuly_proto::{common::PageRequest, stamp::*};
use tokuly_stamp::{SendContext, record_send, stats};
use tokuly_testkit::TestUser;

async fn shelf(
    ctx: &Ctx,
    viewer: Option<&TestUser>,
    section: StoreSection,
    tag: &str,
    price: PriceFilter,
) -> Vec<String> {
    let request = ListStorePacksRequest {
        section: section.into(),
        tag: tag.to_owned(),
        price: price.into(),
        ..Default::default()
    };
    let response: ListStorePacksResponse = ctx.call("Store", "ListStorePacks", viewer, &request).await.unwrap();
    pack_names(&response.packs)
}

async fn search(ctx: &Ctx, query: &str) -> Vec<String> {
    let response: SearchPacksResponse = ctx
        .call("Store", "SearchPacks", None, &SearchPacksRequest { query: query.to_owned(), ..Default::default() })
        .await
        .unwrap();
    pack_names(&response.packs)
}

#[tokio::test]
async fn shelves_list_published_packs_newest_first() {
    let ctx = start().await;
    let creator = ctx.app.user("creator").await;
    let tag = unique("shelf");
    ctx.published(&creator, "ふるい", 0, &[&tag], 1).await;
    ctx.published(&creator, "ゆうりょう", 100, &[&tag], 1).await;
    ctx.published(&creator, "あたらしい", 0, &[&tag], 1).await;
    let draft = ctx.draft(&creator, "したがき", 0, &[&tag]).await;
    ctx.add_stamps(&creator, &id_of(&draft), "s", 1).await;
    ctx.submit(&creator, &id_of(&draft)).await; // in review: still not in the store

    let any = PriceFilter::PRICE_FILTER_UNSPECIFIED;
    assert_eq!(
        shelf(&ctx, None, StoreSection::STORE_SECTION_NEW, &tag, any).await,
        vec!["あたらしい", "ゆうりょう", "ふるい"]
    );
    assert_eq!(
        shelf(&ctx, None, StoreSection::STORE_SECTION_UNSPECIFIED, &tag, any).await,
        vec!["あたらしい", "ゆうりょう", "ふるい"]
    );
    assert_eq!(
        shelf(&ctx, None, StoreSection::STORE_SECTION_NEW, &tag, PriceFilter::PRICE_FILTER_FREE).await,
        vec!["あたらしい", "ふるい"]
    );
    assert_eq!(
        shelf(&ctx, None, StoreSection::STORE_SECTION_NEW, &tag, PriceFilter::PRICE_FILTER_PAID).await,
        vec!["ゆうりょう"]
    );
    assert!(shelf(&ctx, None, StoreSection::STORE_SECTION_OFFICIAL, &tag, any).await.is_empty());
    assert!(shelf(&ctx, None, StoreSection::STORE_SECTION_FEATURED, &tag, any).await.is_empty());
    // Tags are matched in their normalized form.
    assert_eq!(
        shelf(&ctx, None, StoreSection::STORE_SECTION_NEW, &format!("#{}", tag.to_uppercase()), any).await.len(),
        3
    );
    let bad_tag = ListStorePacksRequest { tag: "two words".into(), ..Default::default() };
    assert_eq!(ctx.reason("Store", "ListStorePacks", None, &bad_tag).await, "TAGS_INVALID");

    // Pages do not overlap and end cleanly.
    let page = |cursor: String| ListStorePacksRequest {
        tag: tag.clone(),
        page: Some(PageRequest { limit: 2, cursor, ..Default::default() }).into(),
        ..Default::default()
    };
    let first: ListStorePacksResponse = ctx.call("Store", "ListStorePacks", None, &page(String::new())).await.unwrap();
    assert_eq!(pack_names(&first.packs), vec!["あたらしい", "ゆうりょう"]);
    assert!(first.page.has_more);
    let second: ListStorePacksResponse =
        ctx.call("Store", "ListStorePacks", None, &page(first.page.next_cursor.clone())).await.unwrap();
    assert_eq!(pack_names(&second.packs), vec!["ふるい"]);
    assert!(!second.page.has_more && second.page.next_cursor.is_empty());
    assert_eq!(ctx.reason("Store", "ListStorePacks", None, &page("garbage".into())).await, "INVALID_CURSOR");

    // Cards carry what a shelf shows; anonymous callers get no viewer state.
    let card = &first.packs[0];
    assert_eq!((card.stamp_count, card.previews.len(), card.price), (1, 1, 0));
    assert_eq!(card.creator.id, creator.id.to_string());
    assert!(card.cover.url.ends_with("/512.webp") && card.published_at.is_set() && card.viewer.is_unset());
    let signed_in: ListStorePacksResponse =
        ctx.call("Store", "ListStorePacks", Some(&creator), &page(String::new())).await.unwrap();
    assert!(signed_in.packs[0].viewer.can_manage);
}

#[tokio::test]
async fn moderators_feature_packs_for_the_store_home() {
    let ctx = start().await;
    let (creator, moderator) = (ctx.app.user("creator").await, ctx.moderator().await);
    let tag = unique("feat");
    let first = id_of(&ctx.published(&creator, "おすすめ1", 0, &[&tag], 1).await);
    let second = id_of(&ctx.published(&creator, "おすすめ2", 0, &[&tag], 1).await);
    let draft = id_of(&ctx.draft(&creator, "したがき", 0, &[&tag]).await);
    let feature = |pack_id: &str, featured: bool| SetPackFeaturedRequest {
        pack_id: pack_id.to_owned(),
        featured,
        ..Default::default()
    };

    assert_eq!(ctx.reason("Review", "SetPackFeatured", Some(&creator), &feature(&first, true)).await, "FORBIDDEN");
    assert_eq!(ctx.reason("Review", "SetPackFeatured", None, &feature(&first, true)).await, "SIGN_IN_REQUIRED");
    assert_eq!(
        ctx.reason("Review", "SetPackFeatured", Some(&moderator), &feature(&draft, true)).await,
        "PACK_NOT_PUBLISHED"
    );
    assert_eq!(
        ctx.reason("Review", "SetPackFeatured", Some(&moderator), &feature(&uuid::Uuid::now_v7().to_string(), true))
            .await,
        "PACK_NOT_FOUND"
    );

    let featured: SetPackFeaturedResponse =
        ctx.call("Review", "SetPackFeatured", Some(&moderator), &feature(&first, true)).await.unwrap();
    assert!(featured.pack.featured);
    let _: SetPackFeaturedResponse =
        ctx.call("Review", "SetPackFeatured", Some(&moderator), &feature(&second, true)).await.unwrap();
    // Featuring again keeps the pack's place.
    let _: SetPackFeaturedResponse =
        ctx.call("Review", "SetPackFeatured", Some(&moderator), &feature(&first, true)).await.unwrap();
    let any = PriceFilter::PRICE_FILTER_UNSPECIFIED;
    assert_eq!(
        shelf(&ctx, None, StoreSection::STORE_SECTION_FEATURED, &tag, any).await,
        vec!["おすすめ2", "おすすめ1"]
    );

    let home: GetStoreHomeResponse =
        ctx.call("Store", "GetStoreHome", None, &GetStoreHomeRequest::default()).await.unwrap();
    assert_eq!(
        pack_names(&home.featured)[..2],
        ["おすすめ2", "おすすめ1"],
        "just featured, so first on the home shelf"
    );
    assert!(home.newest.len() >= 2 && home.newest.len() <= 12);
    assert!(home.featured.iter().all(|pack| pack.featured && pack.status == PackStatus::PACK_STATUS_PUBLISHED));

    let unfeatured: SetPackFeaturedResponse =
        ctx.call("Review", "SetPackFeatured", Some(&moderator), &feature(&second, false)).await.unwrap();
    assert!(!unfeatured.pack.featured);
    assert_eq!(shelf(&ctx, None, StoreSection::STORE_SECTION_FEATURED, &tag, any).await, vec!["おすすめ1"]);
    // A pack that leaves the store leaves the shelf, and does not come back featured.
    ctx.unpublish(&creator, &first).await;
    ctx.submit(&creator, &first).await;
    assert!(shelf(&ctx, None, StoreSection::STORE_SECTION_FEATURED, &tag, any).await.is_empty());
}

#[tokio::test]
async fn the_popular_shelf_ranks_by_recent_sends_installs_and_sales() {
    let ctx = start().await;
    let (creator, fan) = (ctx.app.user("creator").await, ctx.app.user("fan").await);
    let tag = unique("pop");
    let quiet = ctx.published(&creator, "しずか", 0, &[&tag], 1).await;
    let sent = ctx.published(&creator, "よくつかう", 0, &[&tag], 1).await;
    let installed = ctx.published(&creator, "インストール", 0, &[&tag], 1).await;
    let core = &ctx.app.core;

    for _ in 0..3 {
        record_send(core, fan.id, stamp_uuid(&sent, 0), SendContext::Chat).await.unwrap();
    }
    ctx.install(&fan, &id_of(&installed)).await;
    // Yesterday's activity counts for less, and last month's for nothing.
    sqlx::query("INSERT INTO stamp_pack_daily (pack_id, day, chat_sends) VALUES ($1, (now() AT TIME ZONE 'Asia/Tokyo')::date - 30, 1000)")
        .bind(uuid::Uuid::parse_str(&id_of(&quiet)).unwrap())
        .execute(&core.db)
        .await
        .unwrap();

    let any = PriceFilter::PRICE_FILTER_UNSPECIFIED;
    assert!(stats::refresh_rankings(core).await.unwrap() >= 2);
    // An install weighs 5 sends: 5 > 3 > 0.
    assert_eq!(
        shelf(&ctx, None, StoreSection::STORE_SECTION_POPULAR, &tag, any).await,
        vec!["インストール", "よくつかう", "しずか"]
    );
    let listed = ctx.get_pack(None, &id_of(&sent)).await.unwrap();
    assert_eq!(listed.summary.send_count, 3, "the send counter is refreshed along with the ranking");

    for _ in 0..4 {
        record_send(core, fan.id, stamp_uuid(&sent, 0), SendContext::LiveChat).await.unwrap();
    }
    stats::refresh_rankings(core).await.unwrap();
    assert_eq!(
        shelf(&ctx, None, StoreSection::STORE_SECTION_POPULAR, &tag, any).await,
        vec!["よくつかう", "インストール", "しずか"]
    );

    // Popular pages use the score as their key.
    let page = |cursor: String| ListStorePacksRequest {
        section: StoreSection::STORE_SECTION_POPULAR.into(),
        tag: tag.clone(),
        page: Some(PageRequest { limit: 1, cursor, ..Default::default() }).into(),
        ..Default::default()
    };
    let mut names = Vec::new();
    let mut cursor = String::new();
    loop {
        let response: ListStorePacksResponse = ctx.call("Store", "ListStorePacks", None, &page(cursor)).await.unwrap();
        names.extend(pack_names(&response.packs));
        if !response.page.has_more {
            break;
        }
        cursor = response.page.next_cursor.clone();
    }
    assert_eq!(names, vec!["よくつかう", "インストール", "しずか"]);

    // Activity that ages out of the window drops the score to zero.
    sqlx::query("UPDATE stamp_pack_daily SET day = day - 10 WHERE pack_id = $1")
        .bind(uuid::Uuid::parse_str(&id_of(&installed)).unwrap())
        .execute(&core.db)
        .await
        .unwrap();
    stats::refresh_rankings(core).await.unwrap();
    assert_eq!(shelf(&ctx, None, StoreSection::STORE_SECTION_POPULAR, &tag, any).await[0], "よくつかう");
    let score: f64 = sqlx::query_scalar("SELECT popular_score FROM stamp_packs WHERE id = $1")
        .bind(uuid::Uuid::parse_str(&id_of(&installed)).unwrap())
        .fetch_one(&core.db)
        .await
        .unwrap();
    assert_eq!(score, 0.0);
}

#[tokio::test]
async fn search_matches_names_descriptions_and_tags_of_published_packs() {
    let ctx = start().await;
    let creator = ctx.app.user("creator").await;
    let (word, tag) = (unique("さがす"), unique("tag"));
    ctx.published(&creator, &format!("{word}ねこ"), 0, &[&tag], 1).await;
    ctx.published(&creator, &format!("いぬ{word}"), 100, &[], 1).await;
    ctx.published(&creator, "なまえはべつ", 0, &[&tag, "other"], 1).await;
    ctx.draft(&creator, &format!("{word}したがき"), 0, &[&tag]).await;

    let mut by_name = search(&ctx, &word).await;
    by_name.sort();
    assert_eq!(
        by_name,
        vec![format!("いぬ{word}"), format!("{word}ねこ")],
        "substring match; drafts are not searchable"
    );
    // The best match comes first: the exact name, then names starting with
    // the query, before merely similar ones.
    assert_eq!(search(&ctx, &format!("{word}ねこ")).await[0], format!("{word}ねこ"));
    assert_eq!(search(&ctx, &format!("{word}ね")).await[0], format!("{word}ねこ"));
    assert_eq!(search(&ctx, &format!("いぬ{word}")).await[0], format!("いぬ{word}"));

    // A tag matches as a word, and exclusively with a leading '#'.
    let mut by_tag = search(&ctx, &tag).await;
    by_tag.sort();
    assert_eq!(by_tag, vec![format!("{word}ねこ"), "なまえはべつ".to_owned()]);
    assert_eq!(search(&ctx, &format!("#{tag}")).await.len(), 2);
    assert!(search(&ctx, &format!("#{word}")).await.is_empty(), "'#' searches tags only, not names");
    // Descriptions are searched too (the helper writes "<name> の説明").
    assert_eq!(search(&ctx, "なまえはべつ の説明").await, vec!["なまえはべつ"]);

    let paid_only =
        SearchPacksRequest { query: word.clone(), price: PriceFilter::PRICE_FILTER_PAID.into(), ..Default::default() };
    let paid: SearchPacksResponse = ctx.call("Store", "SearchPacks", None, &paid_only).await.unwrap();
    assert_eq!(pack_names(&paid.packs), vec![format!("いぬ{word}")]);

    let page = |cursor: String| SearchPacksRequest {
        query: word.clone(),
        page: Some(PageRequest { limit: 1, cursor, ..Default::default() }).into(),
        ..Default::default()
    };
    let first: SearchPacksResponse = ctx.call("Store", "SearchPacks", None, &page(String::new())).await.unwrap();
    assert!(first.page.has_more);
    let second: SearchPacksResponse =
        ctx.call("Store", "SearchPacks", None, &page(first.page.next_cursor.clone())).await.unwrap();
    assert!(!second.page.has_more);
    assert_ne!(first.packs[0].id, second.packs[0].id);

    assert!(search(&ctx, "   ").await.is_empty(), "an empty query finds nothing");
    assert!(search(&ctx, "100%_no_such_pack").await.is_empty(), "wildcards in the query are literal");
    let too_long = SearchPacksRequest { query: "q".repeat(65), ..Default::default() };
    assert_eq!(ctx.reason("Store", "SearchPacks", None, &too_long).await, "QUERY_TOO_LONG");
}

#[tokio::test]
async fn popular_tags_count_published_packs() {
    let ctx = start().await;
    let creator = ctx.app.user("creator").await;
    // Tags that sort to the very top, whatever other tests publish meanwhile.
    let (big, small) = (unique("tagbig"), unique("tagsmall"));
    for n in 0..3 {
        ctx.published(&creator, &format!("パック{n}"), 0, &[&big], 1).await;
    }
    ctx.published(&creator, "もうひとつ", 0, &[&big, &small], 1).await;
    ctx.draft(&creator, "したがき", 0, &[&small]).await;

    let tags: ListPopularTagsResponse = ctx
        .call("Store", "ListPopularTags", None, &ListPopularTagsRequest { limit: 100, ..Default::default() })
        .await
        .unwrap();
    let count = |tag: &str| tags.tags.iter().find(|t| t.tag == tag).map(|t| t.pack_count);
    assert_eq!(count(&big), Some(4));
    assert_eq!(count(&small), Some(1), "drafts do not count");
    assert!(tags.tags.windows(2).all(|pair| pair[0].pack_count >= pair[1].pack_count), "most used first");
    let position = |tag: &str| tags.tags.iter().position(|t| t.tag == tag).unwrap();
    assert!(position(&big) < position(&small));

    let one: ListPopularTagsResponse = ctx
        .call("Store", "ListPopularTags", None, &ListPopularTagsRequest { limit: 1, ..Default::default() })
        .await
        .unwrap();
    assert_eq!(one.tags.len(), 1);
    let home: GetStoreHomeResponse =
        ctx.call("Store", "GetStoreHome", None, &GetStoreHomeRequest::default()).await.unwrap();
    assert!(!home.tags.is_empty() && home.tags.len() <= 20);
}

#[tokio::test]
async fn creator_pages_list_their_published_packs() {
    let ctx = start().await;
    let (creator, visitor) = (ctx.app.user("creator").await, ctx.app.user("visitor").await);
    ctx.published(&creator, "いち", 0, &[], 1).await;
    ctx.published(&creator, "に", 50, &[], 1).await;
    ctx.draft(&creator, "したがき", 0, &[]).await;
    ctx.published(&visitor, "ほかの人の", 0, &[], 1).await;

    let request = ListCreatorPacksRequest { user_id: creator.id.to_string(), ..Default::default() };
    let page: ListCreatorPacksResponse = ctx.call("Store", "ListCreatorPacks", Some(&visitor), &request).await.unwrap();
    assert_eq!(pack_names(&page.packs), vec!["に", "いち"]);
    assert_eq!(page.creator.handle, creator.handle);
    let anonymous: ListCreatorPacksResponse = ctx.call("Store", "ListCreatorPacks", None, &request).await.unwrap();
    assert_eq!(anonymous.packs.len(), 2);

    let paged = ListCreatorPacksRequest {
        page: Some(PageRequest { limit: 1, ..Default::default() }).into(),
        ..request.clone()
    };
    let first: ListCreatorPacksResponse = ctx.call("Store", "ListCreatorPacks", None, &paged).await.unwrap();
    assert!(first.page.has_more);
    let next = ListCreatorPacksRequest {
        page: Some(PageRequest { limit: 1, cursor: first.page.next_cursor.clone(), ..Default::default() }).into(),
        ..request.clone()
    };
    let second: ListCreatorPacksResponse = ctx.call("Store", "ListCreatorPacks", None, &next).await.unwrap();
    assert_eq!(pack_names(&second.packs), vec!["いち"]);

    let unknown = ListCreatorPacksRequest { user_id: uuid::Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.reason("Store", "ListCreatorPacks", None, &unknown).await, "USER_NOT_FOUND");
    sqlx::query("UPDATE users SET banned_at = now() WHERE id = $1")
        .bind(creator.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(
        ctx.reason("Store", "ListCreatorPacks", None, &request).await,
        "USER_NOT_FOUND",
        "suspended accounts have no page"
    );
}

#[tokio::test]
async fn pack_pages_show_each_viewer_what_they_may_see() {
    let ctx = start().await;
    let (creator, holder, stranger, moderator) = (
        ctx.app.user("creator").await,
        ctx.app.user("holder").await,
        ctx.app.user("stranger").await,
        ctx.moderator().await,
    );
    let pack = ctx.published(&creator, "みんなの", 0, &["みる"], 7).await;
    let id = id_of(&pack);

    // In the store: public, by UUID or short ID, with at most five previews.
    let public = ctx.get_pack(None, &pack.summary.short_id).await.unwrap();
    assert_eq!(public.summary.id, id);
    assert_eq!((public.stamps.len(), public.summary.previews.len()), (7, 5));
    assert_eq!(public.description, "みんなの の説明");
    assert!(public.summary.viewer.is_unset() && public.review.is_unset());
    assert!(public.stamps.iter().all(|s| !s.usable && !s.pending_review), "anonymous viewers cannot send anything");
    let seen: Pack = ctx.get_pack(Some(&stranger), &id).await.unwrap();
    let viewer = seen.summary.viewer.as_option().unwrap();
    assert!(
        viewer.usable && !viewer.installed && !viewer.owned && !viewer.can_manage,
        "free packs are usable without installing"
    );
    assert!(seen.stamps.iter().all(|s| s.usable));

    // Out of the store: only for those who hold it, manage it or moderate.
    ctx.install(&holder, &id).await;
    ctx.unpublish(&creator, &id).await;
    assert_eq!(ctx.get_pack(Some(&stranger), &id).await.unwrap_err().reason, "PACK_NOT_FOUND");
    assert_eq!(ctx.get_pack(None, &id).await.unwrap_err().reason, "PACK_NOT_FOUND");
    let held = ctx.get_pack(Some(&holder), &id).await.unwrap();
    assert_eq!(held.summary.status, PackStatus::PACK_STATUS_DRAFT);
    assert!(held.summary.viewer.usable && held.stamps.iter().all(|s| s.usable));
    let moderated = ctx.get_pack(Some(&moderator), &id).await.unwrap();
    assert!(moderated.review.is_set() && !moderated.summary.viewer.can_manage);
    assert!(ctx.get_pack(Some(&creator), &id).await.unwrap().review.is_set());

    assert_eq!(ctx.get_pack(None, "nonexistent1").await.unwrap_err().reason, "PACK_NOT_FOUND");
    assert_eq!(ctx.get_pack(None, "?!").await.unwrap_err().reason, "INVALID_ID");
}

#[tokio::test]
async fn a_tapped_stamp_shows_itself_and_its_pack() {
    let ctx = start().await;
    let (creator, viewer) = (ctx.app.user("creator").await, ctx.app.user("viewer").await);
    let pack = ctx.published(&creator, "タップ", 80, &[], 2).await;
    let get = |stamp_id: &str| GetStampRequest { stamp_id: stamp_id.to_owned(), ..Default::default() };
    let stamp_id = pack.stamps[1].stamp_ref.id.clone();

    let anonymous: GetStampResponse = ctx.call("Store", "GetStamp", None, &get(&stamp_id)).await.unwrap();
    assert_eq!(anonymous.stamp.stamp_ref.id, stamp_id);
    assert_eq!((anonymous.pack.id.clone(), anonymous.pack.price), (id_of(&pack), 80));
    assert!(!anonymous.can_save && !anonymous.stamp.usable);
    let seen: GetStampResponse = ctx.call("Store", "GetStamp", Some(&viewer), &get(&stamp_id)).await.unwrap();
    assert!(!seen.stamp.usable && !seen.can_save && !seen.pack.viewer.owned, "paid and not bought");
    let own: GetStampResponse = ctx.call("Store", "GetStamp", Some(&creator), &get(&stamp_id)).await.unwrap();
    assert!(own.stamp.usable && own.pack.viewer.can_manage);

    // A stamp of a pack that left the store still shows, without the pack.
    ctx.unpublish(&creator, &id_of(&pack)).await;
    let orphan: GetStampResponse = ctx.call("Store", "GetStamp", Some(&viewer), &get(&stamp_id)).await.unwrap();
    assert!(orphan.pack.is_unset() && !orphan.can_save);

    let personal = ctx.personal(&creator, "じぶんの").await;
    let theirs: GetStampResponse = ctx.call("Store", "GetStamp", None, &get(&personal.stamp_ref.id)).await.unwrap();
    assert!(theirs.stamp.personal && theirs.pack.is_unset() && !theirs.can_save);

    assert_eq!(ctx.reason("Store", "GetStamp", None, &get(&uuid::Uuid::now_v7().to_string())).await, "STAMP_NOT_FOUND");
    assert_eq!(ctx.reason("Store", "GetStamp", None, &get("x")).await, "INVALID_ID");
}
