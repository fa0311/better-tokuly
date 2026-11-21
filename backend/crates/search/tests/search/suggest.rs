//! `SearchService.Suggest`: typeahead hits and query completions.
use tokuly_proto::search::*;
use tokuly_search::SearchConfig;
use tokuly_testkit::TestUser;

use crate::common::*;

async fn try_suggest(ctx: &Ctx, user: Option<&TestUser>, request: &SuggestRequest) -> Result<SuggestResponse, tokuly_testkit::RpcError> {
    ctx.call(SEARCH, "Suggest", user, request).await
}

fn completions(response: &SuggestResponse) -> Vec<(&str, CompletionSource)> {
    response.completions.iter().map(|c| (c.query.as_str(), c.source.as_known().unwrap())).collect()
}

/// `count` different users search `query` (which must find something), then
/// the trending job runs.
async fn make_popular(ctx: &Ctx, query: &str, count: usize) {
    for n in 0..count {
        let user = ctx.app.user(&format!("searcher{n}")).await;
        ctx.find(Some(&user), query).await;
    }
    ctx.search.refresh_trending().await.unwrap();
}

#[tokio::test]
async fn typing_a_prefix_suggests_matching_names() {
    let ctx = start().await;
    let stem = unique("prefix");
    let user = ctx.app.user("typed").await;
    sqlx::query("UPDATE users SET display_name = $2 WHERE id = $1")
        .bind(user.id)
        .bind(format!("{stem}taro"))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.index_user(&user).await;
    let titled = doc("video", None, &format!("{stem}video の日"));
    let mut tagged = doc("note", None, "なんでもない日");
    tagged.tags = vec![format!("{stem}tag")];
    // Mentions the word in its body only: found by search, not suggested.
    let mut body_only = doc("note", None, "別の話");
    body_only.text = format!("{stem}body が出てくる長い本文");
    ctx.index(&[titled.clone(), tagged.clone(), body_only.clone()]).await;

    // Half-typed input.
    let typed = &stem[..stem.len() - 3];
    let response = ctx.suggest(None, typed).await;
    let mut ids = hit_ids(&response.hits);
    ids.sort();
    let mut expected = vec![user.id.to_string(), titled.id.to_string(), tagged.id.to_string()];
    expected.sort();
    assert_eq!(ids, expected);
    // The user comes first: typeahead favours names.
    assert_eq!(response.hits[0].id, user.id.to_string());
    for hit in &response.hits {
        assert!(hit.snippet.is_empty(), "typeahead rows have no excerpt");
    }
    let user_hit = response.hits.iter().find(|h| h.id == user.id.to_string()).unwrap();
    assert_eq!(highlighted(&user_hit.title_segments), vec![typed], "the typed part of the name is marked");
    assert_eq!(user_hit.owner.as_option().unwrap().handle, user.handle);

    // The full search does find the body match.
    assert!(hit_ids(&ctx.find(None, &format!("{stem}body")).await.hits).contains(&body_only.id.to_string()));
}

#[tokio::test]
async fn suggestions_can_be_limited_to_a_tab_and_in_number() {
    let ctx = start().await;
    let stem = unique("limit");
    let mut docs = Vec::new();
    for n in 0..12 {
        docs.push(doc(if n < 8 { "video" } else { "short" }, None, &format!("{stem} number {n}")));
    }
    ctx.index(&docs).await;

    let default = ctx.suggest(None, &stem).await;
    assert_eq!(default.hits.len(), 6);
    let wide = try_suggest(&ctx, None, &SuggestRequest { query: stem.clone(), hit_limit: 99, ..Default::default() }).await.unwrap();
    assert_eq!(wide.hits.len(), 10);
    let two = try_suggest(&ctx, None, &SuggestRequest { query: stem.clone(), hit_limit: 2, ..Default::default() }).await.unwrap();
    assert_eq!(two.hits.len(), 2);
    let shorts = try_suggest(
        &ctx,
        None,
        &SuggestRequest { query: stem.clone(), kind: SearchKind::SEARCH_KIND_SHORT.into(), hit_limit: 10, ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!(shorts.hits.len(), 4);
    assert!(shorts.hits.iter().all(|hit| kind_of(hit) == SearchKind::SEARCH_KIND_SHORT));
}

#[tokio::test]
async fn completions_offer_the_callers_history_first_and_then_popular_queries() {
    let ctx = start().await;
    let stem = unique("complete");
    ctx.index(&[doc("video", None, &format!("{stem} apple banana cherry"))]).await;
    let alice = ctx.app.user("alice").await;
    let (popular, popular_too, own) = (format!("{stem} apple"), format!("{stem} banana"), format!("{stem} Cherry Pie"));
    make_popular(&ctx, &popular, 4).await;
    make_popular(&ctx, &popular_too, 3).await;
    ctx.find(Some(&alice), &own).await;
    ctx.find(Some(&alice), &popular).await;

    // Alice: her own searches (newest first), then what many searched; a
    // query she searched herself is not repeated as a popular one.
    let hers = ctx.suggest(Some(&alice), &stem[..stem.len() - 2]).await;
    assert_eq!(
        completions(&hers),
        vec![
            (popular.as_str(), CompletionSource::COMPLETION_SOURCE_RECENT),
            (own.as_str(), CompletionSource::COMPLETION_SOURCE_RECENT),
            (popular_too.as_str(), CompletionSource::COMPLETION_SOURCE_POPULAR),
        ]
    );
    let recent = ctx.recent(&alice).await;
    assert_eq!(hers.completions[1].recent_search_id, recent.searches.iter().find(|r| r.query == own).unwrap().id);
    assert_eq!(hers.completions[2].recent_search_id, "");

    // A guest gets the popular queries, most searched first, and never
    // something only one person typed.
    let guests = ctx.suggest(None, &stem).await;
    assert_eq!(
        completions(&guests),
        vec![
            (popular.as_str(), CompletionSource::COMPLETION_SOURCE_POPULAR),
            (popular_too.as_str(), CompletionSource::COMPLETION_SOURCE_POPULAR),
        ]
    );

    // Completions extend the input: a longer prefix narrows them, the input
    // itself is not offered back, and case does not matter.
    let narrowed = ctx.suggest(None, &format!("{} B", stem.to_uppercase())).await;
    assert_eq!(completions(&narrowed), vec![(popular_too.as_str(), CompletionSource::COMPLETION_SOURCE_POPULAR)]);
    assert!(ctx.suggest(None, &popular).await.completions.is_empty());

    let one = try_suggest(&ctx, None, &SuggestRequest { query: stem.clone(), completion_limit: 1, ..Default::default() }).await.unwrap();
    assert_eq!(one.completions.len(), 1);
}

#[tokio::test]
async fn suggesting_remembers_nothing() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let word = unique("quiet");
    ctx.index(&[doc("video", None, &word)]).await;

    for _ in 0..5 {
        ctx.suggest(Some(&alice), &word).await;
    }
    assert!(ctx.recent(&alice).await.searches.is_empty());
}

#[tokio::test]
async fn empty_and_invalid_input() {
    let ctx = start().await;
    for empty in ["", "   ", "\n"] {
        let response = ctx.suggest(None, empty).await;
        assert!(response.hits.is_empty() && response.completions.is_empty());
    }
    // A lone '#' is a tag search without a tag yet.
    assert!(ctx.suggest(None, "#").await.hits.is_empty());

    let too_long = try_suggest(&ctx, None, &SuggestRequest { query: "a".repeat(101), ..Default::default() }).await.unwrap_err();
    assert_eq!((too_long.code.as_str(), too_long.reason.as_str()), ("invalid_argument", "QUERY_TOO_LONG"));
    let unknown_kind = ctx
        .call::<_, SuggestResponse>(SEARCH, "Suggest", None, &serde_json::json!({ "query": "ok", "kind": 250 }))
        .await
        .unwrap_err();
    assert_eq!(unknown_kind.reason, "KIND_UNKNOWN");
}

#[tokio::test]
async fn tag_typeahead_completes_tags() {
    let ctx = start().await;
    let stem = unique("hash");
    let mut tagged = doc("hashtag", None, &format!("{stem}walk"));
    tagged.tags = vec![format!("{stem}walk")];
    let titled_only = doc("video", None, &format!("{stem}walk in the title"));
    ctx.index(&[tagged.clone(), titled_only.clone()]).await;

    let response = ctx.suggest(None, &format!("#{stem}")).await;
    assert_eq!(hit_ids(&response.hits), vec![tagged.id.to_string()]);
}

#[tokio::test]
async fn typeahead_is_rate_limited() {
    let ctx = start_with(|cfg: &mut SearchConfig| cfg.suggest_rate_per_minute = 4).await;
    let alice = ctx.app.user("alice").await;
    let request = SuggestRequest { query: unique("fast"), ..Default::default() };

    // Guests get half the allowance of an account.
    for _ in 0..2 {
        try_suggest(&ctx, None, &request).await.unwrap();
    }
    assert_eq!(try_suggest(&ctx, None, &request).await.unwrap_err().reason, "RATE_LIMITED");
    for _ in 0..4 {
        try_suggest(&ctx, Some(&alice), &request).await.unwrap();
    }
    assert_eq!(try_suggest(&ctx, Some(&alice), &request).await.unwrap_err().reason, "RATE_LIMITED");
}
