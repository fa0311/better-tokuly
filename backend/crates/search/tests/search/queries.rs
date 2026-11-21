//! `SearchService.Search`: hits, tabs, counts, order, paging, Japanese.
use tokuly_proto::{common::PageRequest, search::*};
use tokuly_search::SearchConfig;
use tokuly_testkit::TestUser;

use crate::common::*;

fn page<P: buffa::ProtoBox<PageRequest>>(limit: i32, cursor: &str) -> buffa::MessageField<PageRequest, P> {
    buffa::MessageField::some(PageRequest { limit, cursor: cursor.to_owned(), ..Default::default() })
}

#[tokio::test]
async fn a_hit_carries_everything_a_result_row_needs() {
    let ctx = start().await;
    let creator = ctx.app.user("creator").await;
    let word = unique("row");
    let mut video = doc("video", Some(creator.id), &format!("{word} 渋谷の夜を歩く"));
    video.text = format!("東京の街を歩きながら {word} について話します。");
    video.tags = vec!["散歩".into(), "東京".into()];
    video.image_url = Some("https://media.example/poster.webp".into());
    video.path = "/watch/abc123XYZ_0".into();
    video.created_at = 1_750_000_000;
    video.extra = serde_json::json!({ "duration_seconds": 754, "short_id": "abc123XYZ_0" });
    ctx.index(std::slice::from_ref(&video)).await;

    let found = ctx.find(None, &word).await;
    assert_eq!(found.query, word);
    assert_eq!(found.total, 1);
    let hit = &found.hits[0];
    assert_eq!(kind_of(hit), SearchKind::SEARCH_KIND_VIDEO);
    assert_eq!(hit.id, video.id.to_string());
    assert_eq!(hit.title, video.title);
    assert_eq!(joined(&hit.title_segments), video.title, "segments reassemble the title");
    assert_eq!(highlighted(&hit.title_segments), vec![word.as_str()]);
    assert_eq!(highlighted(&hit.snippet), vec![word.as_str()], "the excerpt is cut around the match");
    assert!(joined(&hit.snippet).contains("について話します"));
    assert_eq!(hit.image_url, "https://media.example/poster.webp");
    assert_eq!(hit.path, "/watch/abc123XYZ_0");
    assert_eq!(hit.tags, vec!["散歩", "東京"]);
    assert_eq!(hit.created_at.as_option().unwrap().seconds, 1_750_000_000);
    let owner = hit.owner.as_option().expect("owner summary");
    assert_eq!(
        (owner.id.clone(), owner.handle.clone(), owner.display_name.as_str()),
        (creator.id.to_string(), creator.handle.clone(), "creator")
    );
    let extra: serde_json::Value = serde_json::from_str(&hit.extra_json).unwrap();
    assert_eq!(extra["duration_seconds"], 754);
}

#[tokio::test]
async fn a_document_without_body_or_image_has_an_empty_snippet_and_no_owner() {
    let ctx = start().await;
    let word = unique("bare");
    let place = doc("place", None, &format!("{word} 公園"));
    ctx.index(std::slice::from_ref(&place)).await;

    let hit = ctx.find(None, &word).await.hits.remove(0);
    assert!(hit.snippet.is_empty());
    assert_eq!(hit.image_url, "");
    assert!(hit.owner.as_option().is_none());
    assert_eq!(hit.extra_json, "{}");
}

#[tokio::test]
async fn tabs_narrow_the_hits_while_counts_cover_every_kind() {
    let ctx = start().await;
    let word = unique("tabs");
    let docs = vec![
        doc("video", None, &format!("{word} one")),
        doc("video", None, &format!("{word} two")),
        doc("short", None, &format!("{word} three")),
        doc("note", None, &format!("{word} four")),
        doc("stamp_pack", None, &format!("{word} five")),
    ];
    ctx.index(&docs).await;
    let expected_counts = [
        (SearchKind::SEARCH_KIND_VIDEO, 2),
        (SearchKind::SEARCH_KIND_SHORT, 1),
        (SearchKind::SEARCH_KIND_NOTE, 1),
        (SearchKind::SEARCH_KIND_STAMP_PACK, 1),
        (SearchKind::SEARCH_KIND_USER, 0),
        (SearchKind::SEARCH_KIND_LIVE, 0),
    ];

    // The "all" tab.
    let all = ctx.find(None, &word).await;
    assert_eq!((all.hits.len(), all.total), (5, 5));
    for (kind, count) in expected_counts {
        assert_eq!(count_of(&all, kind), count, "{kind:?} in the all tab");
    }
    assert_eq!(all.counts.len(), 4, "kinds without matches are absent");
    let order: Vec<i32> = all.counts.iter().map(|c| c.kind.to_i32()).collect();
    assert!(order.is_sorted(), "counts come in SearchKind order");

    // One tab: only its hits, the same counts.
    let videos = ctx
        .search(
            None,
            &SearchRequest { query: word.clone(), kind: SearchKind::SEARCH_KIND_VIDEO.into(), ..Default::default() },
        )
        .await;
    assert_eq!((videos.hits.len(), videos.total), (2, 2));
    assert!(videos.hits.iter().all(|hit| kind_of(hit) == SearchKind::SEARCH_KIND_VIDEO));
    for (kind, count) in expected_counts {
        assert_eq!(count_of(&videos, kind), count, "{kind:?} in the video tab");
    }

    // A tab without matches still shows where the matches are.
    let lives = ctx
        .search(
            None,
            &SearchRequest { query: word.clone(), kind: SearchKind::SEARCH_KIND_LIVE.into(), ..Default::default() },
        )
        .await;
    assert!(lives.hits.is_empty() && lives.total == 0);
    assert_eq!(count_of(&lives, SearchKind::SEARCH_KIND_VIDEO), 2);
}

/// Walks every page of a query and returns the IDs in order.
async fn walk(ctx: &Ctx, user: Option<&TestUser>, mut request: SearchRequest, expected_pages: usize) -> Vec<String> {
    let limit = request.page.as_option().map(|p| p.limit).unwrap_or(0);
    let mut ids = Vec::new();
    let mut pages = 0;
    loop {
        let response = ctx.search(user, &request).await;
        pages += 1;
        assert_eq!(response.counts.is_empty(), pages > 1, "counts come with the first page only");
        ids.extend(hit_ids(&response.hits));
        let info = response.page.as_option().expect("page info");
        assert_eq!(info.has_more, !info.next_cursor.is_empty());
        if !info.has_more {
            break;
        }
        assert_eq!(response.hits.len(), limit as usize, "a page before the last is full");
        request.page = page(limit, &info.next_cursor);
        assert!(pages < 20, "paging does not end");
    }
    assert_eq!(pages, expected_pages);
    ids
}

#[tokio::test]
async fn paging_walks_every_result_exactly_once() {
    let ctx = start().await;
    let word = unique("paging");
    let mut docs = Vec::new();
    for n in 0..7 {
        let mut video = doc(if n % 2 == 0 { "video" } else { "short" }, None, &format!("{word} {n}"));
        video.score = n as f64;
        docs.push(video);
    }
    ctx.index(&docs).await;
    let mut all_ids: Vec<String> = docs.iter().map(|d| d.id.to_string()).collect();
    all_ids.sort();

    // Across every kind (the engine merges the indexes)...
    let mut walked =
        walk(&ctx, None, SearchRequest { query: word.clone(), page: page(3, ""), ..Default::default() }, 3).await;
    assert_eq!(walked.len(), 7);
    walked.sort();
    assert_eq!(walked, all_ids);

    // ...and inside one tab.
    let videos = walk(
        &ctx,
        None,
        SearchRequest {
            query: word.clone(),
            kind: SearchKind::SEARCH_KIND_VIDEO.into(),
            page: page(3, ""),
            ..Default::default()
        },
        2,
    )
    .await;
    assert_eq!(videos.len(), 4);

    // An exact fit has no further page.
    let exact = ctx.search(None, &SearchRequest { query: word.clone(), page: page(7, ""), ..Default::default() }).await;
    assert_eq!(exact.hits.len(), 7);
    assert!(!exact.page.as_option().unwrap().has_more);
}

#[tokio::test]
async fn page_sizes_are_defaulted_and_clamped_and_cursors_belong_to_their_query() {
    let ctx = start().await;
    let (word, other_word) = (unique("clamp"), unique("clampb"));
    let mut docs: Vec<_> = (0..55).map(|n| doc("note", None, &format!("{word} {n}"))).collect();
    docs.push(doc("note", None, &other_word));
    ctx.index(&docs).await;

    let default = ctx.find(None, &word).await;
    assert_eq!(default.hits.len(), 20);
    assert_eq!(default.total, 55);
    let clamped =
        ctx.search(None, &SearchRequest { query: word.clone(), page: page(500, ""), ..Default::default() }).await;
    assert_eq!(clamped.hits.len(), 50);

    let cursor = default.page.as_option().unwrap().next_cursor.clone();
    assert!(!cursor.is_empty());
    // The cursor continues its own query...
    let second =
        ctx.search(None, &SearchRequest { query: word.clone(), page: page(0, &cursor), ..Default::default() }).await;
    assert_eq!(second.hits.len(), 20);
    // ...and no other: not other words, another tab or another order.
    for request in [
        SearchRequest { query: other_word.clone(), page: page(0, &cursor), ..Default::default() },
        SearchRequest {
            query: word.clone(),
            kind: SearchKind::SEARCH_KIND_NOTE.into(),
            page: page(0, &cursor),
            ..Default::default()
        },
        SearchRequest {
            query: word.clone(),
            sort: SearchSort::SEARCH_SORT_NEWEST.into(),
            page: page(0, &cursor),
            ..Default::default()
        },
        SearchRequest { query: word.clone(), page: page(0, "not-a-cursor"), ..Default::default() },
    ] {
        let error = ctx.try_search(None, &request).await.unwrap_err();
        assert_eq!((error.code.as_str(), error.reason.as_str()), ("invalid_argument", "INVALID_CURSOR"));
    }
}

#[tokio::test]
async fn results_can_be_ordered_by_relevance_recency_or_popularity() {
    let ctx = start().await;
    let word = unique("order");
    let day = 86_400;
    let now = chrono::Utc::now().timestamp();
    let mut old_hit = doc("video", None, &format!("{word} classic"));
    (old_hit.created_at, old_hit.score) = (now - 400 * day, 50_000.0);
    let mut fresh = doc("video", None, &format!("{word} fresh"));
    (fresh.created_at, fresh.score) = (now, 10.0);
    let mut middling = doc("video", None, &format!("{word} middling"));
    (middling.created_at, middling.score) = (now - 20 * day, 400.0);
    // Mentions the word only in its description: a weaker match than a title.
    let mut mention = doc("video", None, "unrelated title");
    (mention.created_at, mention.score, mention.text) = (now - day, 9_000_000.0, format!("see also {word}"));
    ctx.index(&[old_hit.clone(), fresh.clone(), middling.clone(), mention.clone()]).await;
    let sorted = async |sort: SearchSort| {
        let request = SearchRequest {
            query: word.clone(),
            kind: SearchKind::SEARCH_KIND_VIDEO.into(),
            sort: sort.into(),
            ..Default::default()
        };
        hit_ids(&ctx.search(None, &request).await.hits)
    };
    let ids = |docs: &[&tokuly_core::outbound::SearchDoc]| docs.iter().map(|d| d.id.to_string()).collect::<Vec<_>>();

    // Explicit orders are strict, whatever matched where.
    assert_eq!(sorted(SearchSort::SEARCH_SORT_NEWEST).await, ids(&[&fresh, &mention, &middling, &old_hit]));
    assert_eq!(sorted(SearchSort::SEARCH_SORT_POPULAR).await, ids(&[&mention, &old_hit, &middling, &fresh]));
    // Relevance: title matches before the mention, however popular it is;
    // among equal matches popularity is traded against age (a video loses a
    // factor of ten per quarter): 400 views twenty days ago beat 10 today,
    // and both beat 50,000 from over a year ago.
    let by_relevance = ids(&[&middling, &fresh, &old_hit, &mention]);
    assert_eq!(sorted(SearchSort::SEARCH_SORT_RELEVANCE).await, by_relevance);
    assert_eq!(sorted(SearchSort::SEARCH_SORT_UNSPECIFIED).await, by_relevance);
    // The all tab merges indexes by the same rules.
    assert_eq!(hit_ids(&ctx.find(None, &word).await.hits), by_relevance);
}

#[tokio::test]
async fn kinds_are_merged_by_relevance_and_rank_in_the_all_tab() {
    let ctx = start().await;
    let word = unique("merge");
    let now = chrono::Utc::now().timestamp();
    let mut video = doc("video", None, &format!("{word} video"));
    (video.created_at, video.score) = (now, 5.0);
    let mut popular_short = doc("short", None, &format!("{word} short"));
    (popular_short.created_at, popular_short.score) = (now, 5_000_000.0);
    // Evergreen kinds are ranked on their own time scale: a tie with timely
    // content goes to the timely document.
    let mut playlist = doc("playlist", None, &format!("{word} playlist"));
    (playlist.created_at, playlist.score) = (now, 9_000_000.0);
    let mut body_only = doc("note", None, "something else");
    (body_only.created_at, body_only.text, body_only.score) = (now, format!("a note about {word}"), 9_000_000.0);
    ctx.index(&[video.clone(), popular_short.clone(), playlist.clone(), body_only.clone()]).await;

    let merged = hit_ids(&ctx.find(None, &word).await.hits);
    assert_eq!(
        merged,
        vec![popular_short.id.to_string(), video.id.to_string(), playlist.id.to_string(), body_only.id.to_string()],
        "title matches by rank across kinds, then the body match"
    );
}

#[tokio::test]
async fn japanese_queries_find_japanese_titles() {
    let ctx = start().await;
    let owner = ctx.app.user("jp").await;
    let night = doc("video", Some(owner.id), "渋谷の夜を歩く");
    let mut crossing = doc("video", Some(owner.id), "スクランブル交差点ライブカメラ");
    crossing.text = "渋谷駅前の様子をお届けします".into();
    let walk = doc("note", Some(owner.id), "たのしいおさんぽ");
    let tower = doc("place", Some(owner.id), "東京タワー");
    let english = doc("video", Some(owner.id), "Tokyo night walk");
    ctx.index(&[night.clone(), crossing.clone(), walk.clone(), tower.clone(), english.clone()]).await;
    let found = async |query: &str| {
        let mut ids = hit_ids(&ctx.find_owned(None, owner.id, query).await.hits);
        ids.sort();
        ids
    };
    let ids = |docs: &[&tokuly_core::outbound::SearchDoc]| {
        let mut ids: Vec<String> = docs.iter().map(|d| d.id.to_string()).collect();
        ids.sort();
        ids
    };

    // Kanji, in a title and in a body.
    assert_eq!(found("渋谷").await, ids(&[&night, &crossing]));
    // Hiragana words, whole and as the stem the tokenizer finds.
    assert_eq!(found("おさんぽ").await, ids(&[&walk]));
    assert_eq!(found("さんぽ").await, ids(&[&walk]));
    // Katakana, also typed in half-width.
    assert_eq!(found("タワー").await, ids(&[&tower]));
    assert_eq!(found("ﾀﾜｰ").await, ids(&[&tower]));
    assert_eq!(found("交差点").await, ids(&[&crossing]));
    // Full-width Latin letters find Latin text.
    assert_eq!(found("ｔｏｋｙｏ").await, ids(&[&english]));

    // The match is highlighted inside the Japanese sentence.
    let hit =
        ctx.find_owned(None, owner.id, "渋谷").await.hits.into_iter().find(|h| h.id == night.id.to_string()).unwrap();
    assert_eq!(highlighted(&hit.title_segments), vec!["渋谷"]);
    assert_eq!(joined(&hit.title_segments), "渋谷の夜を歩く");
}

#[tokio::test]
async fn every_word_must_match_and_quotes_make_a_phrase() {
    let ctx = start().await;
    let word = unique("phrase");
    let (in_order, reversed, partial) = (
        doc("note", None, &format!("{word} red apple pie")),
        doc("note", None, &format!("{word} apple red tart")),
        doc("note", None, &format!("{word} red wine")),
    );
    ctx.index(&[in_order.clone(), reversed.clone(), partial.clone()]).await;
    let found = async |query: String| {
        let mut ids = hit_ids(&ctx.find(None, &query).await.hits);
        ids.sort();
        ids
    };
    let ids = |docs: &[&tokuly_core::outbound::SearchDoc]| {
        let mut ids: Vec<String> = docs.iter().map(|d| d.id.to_string()).collect();
        ids.sort();
        ids
    };

    // All words, in any order; a document with only some of them is no match.
    assert_eq!(found(format!("{word} red")).await, ids(&[&in_order, &reversed, &partial]));
    assert_eq!(found(format!("{word} apple red")).await, ids(&[&in_order, &reversed]));
    assert!(found(format!("{word} red banana")).await.is_empty());
    // The last word may still be being typed.
    assert_eq!(found(format!("{word} red app")).await, ids(&[&in_order, &reversed]));
    // Quoted words must appear together and in that order.
    assert_eq!(found(format!("{word} \"red apple\"")).await, ids(&[&in_order]));
}

#[tokio::test]
async fn a_hash_prefix_searches_tags_only() {
    let ctx = start().await;
    let word = unique("tag");
    let mut tagged = doc("note", None, "週末のこと");
    tagged.tags = vec![word.clone(), "散歩".into()];
    let titled = doc("note", None, &format!("{word} in the title"));
    ctx.index(&[tagged.clone(), titled.clone()]).await;

    let mut plain = hit_ids(&ctx.find(None, &word).await.hits);
    plain.sort();
    let mut both = vec![tagged.id.to_string(), titled.id.to_string()];
    both.sort();
    assert_eq!(plain, both);

    for query in [format!("#{word}"), format!("＃{word}"), format!("  # {word}")] {
        let by_tag = ctx.find(None, &query).await;
        assert_eq!(hit_ids(&by_tag.hits), vec![tagged.id.to_string()], "query {query:?}");
        assert!(by_tag.query.starts_with('#'));
    }
}

#[tokio::test]
async fn typos_are_forgiven_in_words_but_not_in_handles() {
    let ctx = start().await;
    let user = ctx.app.user("typo").await;
    let stem = unique("crossing");
    let video = doc("video", Some(user.id), &format!("{stem} walk"));
    ctx.index_user(&user).await;
    ctx.index(std::slice::from_ref(&video)).await;

    // One letter dropped from a long word still finds the title.
    let mut misspelt = stem.clone();
    misspelt.remove(3);
    assert_eq!(hit_ids(&ctx.find(None, &misspelt).await.hits), vec![video.id.to_string()]);

    // The handle is an identifier: exact tokens only.
    let users = async |query: String| {
        let request = SearchRequest { query, kind: SearchKind::SEARCH_KIND_USER.into(), ..Default::default() };
        hit_ids(&ctx.search(None, &request).await.hits)
    };
    assert_eq!(users(user.handle.clone()).await, vec![user.id.to_string()]);
    let mut wrong_handle = user.handle.clone();
    let last = wrong_handle.pop().unwrap();
    wrong_handle.push(if last == 'x' { 'y' } else { 'x' });
    assert!(!users(wrong_handle).await.contains(&user.id.to_string()));
}

#[tokio::test]
async fn searching_inside_one_owners_content() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let word = unique("mine");
    let (hers, his) = (doc("video", Some(alice.id), &word), doc("video", Some(bob.id), &word));
    ctx.index(&[hers.clone(), his.clone()]).await;

    assert_eq!(ctx.find(None, &word).await.hits.len(), 2);
    let owned = ctx.find_owned(None, alice.id, &word).await;
    assert_eq!(hit_ids(&owned.hits), vec![hers.id.to_string()]);
    assert_eq!((owned.total, count_of(&owned, SearchKind::SEARCH_KIND_VIDEO)), (1, 1));
}

#[tokio::test]
async fn invalid_requests_are_refused_with_their_reason() {
    let ctx = start().await;
    let cases: Vec<(serde_json::Value, &str)> = vec![
        (serde_json::json!({ "query": "" }), "QUERY_REQUIRED"),
        (serde_json::json!({ "query": "  \n　 " }), "QUERY_REQUIRED"),
        (serde_json::json!({ "query": "#" }), "QUERY_REQUIRED"),
        (serde_json::json!({ "query": "あ".repeat(101) }), "QUERY_TOO_LONG"),
        (serde_json::json!({ "query": "ok", "kind": 99 }), "KIND_UNKNOWN"),
        (serde_json::json!({ "query": "ok", "sort": 42 }), "SORT_UNKNOWN"),
        (serde_json::json!({ "query": "ok", "ownerId": "not-a-uuid" }), "INVALID_ID"),
    ];
    for (request, reason) in cases {
        let error = ctx.call::<_, SearchResponse>(SEARCH, "Search", None, &request).await.unwrap_err();
        assert_eq!((error.code.as_str(), error.reason.as_str()), ("invalid_argument", reason), "{request}");
    }
    // 100 characters is fine.
    ctx.find(None, &"あ".repeat(100)).await;
}

#[tokio::test]
async fn a_broken_token_is_not_treated_as_a_guest() {
    let ctx = start().await;
    let user = ctx.app.user("expired").await;
    let broken = TestUser { token: "not.a.token".into(), ..user };
    let error = ctx
        .try_search(Some(&broken), &SearchRequest { query: "anything".into(), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!(error.code, "unauthenticated");
}

#[tokio::test]
async fn searches_are_rate_limited_per_account_and_per_address() {
    let ctx = start_with(|cfg: &mut SearchConfig| {
        cfg.rate_per_minute = 3;
        cfg.anonymous_rate_per_minute = 2;
    })
    .await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let request = SearchRequest { query: unique("limit"), ..Default::default() };

    for _ in 0..2 {
        ctx.search(None, &request).await;
    }
    let error = ctx.try_search(None, &request).await.unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("resource_exhausted", "RATE_LIMITED"));

    // Signing in gives each account its own, larger allowance.
    for _ in 0..3 {
        ctx.search(Some(&alice), &request).await;
    }
    assert_eq!(ctx.try_search(Some(&alice), &request).await.unwrap_err().reason, "RATE_LIMITED");
    ctx.search(Some(&bob), &request).await;
}

#[tokio::test]
async fn first_pages_are_tracked_for_analytics_without_the_query_text() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let word = unique("tracked");
    ctx.index(&[doc("video", None, &format!("{word} a")), doc("video", None, &format!("{word} b"))]).await;

    let first = ctx
        .search(
            Some(&alice),
            &SearchRequest {
                query: word.clone(),
                kind: SearchKind::SEARCH_KIND_VIDEO.into(),
                page: page(1, ""),
                ..Default::default()
            },
        )
        .await;
    let cursor = first.page.as_option().unwrap().next_cursor.clone();
    ctx.search(
        Some(&alice),
        &SearchRequest {
            query: word.clone(),
            kind: SearchKind::SEARCH_KIND_VIDEO.into(),
            page: page(1, &cursor),
            ..Default::default()
        },
    )
    .await;
    ctx.suggest(Some(&alice), &word).await;
    ctx.app.flush_analytics().await;

    let events = &ctx.app.core.analytics.ch;
    let user = alice.id.to_string();
    let count: u64 = events
        .query("SELECT count() FROM events WHERE name = 'search.query' AND toString(user_id) = ?")
        .bind(&user)
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(count, 1, "one event per search, not per page or keystroke");
    let matches: f64 = events
        .query("SELECT value FROM events WHERE name = 'search.query' AND toString(user_id) = ?")
        .bind(&user)
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(matches, 2.0, "the number of matches is the event's value");
    let props: String = events
        .query("SELECT props FROM events WHERE name = 'search.query' AND toString(user_id) = ?")
        .bind(&user)
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&props).unwrap(),
        serde_json::json!({ "kind": "video", "tags_only": false })
    );
    assert!(!props.contains(&word), "what people search for stays out of analytics");
}

#[tokio::test]
async fn missing_indexes_are_created_on_the_first_search() {
    // A deployment whose setup never ran: its own prefix, nothing created.
    let prefix = format!("{}x", *PREFIX);
    let ctx = start_with(|cfg: &mut SearchConfig| cfg.index_prefix = prefix.clone()).await;
    let cfg = &ctx.app.core.cfg;

    let outcome = ctx.try_search(None, &SearchRequest { query: unique("nothing"), ..Default::default() }).await;
    let created = ctx.app.core.search.get_index(format!("{prefix}_video")).await.is_ok();
    delete_indexes_with_prefix(&cfg.meili_url, &cfg.meili_key, &prefix).await;

    let response = outcome.expect("the search succeeds after creating the indexes");
    assert!(response.hits.is_empty() && created);
    assert!(
        ctx.app.core.search.get_index(format!("{prefix}_video")).await.is_err(),
        "the test cleaned up after itself"
    );
}
