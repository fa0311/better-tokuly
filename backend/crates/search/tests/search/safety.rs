//! What must never reach a caller: content hidden by a block, and markup or
//! forged highlights smuggled in through indexed text.
use tokuly_proto::search::*;
use tokuly_search::text::{HIGHLIGHT_END, HIGHLIGHT_START};
use uuid::Uuid;

use crate::common::*;

#[tokio::test]
async fn a_block_hides_content_in_both_directions() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    let word = unique("block");
    let (alices, bobs, carols) = (
        doc("video", Some(alice.id), &format!("{word} by alice")),
        doc("video", Some(bob.id), &format!("{word} by bob")),
        doc("note", Some(carol.id), &format!("{word} by carol")),
    );
    ctx.index(&[alices.clone(), bobs.clone(), carols.clone()]).await;
    let seen_by = async |user: Option<&tokuly_testkit::TestUser>| {
        let mut ids = hit_ids(&ctx.find(user, &word).await.hits);
        ids.sort();
        ids
    };
    let ids = |docs: &[&tokuly_core::outbound::SearchDoc]| {
        let mut ids: Vec<String> = docs.iter().map(|d| d.id.to_string()).collect();
        ids.sort();
        ids
    };
    assert_eq!(seen_by(Some(&alice)).await, ids(&[&alices, &bobs, &carols]));

    ctx.block(&alice, &bob).await;

    // The blocker does not see the blocked user's content...
    let alice_view = ctx.find(Some(&alice), &word).await;
    assert_eq!(seen_by(Some(&alice)).await, ids(&[&alices, &carols]));
    // ...and the counts and totals do not give it away either.
    assert_eq!(alice_view.total, 2);
    assert_eq!(count_of(&alice_view, SearchKind::SEARCH_KIND_VIDEO), 1);
    // The blocked user does not see the blocker's content.
    assert_eq!(seen_by(Some(&bob)).await, ids(&[&bobs, &carols]));
    // Everyone else, signed in or not, sees all of it.
    assert_eq!(seen_by(Some(&carol)).await, ids(&[&alices, &bobs, &carols]));
    assert_eq!(seen_by(None).await, ids(&[&alices, &bobs, &carols]));

    // The video tab and the typeahead apply the same rule.
    let videos = ctx
        .search(
            Some(&alice),
            &SearchRequest { query: word.clone(), kind: SearchKind::SEARCH_KIND_VIDEO.into(), ..Default::default() },
        )
        .await;
    assert_eq!(hit_ids(&videos.hits), vec![alices.id.to_string()]);
    let suggested = hit_ids(&ctx.suggest(Some(&alice), &word).await.hits);
    assert!(suggested.contains(&alices.id.to_string()) && !suggested.contains(&bobs.id.to_string()));
    // Asking for the blocked user's content by owner finds nothing.
    assert!(ctx.find_owned(Some(&alice), bob.id, &word).await.hits.is_empty());
}

#[tokio::test]
async fn a_blocked_user_cannot_be_found_as_a_user_either() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.index_user(&alice).await;
    ctx.index_user(&bob).await;
    ctx.flush().await;
    let users = async |viewer: &tokuly_testkit::TestUser, handle: &str| {
        let request =
            SearchRequest { query: handle.to_owned(), kind: SearchKind::SEARCH_KIND_USER.into(), ..Default::default() };
        hit_ids(&ctx.search(Some(viewer), &request).await.hits)
    };
    assert_eq!(users(&alice, &bob.handle).await, vec![bob.id.to_string()]);

    ctx.block(&bob, &alice).await;

    assert!(users(&alice, &bob.handle).await.is_empty(), "the blocked user cannot find the blocker");
    assert!(users(&bob, &alice.handle).await.is_empty(), "and the blocker does not find them");
    assert_eq!(users(&alice, &alice.handle).await, vec![alice.id.to_string()]);
}

#[tokio::test]
async fn content_without_an_owning_account_survives_the_block_filter() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.block(&alice, &bob).await;
    let word = unique("official");
    // Official content: no owner, or the nil UUID.
    let (ownerless, official) =
        (doc("news", None, &format!("{word} news")), doc("stamp_pack", Some(Uuid::nil()), &format!("{word} pack")));
    ctx.index(&[ownerless.clone(), official.clone()]).await;

    let found = ctx.find(Some(&alice), &word).await;
    let mut ids = hit_ids(&found.hits);
    ids.sort();
    let mut expected = vec![ownerless.id.to_string(), official.id.to_string()];
    expected.sort();
    assert_eq!(ids, expected);
    assert!(found.hits.iter().all(|hit| hit.owner.as_option().is_none()));
}

#[tokio::test]
async fn markup_in_indexed_text_stays_text() {
    let ctx = start().await;
    let word = unique("xss");
    let mut note = doc("note", None, &format!("<script>alert('{word}')</script> <b>{word}</b>"));
    note.text = format!("<img src=x onerror=alert(1)> {word} &amp; <em>fake</em>");
    ctx.index(std::slice::from_ref(&note)).await;

    let hit = ctx.find(None, &word).await.hits.remove(0);
    // The text comes back exactly as written, as text...
    assert_eq!(hit.title, note.title);
    assert_eq!(joined(&hit.title_segments), note.title);
    assert_eq!(joined(&hit.snippet), note.text);
    // ...and only real matches are marked, as flags on plain-text segments.
    let marked = highlighted(&hit.title_segments);
    assert!(!marked.is_empty() && marked.iter().all(|run| *run == word), "{marked:?}");
    assert_eq!(highlighted(&hit.snippet), vec![word.as_str()]);
    let everything = serde_json::to_string(&hit).unwrap();
    assert!(
        !everything.contains(HIGHLIGHT_START) && !everything.contains(HIGHLIGHT_END),
        "the engine's markers never leave the server"
    );
}

#[tokio::test]
async fn content_cannot_forge_a_highlight() {
    let ctx = start().await;
    let word = unique("forge");
    // The author wraps a phrase in the engine's own highlight markers.
    let mut note = doc("note", None, &format!("{word} {HIGHLIGHT_START}公式アカウント{HIGHLIGHT_END}"));
    note.text = format!("{HIGHLIGHT_START}本物です{HIGHLIGHT_END} {word}\u{0}\u{7}");
    note.tags = vec![format!("{HIGHLIGHT_START}tag{HIGHLIGHT_END}")];
    ctx.index(std::slice::from_ref(&note)).await;

    let hit = ctx.find(None, &word).await.hits.remove(0);
    assert_eq!(hit.title, format!("{word} 公式アカウント"));
    assert_eq!(highlighted(&hit.title_segments), vec![word.as_str()], "only the real match is highlighted");
    assert_eq!(highlighted(&hit.snippet), vec![word.as_str()]);
    assert_eq!(joined(&hit.snippet), format!("本物です {word}"));
    assert_eq!(hit.tags, vec!["tag"]);
}

#[tokio::test]
async fn a_query_cannot_inject_a_filter_or_markers() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    ctx.block(&alice, &bob).await;
    let word = unique("inject");
    // The title contains the words the hostile queries below are made of
    // (quotes in a query make a phrase, so in the same order).
    let bobs = doc("video", Some(bob.id), &format!("{word} OR owner_id = {} OR kind = video OR owner_id IN", bob.id));
    ctx.index(std::slice::from_ref(&bobs)).await;

    // Filter syntax and markers in a query are just words to look for: an
    // uninvolved user finds the document by them, the blocker still does not.
    for query in [
        format!("{word} \" OR owner_id = \"{}", bob.id),
        format!("{word}\"] OR kind = video OR owner_id IN [\""),
        format!("{HIGHLIGHT_START}{word}{HIGHLIGHT_END}"),
    ] {
        let neutral = ctx.find(Some(&carol), &query).await;
        assert_eq!(hit_ids(&neutral.hits), vec![bobs.id.to_string()], "query {query:?} is searched as words");
        assert!(!neutral.query.contains(HIGHLIGHT_START));
        let blocker = ctx.find(Some(&alice), &query).await;
        assert!(blocker.hits.is_empty(), "query {query:?} must not reveal blocked content");
    }
}

#[tokio::test]
async fn channel_content_never_names_the_account_behind_it() {
    let ctx = start().await;
    let (owner, stranger) = (ctx.app.user("creator").await, ctx.app.user("stranger").await);
    let word = unique("channelowner");
    let channel = doc("channel", Some(owner.id), &format!("{word} channel"));
    let video = doc("video", Some(owner.id), &format!("{word} video"));
    let short = doc("short", Some(owner.id), &format!("{word} short"));
    let live = doc("live", Some(owner.id), &format!("{word} live"));
    let mut on_channel = doc("playlist", Some(owner.id), &format!("{word} channel list"));
    on_channel.extra = serde_json::json!({ "channel_id": Uuid::now_v7() });
    let mut personal = doc("playlist", Some(owner.id), &format!("{word} personal list"));
    personal.extra = serde_json::json!({ "channel_id": null });
    let note = doc("note", Some(owner.id), &format!("{word} note"));
    let all = [channel, video.clone(), short, live, on_channel, personal.clone(), note.clone()];
    ctx.index(&all).await;
    let named = [personal.id.to_string(), note.id.to_string()];

    // Found, but only a personal playlist and the note name the account.
    let found = ctx.find(Some(&stranger), &word).await;
    assert_eq!(found.hits.len(), all.len());
    for hit in found.hits.iter().chain(ctx.suggest(Some(&stranger), &word).await.hits.iter()) {
        assert_eq!(hit.owner.as_option().is_some(), named.contains(&hit.id), "{}", hit.title);
    }

    // "Search this profile" does not list the account's channel content...
    let profile = ctx.find_owned(Some(&stranger), owner.id, &word).await;
    assert_eq!(hit_ids(&profile.hits), vec![note.id.to_string()]);
    assert_eq!((profile.total, count_of(&profile, SearchKind::SEARCH_KIND_VIDEO)), (1, 0));
    let tab = ctx
        .search(
            None,
            &SearchRequest {
                query: word.clone(),
                kind: SearchKind::SEARCH_KIND_VIDEO.into(),
                owner_id: owner.id.to_string(),
                ..Default::default()
            },
        )
        .await;
    assert!(
        tab.hits.is_empty()
            && tab.total == 0
            && tab.counts.iter().all(|c| c.count == 0 || c.kind.as_known() == Some(SearchKind::SEARCH_KIND_NOTE))
    );

    // ...while the owner finds all of it, with themselves as the owner.
    let mine = ctx.find_owned(Some(&owner), owner.id, &word).await;
    assert_eq!(mine.hits.len(), all.len());
    assert!(mine.hits.iter().all(|hit| hit.owner.as_option().is_some_and(|o| o.id == owner.id.to_string())));
    assert!(hit_ids(&mine.hits).contains(&video.id.to_string()));
}

#[tokio::test]
async fn a_channel_clip_is_left_out_of_someone_elses_search_of_the_account() {
    let ctx = start().await;
    let (owner, stranger) = (ctx.app.user("creator").await, ctx.app.user("stranger").await);
    let word = unique("channelclip");
    let mut own = doc("clip", Some(owner.id), &format!("{word} own clip"));
    own.extra = serde_json::json!({ "channel_id": Uuid::now_v7(), "channel_clip": true });
    let mut fan = doc("clip", Some(owner.id), &format!("{word} fan clip"));
    fan.extra = serde_json::json!({ "channel_id": Uuid::now_v7(), "channel_clip": false });
    let note = doc("note", Some(owner.id), &format!("{word} note"));
    ctx.index(&[own.clone(), fan.clone(), note.clone()]).await;

    // Found by its words, without the account's name.
    let found = ctx.find(Some(&stranger), &word).await;
    assert_eq!(found.hits.len(), 3);
    let own_hit = found.hits.iter().find(|hit| hit.id == own.id.to_string()).unwrap();
    assert!(own_hit.owner.as_option().is_none());

    // Searching the account: not there, neither in the hits nor in the counts.
    let mut expected = hit_ids(&ctx.find_owned(Some(&stranger), owner.id, &word).await.hits);
    expected.sort();
    let mut want = vec![fan.id.to_string(), note.id.to_string()];
    want.sort();
    assert_eq!(expected, want);
    let tab = ctx
        .search(
            Some(&stranger),
            &SearchRequest {
                query: word.clone(),
                kind: SearchKind::SEARCH_KIND_CLIP.into(),
                owner_id: owner.id.to_string(),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(hit_ids(&tab.hits), vec![fan.id.to_string()]);
    assert_eq!((tab.total, count_of(&tab, SearchKind::SEARCH_KIND_CLIP)), (1, 1));

    // The owner searching themselves finds it.
    let mine = ctx.find_owned(Some(&owner), owner.id, &word).await;
    assert!(hit_ids(&mine.hits).contains(&own.id.to_string()));
}
