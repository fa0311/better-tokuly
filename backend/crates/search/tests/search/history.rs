//! The caller's recent searches: remembering, listing, deleting, pausing.
use tokuly_proto::{common::PageRequest, search::*};
use tokuly_testkit::TestUser;

use crate::common::*;

fn queries(response: &ListRecentSearchesResponse) -> Vec<&str> {
    response.searches.iter().map(|r| r.query.as_str()).collect()
}

async fn set_paused(ctx: &Ctx, user: &TestUser, paused: bool) -> bool {
    let response: SetHistoryPausedResponse =
        ctx.call(SEARCH, "SetHistoryPaused", Some(user), &SetHistoryPausedRequest { paused, ..Default::default() }).await.unwrap();
    response.paused
}

#[tokio::test]
async fn searches_are_remembered_newest_first_without_duplicates() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let (first, second) = (unique("First"), unique("second"));

    ctx.find(Some(&alice), &first).await;
    ctx.search(Some(&alice), &SearchRequest { query: second.clone(), kind: SearchKind::SEARCH_KIND_VIDEO.into(), ..Default::default() }).await;
    let recent = ctx.recent(&alice).await;
    assert_eq!(queries(&recent), vec![second.as_str(), first.as_str()]);
    assert!(!recent.history_paused);
    let (newest, oldest) = (&recent.searches[0], &recent.searches[1]);
    assert_eq!(newest.kind.as_known(), Some(SearchKind::SEARCH_KIND_VIDEO), "the tab is remembered");
    assert_eq!(oldest.kind.as_known(), Some(SearchKind::SEARCH_KIND_UNSPECIFIED));
    assert!(newest.searched_at.as_option().is_some() && !newest.id.is_empty());

    // The same words again, in another casing and tab: the entry moves up
    // and shows the latest form; no second entry appears.
    ctx.search(Some(&alice), &SearchRequest { query: format!(" {} ", first.to_uppercase()), kind: SearchKind::SEARCH_KIND_NOTE.into(), ..Default::default() })
        .await;
    let recent = ctx.recent(&alice).await;
    assert_eq!(queries(&recent), vec![first.to_uppercase().as_str(), second.as_str()]);
    assert_eq!(recent.searches[0].id, oldest.id, "it is the same entry");
    assert_eq!(recent.searches[0].kind.as_known(), Some(SearchKind::SEARCH_KIND_NOTE));

    // A search that finds nothing is remembered too; a tag search keeps its '#'.
    ctx.find(Some(&alice), &format!("#{second}")).await;
    assert_eq!(ctx.recent(&alice).await.searches[0].query, format!("#{second}"));
}

#[tokio::test]
async fn only_new_searches_are_remembered() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let word = unique("pages");
    ctx.index(&[doc("note", None, &format!("{word} a")), doc("note", None, &format!("{word} b"))]).await;
    let other = unique("silent");

    // Not remembered: flagged searches, further pages, typeahead, and
    // requests that fail validation.
    ctx.search(Some(&alice), &SearchRequest { query: other.clone(), skip_history: true, ..Default::default() }).await;
    ctx.suggest(Some(&alice), &other).await;
    let _ = ctx.try_search(Some(&alice), &SearchRequest { query: "#".into(), ..Default::default() }).await.unwrap_err();
    assert!(ctx.recent(&alice).await.searches.is_empty());

    let first = ctx
        .search(Some(&alice), &SearchRequest { query: word.clone(), page: Some(PageRequest { limit: 1, ..Default::default() }).into(), ..Default::default() })
        .await;
    let remembered_at = ctx.recent(&alice).await.searches[0].searched_at.clone();
    let cursor = first.page.as_option().unwrap().next_cursor.clone();
    ctx.search(Some(&alice), &SearchRequest { query: word.clone(), page: Some(PageRequest { limit: 1, cursor, ..Default::default() }).into(), ..Default::default() })
        .await;
    let recent = ctx.recent(&alice).await;
    assert_eq!(queries(&recent), vec![word.as_str()]);
    assert_eq!(recent.searches[0].searched_at, remembered_at, "loading the next page is not a new search");
}

#[tokio::test]
async fn history_is_private_to_its_owner() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let word = unique("secret");
    ctx.find(Some(&alice), &word).await;
    let entry = ctx.recent(&alice).await.searches.remove(0);

    assert!(ctx.recent(&bob).await.searches.is_empty());
    let stolen = ctx
        .call::<_, DeleteRecentSearchResponse>(SEARCH, "DeleteRecentSearch", Some(&bob), &DeleteRecentSearchRequest { id: entry.id.clone(), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!((stolen.code.as_str(), stolen.reason.as_str()), ("not_found", "RECENT_SEARCH_NOT_FOUND"));
    let cleared: ClearRecentSearchesResponse = ctx.call(SEARCH, "ClearRecentSearches", Some(&bob), &ClearRecentSearchesRequest::default()).await.unwrap();
    assert_eq!(cleared.deleted_count, 0);
    assert_eq!(ctx.recent(&alice).await.searches.len(), 1);
    // Bob's typeahead does not complete with Alice's query either.
    assert!(ctx.suggest(Some(&bob), &word[..word.len() - 2]).await.completions.is_empty());
}

#[tokio::test]
async fn entries_can_be_deleted_one_by_one_or_all_at_once() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let words = [unique("one"), unique("two"), unique("three")];
    for word in &words {
        ctx.find(Some(&alice), word).await;
    }
    let recent = ctx.recent(&alice).await;
    let middle = recent.searches[1].clone();

    let _: DeleteRecentSearchResponse =
        ctx.call(SEARCH, "DeleteRecentSearch", Some(&alice), &DeleteRecentSearchRequest { id: middle.id.clone(), ..Default::default() }).await.unwrap();
    assert_eq!(queries(&ctx.recent(&alice).await), vec![words[2].as_str(), words[0].as_str()]);

    // Deleting it again, or something that is not an ID.
    let again = ctx
        .call::<_, DeleteRecentSearchResponse>(SEARCH, "DeleteRecentSearch", Some(&alice), &DeleteRecentSearchRequest { id: middle.id, ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!(again.reason, "RECENT_SEARCH_NOT_FOUND");
    let invalid = ctx
        .call::<_, DeleteRecentSearchResponse>(SEARCH, "DeleteRecentSearch", Some(&alice), &DeleteRecentSearchRequest { id: "nope".into(), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!((invalid.code.as_str(), invalid.reason.as_str()), ("invalid_argument", "INVALID_ID"));

    let cleared: ClearRecentSearchesResponse = ctx.call(SEARCH, "ClearRecentSearches", Some(&alice), &ClearRecentSearchesRequest::default()).await.unwrap();
    assert_eq!(cleared.deleted_count, 2);
    assert!(ctx.recent(&alice).await.searches.is_empty());
}

#[tokio::test]
async fn pausing_stops_remembering_until_resumed() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let (before, during, after) = (unique("before"), unique("during"), unique("after"));
    ctx.find(Some(&alice), &before).await;

    assert!(set_paused(&ctx, &alice, true).await);
    ctx.find(Some(&alice), &during).await;
    let paused = ctx.recent(&alice).await;
    assert!(paused.history_paused);
    assert_eq!(queries(&paused), vec![before.as_str()], "what was remembered before stays");

    // Pausing twice is fine; resuming picks up again.
    assert!(set_paused(&ctx, &alice, true).await);
    assert!(!set_paused(&ctx, &alice, false).await);
    ctx.find(Some(&alice), &after).await;
    let resumed = ctx.recent(&alice).await;
    assert!(!resumed.history_paused);
    assert_eq!(queries(&resumed), vec![after.as_str(), before.as_str()]);
}

#[tokio::test]
async fn the_newest_fifty_searches_are_kept_and_listed_in_pages() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let stem = unique("many");
    for n in 0..53 {
        ctx.search(Some(&alice), &SearchRequest { query: format!("{stem} {n:02}"), ..Default::default() }).await;
    }

    let mut listed: Vec<String> = Vec::new();
    let mut cursor = String::new();
    let mut pages = 0;
    loop {
        let response: ListRecentSearchesResponse = ctx
            .call(SEARCH, "ListRecentSearches", Some(&alice), &ListRecentSearchesRequest { page: Some(PageRequest { limit: 20, cursor, ..Default::default() }).into(), ..Default::default() })
            .await
            .unwrap();
        pages += 1;
        listed.extend(response.searches.iter().map(|r| r.query.clone()));
        let info = response.page.as_option().unwrap();
        if !info.has_more {
            break;
        }
        cursor = info.next_cursor.clone();
    }
    assert_eq!(pages, 3);
    let expected: Vec<String> = (3..53).rev().map(|n| format!("{stem} {n:02}")).collect();
    assert_eq!(listed, expected, "the three oldest fell out; the rest is newest first");

    // Page size: 20 by default, 50 at most.
    assert_eq!(ctx.recent(&alice).await.searches.len(), 20);
    let all: ListRecentSearchesResponse = ctx
        .call(SEARCH, "ListRecentSearches", Some(&alice), &ListRecentSearchesRequest { page: Some(PageRequest { limit: 999, ..Default::default() }).into(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(all.searches.len(), 50);
    let bad_cursor = ctx
        .call::<_, ListRecentSearchesResponse>(
            SEARCH,
            "ListRecentSearches",
            Some(&alice),
            &ListRecentSearchesRequest { page: Some(PageRequest { cursor: "garbage".into(), ..Default::default() }).into(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(bad_cursor.reason, "INVALID_CURSOR");
}

#[tokio::test]
async fn history_needs_an_account() {
    let ctx = start().await;
    let reasons = [
        ctx.call::<_, ListRecentSearchesResponse>(SEARCH, "ListRecentSearches", None, &ListRecentSearchesRequest::default()).await.unwrap_err(),
        ctx.call::<_, DeleteRecentSearchResponse>(SEARCH, "DeleteRecentSearch", None, &DeleteRecentSearchRequest::default()).await.unwrap_err(),
        ctx.call::<_, ClearRecentSearchesResponse>(SEARCH, "ClearRecentSearches", None, &ClearRecentSearchesRequest::default()).await.unwrap_err(),
        ctx.call::<_, SetHistoryPausedResponse>(SEARCH, "SetHistoryPaused", None, &SetHistoryPausedRequest { paused: true, ..Default::default() }).await.unwrap_err(),
    ];
    for error in reasons {
        assert_eq!((error.code.as_str(), error.reason.as_str()), ("unauthenticated", "SIGN_IN_REQUIRED"));
    }
}

#[tokio::test]
async fn the_data_export_contains_the_search_history() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let word = unique("export");
    ctx.search(Some(&alice), &SearchRequest { query: word.clone(), kind: SearchKind::SEARCH_KIND_PLACE.into(), ..Default::default() }).await;
    set_paused(&ctx, &alice, true).await;

    let exporter = ctx.app.core.exporters.all().into_iter().find(|e| e.name() == "search").expect("search registered an exporter");
    let exported = exporter.export(alice.id).await.unwrap();
    assert_eq!(exported["history_paused"], true);
    assert_eq!(exported["recent_searches"].as_array().unwrap().len(), 1);
    assert_eq!(exported["recent_searches"][0]["query"], word);
    assert_eq!(exported["recent_searches"][0]["kind"], "place");
    assert!(exported["recent_searches"][0]["searched_at"].is_string());
}
