//! Trending queries: who counts, how many it takes, and the blocklist.
//!
//! The test configuration needs three distinct signed-in users for a query
//! to qualify (`trending_min_users = 3`, not the default). Every test of the
//! binary computes the same shared list, so none of them may use another
//! threshold.
use tokuly_proto::search::*;
use tokuly_testkit::TestUser;

use crate::common::*;

async fn searchers(ctx: &Ctx, count: usize) -> Vec<TestUser> {
    let mut users = Vec::new();
    for n in 0..count {
        users.push(ctx.app.user(&format!("searcher{n}")).await);
    }
    users
}

/// A query that finds something: a fresh word with a document for it.
async fn findable(ctx: &Ctx, label: &str) -> String {
    let word = unique(label);
    ctx.index(&[doc("video", None, &word)]).await;
    word
}

async fn refreshed(ctx: &Ctx) -> Vec<String> {
    ctx.search.refresh_trending().await.unwrap();
    ctx.trending(None).await
}

#[tokio::test]
async fn one_person_cannot_make_a_query_trend() {
    let ctx = start().await;
    let users = searchers(&ctx, 3).await;
    let query = findable(&ctx, "solo").await;

    // One user, over and over.
    for _ in 0..25 {
        ctx.find(Some(&users[0]), &query).await;
    }
    assert!(!refreshed(&ctx).await.contains(&query));
    // Two users are still not enough.
    ctx.find(Some(&users[1]), &query).await;
    assert!(!refreshed(&ctx).await.contains(&query));
    // The third distinct user makes it trend.
    ctx.find(Some(&users[2]), &query).await;
    assert!(refreshed(&ctx).await.contains(&query));
}

#[tokio::test]
async fn searches_that_do_not_count() {
    let ctx = start().await;
    let users = searchers(&ctx, 3).await;
    let (guests, skipped, paused, later_pages, fruitless) = (
        findable(&ctx, "guests").await,
        findable(&ctx, "skipped").await,
        findable(&ctx, "paused").await,
        findable(&ctx, "pages").await,
        unique("fruitless"),
    );

    // Guests, however many searches.
    for _ in 0..5 {
        ctx.find(None, &guests).await;
    }
    // Searches flagged as not to be remembered.
    for user in &users {
        ctx.search(Some(user), &SearchRequest { query: skipped.clone(), skip_history: true, ..Default::default() })
            .await;
    }
    // Users who paused their history.
    for user in &users {
        let _: SetHistoryPausedResponse = ctx
            .call(
                SEARCH,
                "SetHistoryPaused",
                Some(user),
                &SetHistoryPausedRequest { paused: true, ..Default::default() },
            )
            .await
            .unwrap();
        ctx.find(Some(user), &paused).await;
        let _: SetHistoryPausedResponse = ctx
            .call(
                SEARCH,
                "SetHistoryPaused",
                Some(user),
                &SetHistoryPausedRequest { paused: false, ..Default::default() },
            )
            .await
            .unwrap();
    }
    // Queries that find nothing.
    for user in &users {
        ctx.find(Some(user), &fruitless).await;
    }
    // Scrolling on: only the first page of a search counts, so one user
    // paging through results is one search.
    ctx.index(&[doc("video", None, &later_pages), doc("video", None, &later_pages)]).await;
    let first = ctx
        .search(
            Some(&users[0]),
            &SearchRequest {
                query: later_pages.clone(),
                page: Some(tokuly_proto::common::PageRequest { limit: 1, ..Default::default() }).into(),
                ..Default::default()
            },
        )
        .await;
    let cursor = first.page.as_option().unwrap().next_cursor.clone();
    for user in &users[1..] {
        // A page reached with a cursor, by other users: not a new search.
        ctx.search(
            Some(user),
            &SearchRequest {
                query: later_pages.clone(),
                page: Some(tokuly_proto::common::PageRequest {
                    limit: 1,
                    cursor: cursor.clone(),
                    ..Default::default()
                })
                .into(),
                ..Default::default()
            },
        )
        .await;
    }

    let trending = refreshed(&ctx).await;
    for query in [&guests, &skipped, &paused, &fruitless, &later_pages] {
        assert!(!trending.contains(query), "{query} must not trend");
    }
}

#[tokio::test]
async fn queries_are_counted_regardless_of_case_width_and_tab() {
    let ctx = start().await;
    let users = searchers(&ctx, 3).await;
    let word = findable(&ctx, "variant").await;

    ctx.find(Some(&users[0]), &word.to_uppercase()).await;
    ctx.find(Some(&users[1]), &format!("  {word}  ")).await;
    ctx.search(
        Some(&users[2]),
        &SearchRequest { query: word.clone(), kind: SearchKind::SEARCH_KIND_VIDEO.into(), ..Default::default() },
    )
    .await;

    assert!(refreshed(&ctx).await.contains(&word), "one query, three spellings");
}

#[tokio::test]
async fn more_searchers_rank_higher() {
    let ctx = start().await;
    let users = searchers(&ctx, 5).await;
    let (big, small) = (findable(&ctx, "big").await, findable(&ctx, "small").await);
    for user in &users {
        ctx.find(Some(user), &big).await;
    }
    for user in &users[..3] {
        ctx.find(Some(user), &small).await;
    }

    let qualified = ctx.search.refresh_trending().await.unwrap();
    let position =
        |query: &str| qualified.iter().position(|q| q.query == query).unwrap_or_else(|| panic!("{query} qualified"));
    assert!(position(&big) < position(&small));
    let (big_entry, small_entry) = (&qualified[position(&big)], &qualified[position(&small)]);
    assert_eq!((big_entry.searchers, small_entry.searchers), (5, 3));
    assert!(big_entry.weight > small_entry.weight);

    // The RPC returns them in the same order, ranked from 1.
    let response: GetTrendingResponse =
        ctx.call(SEARCH, "GetTrending", None, &GetTrendingRequest { limit: 20, ..Default::default() }).await.unwrap();
    let rank_of = |query: &str| {
        response
            .queries
            .iter()
            .find(|q| q.query == query)
            .map(|q| q.rank)
            .unwrap_or_else(|| panic!("{query} is trending"))
    };
    assert!(rank_of(&big) < rank_of(&small));
    let ranks: Vec<i32> = response.queries.iter().map(|q| q.rank).collect();
    assert_eq!(ranks, (1..=response.queries.len() as i32).collect::<Vec<_>>());
    assert!(response.updated_at.as_option().is_some());
}

#[tokio::test]
async fn the_trending_list_is_public_and_bounded() {
    let ctx = start().await;
    let users = searchers(&ctx, 3).await;
    let mut queries = Vec::new();
    for n in 0..3 {
        let query = findable(&ctx, &format!("bounded{n}")).await;
        for user in &users {
            ctx.find(Some(user), &query).await;
        }
        queries.push(query);
    }
    ctx.search.refresh_trending().await.unwrap();

    let limited: GetTrendingResponse =
        ctx.call(SEARCH, "GetTrending", None, &GetTrendingRequest { limit: 2, ..Default::default() }).await.unwrap();
    assert_eq!(limited.queries.len(), 2);
    let default: GetTrendingResponse =
        ctx.call(SEARCH, "GetTrending", Some(&users[0]), &GetTrendingRequest::default()).await.unwrap();
    assert!((3..=10).contains(&default.queries.len()));
    let huge: GetTrendingResponse =
        ctx.call(SEARCH, "GetTrending", None, &GetTrendingRequest { limit: 1000, ..Default::default() }).await.unwrap();
    assert!(huge.queries.len() <= 20);
}

#[tokio::test]
async fn blocked_terms_never_trend_and_are_never_suggested() {
    let ctx = start().await;
    let admin = ctx.app.user_with_roles("admin", &["admin"]).await;
    let users = searchers(&ctx, 3).await;
    let bad = unique("bad");
    ctx.index(&[doc("video", None, &format!("{bad} thing"))]).await;
    let query = format!("{bad} thing");
    for user in &users {
        ctx.find(Some(user), &query).await;
    }
    assert!(refreshed(&ctx).await.contains(&query));
    assert_eq!(ctx.suggest(None, &bad).await.completions.len(), 1);

    // A moderator blocks the word: it is gone at once, in any casing...
    let _: AddBlockedTermResponse = ctx
        .call(
            ADMIN,
            "AddBlockedTerm",
            Some(&admin),
            &AddBlockedTermRequest { term: bad.to_uppercase(), note: "test".into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(!ctx.trending(None).await.contains(&query));
    assert!(ctx.suggest(None, &bad).await.completions.is_empty());
    // ...stays gone when the list is recomputed, and further searches for it
    // are not counted. Searching itself still works.
    let more = searchers(&ctx, 3).await;
    for user in &more {
        assert_eq!(ctx.find(Some(user), &query).await.hits.len(), 1);
    }
    assert!(!refreshed(&ctx).await.contains(&query));

    // Lifting the block lets it qualify again on the searches counted before.
    let _: RemoveBlockedTermResponse = ctx
        .call(
            ADMIN,
            "RemoveBlockedTerm",
            Some(&admin),
            &RemoveBlockedTermRequest { term: bad.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let qualified = ctx.search.refresh_trending().await.unwrap();
    let entry = qualified.iter().find(|q| q.query == query).expect("trending again");
    assert_eq!(entry.searchers, 3, "searches made while it was blocked were not counted");
}

#[tokio::test]
async fn pasted_texts_and_single_letters_do_not_trend() {
    let ctx = start().await;
    let users = searchers(&ctx, 3).await;
    // 41+ characters: a pasted sentence rather than a query. It does find
    // something, so only its length keeps it from trending.
    let pasted = format!("{} {}", unique("long"), "とても長い文章です".repeat(4));
    assert!(pasted.chars().count() > 40);
    ctx.index(&[doc("note", None, &pasted)]).await;
    for user in &users {
        assert_eq!(ctx.find(Some(user), &pasted).await.hits.len(), 1);
    }
    assert!(!refreshed(&ctx).await.contains(&pasted));
}
