//! The user's library: installed packs, favourites, recently used stamps,
//! the usage log and the picker search.
mod common;

use common::*;
use tokuly_proto::{common::PageRequest, stamp::*};
use tokuly_stamp::{SendContext, record_send};

fn pack_ref(pack_id: &str) -> InstallPackRequest {
    InstallPackRequest { pack_id: pack_id.to_owned(), ..Default::default() }
}

async fn installed(ctx: &Ctx, user: &tokuly_testkit::TestUser) -> Vec<PackSummary> {
    let response: ListInstalledPacksResponse =
        ctx.call("Library", "ListInstalledPacks", Some(user), &ListInstalledPacksRequest::default()).await.unwrap();
    response.packs
}

async fn recents(ctx: &Ctx, user: &tokuly_testkit::TestUser, order: RecentOrder) -> Vec<String> {
    let request = ListRecentStampsRequest { order: order.into(), ..Default::default() };
    let response: ListRecentStampsResponse =
        ctx.call("Library", "ListRecentStamps", Some(user), &request).await.unwrap();
    stamp_names(&response.stamps)
}

#[tokio::test]
async fn packs_are_installed_ordered_and_uninstalled() {
    let ctx = start().await;
    let (creator, user) = (ctx.app.user("creator").await, ctx.app.user("user").await);
    let a = id_of(&ctx.published(&creator, "A", 0, &[], 1).await);
    let b = id_of(&ctx.published(&creator, "B", 0, &[], 1).await);
    let c = id_of(&ctx.published(&creator, "C", 0, &[], 1).await);

    let summary = ctx.install(&user, &a).await;
    assert!(summary.viewer.installed && summary.viewer.usable && !summary.viewer.owned);
    assert_eq!(summary.install_count, 1);
    assert_eq!(ctx.install(&user, &a).await.install_count, 1, "installing twice counts once");
    ctx.install(&user, &b).await;
    ctx.install(&user, &c).await;
    assert_eq!(pack_names(&installed(&ctx, &user).await), vec!["C", "B", "A"], "new installs go first");
    assert_eq!(ctx.app.events_by("stamp.pack.installed", user.id).await.len(), 3);

    // Listed packs move to the front; the rest keep their order.
    let reorder = ReorderInstalledPacksRequest { pack_ids: vec![a.clone(), c.clone()], ..Default::default() };
    let _: ReorderInstalledPacksResponse =
        ctx.call("Library", "ReorderInstalledPacks", Some(&user), &reorder).await.unwrap();
    assert_eq!(pack_names(&installed(&ctx, &user).await), vec!["A", "C", "B"]);
    let unknown = ReorderInstalledPacksRequest {
        pack_ids: vec![a.clone(), uuid::Uuid::now_v7().to_string()],
        ..Default::default()
    };
    assert_eq!(ctx.reason("Library", "ReorderInstalledPacks", Some(&user), &unknown).await, "ORDER_MISMATCH");
    let twice = ReorderInstalledPacksRequest { pack_ids: vec![a.clone(), a.clone()], ..Default::default() };
    assert_eq!(ctx.reason("Library", "ReorderInstalledPacks", Some(&user), &twice).await, "ORDER_MISMATCH");

    // Pages follow the user's order.
    let first_page = ListInstalledPacksRequest {
        page: Some(PageRequest { limit: 2, ..Default::default() }).into(),
        ..Default::default()
    };
    let first: ListInstalledPacksResponse =
        ctx.call("Library", "ListInstalledPacks", Some(&user), &first_page).await.unwrap();
    assert_eq!(pack_names(&first.packs), vec!["A", "C"]);
    assert!(first.page.has_more);
    let second_page = ListInstalledPacksRequest {
        page: Some(PageRequest { limit: 2, cursor: first.page.next_cursor.clone(), ..Default::default() }).into(),
        ..Default::default()
    };
    let second: ListInstalledPacksResponse =
        ctx.call("Library", "ListInstalledPacks", Some(&user), &second_page).await.unwrap();
    assert_eq!(pack_names(&second.packs), vec!["B"]);
    assert!(!second.page.has_more);

    ctx.uninstall(&user, &c).await;
    ctx.uninstall(&user, &c).await;
    assert_eq!(pack_names(&installed(&ctx, &user).await), vec!["A", "B"]);
    assert_eq!(ctx.get_pack(Some(&user), &c).await.unwrap().summary.install_count, 0);
    assert_eq!(ctx.app.events_by("stamp.pack.uninstalled", user.id).await.len(), 1);
    // Installing and uninstalling in a loop does not add up in the day's numbers (they feed the ranking).
    ctx.install(&user, &c).await;
    ctx.uninstall(&user, &c).await;
    ctx.install(&user, &c).await;
    let installs_today: i64 = sqlx::query_scalar("SELECT installs FROM stamp_pack_daily WHERE pack_id = $1")
        .bind(uuid::Uuid::parse_str(&c).unwrap())
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(installs_today, 1);
    ctx.uninstall(&user, &c).await;

    assert_eq!(ctx.reason("Library", "InstallPack", None, &pack_ref(&a)).await, "SIGN_IN_REQUIRED");
    assert_eq!(
        ctx.reason("Library", "ListInstalledPacks", None, &ListInstalledPacksRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
}

#[tokio::test]
async fn only_packs_in_the_store_can_be_installed_for_free() {
    let ctx = start().await;
    let (creator, user) = (ctx.app.user("creator").await, ctx.app.user("user").await);
    let paid = id_of(&ctx.published(&creator, "ゆうりょう", 150, &[], 1).await);
    assert_eq!(ctx.reason("Library", "InstallPack", Some(&user), &pack_ref(&paid)).await, "PURCHASE_REQUIRED");

    let draft = id_of(&ctx.draft(&creator, "したがき", 0, &[]).await);
    assert_eq!(ctx.reason("Library", "InstallPack", Some(&user), &pack_ref(&draft)).await, "PACK_NOT_FOUND");
    assert_eq!(
        ctx.reason("Library", "InstallPack", Some(&user), &pack_ref(&uuid::Uuid::now_v7().to_string())).await,
        "PACK_NOT_FOUND"
    );
    assert_eq!(ctx.reason("Library", "InstallPack", Some(&user), &pack_ref("not an id")).await, "INVALID_ID");

    // The creator can put their own paid pack or draft back into the picker.
    ctx.uninstall(&creator, &draft).await;
    assert!(ctx.install(&creator, &draft).await.viewer.installed);
    assert_eq!(ctx.get_pack(Some(&creator), &draft).await.unwrap().summary.install_count, 0);
}

#[tokio::test]
async fn favourites_are_shortcuts_to_stamps_you_can_send() {
    let ctx = start().await;
    let (creator, user) = (ctx.app.user("creator").await, ctx.app.user("user").await);
    let free = ctx.published(&creator, "むりょう", 0, &[], 3).await;
    let paid = ctx.published(&creator, "ゆうりょう", 100, &[], 1).await;
    let favorite = |pack: &Pack, index: usize| FavoriteStampRequest {
        stamp_id: pack.stamps[index].stamp_ref.id.clone(),
        ..Default::default()
    };

    for index in [0, 2] {
        let _: FavoriteStampResponse =
            ctx.call("Library", "FavoriteStamp", Some(&user), &favorite(&free, index)).await.unwrap();
    }
    let _: FavoriteStampResponse =
        ctx.call("Library", "FavoriteStamp", Some(&user), &favorite(&free, 0)).await.unwrap();
    let list: ListFavoriteStampsResponse =
        ctx.call("Library", "ListFavoriteStamps", Some(&user), &ListFavoriteStampsRequest::default()).await.unwrap();
    assert_eq!(stamp_names(&list.stamps), vec!["スタンプ3", "スタンプ1"], "most recently added first, no duplicates");
    assert!(list.stamps.iter().all(|s| s.favorited && s.usable));
    // The flag shows up wherever the stamp does.
    let seen = ctx.get_pack(Some(&user), &id_of(&free)).await.unwrap();
    assert_eq!(seen.stamps.iter().map(|s| s.favorited).collect::<Vec<_>>(), vec![true, false, true]);
    assert!(
        ctx.get_pack(Some(&creator), &id_of(&free)).await.unwrap().stamps.iter().all(|s| !s.favorited),
        "favourites are per user"
    );

    assert_eq!(ctx.reason("Library", "FavoriteStamp", Some(&user), &favorite(&paid, 0)).await, "STAMP_NOT_USABLE");
    let unknown = FavoriteStampRequest { stamp_id: uuid::Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.reason("Library", "FavoriteStamp", Some(&user), &unknown).await, "STAMP_NOT_FOUND");
    assert_eq!(ctx.reason("Library", "FavoriteStamp", None, &favorite(&free, 0)).await, "SIGN_IN_REQUIRED");

    let unfavorite = UnfavoriteStampRequest { stamp_id: free.stamps[2].stamp_ref.id.clone(), ..Default::default() };
    let _: UnfavoriteStampResponse = ctx.call("Library", "UnfavoriteStamp", Some(&user), &unfavorite).await.unwrap();
    let _: UnfavoriteStampResponse = ctx.call("Library", "UnfavoriteStamp", Some(&user), &unfavorite).await.unwrap();
    let list: ListFavoriteStampsResponse =
        ctx.call("Library", "ListFavoriteStamps", Some(&user), &ListFavoriteStampsRequest::default()).await.unwrap();
    assert_eq!(stamp_names(&list.stamps), vec!["スタンプ1"]);

    // A favourite the user can no longer send is hidden, and comes back with the pack.
    ctx.unpublish(&creator, &id_of(&free)).await;
    let hidden: ListFavoriteStampsResponse =
        ctx.call("Library", "ListFavoriteStamps", Some(&user), &ListFavoriteStampsRequest::default()).await.unwrap();
    assert!(hidden.stamps.is_empty());
    ctx.submit(&creator, &id_of(&free)).await;
    let back: ListFavoriteStampsResponse =
        ctx.call("Library", "ListFavoriteStamps", Some(&user), &ListFavoriteStampsRequest::default()).await.unwrap();
    assert_eq!(stamp_names(&back.stamps), vec!["スタンプ1"]);
}

#[tokio::test]
async fn sending_stamps_fills_recents_and_the_usage_log() {
    let ctx = start().await;
    let (creator, user) = (ctx.app.user("creator").await, ctx.app.user("user").await);
    let pack = ctx.published(&creator, "つかう", 0, &[], 3).await;
    let (one, two, three) = (stamp_uuid(&pack, 0), stamp_uuid(&pack, 1), stamp_uuid(&pack, 2));
    let core = &ctx.app.core;

    for (stamp, context) in [
        (one, SendContext::Chat),
        (two, SendContext::Note),
        (one, SendContext::LiveChat),
        (three, SendContext::Map),
        (one, SendContext::Comment),
    ] {
        record_send(core, user.id, stamp, context).await.unwrap();
    }
    // A stamp that vanished is ignored rather than failing the message that carried it.
    record_send(core, user.id, uuid::Uuid::now_v7(), SendContext::Chat).await.unwrap();

    assert_eq!(
        recents(&ctx, &user, RecentOrder::RECENT_ORDER_UNSPECIFIED).await,
        vec!["スタンプ1", "スタンプ3", "スタンプ2"]
    );
    record_send(core, user.id, two, SendContext::Chat).await.unwrap();
    assert_eq!(
        recents(&ctx, &user, RecentOrder::RECENT_ORDER_LAST_USED).await,
        vec!["スタンプ2", "スタンプ1", "スタンプ3"]
    );
    assert_eq!(
        recents(&ctx, &user, RecentOrder::RECENT_ORDER_MOST_USED).await,
        vec!["スタンプ1", "スタンプ2", "スタンプ3"]
    );
    let limited = ListRecentStampsRequest { limit: 1, ..Default::default() };
    let limited: ListRecentStampsResponse =
        ctx.call("Library", "ListRecentStamps", Some(&user), &limited).await.unwrap();
    assert_eq!(stamp_names(&limited.stamps), vec!["スタンプ2"]);
    assert!(recents(&ctx, &creator, RecentOrder::RECENT_ORDER_LAST_USED).await.is_empty(), "recents are per user");

    let log: ListUsageLogResponse = ctx
        .call(
            "Library",
            "ListUsageLog",
            Some(&user),
            &ListUsageLogRequest {
                page: Some(PageRequest { limit: 4, ..Default::default() }).into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let entries: Vec<(String, UsageContext)> =
        log.entries.iter().map(|e| (e.stamp.stamp_ref.name.clone(), e.context.as_known().unwrap())).collect();
    assert_eq!(
        entries,
        vec![
            ("スタンプ2".to_owned(), UsageContext::USAGE_CONTEXT_CHAT),
            ("スタンプ1".to_owned(), UsageContext::USAGE_CONTEXT_COMMENT),
            ("スタンプ3".to_owned(), UsageContext::USAGE_CONTEXT_MAP),
            ("スタンプ1".to_owned(), UsageContext::USAGE_CONTEXT_LIVE_CHAT),
        ]
    );
    assert!(log.page.has_more && log.entries.iter().all(|e| e.sent_at.is_set() && !e.id.is_empty()));
    let rest: ListUsageLogResponse = ctx
        .call(
            "Library",
            "ListUsageLog",
            Some(&user),
            &ListUsageLogRequest {
                page: Some(PageRequest { limit: 4, cursor: log.page.next_cursor.clone(), ..Default::default() }).into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(rest.entries.len(), 2);
    assert!(!rest.page.has_more);
    assert_eq!(ctx.reason("Library", "ListUsageLog", None, &ListUsageLogRequest::default()).await, "SIGN_IN_REQUIRED");

    // Forgetting one stamp, then all of them. The log is a record and stays.
    let _: ClearRecentStampsResponse = ctx
        .call(
            "Library",
            "ClearRecentStamps",
            Some(&user),
            &ClearRecentStampsRequest { stamp_id: one.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(recents(&ctx, &user, RecentOrder::RECENT_ORDER_LAST_USED).await, vec!["スタンプ2", "スタンプ3"]);
    let _: ClearRecentStampsResponse =
        ctx.call("Library", "ClearRecentStamps", Some(&user), &ClearRecentStampsRequest::default()).await.unwrap();
    assert!(recents(&ctx, &user, RecentOrder::RECENT_ORDER_LAST_USED).await.is_empty());
    let log: ListUsageLogResponse =
        ctx.call("Library", "ListUsageLog", Some(&user), &ListUsageLogRequest::default()).await.unwrap();
    assert_eq!(log.entries.len(), 6);
}

#[tokio::test]
async fn recents_remember_only_the_latest_stamps() {
    let ctx = start().await;
    let (creator, user) = (ctx.app.user("creator").await, ctx.app.user("user").await);
    let pack = ctx.draft(&creator, "たくさん", 0, &[]).await;
    let id = id_of(&pack);
    for batch in 0..3 {
        ctx.add_stamps(&creator, &id, &format!("b{batch}-"), 30).await;
    }
    ctx.submit(&creator, &id).await;
    let pack = ctx.approve(&id).await;
    for index in 0..70 {
        record_send(&ctx.app.core, user.id, stamp_uuid(&pack, index), SendContext::Chat).await.unwrap();
    }
    let remembered: i64 = sqlx::query_scalar("SELECT count(*) FROM stamp_recents WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(remembered, 64);
    let all = ListRecentStampsRequest { limit: 100, ..Default::default() };
    let listed: ListRecentStampsResponse = ctx.call("Library", "ListRecentStamps", Some(&user), &all).await.unwrap();
    assert_eq!(listed.stamps.len(), 64);
    assert_eq!(
        listed.stamps[0].stamp_ref.id, pack.stamps[69].stamp_ref.id,
        "the newest is first; the oldest six were dropped"
    );
}

#[tokio::test]
async fn the_picker_search_finds_sendable_stamps_by_name_keyword_and_emoji() {
    let ctx = start().await;
    let (creator, user) = (ctx.app.user("creator").await, ctx.app.user("user").await);
    let word = unique("kw");
    let pack = ctx.draft(&creator, "けんさく", 0, &[]).await;
    let id = id_of(&pack);
    let stamps = vec![
        NewStamp {
            asset_id: ctx.upload(&creator, Upload::Still).await,
            name: "ありがとう".into(),
            emoji: "🙏".into(),
            keywords: vec![word.clone(), "thanks".into()],
            ..Default::default()
        },
        NewStamp {
            asset_id: ctx.upload(&creator, Upload::Still).await,
            name: "おやすみ".into(),
            emoji: "😴".into(),
            keywords: vec!["ねむい".into()],
            ..Default::default()
        },
    ];
    let _: AddStampsResponse = ctx
        .call(
            "Studio",
            "AddStamps",
            Some(&creator),
            &AddStampsRequest { pack_id: id.clone(), stamps, ..Default::default() },
        )
        .await
        .unwrap();
    ctx.submit(&creator, &id).await;
    ctx.approve(&id).await;
    let search = |query: &str| SearchLibraryStampsRequest { query: query.to_owned(), ..Default::default() };
    let find = |query: &'static str| {
        let request = search(query);
        let (ctx, user) = (&ctx, &user);
        async move {
            let response: SearchLibraryStampsResponse =
                ctx.call("Library", "SearchLibraryStamps", Some(user), &request).await.unwrap();
            stamp_names(&response.stamps)
        }
    };

    // Nothing is found in packs the user has not installed.
    assert!(find("ありがとう").await.is_empty());
    ctx.install(&user, &id).await;
    assert_eq!(find("ありが").await, vec!["ありがとう"], "names match by substring");
    assert_eq!(find("THANK").await, vec!["ありがとう"], "keywords match by prefix, whatever the case");
    assert_eq!(find("ねむ").await, vec!["おやすみ"]);
    assert_eq!(find("😴").await, vec!["おやすみ"], "the emoji a stamp stands for");
    assert!(find("hank").await.is_empty(), "keywords do not match in the middle");
    assert!(find("  ").await.is_empty());
    let by_unique_word: SearchLibraryStampsResponse =
        ctx.call("Library", "SearchLibraryStamps", Some(&user), &search(&word)).await.unwrap();
    assert_eq!(stamp_names(&by_unique_word.stamps), vec!["ありがとう"]);

    // Personal stamps are searched too, and recently used stamps come first.
    ctx.personal(&user, "ありがとうございます").await;
    assert_eq!(find("ありがとう").await.len(), 2);
    record_send(
        &ctx.app.core,
        user.id,
        stamp_uuid(&ctx.get_pack(Some(&user), &id).await.unwrap(), 0),
        SendContext::Chat,
    )
    .await
    .unwrap();
    assert_eq!(find("ありがとう").await, vec!["ありがとう", "ありがとうございます"]);

    assert_eq!(
        ctx.reason("Library", "SearchLibraryStamps", Some(&user), &search(&"q".repeat(65))).await,
        "QUERY_TOO_LONG"
    );
    assert_eq!(ctx.reason("Library", "SearchLibraryStamps", None, &search("x")).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn the_library_opens_in_one_call() {
    let ctx = start().await;
    let (creator, user) = (ctx.app.user("creator").await, ctx.app.user("user").await);
    let pack = ctx.published(&creator, "ライブラリ", 0, &[], 2).await;
    ctx.install(&user, &id_of(&pack)).await;
    ctx.personal(&user, "じぶんの").await;
    record_send(&ctx.app.core, user.id, stamp_uuid(&pack, 1), SendContext::Chat).await.unwrap();
    ctx.favorite(&user, &pack.stamps[0].stamp_ref.id).await;

    let library: GetLibraryResponse =
        ctx.call("Library", "GetLibrary", Some(&user), &GetLibraryRequest::default()).await.unwrap();
    assert_eq!(pack_names(&library.installed), vec!["ライブラリ"]);
    assert_eq!(library.installed[0].previews.len(), 2);
    assert_eq!(stamp_names(&library.recents), vec!["スタンプ2"]);
    assert_eq!(stamp_names(&library.favorites), vec!["スタンプ1"]);
    assert_eq!(stamp_names(&library.personal), vec!["じぶんの"]);
    assert!(!library.more_installed && !library.more_favorites && !library.more_personal);

    let empty: GetLibraryResponse = ctx
        .call("Library", "GetLibrary", Some(&ctx.app.user("newcomer").await), &GetLibraryRequest::default())
        .await
        .unwrap();
    assert!(
        empty.installed.is_empty()
            && empty.recents.is_empty()
            && empty.favorites.is_empty()
            && empty.personal.is_empty()
    );
    assert_eq!(ctx.reason("Library", "GetLibrary", None, &GetLibraryRequest::default()).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn installing_in_a_loop_is_rate_limited() {
    let ctx = start().await;
    let (creator, user) = (ctx.app.user("creator").await, ctx.app.user("looper").await);
    let id = id_of(&ctx.published(&creator, "ぐるぐる", 0, &[], 1).await);
    let install = InstallPackRequest { pack_id: id.clone(), ..Default::default() };
    let uninstall = UninstallPackRequest { pack_id: id.clone(), ..Default::default() };
    let mut limited = None;
    for round in 0..100 {
        if let Err(e) = ctx.call::<_, InstallPackResponse>("Library", "InstallPack", Some(&user), &install).await {
            limited = Some((round, e.reason));
            break;
        }
        if let Err(e) = ctx.call::<_, UninstallPackResponse>("Library", "UninstallPack", Some(&user), &uninstall).await
        {
            limited = Some((round, e.reason));
            break;
        }
    }
    let (round, reason) = limited.expect("a hundred install/uninstall rounds in a row are refused at some point");
    assert_eq!(reason, "RATE_LIMITED");
    assert!(round >= 30, "normal use is not limited (stopped at round {round})");
}
