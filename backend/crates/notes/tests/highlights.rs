//! Highlights: titled groups of stories that stay on a profile.
mod common;

use common::*;
use tokuly_proto::notes::*;
use tokuly_testkit::TestUser;
use uuid::Uuid;

fn ids(stories: &[&Story]) -> Vec<String> {
    stories.iter().map(|story| story.id.clone()).collect()
}

async fn create(ctx: &Ctx, owner: &TestUser, title: &str, stories: &[&Story]) -> Highlight {
    let request = CreateHighlightRequest { title: title.to_owned(), story_ids: ids(stories), ..Default::default() };
    let response: CreateHighlightResponse = ctx
        .story("CreateHighlight", Some(owner), &request)
        .await
        .unwrap_or_else(|e| panic!("creating highlight {title:?} failed: {e}"));
    response.highlight.as_option().cloned().expect("the highlight")
}

async fn list(ctx: &Ctx, viewer: &TestUser, owner: &TestUser) -> Vec<Highlight> {
    let response: ListHighlightsResponse = ctx
        .story(
            "ListHighlights",
            Some(viewer),
            &ListHighlightsRequest { user_id: owner.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    response.highlights
}

async fn get(
    ctx: &Ctx,
    viewer: &TestUser,
    highlight_id: &str,
) -> Result<GetHighlightResponse, tokuly_testkit::RpcError> {
    ctx.story(
        "GetHighlight",
        Some(viewer),
        &GetHighlightRequest { highlight_id: highlight_id.to_owned(), ..Default::default() },
    )
    .await
}

fn titles(highlights: &[Highlight]) -> Vec<&str> {
    highlights.iter().map(|highlight| highlight.title.as_str()).collect()
}

fn story_titles(response: &GetHighlightResponse) -> Vec<&str> {
    response.stories.iter().map(|story| story.title.as_str()).collect()
}

#[tokio::test]
async fn highlights_are_created_changed_ordered_and_removed_by_their_owner() {
    let ctx = start().await;
    let (alice, friend) = (ctx.app.user("alice").await, ctx.app.user("friend").await);
    ctx.befriend(&alice, &friend).await;
    let (s1, s2, s3) =
        (ctx.post_story(&alice, "s1").await, ctx.post_story(&alice, "s2").await, ctx.post_story(&alice, "s3").await);
    let friends_story = ctx.post_story(&friend, "not mine").await;

    let summer = create(&ctx, &alice, "  夏の思い出  ", &[&s2, &s1, &s2]).await;
    assert_eq!(summer.title, "夏の思い出");
    assert_eq!(summer.owner_id, alice.id.to_string());
    assert_eq!(summer.story_count, 2, "a story is in a highlight once");
    assert_eq!(summer.cover.asset_id, s2.items[0].media.asset_id, "the cover is the first story's first photo");
    assert!(summer.updated_at.as_option().is_some());
    let shown = get(&ctx, &alice, &summer.id).await.unwrap();
    assert_eq!(story_titles(&shown), vec!["s2", "s1"], "stories play in the order given");
    assert!(shown.stories.iter().all(|story| story.highlighted));
    assert!(!ctx.get_story(&alice, &s3.id).await.unwrap().highlighted);

    let request = |title: &str, stories: &[&Story]| CreateHighlightRequest {
        title: title.to_owned(),
        story_ids: ids(stories),
        ..Default::default()
    };
    assert_eq!(ctx.story_reason("CreateHighlight", None, &request("x", &[&s1])).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.story_reason("CreateHighlight", Some(&alice), &request("  ", &[&s1])).await, "TITLE_INVALID");
    assert_eq!(
        ctx.story_reason("CreateHighlight", Some(&alice), &request(&"あ".repeat(31), &[&s1])).await,
        "TITLE_INVALID"
    );
    assert_eq!(ctx.story_reason("CreateHighlight", Some(&alice), &request("x", &[])).await, "HIGHLIGHT_EMPTY");
    let many = CreateHighlightRequest {
        title: "x".into(),
        story_ids: (0..101).map(|_| Uuid::now_v7().to_string()).collect(),
        ..Default::default()
    };
    assert_eq!(ctx.story_reason("CreateHighlight", Some(&alice), &many).await, "TOO_MANY_STORIES");
    assert_eq!(
        ctx.story_reason("CreateHighlight", Some(&alice), &request("x", &[&s1, &friends_story])).await,
        "STORY_NOT_FOUND"
    );
    let malformed = CreateHighlightRequest { title: "x".into(), story_ids: vec!["nope".into()], ..Default::default() };
    assert_eq!(ctx.story_reason("CreateHighlight", Some(&alice), &malformed).await, "INVALID_ID");

    // Updating replaces title and stories.
    let update = |highlight_id: &str, title: &str, stories: &[&Story]| UpdateHighlightRequest {
        highlight_id: highlight_id.to_owned(),
        title: title.to_owned(),
        story_ids: ids(stories),
        ..Default::default()
    };
    let changed: UpdateHighlightResponse =
        ctx.story("UpdateHighlight", Some(&alice), &update(&summer.id, "夏", &[&s3, &s2])).await.unwrap();
    assert_eq!((changed.highlight.title.as_str(), changed.highlight.story_count), ("夏", 2));
    assert_eq!(changed.highlight.cover.asset_id, s3.items[0].media.asset_id);
    assert_eq!(story_titles(&get(&ctx, &alice, &summer.id).await.unwrap()), vec!["s3", "s2"]);
    assert_eq!(
        ctx.story_reason("UpdateHighlight", Some(&friend), &update(&summer.id, "mine now", &[&friends_story])).await,
        "HIGHLIGHT_NOT_FOUND"
    );
    assert_eq!(
        ctx.story_reason("UpdateHighlight", Some(&alice), &update(&Uuid::now_v7().to_string(), "x", &[&s1])).await,
        "HIGHLIGHT_NOT_FOUND"
    );
    assert_eq!(
        ctx.story_reason("UpdateHighlight", Some(&alice), &update(&summer.id, "", &[&s1])).await,
        "TITLE_INVALID"
    );
    assert_eq!(
        ctx.story_reason("UpdateHighlight", Some(&alice), &update(&summer.id, "x", &[])).await,
        "HIGHLIGHT_EMPTY"
    );
    assert_eq!(
        ctx.story_reason("UpdateHighlight", Some(&alice), &update(&summer.id, "x", &[&friends_story])).await,
        "STORY_NOT_FOUND"
    );
    assert_eq!(get(&ctx, &alice, &summer.id).await.unwrap().highlight.title, "夏", "a refused update changes nothing");

    // New highlights go to the end; the owner sets the order.
    let trip = create(&ctx, &alice, "旅行", &[&s1]).await;
    let food = create(&ctx, &alice, "ごはん", &[&s1, &s3]).await;
    assert_eq!(titles(&list(&ctx, &alice, &alice).await), vec!["夏", "旅行", "ごはん"]);
    let reorder = |order: &[&Highlight]| ReorderHighlightsRequest {
        highlight_ids: order.iter().map(|highlight| highlight.id.clone()).collect(),
        ..Default::default()
    };
    let _: ReorderHighlightsResponse =
        ctx.story("ReorderHighlights", Some(&alice), &reorder(&[&food, &summer, &trip])).await.unwrap();
    assert_eq!(titles(&list(&ctx, &friend, &alice).await), vec!["ごはん", "夏", "旅行"]);
    assert_eq!(
        ctx.story_reason("ReorderHighlights", Some(&alice), &reorder(&[&food, &summer])).await,
        "HIGHLIGHTS_MISMATCH"
    );
    assert_eq!(
        ctx.story_reason("ReorderHighlights", Some(&alice), &reorder(&[&food, &summer, &trip, &trip])).await,
        "HIGHLIGHTS_MISMATCH"
    );
    assert_eq!(
        ctx.story_reason("ReorderHighlights", Some(&friend), &reorder(&[&food, &summer, &trip])).await,
        "HIGHLIGHTS_MISMATCH"
    );
    let malformed = ReorderHighlightsRequest { highlight_ids: vec!["nope".into()], ..Default::default() };
    assert_eq!(ctx.story_reason("ReorderHighlights", Some(&alice), &malformed).await, "HIGHLIGHTS_MISMATCH");
    assert_eq!(ctx.story_reason("ReorderHighlights", None, &reorder(&[&food])).await, "SIGN_IN_REQUIRED");

    // Deleting a highlight keeps its stories.
    let delete =
        |highlight_id: &str| DeleteHighlightRequest { highlight_id: highlight_id.to_owned(), ..Default::default() };
    assert_eq!(ctx.story_reason("DeleteHighlight", Some(&friend), &delete(&trip.id)).await, "HIGHLIGHT_NOT_FOUND");
    assert_eq!(ctx.story_reason("DeleteHighlight", None, &delete(&trip.id)).await, "SIGN_IN_REQUIRED");
    let _: DeleteHighlightResponse = ctx.story("DeleteHighlight", Some(&alice), &delete(&trip.id)).await.unwrap();
    assert_eq!(ctx.story_reason("DeleteHighlight", Some(&alice), &delete(&trip.id)).await, "HIGHLIGHT_NOT_FOUND");
    assert_eq!(titles(&list(&ctx, &alice, &alice).await), vec!["ごはん", "夏"]);
    assert!(ctx.get_story(&alice, &s1.id).await.is_ok());

    // Deleting a story takes it out of its highlights; a highlight left empty goes with it.
    let _: DeleteStoryResponse = ctx
        .story("DeleteStory", Some(&alice), &DeleteStoryRequest { story_id: s3.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(story_titles(&get(&ctx, &alice, &food.id).await.unwrap()), vec!["s1"]);
    let _: DeleteStoryResponse = ctx
        .story("DeleteStory", Some(&alice), &DeleteStoryRequest { story_id: s2.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(titles(&list(&ctx, &alice, &alice).await), vec!["ごはん"]);
    assert_eq!(get(&ctx, &alice, &summer.id).await.unwrap_err().reason, "HIGHLIGHT_NOT_FOUND");
}

#[tokio::test]
async fn a_highlight_shows_each_viewer_the_stories_they_may_see() {
    let ctx = start().await;
    let (alice, bob, carol, stranger) = (
        ctx.app.user("alice").await,
        ctx.app.user("bob").await,
        ctx.app.user("carol").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&alice, &bob).await;
    ctx.befriend(&alice, &carol).await;
    let open = ctx.post_story(&alice, "for all friends").await;
    let chosen = ctx
        .story_with(&alice, "for bob", 1, |r| {
            r.audience = StoryAudience::STORY_AUDIENCE_CLOSE_FRIENDS.into();
            r.audience_user_ids = vec![bob.id.to_string()];
        })
        .await;
    // Both stories are long expired: only highlights keep them visible.
    ctx.age_story(&open, 48).await;
    ctx.age_story(&chosen, 48).await;
    assert_eq!(ctx.get_story(&bob, &open.id).await.unwrap_err().reason, "STORY_NOT_FOUND");

    let mixed = create(&ctx, &alice, "mixed", &[&chosen, &open]).await;
    let secret = create(&ctx, &alice, "secret", &[&chosen]).await;
    assert_eq!(mixed.story_count, 2);

    // Bob was chosen: he sees everything.
    assert_eq!(titles(&list(&ctx, &bob, &alice).await), vec!["mixed", "secret"]);
    assert_eq!(story_titles(&get(&ctx, &bob, &mixed.id).await.unwrap()), vec!["for bob", "for all friends"]);
    assert!(ctx.get_story(&bob, &open.id).await.unwrap().expired, "a highlighted story can be opened again");
    // Carol is a friend who was not chosen: one story, its photo as the cover, and no second highlight.
    let for_carol = list(&ctx, &carol, &alice).await;
    assert_eq!(titles(&for_carol), vec!["mixed"]);
    assert_eq!(for_carol[0].story_count, 1);
    assert_eq!(for_carol[0].cover.asset_id, open.items[0].media.asset_id);
    assert_eq!(story_titles(&get(&ctx, &carol, &mixed.id).await.unwrap()), vec!["for all friends"]);
    assert_eq!(get(&ctx, &carol, &secret.id).await.unwrap_err().reason, "HIGHLIGHT_NOT_FOUND");
    assert_eq!(ctx.get_story(&carol, &chosen.id).await.unwrap_err().reason, "STORY_NOT_FOUND");
    // Strangers see no highlights at all.
    assert!(list(&ctx, &stranger, &alice).await.is_empty());
    assert_eq!(get(&ctx, &stranger, &mixed.id).await.unwrap_err().reason, "HIGHLIGHT_NOT_FOUND");
    assert_eq!(
        ctx.story_reason(
            "ListHighlights",
            None,
            &ListHighlightsRequest { user_id: alice.id.to_string(), ..Default::default() }
        )
        .await,
        "SIGN_IN_REQUIRED"
    );
    assert_eq!(
        ctx.story_reason(
            "GetHighlight",
            None,
            &GetHighlightRequest { highlight_id: mixed.id.clone(), ..Default::default() }
        )
        .await,
        "SIGN_IN_REQUIRED"
    );
    assert!(list(&ctx, &bob, &stranger).await.is_empty());

    // Taking a story out of every highlight hides it from friends again.
    let _: DeleteHighlightResponse = ctx
        .story(
            "DeleteHighlight",
            Some(&alice),
            &DeleteHighlightRequest { highlight_id: mixed.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(ctx.get_story(&bob, &open.id).await.unwrap_err().reason, "STORY_NOT_FOUND");
    assert!(ctx.get_story(&bob, &chosen.id).await.is_ok(), "still in the other highlight");
    // A block hides highlights like everything else.
    ctx.block(&alice, &bob).await;
    assert!(list(&ctx, &bob, &alice).await.is_empty());
}

#[tokio::test]
async fn a_profile_holds_a_limited_number_of_highlights() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let story = ctx.post_story(&alice, "s").await;
    for n in 0..50 {
        create(&ctx, &alice, &format!("h{n}"), &[&story]).await;
    }
    let one_more =
        CreateHighlightRequest { title: "h50".into(), story_ids: vec![story.id.clone()], ..Default::default() };
    assert_eq!(ctx.story_reason("CreateHighlight", Some(&alice), &one_more).await, "TOO_MANY_HIGHLIGHTS");
    assert_eq!(list(&ctx, &alice, &alice).await.len(), 50);
}
