//! Timelines, threads, quotes, batch reads and hashtags: what each viewer
//! gets, in which order, page by page.
mod common;

use common::*;
use tokuly_proto::{common::Visibility, notes::*};
use tokuly_testkit::TestUser;
use uuid::Uuid;

/// Reads a whole timeline two rows at a time and returns the texts in order.
async fn read_all<Resp, F>(mut fetch: F) -> Vec<String>
where
    F: AsyncFnMut(String) -> Resp,
    Resp: Paged,
{
    let mut texts = Vec::new();
    let mut cursor = String::new();
    for _ in 0..50 {
        let response = fetch(cursor).await;
        texts.extend(response.texts());
        if !response.info().has_more {
            assert!(response.info().next_cursor.is_empty(), "the last page has no cursor");
            return texts;
        }
        cursor = response.info().next_cursor.clone();
    }
    panic!("the timeline never ends");
}

trait Paged {
    fn texts(&self) -> Vec<String>;
    fn info(&self) -> &tokuly_proto::common::PageInfo;
}

macro_rules! paged {
    ($($response:ty),*) => {$(
        impl Paged for $response {
            fn texts(&self) -> Vec<String> {
                self.entries.iter().map(|entry| entry.note.text.clone()).collect()
            }
            fn info(&self) -> &tokuly_proto::common::PageInfo {
                &self.page
            }
        }
    )*};
}
paged!(
    GetHomeTimelineResponse,
    GetGlobalTimelineResponse,
    GetUserTimelineResponse,
    GetHashtagTimelineResponse,
    GetMentionsTimelineResponse
);

async fn home(ctx: &Ctx, viewer: &TestUser) -> Vec<String> {
    read_all(async |cursor: String| {
        ctx.note::<_, GetHomeTimelineResponse>(
            "GetHomeTimeline",
            Some(viewer),
            &GetHomeTimelineRequest { page: page(2, &cursor), ..Default::default() },
        )
        .await
        .unwrap()
    })
    .await
}

async fn user_timeline(
    ctx: &Ctx,
    viewer: Option<&TestUser>,
    user: &TestUser,
    filter: UserTimelineFilter,
) -> Vec<String> {
    read_all(async |cursor: String| {
        let request = GetUserTimelineRequest {
            user_id: user.id.to_string(),
            filter: filter.into(),
            page: page(2, &cursor),
            ..Default::default()
        };
        ctx.note::<_, GetUserTimelineResponse>("GetUserTimeline", viewer, &request).await.unwrap()
    })
    .await
}

async fn hashtag_timeline(ctx: &Ctx, viewer: Option<&TestUser>, tag: &str) -> Vec<String> {
    read_all(async |cursor: String| {
        let request = GetHashtagTimelineRequest { tag: tag.to_owned(), page: page(2, &cursor), ..Default::default() };
        ctx.note::<_, GetHashtagTimelineResponse>("GetHashtagTimeline", viewer, &request).await.unwrap()
    })
    .await
}

/// The global timeline as `viewer` sees it, reduced to notes of `authors`
/// (the database is shared with other tests).
async fn global(ctx: &Ctx, viewer: Option<&TestUser>, authors: &[&TestUser]) -> Vec<String> {
    let author_ids: Vec<String> = authors.iter().map(|author| author.id.to_string()).collect();
    let mut texts = Vec::new();
    let mut cursor = String::new();
    loop {
        let request = GetGlobalTimelineRequest { page: page(100, &cursor), ..Default::default() };
        let response: GetGlobalTimelineResponse = ctx.note("GetGlobalTimeline", viewer, &request).await.unwrap();
        texts.extend(
            response
                .entries
                .iter()
                .filter(|entry| author_ids.contains(&entry.note.author.id))
                .map(|entry| entry.note.text.clone()),
        );
        if !response.page.has_more {
            return texts;
        }
        cursor = response.page.next_cursor.clone();
    }
}

#[tokio::test]
async fn the_home_timeline_is_the_viewers_circle() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.member("alice").await, ctx.member("bob").await, ctx.member("carol").await);
    ctx.befriend(&alice, &bob).await;

    ctx.post(&bob, "bob public").await;
    ctx.post_visible_to(&bob, "bob friends", Visibility::VISIBILITY_FRIENDS).await;
    ctx.post_visible_to(&bob, "bob private", Visibility::VISIBILITY_PRIVATE).await;
    let carols = ctx.post(&carol, "carol public").await;
    let mine = ctx.post(&alice, "alice public").await;
    ctx.post_visible_to(&alice, "alice private", Visibility::VISIBILITY_PRIVATE).await;
    ctx.reply(&bob, &carols, "bob to carol").await;
    ctx.reply(&bob, &mine, "bob to alice").await;
    ctx.renote(&bob, &carols).await;

    // Friends' and own notes, replies exchanged within the circle, and what friends renoted.
    assert_eq!(
        home(&ctx, &alice).await,
        vec!["carol public", "bob to alice", "alice private", "alice public", "bob friends", "bob public"]
    );
    // Bob sees his own private note but not alice's; his reply to a stranger is not a conversation within his circle.
    assert_eq!(
        home(&ctx, &bob).await,
        vec!["carol public", "bob to alice", "alice public", "bob private", "bob friends", "bob public"]
    );
    assert_eq!(home(&ctx, &carol).await, vec!["carol public"]);
    assert_eq!(ctx.note_reason("GetHomeTimeline", None, &GetHomeTimelineRequest::default()).await, "SIGN_IN_REQUIRED");

    // The renote says who renoted, and when.
    let first: GetHomeTimelineResponse = ctx
        .note("GetHomeTimeline", Some(&alice), &GetHomeTimelineRequest { page: page(1, ""), ..Default::default() })
        .await
        .unwrap();
    let renote = first.entries[0].renote.as_option().expect("the newest row is bob's renote");
    assert_eq!(renote.user.id, bob.id.to_string());
    assert!(renote.renoted_at.as_option().is_some());
    assert_eq!(first.entries[0].note.author.id, carol.id.to_string());
    assert_eq!(first.entries[0].note.counters.renote_count, 1);

    // A note that is both a friend's note and renoted by a friend shows once, however the timeline is paged.
    ctx.renote(&alice, &mine).await;
    assert_eq!(home(&ctx, &bob).await.iter().filter(|text| *text == "alice public").count(), 1);
    let page_one: GetHomeTimelineResponse = ctx
        .note("GetHomeTimeline", Some(&bob), &GetHomeTimelineRequest { page: page(100, ""), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(page_one.entries[0].note.text, "alice public");
    assert_eq!(page_one.entries[0].renote.user.id, alice.id.to_string(), "it sits where the latest renote put it");

    // Undoing the renote of carol's note takes it out of alice's timeline again.
    let _: UndoRenoteResponse = ctx
        .note("UndoRenote", Some(&bob), &UndoRenoteRequest { note_id: carols.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert!(!home(&ctx, &alice).await.contains(&"carol public".to_owned()));
    // A block ends the friendship and hides the two from each other at once.
    ctx.block(&alice, &bob).await;
    assert_eq!(home(&ctx, &alice).await, vec!["alice public", "alice private"]);
    assert!(home(&ctx, &bob).await.iter().all(|text| text.starts_with("bob")));
}

#[tokio::test]
async fn the_global_timeline_is_every_public_note_that_is_not_a_reply() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.member("alice").await, ctx.member("bob").await, ctx.member("carol").await);
    ctx.befriend(&alice, &bob).await;
    let all = [&alice, &bob, &carol];
    let first = ctx.post(&alice, "g1").await;
    ctx.post_visible_to(&alice, "g friends", Visibility::VISIBILITY_FRIENDS).await;
    ctx.post_visible_to(&alice, "g private", Visibility::VISIBILITY_PRIVATE).await;
    ctx.reply(&bob, &first, "g reply").await;
    ctx.post(&bob, "g2").await;
    ctx.post(&carol, "g3").await;

    assert_eq!(global(&ctx, None, &all).await, vec!["g3", "g2", "g1"]);
    assert_eq!(
        global(&ctx, Some(&bob), &all).await,
        vec!["g3", "g2", "g1"],
        "the global timeline is the same for friends"
    );

    // Blocks hide in both directions.
    ctx.block(&carol, &alice).await;
    assert_eq!(global(&ctx, Some(&carol), &all).await, vec!["g3", "g2"]);
    assert_eq!(global(&ctx, Some(&alice), &all).await, vec!["g2", "g1"]);
    // Notes of a suspended account are shown to nobody.
    sqlx::query("UPDATE users SET banned_at = now() WHERE id = $1")
        .bind(bob.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(global(&ctx, None, &all).await, vec!["g3", "g1"]);
    assert_eq!(ctx.get_note(None, &ctx.post(&carol, "still here").await.id).await.unwrap().text, "still here");
}

#[tokio::test]
async fn user_timelines_have_tabs_and_respect_each_viewer() {
    let ctx = start().await;
    let (alice, friend, stranger) =
        (ctx.member("alice").await, ctx.member("friend").await, ctx.member("stranger").await);
    ctx.befriend(&alice, &friend).await;
    let others = ctx.post(&stranger, "stranger's note").await;
    let image = ctx.note_image(&alice).await;

    let first = ctx.post(&alice, "n1").await;
    ctx.post_with(&alice, "n2 photo", |r| r.asset_ids = vec![image]).await;
    ctx.post_visible_to(&alice, "n3 friends", Visibility::VISIBILITY_FRIENDS).await;
    ctx.post_visible_to(&alice, "n4 private", Visibility::VISIBILITY_PRIVATE).await;
    ctx.reply(&alice, &others, "n5 reply").await;
    ctx.renote(&alice, &others).await;
    ctx.like(&alice, &others).await;
    ctx.like(&alice, &first).await;

    use UserTimelineFilter::*;
    let notes = USER_TIMELINE_FILTER_NOTES;
    assert_eq!(user_timeline(&ctx, None, &alice, notes).await, vec!["stranger's note", "n2 photo", "n1"]);
    assert_eq!(
        user_timeline(&ctx, None, &alice, USER_TIMELINE_FILTER_UNSPECIFIED).await,
        vec!["stranger's note", "n2 photo", "n1"]
    );
    assert_eq!(user_timeline(&ctx, Some(&stranger), &alice, notes).await, vec!["stranger's note", "n2 photo", "n1"]);
    assert_eq!(
        user_timeline(&ctx, Some(&friend), &alice, notes).await,
        vec!["stranger's note", "n3 friends", "n2 photo", "n1"]
    );
    assert_eq!(
        user_timeline(&ctx, Some(&alice), &alice, notes).await,
        vec!["stranger's note", "n4 private", "n3 friends", "n2 photo", "n1"]
    );

    assert_eq!(
        user_timeline(&ctx, None, &alice, USER_TIMELINE_FILTER_WITH_REPLIES).await,
        vec!["stranger's note", "n5 reply", "n2 photo", "n1"]
    );
    assert_eq!(user_timeline(&ctx, None, &alice, USER_TIMELINE_FILTER_MEDIA).await, vec!["n2 photo"]);
    assert_eq!(
        user_timeline(&ctx, None, &alice, USER_TIMELINE_FILTER_LIKES).await,
        vec!["n1", "stranger's note"],
        "most recently liked first"
    );

    // The renote row names the profile's owner as the renoter.
    let request = GetUserTimelineRequest { user_id: alice.id.to_string(), page: page(1, ""), ..Default::default() };
    let top: GetUserTimelineResponse = ctx.note("GetUserTimeline", None, &request).await.unwrap();
    assert_eq!(top.entries[0].renote.user.id, alice.id.to_string());
    assert_eq!(top.entries[0].note.author.id, stranger.id.to_string());

    let of = |user_id: String| GetUserTimelineRequest { user_id, ..Default::default() };
    assert_eq!(ctx.note_reason("GetUserTimeline", None, &of(Uuid::now_v7().to_string())).await, "USER_NOT_FOUND");
    assert_eq!(ctx.note_reason("GetUserTimeline", None, &of("nope".into())).await, "INVALID_ID");
    ctx.block(&alice, &stranger).await;
    assert_eq!(ctx.note_reason("GetUserTimeline", Some(&stranger), &of(alice.id.to_string())).await, "USER_NOT_FOUND");
    assert_eq!(ctx.note_reason("GetUserTimeline", Some(&alice), &of(stranger.id.to_string())).await, "USER_NOT_FOUND");
    // What alice renoted and liked of the stranger's is hidden from alice herself now,
    // while her friend, who is in no block with the stranger, still sees the renote.
    assert_eq!(
        user_timeline(&ctx, Some(&alice), &alice, notes).await,
        vec!["n4 private", "n3 friends", "n2 photo", "n1"]
    );
    assert_eq!(
        user_timeline(&ctx, Some(&friend), &alice, notes).await,
        vec!["stranger's note", "n3 friends", "n2 photo", "n1"]
    );
    assert_eq!(user_timeline(&ctx, Some(&alice), &alice, USER_TIMELINE_FILTER_LIKES).await, vec!["n1"]);
}

#[tokio::test]
async fn hashtag_timelines_fold_spellings_and_count_public_notes() {
    let ctx = start().await;
    let (alice, friend, stranger) =
        (ctx.member("alice").await, ctx.member("friend").await, ctx.member("stranger").await);
    ctx.befriend(&alice, &friend).await;
    let tag = unique("sakura");

    ctx.post(&alice, &format!("h1 #{tag}")).await;
    ctx.post(&stranger, &format!("h2 #{}", tag.to_uppercase())).await;
    ctx.post_visible_to(&alice, &format!("h3 ＃{tag} friends"), Visibility::VISIBILITY_FRIENDS).await;
    ctx.post(&alice, &format!("no tag here, only a link https://example.com/#{tag}")).await;

    assert_eq!(
        hashtag_timeline(&ctx, None, &tag).await,
        vec!["h2 #".to_owned() + &tag.to_uppercase(), format!("h1 #{tag}")]
    );
    assert_eq!(
        hashtag_timeline(&ctx, Some(&friend), &format!("#{}", tag.to_uppercase())).await.len(),
        3,
        "a friend also gets the friends-only note"
    );
    assert_eq!(hashtag_timeline(&ctx, Some(&stranger), &format!("＃{tag}")).await.len(), 2);

    let response: GetHashtagTimelineResponse = ctx
        .note(
            "GetHashtagTimeline",
            Some(&friend),
            &GetHashtagTimelineRequest { tag: tag.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(
        (response.hashtag.tag.as_str(), response.hashtag.note_count),
        (tag.as_str(), 2),
        "the count is of public notes"
    );

    let unused: GetHashtagTimelineResponse = ctx
        .note("GetHashtagTimeline", None, &GetHashtagTimelineRequest { tag: unique("unused"), ..Default::default() })
        .await
        .unwrap();
    assert!(unused.entries.is_empty() && unused.hashtag.note_count == 0 && !unused.page.has_more);
    for bad in ["", "#", "two words", "12345", "semi;colon"] {
        assert_eq!(
            ctx.note_reason(
                "GetHashtagTimeline",
                None,
                &GetHashtagTimelineRequest { tag: bad.into(), ..Default::default() }
            )
            .await,
            "INVALID_HASHTAG",
            "{bad:?}"
        );
    }
}

#[tokio::test]
async fn the_mentions_timeline_collects_mentions_and_replies() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.member("alice").await, ctx.member("bob").await, ctx.member("carol").await);
    let mine = ctx.post(&alice, "alice's note").await;
    ctx.post(&bob, &format!("m1 @{}", alice.handle)).await;
    ctx.reply(&carol, &mine, "m2 reply").await;
    ctx.reply(&bob, &mine, &format!("m3 reply and @{}", alice.handle)).await;
    ctx.post(&alice, &format!("m4 talking to myself @{}", alice.handle)).await;
    ctx.post_visible_to(&bob, &format!("m5 private @{}", alice.handle), Visibility::VISIBILITY_PRIVATE).await;

    let mentions = |viewer: TestUser| {
        let ctx = &ctx;
        async move {
            read_all(async |cursor: String| {
                ctx.note::<_, GetMentionsTimelineResponse>(
                    "GetMentionsTimeline",
                    Some(&viewer),
                    &GetMentionsTimelineRequest { page: page(2, &cursor), ..Default::default() },
                )
                .await
                .unwrap()
            })
            .await
        }
    };
    let expected =
        vec![format!("m3 reply and @{}", alice.handle), "m2 reply".to_owned(), format!("m1 @{}", alice.handle)];
    assert_eq!(mentions(alice.clone()).await, expected, "each note once, newest first, without notes alice cannot see");
    assert!(mentions(carol.clone()).await.is_empty());
    assert_eq!(
        ctx.note_reason("GetMentionsTimeline", None, &GetMentionsTimelineRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );

    ctx.block(&alice, &bob).await;
    assert_eq!(mentions(alice.clone()).await, vec!["m2 reply"]);
}

#[tokio::test]
async fn a_thread_is_the_notes_above_the_note_and_the_replies_below() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.member("alice").await, ctx.member("bob").await, ctx.member("carol").await);
    ctx.befriend(&alice, &bob).await;
    let root = ctx.post(&alice, "root").await;
    let middle = ctx.reply(&bob, &root, "middle").await;
    let leaf = ctx.reply(&alice, &middle, "leaf").await;
    for n in 1..=5 {
        ctx.reply(&carol, &leaf, &format!("r{n}")).await;
    }
    ctx.post_with(&bob, "r6 friends", |r| {
        r.reply_to_id = leaf.id.clone();
        r.visibility = Visibility::VISIBILITY_FRIENDS.into();
    })
    .await;
    let thread = |viewer: Option<TestUser>, note_id: String, limit: i32, cursor: String| {
        let request = GetNoteThreadRequest { note_id, page: page(limit, &cursor), ..Default::default() };
        let ctx = &ctx;
        async move { ctx.note::<_, GetNoteThreadResponse>("GetNoteThread", viewer.as_ref(), &request).await }
    };

    let first = thread(None, leaf.id.clone(), 3, String::new()).await.unwrap();
    assert_eq!(note_texts(&first.ancestors), vec!["root", "middle"], "the start of the conversation comes first");
    assert_eq!(first.note.text, "leaf");
    assert_eq!(first.note.counters.reply_count, 6);
    assert_eq!(note_texts(&first.replies), vec!["r1", "r2", "r3"], "replies are oldest first");
    assert!(first.page.has_more);
    let second = thread(None, leaf.id.clone(), 3, first.page.next_cursor.clone()).await.unwrap();
    assert!(second.ancestors.is_empty(), "ancestors come with the first page only");
    assert_eq!(note_texts(&second.replies), vec!["r4", "r5"], "the friends-only reply is not for anonymous readers");
    assert!(!second.page.has_more);
    // The author of the answered note gets a friends-only reply to it.
    let for_alice = thread(Some(alice.clone()), leaf.id.clone(), 100, String::new()).await.unwrap();
    assert_eq!(for_alice.replies.len(), 6);

    let top = thread(None, root.id.clone(), 100, String::new()).await.unwrap();
    assert!(top.ancestors.is_empty());
    assert_eq!(note_texts(&top.replies), vec!["middle"], "only direct replies");

    // The chain ends at the first note the viewer may not see…
    let hide = SetNoteVisibilityRequest {
        note_id: middle.id.clone(),
        visibility: Visibility::VISIBILITY_PRIVATE.into(),
        ..Default::default()
    };
    let _: SetNoteVisibilityResponse = ctx.note("SetNoteVisibility", Some(&bob), &hide).await.unwrap();
    let cut = thread(Some(carol.clone()), leaf.id.clone(), 1, String::new()).await.unwrap();
    assert!(cut.ancestors.is_empty());
    assert!(cut.note.reply_to.unavailable);
    assert_eq!(
        thread(Some(bob.clone()), leaf.id.clone(), 1, String::new()).await.unwrap().ancestors.len(),
        2,
        "its author still sees the whole chain"
    );
    // …or that was deleted.
    ctx.delete_note(&bob, &middle).await;
    assert!(thread(Some(bob.clone()), leaf.id.clone(), 1, String::new()).await.unwrap().ancestors.is_empty());

    assert_eq!(thread(None, Uuid::now_v7().to_string(), 10, String::new()).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    let private = ctx.post_visible_to(&alice, "mine", Visibility::VISIBILITY_PRIVATE).await;
    assert_eq!(
        thread(Some(carol.clone()), private.id.clone(), 10, String::new()).await.unwrap_err().reason,
        "NOTE_NOT_FOUND"
    );
}

#[tokio::test]
async fn quotes_are_listed_and_notes_are_read_in_batches() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.member("alice").await, ctx.member("bob").await, ctx.member("carol").await);
    let original = ctx.post(&alice, "original").await;
    let q1 = ctx.quote(&bob, &original, "q1").await;
    let q2 = ctx.quote(&carol, &original, "q2").await;
    ctx.post_with(&carol, "q3 private", |r| {
        r.quote_of_id = original.id.clone();
        r.visibility = Visibility::VISIBILITY_PRIVATE.into();
    })
    .await;

    let quotes: ListNoteQuotesResponse = ctx
        .note(
            "ListNoteQuotes",
            None,
            &ListNoteQuotesRequest { note_id: original.id.clone(), page: page(1, ""), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(note_texts(&quotes.notes), vec!["q2"]);
    assert!(quotes.page.has_more);
    let rest: ListNoteQuotesResponse = ctx
        .note(
            "ListNoteQuotes",
            None,
            &ListNoteQuotesRequest {
                note_id: original.id.clone(),
                page: page(1, &quotes.page.next_cursor),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(note_texts(&rest.notes), vec!["q1"]);
    assert!(!rest.page.has_more);
    assert_eq!(rest.notes[0].quote.note.text, "original");
    let unknown = ListNoteQuotesRequest { note_id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.note_reason("ListNoteQuotes", None, &unknown).await, "NOTE_NOT_FOUND");

    // Batches keep the order asked for and silently drop what is missing or hidden.
    let private = ctx.post_visible_to(&alice, "mine", Visibility::VISIBILITY_PRIVATE).await;
    let ids = vec![
        q2.id.clone(),
        Uuid::now_v7().to_string(),
        private.id.clone(),
        original.id.clone(),
        q1.id.clone(),
        q2.id.clone(),
    ];
    let batch: BatchGetNotesResponse = ctx
        .note("BatchGetNotes", Some(&bob), &BatchGetNotesRequest { note_ids: ids.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(note_texts(&batch.notes), vec!["q2", "original", "q1"]);
    let own: BatchGetNotesResponse = ctx
        .note("BatchGetNotes", Some(&alice), &BatchGetNotesRequest { note_ids: ids, ..Default::default() })
        .await
        .unwrap();
    assert_eq!(note_texts(&own.notes), vec!["q2", "mine", "original", "q1"]);
    let too_many = BatchGetNotesRequest { note_ids: vec![original.id.clone(); 101], ..Default::default() };
    assert_eq!(ctx.note_reason("BatchGetNotes", None, &too_many).await, "TOO_MANY_NOTES");
    let empty: BatchGetNotesResponse = ctx.note("BatchGetNotes", None, &BatchGetNotesRequest::default()).await.unwrap();
    assert!(empty.notes.is_empty());
}

#[tokio::test]
async fn hashtags_trend_when_several_people_use_them_and_complete_while_typing() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.member("alice").await, ctx.member("bob").await, ctx.member("carol").await);
    let stem = unique("trend");
    let (hot, warm, solo, quiet) =
        (format!("{stem}hot"), format!("{stem}warm"), format!("{stem}solo"), format!("{stem}quiet"));

    for author in [&alice, &bob, &carol] {
        ctx.post(author, &format!("#{hot}")).await;
    }
    for author in [&alice, &bob] {
        ctx.post(author, &format!("#{warm}")).await;
    }
    for n in 0..4 {
        ctx.post(&alice, &format!("{n} #{solo}")).await; // one person, however loud, is not a trend
    }
    for author in [&alice, &bob] {
        ctx.post_visible_to(author, &format!("#{quiet}"), Visibility::VISIBILITY_FRIENDS).await; // not public
    }

    assert!(ctx.notes.refresh_trends().await.unwrap() >= 2);
    let trending: ListTrendingHashtagsResponse = ctx
        .note("ListTrendingHashtags", None, &ListTrendingHashtagsRequest { limit: 50, ..Default::default() })
        .await
        .unwrap();
    assert!(trending.computed_at.as_option().is_some());
    assert!(trending.hashtags.windows(2).all(|pair| pair[0].rank < pair[1].rank), "best first");
    let mine: Vec<&TrendingHashtag> =
        trending.hashtags.iter().filter(|trend| trend.hashtag.tag.starts_with(&stem)).collect();
    let tags: Vec<&str> = mine.iter().map(|trend| trend.hashtag.tag.as_str()).collect();
    assert_eq!(tags, vec![hot.as_str(), warm.as_str()]);
    assert_eq!((mine[0].recent_note_count, mine[0].hashtag.note_count), (3, 3));
    assert_eq!(mine[1].recent_note_count, 2);

    // Yesterday's topic leaves the ranking.
    sqlx::query("UPDATE notes SET created_at = created_at - interval '2 days' WHERE $1 = ANY(hashtags)")
        .bind(&warm)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.notes.refresh_trends().await.unwrap();
    let trending: ListTrendingHashtagsResponse = ctx
        .note("ListTrendingHashtags", Some(&alice), &ListTrendingHashtagsRequest { limit: 50, ..Default::default() })
        .await
        .unwrap();
    let tags: Vec<&str> =
        trending.hashtags.iter().map(|trend| trend.hashtag.tag.as_str()).filter(|tag| tag.starts_with(&stem)).collect();
    assert_eq!(tags, vec![hot.as_str()]);

    // Completion: by prefix, most used first, public use only, any spelling.
    let search = |prefix: String| {
        let ctx = &ctx;
        async move {
            let found: SearchHashtagsResponse = ctx
                .note("SearchHashtags", None, &SearchHashtagsRequest { prefix, limit: 30, ..Default::default() })
                .await
                .unwrap();
            found.hashtags.iter().map(|hashtag| (hashtag.tag.clone(), hashtag.note_count)).collect::<Vec<_>>()
        }
    };
    assert_eq!(search(stem.clone()).await, vec![(solo.clone(), 4), (hot.clone(), 3), (warm.clone(), 2)]);
    assert_eq!(search(format!("#{}", stem.to_uppercase())).await.len(), 3);
    assert_eq!(search(format!("{stem}h")).await, vec![(hot.clone(), 3)]);
    assert!(search(format!("{stem}%")).await.is_empty(), "wildcards in what was typed are not wildcards");
    assert!(search("not a tag!".to_owned()).await.is_empty());
}
