//! `ReactionService`, `BookmarkService`, `EngagementService` and
//! `ReportService`.
mod common;

use common::*;
use tokuly_engagement::EngagementConfig;
use tokuly_proto::{
    common::{SubjectRef, SubjectType},
    engagement::{__buffa::oneof::engagement_event::Event as Pushed, *},
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

fn set(subject: SubjectRef, reaction: Reaction) -> SetReactionRequest {
    SetReactionRequest { subject: subject.into(), reaction: reaction.into(), ..Default::default() }
}

fn unreact(subject: SubjectRef) -> RemoveReactionRequest {
    RemoveReactionRequest { subject: subject.into(), ..Default::default() }
}

fn missing_video() -> SubjectRef {
    SubjectRef { r#type: SubjectType::SUBJECT_TYPE_VIDEO.into(), id: Uuid::now_v7().to_string(), ..Default::default() }
}

fn unserved() -> SubjectRef {
    SubjectRef { r#type: SubjectType::SUBJECT_TYPE_PLACE.into(), id: Uuid::now_v7().to_string(), ..Default::default() }
}

// ───────────────────────────── reactions ─────────────────────────────

#[tokio::test]
async fn one_reaction_per_user_moves_the_counters() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "夏の思い出");
    let mut topic = ctx.app.listen_topic(&video.topic()).await;

    // A like, by short ID.
    let liked: SetReactionResponse =
        ctx.call("ReactionService", "SetReaction", Some(&bob), &set(video.short_ref(), like())).await.unwrap();
    assert_eq!((liked.reactions.like_count, liked.reactions.dislike_count, liked.reactions.emoji_total), (1, 0, 0));
    assert_eq!(liked.reactions.viewer_reaction.kind, ReactionKind::REACTION_KIND_LIKE);
    assert_eq!(ctx.videos.counter(video.id, "like_count"), 1);
    // The first push carries the new totals at once.
    let events = drain(&mut topic).await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Pushed::CountersChanged(counters) if counters.reactions.like_count == 1))
    );

    // Liking again changes nothing.
    let again = ctx.react(&bob, video.subject_ref(), like()).await;
    assert_eq!(again.like_count, 1);
    assert_eq!(ctx.videos.counter(video.id, "like_count"), 1);

    // A different reaction replaces the old one.
    let disliked = ctx.react(&bob, video.subject_ref(), dislike()).await;
    assert_eq!((disliked.like_count, disliked.dislike_count), (0, 1));
    assert_eq!((ctx.videos.counter(video.id, "like_count"), ctx.videos.counter(video.id, "dislike_count")), (0, 1));
    let partied = ctx.react(&bob, video.subject_ref(), emoji("🎉")).await;
    assert_eq!((partied.dislike_count, partied.emoji_total), (0, 1));
    assert_eq!(
        partied.emoji_counts.iter().map(|count| (count.emoji.as_str(), count.count)).collect::<Vec<_>>(),
        [("🎉", 1)]
    );
    assert_eq!(partied.viewer_reaction.emoji, "🎉");
    assert_eq!((ctx.videos.counter(video.id, "dislike_count"), ctx.videos.counter(video.id, "reaction_count")), (0, 1));

    // Several people, several emoji: the most used first.
    ctx.react(&carol, video.subject_ref(), emoji("🎉")).await;
    let from_owner = ctx.react(&owner, video.subject_ref(), emoji("🔥")).await;
    assert_eq!(
        from_owner.emoji_counts.iter().map(|count| (count.emoji.as_str(), count.count)).collect::<Vec<_>>(),
        [("🎉", 2), ("🔥", 1)]
    );
    assert_eq!(from_owner.emoji_total, 3);

    // Guests see the counts without a reaction of their own.
    let for_guest = ctx.engagement_of(None, &video).await;
    assert_eq!(for_guest.reactions.emoji_total, 3);
    assert!(for_guest.reactions.viewer_reaction.as_option().is_none());

    // Taking it back; doing so twice is fine.
    let removed: RemoveReactionResponse =
        ctx.call("ReactionService", "RemoveReaction", Some(&bob), &unreact(video.subject_ref())).await.unwrap();
    assert_eq!(removed.reactions.emoji_total, 2);
    assert!(removed.reactions.viewer_reaction.as_option().is_none());
    let twice: RemoveReactionResponse =
        ctx.call("ReactionService", "RemoveReaction", Some(&bob), &unreact(video.subject_ref())).await.unwrap();
    assert_eq!(twice.reactions.emoji_total, 2);
    assert_eq!(ctx.videos.counter(video.id, "reaction_count"), 2);

    // The owner heard about bob's like and emoji once (collapsed per
    // subject, and not again for the same subject), never about the dislike.
    let notices = ctx.notifications(owner.id, "engagement.like").await;
    let from_bob: Vec<_> = notices.iter().filter(|notice| notice["actor_id"] == serde_json::json!(bob.id)).collect();
    assert_eq!(from_bob.len(), 1);
    assert_eq!(from_bob[0]["title"]["ja"], "Bobがいいねしたよ!");
    assert_eq!(from_bob[0]["body"]["ja"], "夏の思い出");
    assert_eq!(from_bob[0]["collapse_key"], format!("engagement.like:5:{}", video.id));
    let from_carol: Vec<_> =
        notices.iter().filter(|notice| notice["actor_id"] == serde_json::json!(carol.id)).collect();
    assert_eq!(from_carol[0]["title"]["en"], "Carol reacted 🎉");
    assert_eq!(notices.len(), 2, "the owner's own reaction tells nobody");

    // Every change is an event other domains can follow.
    let changes = ctx.events_about("engagement.reaction.changed", video.id).await;
    let by_bob: Vec<(i64, i64)> = changes
        .iter()
        .filter(|change| change["user_id"] == serde_json::json!(bob.id))
        .map(|change| (change["previous_kind"].as_i64().unwrap(), change["kind"].as_i64().unwrap()))
        .collect();
    assert_eq!(by_bob, [(0, 1), (1, 2), (2, 3), (3, 0)]);
}

#[tokio::test]
async fn reactions_are_validated() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let reason = |request: SetReactionRequest| {
        let ctx = &ctx;
        let bob = &bob;
        async move { ctx.reason("ReactionService", "SetReaction", Some(bob), &request).await }
    };
    assert_eq!(
        ctx.reason("ReactionService", "SetReaction", None, &set(video.subject_ref(), like())).await,
        "SIGN_IN_REQUIRED"
    );
    assert_eq!(reason(set(video.subject_ref(), Reaction::default())).await, "REACTION_INVALID");
    assert_eq!(
        reason(SetReactionRequest { subject: video.subject_ref().into(), ..Default::default() }).await,
        "REACTION_INVALID"
    );
    assert_eq!(reason(set(video.subject_ref(), emoji("すき"))).await, "EMOJI_INVALID");
    assert_eq!(reason(set(video.subject_ref(), emoji("🎉🎉"))).await, "EMOJI_INVALID");
    assert_eq!(reason(set(missing_video(), like())).await, "SUBJECT_NOT_FOUND");
    assert_eq!(reason(set(unserved(), like())).await, "UNSUPPORTED_SUBJECT");
    assert_eq!(
        reason(SetReactionRequest { reaction: like().into(), ..Default::default() }).await,
        "UNSUPPORTED_SUBJECT"
    );

    let closed = ctx.video(&owner, "closed");
    ctx.videos.update(closed.id, |subject| subject.interactions_enabled = false);
    assert_eq!(reason(set(closed.subject_ref(), like())).await, "INTERACTIONS_DISABLED");
    let hidden = ctx.video(&owner, "hidden");
    ctx.videos.update(hidden.id, |subject| subject.audience = Audience::Nobody);
    assert_eq!(reason(set(hidden.subject_ref(), like())).await, "SUBJECT_NOT_FOUND");

    assert_eq!(
        ctx.reason("ReactionService", "RemoveReaction", None, &unreact(video.subject_ref())).await,
        "SIGN_IN_REQUIRED"
    );
    assert_eq!(
        ctx.reason("ReactionService", "RemoveReaction", Some(&bob), &unreact(unserved())).await,
        "UNSUPPORTED_SUBJECT"
    );
    let by_unknown_short_id =
        SubjectRef { r#type: SubjectType::SUBJECT_TYPE_VIDEO.into(), id: "nosuchvideo".into(), ..Default::default() };
    assert_eq!(
        ctx.reason("ReactionService", "RemoveReaction", Some(&bob), &unreact(by_unknown_short_id)).await,
        "SUBJECT_NOT_FOUND"
    );
}

#[tokio::test]
async fn a_reaction_can_be_taken_back_after_the_subject_went_out_of_sight() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    ctx.react(&bob, video.subject_ref(), like()).await;
    ctx.videos.update(video.id, |subject| subject.audience = Audience::Nobody);

    // By UUID it still works, and the counter follows.
    let removed: RemoveReactionResponse =
        ctx.call("ReactionService", "RemoveReaction", Some(&bob), &unreact(video.subject_ref())).await.unwrap();
    assert_eq!(removed.reactions.like_count, 0);
    assert_eq!(ctx.videos.counter(video.id, "like_count"), 0);
    // Reacting anew does not.
    assert_eq!(
        ctx.reason("ReactionService", "SetReaction", Some(&bob), &set(video.subject_ref(), like())).await,
        "SUBJECT_NOT_FOUND"
    );
}

#[tokio::test]
async fn comments_take_likes_and_dislikes() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&bob, &video, "いいねしてね").await;

    let liked = ctx.react(&carol, comment_ref(&comment.id), like()).await;
    assert_eq!(liked.like_count, 1);
    let seen = ctx.get_comment(Some(&carol), &comment.id).await.unwrap();
    assert_eq!((seen.comment.like_count, seen.comment.dislike_count), (1, 0));
    assert_eq!(seen.comment.viewer.reaction.kind, ReactionKind::REACTION_KIND_LIKE);
    // The comment's author hears about the like; the path leads to the comment.
    let notices = ctx.notifications(bob.id, "engagement.like").await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0]["title"]["en"], "Carol liked your comment");
    assert_eq!(notices[0]["path"], format!("{}?comment={}", video.path, comment.id));

    ctx.react(&carol, comment_ref(&comment.id), dislike()).await;
    ctx.react(&owner, comment_ref(&comment.id), like()).await;
    let seen = ctx.get_comment(Some(&bob), &comment.id).await.unwrap();
    assert_eq!((seen.comment.like_count, seen.comment.dislike_count), (1, 1));
    assert!(seen.comment.viewer.reaction.as_option().is_none());

    // Emoji are for content, not for comments.
    let party = set(comment_ref(&comment.id), emoji("🎉"));
    assert_eq!(ctx.reason("ReactionService", "SetReaction", Some(&carol), &party).await, "EMOJI_NOT_ALLOWED");
    // A held comment cannot be liked, even by those who see it.
    ctx.set_rules(&owner, true, false, &[]).await;
    let held = ctx.comment(&bob, &video, "まだ承認待ち").await;
    assert_eq!(
        ctx.reason("ReactionService", "SetReaction", Some(&owner), &set(comment_ref(&held.id), like())).await,
        "INTERACTIONS_DISABLED"
    );
    assert_eq!(
        ctx.reason("ReactionService", "SetReaction", Some(&carol), &set(comment_ref(&held.id), like())).await,
        "SUBJECT_NOT_FOUND"
    );

    let _: RemoveReactionResponse =
        ctx.call("ReactionService", "RemoveReaction", Some(&carol), &unreact(comment_ref(&comment.id))).await.unwrap();
    assert_eq!(ctx.get_comment(None, &comment.id).await.unwrap().comment.dislike_count, 0);
}

#[tokio::test]
async fn who_reacted_is_public_for_emoji_and_private_for_likes() {
    let ctx = start().await;
    let (owner, bob, carol, dave) = (
        ctx.app.user("Owner").await,
        ctx.app.user("Bob").await,
        ctx.app.user("Carol").await,
        ctx.app.user("Dave").await,
    );
    let video = ctx.video(&owner, "v");
    ctx.react(&bob, video.subject_ref(), like()).await;
    ctx.react(&carol, video.subject_ref(), emoji("🎉")).await;
    ctx.react(&dave, video.subject_ref(), emoji("🔥")).await;
    ctx.react(&owner, video.subject_ref(), dislike()).await;

    let list = |viewer: Option<&TestUser>, kind: ReactionKind, emoji: &str| {
        let ctx = &ctx;
        let request = ListReactionsRequest {
            subject: video.subject_ref().into(),
            kind: kind.into(),
            emoji: emoji.to_owned(),
            ..Default::default()
        };
        let viewer = viewer.cloned();
        async move {
            ctx.call::<_, ListReactionsResponse>("ReactionService", "ListReactions", viewer.as_ref(), &request).await
        }
    };
    let handles = |response: &ListReactionsResponse| {
        response.reactors.iter().map(|reactor| reactor.user.handle.clone()).collect::<Vec<_>>()
    };

    // Emoji reactions: anyone, most recent first, optionally one emoji.
    let everyone = list(None, ReactionKind::REACTION_KIND_UNSPECIFIED, "").await.unwrap();
    assert_eq!(handles(&everyone), [dave.handle.clone(), carol.handle.clone()]);
    assert_eq!(everyone.reactors[0].reaction.emoji, "🔥");
    let fire = list(Some(&bob), ReactionKind::REACTION_KIND_EMOJI, "🔥").await.unwrap();
    assert_eq!(handles(&fire), [dave.handle.clone()]);

    // Likes: the owner only. Dislikes: nobody.
    assert_eq!(
        list(Some(&bob), ReactionKind::REACTION_KIND_LIKE, "").await.unwrap_err().reason,
        "REACTION_LIST_PRIVATE"
    );
    assert_eq!(list(None, ReactionKind::REACTION_KIND_LIKE, "").await.unwrap_err().reason, "REACTION_LIST_PRIVATE");
    assert_eq!(
        list(Some(&owner), ReactionKind::REACTION_KIND_DISLIKE, "").await.unwrap_err().reason,
        "REACTION_LIST_PRIVATE"
    );
    let likers = list(Some(&owner), ReactionKind::REACTION_KIND_LIKE, "").await.unwrap();
    assert_eq!(handles(&likers), [bob.handle.clone()]);
    let for_owner = list(Some(&owner), ReactionKind::REACTION_KIND_UNSPECIFIED, "").await.unwrap();
    assert_eq!(handles(&for_owner), [dave.handle.clone(), carol.handle.clone(), bob.handle.clone()]);

    // In pages.
    let paged = ListReactionsRequest { subject: video.subject_ref().into(), page: page(1, ""), ..Default::default() };
    let first: ListReactionsResponse = ctx.call("ReactionService", "ListReactions", None, &paged).await.unwrap();
    assert!(first.page.has_more);
    let next = ListReactionsRequest {
        subject: video.subject_ref().into(),
        page: page(1, &first.page.next_cursor),
        ..Default::default()
    };
    let second: ListReactionsResponse = ctx.call("ReactionService", "ListReactions", None, &next).await.unwrap();
    assert_eq!((handles(&first), handles(&second)), (vec![dave.handle.clone()], vec![carol.handle.clone()]));
    let bad = ListReactionsRequest { subject: video.subject_ref().into(), page: page(1, "nope"), ..Default::default() };
    assert_eq!(ctx.reason("ReactionService", "ListReactions", None, &bad).await, "INVALID_CURSOR");
    let gone = ListReactionsRequest { subject: missing_video().into(), ..Default::default() };
    assert_eq!(ctx.reason("ReactionService", "ListReactions", None, &gone).await, "SUBJECT_NOT_FOUND");

    // People in a block with the viewer are left out.
    ctx.block(&bob, &dave).await;
    let for_bob = list(Some(&bob), ReactionKind::REACTION_KIND_EMOJI, "").await.unwrap();
    assert_eq!(handles(&for_bob), [carol.handle.clone()]);
}

#[tokio::test]
async fn my_reactions_list_what_i_reacted_to() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let (video, liked_video, note) =
        (ctx.video(&owner, "動画A"), ctx.video(&owner, "動画B"), ctx.note(&owner, "ノート"));
    ctx.react(&bob, video.subject_ref(), emoji("🎉")).await;
    ctx.react(&bob, liked_video.subject_ref(), like()).await;
    ctx.react(&bob, note.subject_ref(), like()).await;

    let list = |subject_type: SubjectType, kind: ReactionKind| {
        let ctx = &ctx;
        let bob = &bob;
        let request =
            ListMyReactionsRequest { subject_type: subject_type.into(), kind: kind.into(), ..Default::default() };
        async move {
            ctx.call::<_, ListMyReactionsResponse>("ReactionService", "ListMyReactions", Some(bob), &request)
                .await
                .unwrap()
        }
    };
    let titles = |response: &ListMyReactionsResponse| {
        response.subjects.iter().map(|entry| entry.subject.title.clone()).collect::<Vec<_>>()
    };

    assert_eq!(
        ctx.reason("ReactionService", "ListMyReactions", None, &ListMyReactionsRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
    let all = list(SubjectType::SUBJECT_TYPE_UNSPECIFIED, ReactionKind::REACTION_KIND_UNSPECIFIED).await;
    assert_eq!(titles(&all), ["ノート", "動画B", "動画A"]);
    assert_eq!(all.subjects[2].reaction.emoji, "🎉");
    // "Liked videos".
    let liked_videos = list(SubjectType::SUBJECT_TYPE_VIDEO, ReactionKind::REACTION_KIND_LIKE).await;
    assert_eq!(titles(&liked_videos), ["動画B"]);
    assert_eq!(liked_videos.subjects[0].subject.path, liked_video.path);
    assert_eq!(
        titles(&list(SubjectType::SUBJECT_TYPE_NOTE, ReactionKind::REACTION_KIND_UNSPECIFIED).await),
        ["ノート"]
    );

    // What is gone stays listed as unavailable, so it can be removed.
    ctx.videos.remove(liked_video.id);
    let liked_videos = list(SubjectType::SUBJECT_TYPE_VIDEO, ReactionKind::REACTION_KIND_LIKE).await;
    assert!(!liked_videos.subjects[0].subject.available);
    assert_eq!(liked_videos.subjects[0].subject.subject.id, liked_video.id.to_string());

    let paged = ListMyReactionsRequest { page: page(2, ""), ..Default::default() };
    let first: ListMyReactionsResponse =
        ctx.call("ReactionService", "ListMyReactions", Some(&bob), &paged).await.unwrap();
    assert_eq!(first.subjects.len(), 2);
    let next = ListMyReactionsRequest { page: page(2, &first.page.next_cursor), ..Default::default() };
    let second: ListMyReactionsResponse =
        ctx.call("ReactionService", "ListMyReactions", Some(&bob), &next).await.unwrap();
    assert_eq!((second.subjects.len(), second.page.has_more), (1, false));
}

// ───────────────────────────── bookmarks ─────────────────────────────

fn save(subject: SubjectRef, collection_id: &str) -> AddBookmarkRequest {
    AddBookmarkRequest { subject: subject.into(), collection_id: collection_id.to_owned(), ..Default::default() }
}

fn unsave(subject: SubjectRef, collection_id: &str) -> RemoveBookmarkRequest {
    RemoveBookmarkRequest { subject: subject.into(), collection_id: collection_id.to_owned(), ..Default::default() }
}

impl Ctx {
    async fn collections(&self, user: &TestUser, subject: Option<SubjectRef>) -> Vec<Collection> {
        let request = ListCollectionsRequest { subject: subject.into(), ..Default::default() };
        let response: ListCollectionsResponse =
            self.call("BookmarkService", "ListCollections", Some(user), &request).await.unwrap();
        response.collections
    }

    async fn new_collection(&self, user: &TestUser, name: &str) -> Collection {
        let request = CreateCollectionRequest { name: name.to_owned(), ..Default::default() };
        let response: CreateCollectionResponse =
            self.call("BookmarkService", "CreateCollection", Some(user), &request).await.unwrap();
        response.collection.as_option().unwrap().clone()
    }

    async fn saved(&self, user: &TestUser, collection_id: &str, subject_type: SubjectType) -> Vec<String> {
        let request = ListBookmarksRequest {
            collection_id: collection_id.to_owned(),
            subject_type: subject_type.into(),
            ..Default::default()
        };
        let response: ListBookmarksResponse =
            self.call("BookmarkService", "ListBookmarks", Some(user), &request).await.unwrap();
        response.bookmarks.iter().map(|bookmark| bookmark.subject.title.clone()).collect()
    }
}

#[tokio::test]
async fn subjects_are_saved_into_collections() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let (video, note, other) = (ctx.video(&owner, "動画"), ctx.note(&owner, "ノート"), ctx.video(&owner, "べつの動画"));
    let any = SubjectType::SUBJECT_TYPE_UNSPECIFIED;

    // Nothing saved yet: an empty list, and one default collection.
    assert!(ctx.saved(&bob, "", any).await.is_empty());
    let initial = ctx.collections(&bob, None).await;
    assert_eq!(initial.len(), 1);
    assert!(initial[0].is_default && initial[0].name == "saved" && initial[0].item_count == 0);
    let default_id = initial[0].id.clone();

    // Saving without a collection goes to the default one; twice changes nothing.
    let added: AddBookmarkResponse =
        ctx.call("BookmarkService", "AddBookmark", Some(&bob), &save(video.short_ref(), "")).await.unwrap();
    assert_eq!(added.bookmark_count, 1);
    let again: AddBookmarkResponse =
        ctx.call("BookmarkService", "AddBookmark", Some(&bob), &save(video.subject_ref(), "")).await.unwrap();
    assert_eq!(again.bookmark_count, 1);
    assert_eq!(ctx.videos.counter(video.id, "bookmark_count"), 1);
    assert!(ctx.engagement_of(Some(&bob), &video).await.bookmarked);
    assert!(!ctx.engagement_of(Some(&carol), &video).await.bookmarked);
    assert!(!ctx.engagement_of(None, &video).await.bookmarked);

    // A named collection holds a subset; saving into it saves into the default one too.
    let recipes = ctx.new_collection(&bob, "あとで見る").await;
    assert!(!recipes.is_default && recipes.item_count == 0);
    let _: AddBookmarkResponse =
        ctx.call("BookmarkService", "AddBookmark", Some(&bob), &save(note.subject_ref(), &recipes.id)).await.unwrap();
    let _: AddBookmarkResponse =
        ctx.call("BookmarkService", "AddBookmark", Some(&bob), &save(video.subject_ref(), &recipes.id)).await.unwrap();
    let _: AddBookmarkResponse =
        ctx.call("BookmarkService", "AddBookmark", Some(&bob), &save(other.subject_ref(), "")).await.unwrap();
    assert_eq!(ctx.saved(&bob, "", any).await, ["べつの動画", "ノート", "動画"]);
    assert_eq!(ctx.saved(&bob, &default_id, any).await.len(), 3);
    assert_eq!(ctx.saved(&bob, &recipes.id, any).await, ["動画", "ノート"]);
    // By type.
    assert_eq!(ctx.saved(&bob, "", SubjectType::SUBJECT_TYPE_VIDEO).await, ["べつの動画", "動画"]);
    assert_eq!(ctx.saved(&bob, &recipes.id, SubjectType::SUBJECT_TYPE_NOTE).await, ["ノート"]);

    // The collections with their sizes, the default one first, and which ones hold a subject.
    let listed = ctx.collections(&bob, Some(note.subject_ref())).await;
    assert_eq!(
        listed
            .iter()
            .map(|collection| (collection.name.as_str(), collection.item_count, collection.contains_subject))
            .collect::<Vec<_>>(),
        [("saved", 3, true), ("あとで見る", 2, true)]
    );
    let for_other = ctx.collections(&bob, Some(other.subject_ref())).await;
    assert_eq!(for_other.iter().map(|collection| collection.contains_subject).collect::<Vec<_>>(), [true, false]);

    // Removing from a named collection keeps the subject saved...
    let from_named: RemoveBookmarkResponse = ctx
        .call("BookmarkService", "RemoveBookmark", Some(&bob), &unsave(note.subject_ref(), &recipes.id))
        .await
        .unwrap();
    assert!(from_named.bookmarked && from_named.bookmark_count == 1);
    assert_eq!(ctx.saved(&bob, &recipes.id, any).await, ["動画"]);
    assert_eq!(ctx.notes.counter(note.id, "bookmark_count"), 1);
    // ...removing without a collection (or from the default one) removes it everywhere.
    let everywhere: RemoveBookmarkResponse =
        ctx.call("BookmarkService", "RemoveBookmark", Some(&bob), &unsave(video.subject_ref(), "")).await.unwrap();
    assert!(!everywhere.bookmarked && everywhere.bookmark_count == 0);
    assert!(ctx.saved(&bob, &recipes.id, any).await.is_empty());
    assert_eq!(ctx.saved(&bob, "", any).await, ["べつの動画", "ノート"]);
    assert_eq!(ctx.videos.counter(video.id, "bookmark_count"), 0);
    let from_default: RemoveBookmarkResponse = ctx
        .call("BookmarkService", "RemoveBookmark", Some(&bob), &unsave(note.subject_ref(), &default_id))
        .await
        .unwrap();
    assert!(!from_default.bookmarked);
    let sizes = ctx.collections(&bob, None).await;
    assert_eq!(sizes.iter().map(|collection| collection.item_count).collect::<Vec<_>>(), [1, 0]);

    // A saved subject that went away is listed as unavailable and can still be removed by UUID.
    ctx.videos.remove(other.id);
    let request = ListBookmarksRequest::default();
    let stale: ListBookmarksResponse =
        ctx.call("BookmarkService", "ListBookmarks", Some(&bob), &request).await.unwrap();
    assert_eq!(stale.bookmarks.len(), 1);
    assert!(!stale.bookmarks[0].subject.available && stale.bookmarks[0].saved_at.as_option().is_some());
    let gone: RemoveBookmarkResponse =
        ctx.call("BookmarkService", "RemoveBookmark", Some(&bob), &unsave(other.subject_ref(), "")).await.unwrap();
    assert!(!gone.bookmarked);
    assert!(ctx.saved(&bob, "", any).await.is_empty());

    // Saving follows other domains as events.
    let changes = ctx.events_about("engagement.bookmark.changed", video.id).await;
    assert_eq!(changes.iter().map(|change| change["saved"].as_bool().unwrap()).collect::<Vec<_>>(), [true, false]);
}

#[tokio::test]
async fn bookmarks_and_collections_are_validated() {
    let ctx = start_with(EngagementConfig { max_collections: 3, max_bookmarks: 2, ..test_config() }).await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "v");
    let carols = ctx.new_collection(&carol, "キャロルの").await;
    let default_id = ctx.collections(&bob, None).await[0].id.clone();

    // Signed-in users only; own collections only.
    assert_eq!(
        ctx.reason("BookmarkService", "AddBookmark", None, &save(video.subject_ref(), "")).await,
        "SIGN_IN_REQUIRED"
    );
    assert_eq!(
        ctx.reason("BookmarkService", "RemoveBookmark", None, &unsave(video.subject_ref(), "")).await,
        "SIGN_IN_REQUIRED"
    );
    assert_eq!(
        ctx.reason("BookmarkService", "ListBookmarks", None, &ListBookmarksRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
    assert_eq!(
        ctx.reason("BookmarkService", "ListCollections", None, &ListCollectionsRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
    assert_eq!(
        ctx.reason("BookmarkService", "AddBookmark", Some(&bob), &save(video.subject_ref(), &carols.id)).await,
        "COLLECTION_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason("BookmarkService", "RemoveBookmark", Some(&bob), &unsave(video.subject_ref(), &carols.id)).await,
        "COLLECTION_NOT_FOUND"
    );
    let theirs = ListBookmarksRequest { collection_id: carols.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("BookmarkService", "ListBookmarks", Some(&bob), &theirs).await, "COLLECTION_NOT_FOUND");
    assert_eq!(
        ctx.reason("BookmarkService", "AddBookmark", Some(&bob), &save(video.subject_ref(), "nope")).await,
        "INVALID_ID"
    );
    assert_eq!(
        ctx.reason("BookmarkService", "AddBookmark", Some(&bob), &save(missing_video(), "")).await,
        "SUBJECT_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason("BookmarkService", "AddBookmark", Some(&bob), &save(unserved(), "")).await,
        "UNSUPPORTED_SUBJECT"
    );
    let with_subject = ListCollectionsRequest { subject: unserved().into(), ..Default::default() };
    assert_eq!(
        ctx.reason("BookmarkService", "ListCollections", Some(&bob), &with_subject).await,
        "UNSUPPORTED_SUBJECT"
    );
    let bad_cursor = ListBookmarksRequest { page: page(1, "nope"), ..Default::default() };
    assert_eq!(ctx.reason("BookmarkService", "ListBookmarks", Some(&bob), &bad_cursor).await, "INVALID_CURSOR");

    // Names: 1 to 50 characters, unique whatever the case or width.
    let create = |name: &str| CreateCollectionRequest { name: name.to_owned(), ..Default::default() };
    assert_eq!(ctx.reason("BookmarkService", "CreateCollection", None, &create("x")).await, "SIGN_IN_REQUIRED");
    assert_eq!(
        ctx.reason("BookmarkService", "CreateCollection", Some(&bob), &create("  ")).await,
        "COLLECTION_NAME_INVALID"
    );
    assert_eq!(
        ctx.reason("BookmarkService", "CreateCollection", Some(&bob), &create(&"あ".repeat(51))).await,
        "COLLECTION_NAME_INVALID"
    );
    assert_eq!(
        ctx.reason("BookmarkService", "CreateCollection", Some(&bob), &create("SAVED")).await,
        "COLLECTION_NAME_TAKEN"
    );
    let recipes = ctx.new_collection(&bob, "Recipes").await;
    assert_eq!(
        ctx.reason("BookmarkService", "CreateCollection", Some(&bob), &create("ｒｅｃｉｐｅｓ")).await,
        "COLLECTION_NAME_TAKEN"
    );
    let travel = ctx.new_collection(&bob, "旅行").await;
    assert_eq!(
        ctx.reason("BookmarkService", "CreateCollection", Some(&bob), &create("四つ目")).await,
        "COLLECTION_LIMIT"
    );

    // Renaming and deleting: not the default one, not someone else's.
    let rename = |id: &str, name: &str| UpdateCollectionRequest {
        collection_id: id.to_owned(),
        name: name.to_owned(),
        ..Default::default()
    };
    assert_eq!(
        ctx.reason("BookmarkService", "UpdateCollection", None, &rename(&recipes.id, "x")).await,
        "SIGN_IN_REQUIRED"
    );
    assert_eq!(
        ctx.reason("BookmarkService", "UpdateCollection", Some(&bob), &rename(&default_id, "x")).await,
        "DEFAULT_COLLECTION"
    );
    assert_eq!(
        ctx.reason("BookmarkService", "UpdateCollection", Some(&bob), &rename(&carols.id, "x")).await,
        "COLLECTION_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason("BookmarkService", "UpdateCollection", Some(&bob), &rename(&recipes.id, "")).await,
        "COLLECTION_NAME_INVALID"
    );
    assert_eq!(
        ctx.reason("BookmarkService", "UpdateCollection", Some(&bob), &rename(&recipes.id, "旅行")).await,
        "COLLECTION_NAME_TAKEN"
    );
    let renamed: UpdateCollectionResponse =
        ctx.call("BookmarkService", "UpdateCollection", Some(&bob), &rename(&recipes.id, "レシピ")).await.unwrap();
    assert_eq!(renamed.collection.name, "レシピ");
    // Changing only the case of its own name is fine.
    let _: UpdateCollectionResponse =
        ctx.call("BookmarkService", "UpdateCollection", Some(&bob), &rename(&travel.id, "旅行")).await.unwrap();

    // A full collection takes no more.
    let (second, third) = (ctx.video(&owner, "2"), ctx.video(&owner, "3"));
    for subject in [&video, &second] {
        let _: AddBookmarkResponse = ctx
            .call("BookmarkService", "AddBookmark", Some(&bob), &save(subject.subject_ref(), &recipes.id))
            .await
            .unwrap();
    }
    assert_eq!(
        ctx.reason("BookmarkService", "AddBookmark", Some(&bob), &save(third.subject_ref(), "")).await,
        "BOOKMARK_LIMIT"
    );

    // Deleting a collection leaves what it held saved.
    let delete = |id: &str| DeleteCollectionRequest { collection_id: id.to_owned(), ..Default::default() };
    assert_eq!(ctx.reason("BookmarkService", "DeleteCollection", None, &delete(&recipes.id)).await, "SIGN_IN_REQUIRED");
    assert_eq!(
        ctx.reason("BookmarkService", "DeleteCollection", Some(&bob), &delete(&default_id)).await,
        "DEFAULT_COLLECTION"
    );
    assert_eq!(
        ctx.reason("BookmarkService", "DeleteCollection", Some(&bob), &delete(&carols.id)).await,
        "COLLECTION_NOT_FOUND"
    );
    let _: DeleteCollectionResponse =
        ctx.call("BookmarkService", "DeleteCollection", Some(&bob), &delete(&recipes.id)).await.unwrap();
    assert_eq!(
        ctx.reason("BookmarkService", "DeleteCollection", Some(&bob), &delete(&recipes.id)).await,
        "COLLECTION_NOT_FOUND"
    );
    assert_eq!(ctx.saved(&bob, "", SubjectType::SUBJECT_TYPE_UNSPECIFIED).await.len(), 2);
    assert!(ctx.engagement_of(Some(&bob), &video).await.bookmarked);
    assert_eq!(ctx.engagement_of(Some(&bob), &video).await.bookmark_count, 1);
}

// ───────────────────────────── engagement and shares ─────────────────────────────

#[tokio::test]
async fn engagement_gathers_every_number_of_a_subject() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let (video, note, hidden, closed) = (
        ctx.video(&owner, "動画"),
        ctx.note(&owner, "ノート"),
        ctx.video(&owner, "非公開"),
        ctx.video(&owner, "コメントオフ"),
    );
    ctx.videos.update(hidden.id, |subject| subject.audience = Audience::Only(vec![carol.id]));
    ctx.videos.update(closed.id, |subject| subject.interactions_enabled = false);
    ctx.react(&bob, video.subject_ref(), like()).await;
    ctx.react(&carol, video.subject_ref(), emoji("🎉")).await;
    let top = ctx.comment(&carol, &video, "コメント").await;
    ctx.reply(&bob, &top.id, "返信").await;
    let _: AddBookmarkResponse =
        ctx.call("BookmarkService", "AddBookmark", Some(&bob), &save(video.subject_ref(), "")).await.unwrap();
    let share = RecordShareRequest {
        subject: video.subject_ref().into(),
        target: ShareTarget::SHARE_TARGET_CHAT.into(),
        ..Default::default()
    };
    let _: RecordShareResponse = ctx.call("EngagementService", "RecordShare", Some(&bob), &share).await.unwrap();

    // One subject, by short ID: every number and the caller's own state.
    let request = GetEngagementRequest { subject: video.short_ref().into(), ..Default::default() };
    let one: GetEngagementResponse =
        ctx.call("EngagementService", "GetEngagement", Some(&bob), &request).await.unwrap();
    let engagement = &one.engagement;
    assert_eq!(
        (engagement.subject.id.as_str(), engagement.requested_id.as_str()),
        (video.id.to_string().as_str(), video.short_id.as_str())
    );
    assert_eq!((engagement.reactions.like_count, engagement.reactions.emoji_total), (1, 1));
    assert_eq!(engagement.reactions.viewer_reaction.kind, ReactionKind::REACTION_KIND_LIKE);
    assert_eq!((engagement.comment_count, engagement.bookmark_count, engagement.share_count), (2, 1, 1));
    assert!(engagement.bookmarked && engagement.interactions_enabled);
    // A guest gets the same numbers and no state.
    let guest = ctx.engagement_of(None, &video).await;
    assert_eq!((guest.comment_count, guest.bookmarked), (2, false));
    assert!(guest.reactions.viewer_reaction.as_option().is_none());
    assert!(!ctx.engagement_of(Some(&bob), &closed).await.interactions_enabled);

    assert_eq!(
        ctx.reason(
            "EngagementService",
            "GetEngagement",
            None,
            &GetEngagementRequest { subject: missing_video().into(), ..Default::default() }
        )
        .await,
        "SUBJECT_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(
            "EngagementService",
            "GetEngagement",
            Some(&bob),
            &GetEngagementRequest { subject: hidden.subject_ref().into(), ..Default::default() }
        )
        .await,
        "SUBJECT_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(
            "EngagementService",
            "GetEngagement",
            None,
            &GetEngagementRequest { subject: unserved().into(), ..Default::default() }
        )
        .await,
        "UNSUPPORTED_SUBJECT"
    );
    assert_eq!(
        ctx.reason("EngagementService", "GetEngagement", None, &GetEngagementRequest::default()).await,
        "UNSUPPORTED_SUBJECT"
    );

    // Many at once, in request order, without what the caller cannot see.
    let batch = BatchGetEngagementRequest {
        subjects: vec![
            note.subject_ref(),
            hidden.subject_ref(),
            missing_video(),
            video.short_ref(),
            comment_ref(&top.id),
        ],
        ..Default::default()
    };
    let for_bob: BatchGetEngagementResponse =
        ctx.call("EngagementService", "BatchGetEngagement", Some(&bob), &batch).await.unwrap();
    assert_eq!(
        for_bob.engagements.iter().map(|engagement| engagement.subject.id.clone()).collect::<Vec<_>>(),
        [note.id.to_string(), video.id.to_string(), top.id.clone()]
    );
    assert_eq!(for_bob.engagements[1].requested_id, video.short_id);
    assert_eq!((for_bob.engagements[1].comment_count, for_bob.engagements[1].bookmarked), (2, true));
    assert_eq!(for_bob.engagements[0].comment_count, 0);
    let for_carol: BatchGetEngagementResponse =
        ctx.call("EngagementService", "BatchGetEngagement", Some(&carol), &batch).await.unwrap();
    assert_eq!(for_carol.engagements.len(), 4, "carol may see the private video");
    let for_guest: BatchGetEngagementResponse =
        ctx.call("EngagementService", "BatchGetEngagement", None, &batch).await.unwrap();
    assert_eq!(for_guest.engagements.len(), 3);

    let too_many = BatchGetEngagementRequest { subjects: vec![video.subject_ref(); 101], ..Default::default() };
    assert_eq!(ctx.reason("EngagementService", "BatchGetEngagement", None, &too_many).await, "TOO_MANY_SUBJECTS");
    let with_unserved =
        BatchGetEngagementRequest { subjects: vec![video.subject_ref(), unserved()], ..Default::default() };
    assert_eq!(
        ctx.reason("EngagementService", "BatchGetEngagement", None, &with_unserved).await,
        "UNSUPPORTED_SUBJECT"
    );
    let empty: BatchGetEngagementResponse =
        ctx.call("EngagementService", "BatchGetEngagement", None, &BatchGetEngagementRequest::default()).await.unwrap();
    assert!(empty.engagements.is_empty());
}

#[tokio::test]
async fn shares_are_counted_by_target() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let share = |target: ShareTarget| RecordShareRequest {
        subject: video.subject_ref().into(),
        target: target.into(),
        ..Default::default()
    };

    // Signed-in users and guests alike; the answer carries the link to share.
    let first: RecordShareResponse = ctx
        .call("EngagementService", "RecordShare", Some(&bob), &share(ShareTarget::SHARE_TARGET_LINK_COPY))
        .await
        .unwrap();
    assert_eq!((first.share_count, first.url.as_str()), (1, format!("http://localhost:8080{}", video.path).as_str()));
    let _: RecordShareResponse =
        ctx.call("EngagementService", "RecordShare", None, &share(ShareTarget::SHARE_TARGET_EXTERNAL)).await.unwrap();
    let third: RecordShareResponse = ctx
        .call("EngagementService", "RecordShare", Some(&bob), &share(ShareTarget::SHARE_TARGET_LINK_COPY))
        .await
        .unwrap();
    assert_eq!(third.share_count, 3);
    assert_eq!(ctx.videos.counter(video.id, "share_count"), 3);
    assert_eq!(ctx.engagement_of(None, &video).await.share_count, 3);

    assert_eq!(
        ctx.reason("EngagementService", "RecordShare", Some(&bob), &share(ShareTarget::SHARE_TARGET_UNSPECIFIED)).await,
        "SHARE_TARGET_INVALID"
    );
    let gone = RecordShareRequest {
        subject: missing_video().into(),
        target: ShareTarget::SHARE_TARGET_CHAT.into(),
        ..Default::default()
    };
    assert_eq!(ctx.reason("EngagementService", "RecordShare", Some(&bob), &gone).await, "SUBJECT_NOT_FOUND");
    let nowhere = RecordShareRequest {
        subject: unserved().into(),
        target: ShareTarget::SHARE_TARGET_CHAT.into(),
        ..Default::default()
    };
    assert_eq!(ctx.reason("EngagementService", "RecordShare", None, &nowhere).await, "UNSUPPORTED_SUBJECT");

    // The breakdown is the owner's.
    let stats = GetShareStatsRequest { subject: video.subject_ref().into(), ..Default::default() };
    assert_eq!(ctx.reason("EngagementService", "GetShareStats", None, &stats).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason("EngagementService", "GetShareStats", Some(&bob), &stats).await, "NOT_SUBJECT_OWNER");
    let breakdown: GetShareStatsResponse =
        ctx.call("EngagementService", "GetShareStats", Some(&owner), &stats).await.unwrap();
    assert_eq!(breakdown.share_count, 3);
    assert_eq!(
        breakdown.targets.iter().map(|target| (target.target.as_known().unwrap(), target.count)).collect::<Vec<_>>(),
        [(ShareTarget::SHARE_TARGET_LINK_COPY, 2), (ShareTarget::SHARE_TARGET_EXTERNAL, 1)]
    );
    let gone = GetShareStatsRequest { subject: missing_video().into(), ..Default::default() };
    assert_eq!(ctx.reason("EngagementService", "GetShareStats", Some(&owner), &gone).await, "SUBJECT_NOT_FOUND");

    // Each share is an event; guests share without a user.
    let recorded = ctx.events_about("engagement.share.recorded", video.id).await;
    assert_eq!(recorded.len(), 3);
    assert_eq!(recorded.iter().filter(|share| share["user_id"].is_null()).count(), 1);
    assert_eq!(recorded[0]["target"], 1);
}

// ───────────────────────────── reports ─────────────────────────────

fn report(subject: SubjectRef, category: ReportCategory, text: &str) -> CreateReportRequest {
    CreateReportRequest {
        subject: subject.into(),
        category: category.into(),
        text: text.to_owned(),
        ..Default::default()
    }
}

impl Ctx {
    async fn my_reports(&self, user: &TestUser) -> Vec<Report> {
        let response: ListMyReportsResponse =
            self.call("ReportService", "ListMyReports", Some(user), &ListMyReportsRequest::default()).await.unwrap();
        response.reports
    }
}

#[tokio::test]
async fn a_report_reaches_the_moderation_team() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "あやしい動画");

    let request = report(video.short_ref(), ReportCategory::REPORT_CATEGORY_SPAM, "  宣伝ばかりです  ");
    let created: CreateReportResponse = ctx.call("ReportService", "CreateReport", Some(&bob), &request).await.unwrap();
    let filed = &created.report;
    assert_eq!(filed.subject.id, video.id.to_string());
    assert_eq!(
        (filed.subject_title.as_str(), filed.subject_path.as_str(), filed.text.as_str()),
        ("あやしい動画", video.path.as_str(), "宣伝ばかりです")
    );
    assert_eq!(filed.status, ReportStatus::REPORT_STATUS_SUBMITTED);
    assert_eq!(filed.category, ReportCategory::REPORT_CATEGORY_SPAM);
    assert!(filed.resolved_at.as_option().is_none());

    // The admin domain receives exactly this.
    let events = ctx.app.events_by("engagement.report.created", bob.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].data,
        serde_json::json!({
            "report_id": filed.id,
            "reporter_id": bob.id,
            "subject_type": 5,
            "subject_id": video.id,
            "subject_owner_id": owner.id,
            "category": "SPAM",
            "text": "宣伝ばかりです",
            "subject_title": "あやしい動画",
            "subject_path": video.path,
        })
    );

    // One open report per subject; the owner cannot report their own.
    let again = report(video.subject_ref(), ReportCategory::REPORT_CATEGORY_HATE, "");
    assert_eq!(ctx.reason("ReportService", "CreateReport", Some(&bob), &again).await, "ALREADY_REPORTED");
    assert_eq!(ctx.reason("ReportService", "CreateReport", Some(&owner), &again).await, "CANNOT_REPORT_OWN");
    assert_eq!(ctx.reason("ReportService", "CreateReport", None, &again).await, "SIGN_IN_REQUIRED");
    let other = ctx.video(&owner, "べつの");
    assert_eq!(
        ctx.reason(
            "ReportService",
            "CreateReport",
            Some(&bob),
            &report(other.subject_ref(), ReportCategory::REPORT_CATEGORY_UNSPECIFIED, "")
        )
        .await,
        "REPORT_CATEGORY_INVALID"
    );
    assert_eq!(
        ctx.reason(
            "ReportService",
            "CreateReport",
            Some(&bob),
            &report(other.subject_ref(), ReportCategory::REPORT_CATEGORY_OTHER, &"x".repeat(1001))
        )
        .await,
        "REPORT_TEXT_TOO_LONG"
    );
    assert_eq!(
        ctx.reason(
            "ReportService",
            "CreateReport",
            Some(&bob),
            &report(missing_video(), ReportCategory::REPORT_CATEGORY_OTHER, "")
        )
        .await,
        "SUBJECT_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(
            "ReportService",
            "CreateReport",
            Some(&bob),
            &report(unserved(), ReportCategory::REPORT_CATEGORY_OTHER, "")
        )
        .await,
        "UNSUPPORTED_SUBJECT"
    );

    // Comments and accounts can be reported like content.
    let comment = ctx.comment(&owner, &video, "ひどいコメント").await;
    let on_comment: CreateReportResponse = ctx
        .call(
            "ReportService",
            "CreateReport",
            Some(&bob),
            &report(comment_ref(&comment.id), ReportCategory::REPORT_CATEGORY_HARASSMENT, ""),
        )
        .await
        .unwrap();
    assert_eq!(on_comment.report.subject_title, "ひどいコメント");
    assert_eq!(on_comment.report.subject_path, format!("{}?comment={}", video.path, comment.id));
    let account = SubjectRef {
        r#type: SubjectType::SUBJECT_TYPE_USER.into(),
        id: format!("@{}", owner.handle),
        ..Default::default()
    };
    let on_user: CreateReportResponse = ctx
        .call(
            "ReportService",
            "CreateReport",
            Some(&bob),
            &report(account, ReportCategory::REPORT_CATEGORY_IMPERSONATION, "なりすまし"),
        )
        .await
        .unwrap();
    assert_eq!(on_user.report.subject.id, owner.id.to_string());
    assert_eq!(
        (on_user.report.subject_title.as_str(), on_user.report.subject_path.as_str()),
        ("Owner", format!("/u/{}", owner.handle).as_str())
    );
    let myself =
        SubjectRef { r#type: SubjectType::SUBJECT_TYPE_USER.into(), id: bob.id.to_string(), ..Default::default() };
    assert_eq!(
        ctx.reason(
            "ReportService",
            "CreateReport",
            Some(&bob),
            &report(myself, ReportCategory::REPORT_CATEGORY_OTHER, "")
        )
        .await,
        "CANNOT_REPORT_OWN"
    );

    // My reports, newest first; nobody else's.
    assert_eq!(
        ctx.reason("ReportService", "ListMyReports", None, &ListMyReportsRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
    let mine = ctx.my_reports(&bob).await;
    assert_eq!(
        mine.iter().map(|report| report.category.as_known().unwrap()).collect::<Vec<_>>(),
        [
            ReportCategory::REPORT_CATEGORY_IMPERSONATION,
            ReportCategory::REPORT_CATEGORY_HARASSMENT,
            ReportCategory::REPORT_CATEGORY_SPAM
        ]
    );
    assert!(ctx.my_reports(&owner).await.is_empty());
    let paged = ListMyReportsRequest { page: page(2, ""), ..Default::default() };
    let first: ListMyReportsResponse = ctx.call("ReportService", "ListMyReports", Some(&bob), &paged).await.unwrap();
    assert!(first.reports.len() == 2 && first.page.has_more);
    let bad = ListMyReportsRequest { page: page(2, "nope"), ..Default::default() };
    assert_eq!(ctx.reason("ReportService", "ListMyReports", Some(&bob), &bad).await, "INVALID_CURSOR");
}

#[tokio::test]
async fn report_statuses_follow_the_moderation_team() {
    let ctx = start().await;
    let (owner, bob, carol, moderator) = (
        ctx.app.user("Owner").await,
        ctx.app.user("Bob").await,
        ctx.app.user("Carol").await,
        ctx.app.user("Mod").await,
    );
    let (removed, dismissed) = (ctx.video(&owner, "消される動画"), ctx.video(&owner, "問題なしの動画"));
    for reporter in [&bob, &carol] {
        let _: CreateReportResponse = ctx
            .call(
                "ReportService",
                "CreateReport",
                Some(reporter),
                &report(removed.subject_ref(), ReportCategory::REPORT_CATEGORY_VIOLENCE, ""),
            )
            .await
            .unwrap();
    }
    let other: CreateReportResponse = ctx
        .call(
            "ReportService",
            "CreateReport",
            Some(&bob),
            &report(dismissed.subject_ref(), ReportCategory::REPORT_CATEGORY_OTHER, ""),
        )
        .await
        .unwrap();

    // The content is taken down: every open report about it led to action.
    ctx.deliver(
        "admin.content.removed",
        Some(moderator.id),
        serde_json::json!({ "subject_type": 5, "subject_id": removed.id, "reason": "規約違反", "moderator_id": moderator.id }),
    )
    .await;
    let bobs = ctx.my_reports(&bob).await;
    assert_eq!(bobs[1].status, ReportStatus::REPORT_STATUS_ACTION_TAKEN);
    assert!(bobs[1].resolved_at.as_option().is_some());
    assert_eq!(bobs[0].status, ReportStatus::REPORT_STATUS_SUBMITTED, "the other report is untouched");
    assert_eq!(ctx.my_reports(&carol).await[0].status, ReportStatus::REPORT_STATUS_ACTION_TAKEN);

    // A decision without action, as the admin domain would announce it.
    ctx.deliver(
        "admin.report.resolved",
        Some(moderator.id),
        serde_json::json!({ "report_ids": [other.report.id], "action_taken": false }),
    )
    .await;
    assert_eq!(ctx.my_reports(&bob).await[0].status, ReportStatus::REPORT_STATUS_NO_ACTION);
    // A decided report no longer stands in the way of a new one.
    let again: CreateReportResponse = ctx
        .call(
            "ReportService",
            "CreateReport",
            Some(&bob),
            &report(dismissed.subject_ref(), ReportCategory::REPORT_CATEGORY_SPAM, "またです"),
        )
        .await
        .unwrap();
    assert_eq!(again.report.status, ReportStatus::REPORT_STATUS_SUBMITTED);
}

#[tokio::test]
async fn an_unanswered_report_gives_way_to_a_new_one() {
    // With a repeat window of zero days every open report is "old enough".
    let ctx = start_with(EngagementConfig { report_repeat_days: 0, ..test_config() }).await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let first: CreateReportResponse = ctx
        .call(
            "ReportService",
            "CreateReport",
            Some(&bob),
            &report(video.subject_ref(), ReportCategory::REPORT_CATEGORY_SPAM, "一回目"),
        )
        .await
        .unwrap();
    let second: CreateReportResponse = ctx
        .call(
            "ReportService",
            "CreateReport",
            Some(&bob),
            &report(video.subject_ref(), ReportCategory::REPORT_CATEGORY_SPAM, "二回目"),
        )
        .await
        .unwrap();
    assert_ne!(first.report.id, second.report.id);
    let mine = ctx.my_reports(&bob).await;
    assert_eq!(
        mine.iter().map(|report| (report.text.as_str(), report.status.as_known().unwrap())).collect::<Vec<_>>(),
        [("二回目", ReportStatus::REPORT_STATUS_SUBMITTED), ("一回目", ReportStatus::REPORT_STATUS_SUPERSEDED)]
    );
    assert_eq!(ctx.app.events_by("engagement.report.created", bob.id).await.len(), 2);
}

#[tokio::test]
async fn a_block_does_not_stop_a_report() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    ctx.block(&bob, &owner).await;
    // Bob no longer reaches the video for anything else...
    assert_eq!(
        ctx.reason("ReactionService", "SetReaction", Some(&bob), &set(video.subject_ref(), like())).await,
        "SUBJECT_NOT_FOUND"
    );
    // ...but can still tell the moderation team about it, and about its owner.
    let on_video: CreateReportResponse = ctx
        .call(
            "ReportService",
            "CreateReport",
            Some(&bob),
            &report(video.subject_ref(), ReportCategory::REPORT_CATEGORY_HARASSMENT, ""),
        )
        .await
        .unwrap();
    assert_eq!(on_video.report.status, ReportStatus::REPORT_STATUS_SUBMITTED);
    let account =
        SubjectRef { r#type: SubjectType::SUBJECT_TYPE_USER.into(), id: owner.id.to_string(), ..Default::default() };
    let on_user: CreateReportResponse = ctx
        .call(
            "ReportService",
            "CreateReport",
            Some(&bob),
            &report(account, ReportCategory::REPORT_CATEGORY_HARASSMENT, ""),
        )
        .await
        .unwrap();
    assert_eq!(on_user.report.subject_title, "Owner");
}
