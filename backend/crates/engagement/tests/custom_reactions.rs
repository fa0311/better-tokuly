//! Custom emoji as reactions, one or several reactions per person, the
//! limits, owners' settings, "who reacted" with mutes, the pop on the
//! subject's topic and the grouped notification.
mod common;

use common::*;
use tokuly_engagement::EngagementConfig;
use tokuly_proto::{
    common::SubjectRef,
    engagement::{__buffa::oneof::engagement_event::Event as Pushed, *},
};
use tokuly_testkit::TestUser;

fn set(subject: SubjectRef, reaction: Reaction) -> SetReactionRequest {
    SetReactionRequest { subject: subject.into(), reaction: reaction.into(), ..Default::default() }
}

fn unreact(subject: SubjectRef, reaction: Option<Reaction>) -> RemoveReactionRequest {
    RemoveReactionRequest { subject: subject.into(), reaction: reaction.into(), ..Default::default() }
}

fn count_of<'a>(summary: &'a ReactionSummary, emoji: &str) -> Option<&'a EmojiCount> {
    summary.emoji_counts.iter().find(|count| count.emoji == emoji)
}

fn mine(summary: &ReactionSummary) -> Vec<&str> {
    summary.viewer_reactions.iter().map(|reaction| reaction.emoji.as_str()).collect()
}

async fn reason_of(ctx: &Ctx, user: &TestUser, subject: SubjectRef, reaction: Reaction) -> String {
    ctx.reason("ReactionService", "SetReaction", Some(user), &set(subject, reaction)).await
}

#[tokio::test]
async fn custom_emoji_reactions_come_with_their_picture() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "ねこ動画");
    let (_, neko) = ctx.custom_emoji("neko", false).await;

    // Any case, with colons: stored and returned lowercase.
    let summary = ctx.react(&bob, video.subject_ref(), emoji(&neko.to_uppercase())).await;
    let counted = count_of(&summary, &neko).expect("the custom emoji is counted");
    assert_eq!(counted.count, 1);
    let picture = counted.custom.as_option().expect("drawn without another request");
    assert!(picture.available && !picture.sensitive);
    assert_eq!(format!(":{}:", picture.name), neko);
    assert!(!picture.image.url.is_empty() && !picture.static_url.is_empty());
    assert_eq!(summary.viewer_reaction.emoji, neko);
    assert!(summary.viewer_reaction.custom.as_option().is_some_and(|custom| custom.available));
    let rules = summary.rules.as_option().expect("rules");
    assert!(rules.emoji && !rules.multiple && rules.custom_emoji && !rules.sensitive_emoji);
    assert_eq!(rules.max_per_person, 1);

    // Unicode and custom emoji are counted side by side.
    ctx.react(&carol, video.subject_ref(), emoji("🎉")).await;
    let engagement = ctx.engagement_of(None, &video).await;
    let reactions = engagement.reactions.as_option().unwrap();
    assert_eq!(reactions.emoji_total, 2);
    assert!(count_of(reactions, &neko).unwrap().custom.as_option().is_some());
    assert!(count_of(reactions, "🎉").unwrap().custom.as_option().is_none());

    // Who reacted with it, filtered by the shortcode in any case.
    let reactors = ctx.reactors(None, &video, &neko.to_uppercase()).await;
    assert_eq!(reactors.len(), 1);
    assert_eq!(reactors[0].user.id, bob.id.to_string());
    assert!(reactors[0].reaction.custom.as_option().is_some_and(|custom| custom.available));

    // My reactions show it too.
    let response: ListMyReactionsResponse =
        ctx.call("ReactionService", "ListMyReactions", Some(&bob), &ListMyReactionsRequest::default()).await.unwrap();
    assert_eq!(response.subjects[0].reaction.emoji, neko);
    assert!(response.subjects[0].reaction.custom.as_option().is_some());

    // The use is reported to the emoji domain: it is the first "recently used".
    let recent: tokuly_proto::emoji::ListRecentEmojiResponse = ctx
        .app
        .call(
            "tokuly.emoji.v1.EmojiService/ListRecentEmoji",
            Some(&bob),
            &tokuly_proto::emoji::ListRecentEmojiRequest::default(),
        )
        .await
        .unwrap();
    assert_eq!(recent.items.first().map(|item| item.emoji.as_str()), Some(neko.as_str()));

    // The event names the emoji as it is stored.
    let changed = ctx.events_about("engagement.reaction.changed", video.id).await;
    assert!(changed.iter().any(|event| event["emoji"] == neko.as_str() && event["user_id"] == bob.id.to_string()));
}

#[tokio::test]
async fn only_approved_custom_emoji_can_be_added() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    assert_eq!(reason_of(&ctx, &bob, video.subject_ref(), emoji(":no_such_emoji_here:")).await, "EMOJI_UNAVAILABLE");
    assert_eq!(reason_of(&ctx, &bob, video.subject_ref(), emoji(":not valid:")).await, "EMOJI_INVALID");
    assert_eq!(reason_of(&ctx, &bob, video.subject_ref(), emoji("neko")).await, "EMOJI_INVALID");

    // Comments take likes and dislikes only.
    let comment = ctx.comment(&owner, &video, "こんにちは").await;
    let (_, neko) = ctx.custom_emoji("neko", false).await;
    assert_eq!(reason_of(&ctx, &bob, comment_ref(&comment.id), emoji(&neko)).await, "EMOJI_NOT_ALLOWED");
    let liked = ctx.react(&bob, comment_ref(&comment.id), like()).await;
    assert_eq!(liked.like_count, 1);
    assert!(!liked.rules.as_option().unwrap().emoji);
}

#[tokio::test]
async fn owners_decide_about_custom_and_sensitive_emoji() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let (_, spicy) = ctx.custom_emoji("spicy", true).await;
    let (_, neko) = ctx.custom_emoji("neko", false).await;

    let defaults: GetReactionSettingsResponse = ctx
        .call("ReactionService", "GetReactionSettings", Some(&owner), &GetReactionSettingsRequest::default())
        .await
        .unwrap();
    assert!(defaults.settings.custom_emoji && !defaults.settings.sensitive_emoji);
    assert_eq!(reason_of(&ctx, &bob, video.subject_ref(), emoji(&spicy)).await, "SENSITIVE_EMOJI_NOT_ALLOWED");

    ctx.set_reaction_settings(&owner, true, true).await;
    let summary = ctx.react(&bob, video.subject_ref(), emoji(&spicy)).await;
    assert!(count_of(&summary, &spicy).unwrap().custom.as_option().unwrap().sensitive);
    assert!(summary.rules.as_option().unwrap().sensitive_emoji);

    // Turning custom emoji off keeps what is there and refuses more (Unicode still works).
    ctx.set_reaction_settings(&owner, false, true).await;
    assert_eq!(reason_of(&ctx, &bob, video.subject_ref(), emoji(&neko)).await, "CUSTOM_EMOJI_NOT_ALLOWED");
    let summary = ctx.react(&bob, video.subject_ref(), emoji("🎉")).await;
    let rules = summary.rules.as_option().unwrap();
    assert!(!rules.custom_emoji && !rules.sensitive_emoji);
    let settings: GetReactionSettingsResponse = ctx
        .call("ReactionService", "GetReactionSettings", Some(&owner), &GetReactionSettingsRequest::default())
        .await
        .unwrap();
    assert!(!settings.settings.custom_emoji && settings.settings.sensitive_emoji);

    // Settings are the caller's own: signed in only.
    let anonymous =
        ctx.reason("ReactionService", "GetReactionSettings", None, &GetReactionSettingsRequest::default()).await;
    assert_eq!(anonymous, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn reactions_outlive_their_emoji_and_fall_back() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "v");
    let (emoji_id, neko) = ctx.custom_emoji("neko", false).await;
    ctx.react(&bob, video.subject_ref(), emoji(&neko)).await;

    ctx.disable_emoji(&emoji_id).await;
    let engagement = ctx.engagement_of(Some(&bob), &video).await;
    let reactions = engagement.reactions.as_option().unwrap();
    let counted = count_of(reactions, &neko).expect("still counted");
    assert_eq!(counted.count, 1);
    let fallback = counted.custom.as_option().expect("described");
    assert!(!fallback.available && fallback.image.as_option().is_none() && fallback.static_url.is_empty());
    assert_eq!(format!(":{}:", fallback.name), neko);
    assert!(!reactions.viewer_reaction.custom.available);

    // Nobody can add it any more; its holder can take it back.
    assert_eq!(reason_of(&ctx, &carol, video.subject_ref(), emoji(&neko)).await, "EMOJI_UNAVAILABLE");
    let removed: RemoveReactionResponse = ctx
        .call("ReactionService", "RemoveReaction", Some(&bob), &unreact(video.subject_ref(), Some(emoji(&neko))))
        .await
        .unwrap();
    assert_eq!(removed.reactions.emoji_total, 0);
    assert!(removed.reactions.viewer_reactions.is_empty());
}

#[tokio::test]
async fn group_subjects_take_several_emoji_per_person() {
    let cfg = EngagementConfig { reactions_per_person: 2, ..test_config() };
    let ctx = start_with(cfg).await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let thread = ctx.thread(&owner, "週末どうする?");
    let (_, neko) = ctx.custom_emoji("neko", false).await;

    ctx.react(&bob, thread.subject_ref(), emoji("✅")).await;
    let summary = ctx.react(&bob, thread.subject_ref(), emoji(&neko)).await;
    assert_eq!(mine(&summary), ["✅", neko.as_str()]);
    assert_eq!(summary.emoji_total, 2);
    let rules = summary.rules.as_option().unwrap();
    assert!(rules.multiple);
    assert_eq!(rules.max_per_person, 2);
    // The representative reaction is the newest emoji while there is no like.
    assert_eq!(summary.viewer_reaction.emoji, neko);

    // A like comes on top of the emoji; the person's emoji are limited.
    let summary = ctx.react(&bob, thread.subject_ref(), like()).await;
    assert_eq!((summary.like_count, summary.emoji_total, summary.viewer_reactions.len()), (1, 2, 3));
    assert_eq!(summary.viewer_reaction.kind, ReactionKind::REACTION_KIND_LIKE);
    assert_eq!(reason_of(&ctx, &bob, thread.subject_ref(), emoji("👀")).await, "REACTION_LIMIT");
    // Reacting with what one already holds changes nothing, even at the limit.
    assert_eq!(ctx.react(&bob, thread.subject_ref(), emoji("✅")).await.emoji_total, 2);
    // A dislike replaces the like, nothing else.
    let summary = ctx.react(&bob, thread.subject_ref(), dislike()).await;
    assert_eq!((summary.like_count, summary.dislike_count, summary.emoji_total), (0, 1, 2));

    // Taking one back, then everything.
    let removed: RemoveReactionResponse = ctx
        .call("ReactionService", "RemoveReaction", Some(&bob), &unreact(thread.subject_ref(), Some(emoji("✅"))))
        .await
        .unwrap();
    assert_eq!(removed.reactions.emoji_total, 1);
    assert_eq!(mine(&removed.reactions), [neko.as_str(), ""]);
    let removed: RemoveReactionResponse =
        ctx.call("ReactionService", "RemoveReaction", Some(&bob), &unreact(thread.subject_ref(), None)).await.unwrap();
    let summary = removed.reactions.as_option().unwrap();
    assert_eq!((summary.dislike_count, summary.emoji_total, summary.viewer_reactions.len()), (0, 0, 0));
    assert_eq!(ctx.threads.counter(thread.id, "reaction_count"), 0);

    // Each emoji came and went as its own event.
    let changed = ctx.events_about("engagement.reaction.changed", thread.id).await;
    let added_neko = changed.iter().find(|event| event["emoji"] == neko.as_str()).expect("added");
    assert_eq!(added_neko["previous_kind"], 0);
    assert_eq!(changed.iter().filter(|event| event["kind"] == 0).count(), 3);
}

#[tokio::test]
async fn a_subject_carries_a_limited_number_of_distinct_emoji() {
    let cfg = EngagementConfig { reaction_emoji_per_subject: 2, ..test_config() };
    let ctx = start_with(cfg).await;
    let owner = ctx.app.user("Owner").await;
    let (bob, carol, dave) = (ctx.app.user("Bob").await, ctx.app.user("Carol").await, ctx.app.user("Dave").await);
    let video = ctx.video(&owner, "v");
    ctx.react(&bob, video.subject_ref(), emoji("🎉")).await;
    let summary = ctx.react(&carol, video.subject_ref(), emoji("👀")).await;
    assert_eq!(summary.rules.as_option().unwrap().max_distinct_emoji, 2);
    assert_eq!(summary.distinct_emoji, 2);

    // Full: a new emoji is refused, one that is there can be joined.
    assert_eq!(reason_of(&ctx, &dave, video.subject_ref(), emoji("🙏")).await, "REACTION_EMOJI_LIMIT");
    assert_eq!(ctx.react(&dave, video.subject_ref(), emoji("🎉")).await.emoji_total, 3);
    // Replacing the only 👀 frees its place.
    let summary = ctx.react(&carol, video.subject_ref(), emoji("🙏")).await;
    let shown: Vec<&str> = summary.emoji_counts.iter().map(|count| count.emoji.as_str()).collect();
    assert_eq!(shown, ["🎉", "🙏"]);
    // Likes are not emoji.
    assert_eq!(ctx.react(&ctx.app.user("Erin").await, video.subject_ref(), like()).await.like_count, 1);
}

#[tokio::test]
async fn new_emoji_from_many_people_at_once_respect_the_limit() {
    let cfg = EngagementConfig { reaction_emoji_per_subject: 1, ..test_config() };
    let ctx = start_with(cfg).await;
    let owner = ctx.app.user("Owner").await;
    let video = ctx.video(&owner, "v");
    let mut people = Vec::new();
    for index in 0..6 {
        people.push(ctx.app.user(&format!("Fan{index}")).await);
    }
    let picks = ["🎉", "👀", "🙏", "🔥", "💯", "😂"];
    let requests: Vec<_> = picks.iter().map(|pick| set(video.subject_ref(), emoji(pick))).collect();
    let attempts = people.iter().zip(&requests).map(|(person, request)| {
        ctx.call::<_, SetReactionResponse>("ReactionService", "SetReaction", Some(person), request)
    });
    let results = futures::future::join_all(attempts).await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1, "{results:?}");
    assert!(
        results.iter().filter_map(|result| result.as_ref().err()).all(|error| error.reason == "REACTION_EMOJI_LIMIT")
    );
    let summary = ctx.engagement_of(None, &video).await;
    assert_eq!((summary.reactions.emoji_total, summary.reactions.emoji_counts.len()), (1, 1));
}

#[tokio::test]
async fn one_person_tapping_many_emoji_at_once_keeps_every_count_right() {
    // As many requests as the test pool has connections, all from one
    // account, on the lock every reaction takes: none may wait for a
    // second connection while holding the first.
    let cfg = EngagementConfig { reactions_per_person: 3, ..test_config() };
    let ctx = start_with(cfg).await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let thread = ctx.thread(&owner, "t");
    let (_, neko) = ctx.custom_emoji("neko", false).await;
    let picks = ["🎉", "👀", "🙏", "🔥", neko.as_str(), "🎉", "👀", "🙏", "🔥", neko.as_str()];
    let requests: Vec<_> = picks.iter().map(|pick| set(thread.subject_ref(), emoji(pick))).collect();
    let attempts = requests
        .iter()
        .map(|request| ctx.call::<_, SetReactionResponse>("ReactionService", "SetReaction", Some(&bob), request));
    let results = futures::future::join_all(attempts).await;
    assert!(
        results.iter().filter_map(|result| result.as_ref().err()).all(|error| error.reason == "REACTION_LIMIT"),
        "{results:?}"
    );
    let summary = ctx.engagement_of(Some(&bob), &thread).await;
    let reactions = summary.reactions.as_option().unwrap();
    assert_eq!(reactions.viewer_reactions.len(), 3);
    assert_eq!(reactions.emoji_total, 3);
    assert_eq!(reactions.emoji_counts.iter().map(|count| count.count).sum::<i64>(), 3);
    assert_eq!(ctx.threads.counter(thread.id, "reaction_count"), 3);
}

#[tokio::test]
async fn who_reacted_leaves_out_blocks_and_mutes() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let (bob, carol, viewer) = (ctx.app.user("Bob").await, ctx.app.user("Carol").await, ctx.app.user("Viewer").await);
    let video = ctx.video(&owner, "v");
    ctx.react(&bob, video.subject_ref(), emoji("🎉")).await;
    ctx.react(&carol, video.subject_ref(), emoji("🎉")).await;
    let names =
        |reactors: Vec<Reactor>| -> Vec<String> { reactors.into_iter().map(|r| r.user.handle.clone()).collect() };
    assert_eq!(names(ctx.reactors(Some(&viewer), &video, "🎉").await).len(), 2);

    // The viewer mutes Bob from the notification settings: Bob leaves their list only.
    let at = chrono::Utc::now();
    ctx.deliver_at(
        "notify.mute.changed",
        serde_json::json!({ "user_id": viewer.id, "muted_user_id": bob.id, "muted": true }),
        at,
    )
    .await;
    assert_eq!(names(ctx.reactors(Some(&viewer), &video, "").await), [carol.handle.clone()]);
    assert_eq!(names(ctx.reactors(None, &video, "").await).len(), 2);
    // An older unmute arriving late changes nothing; a newer one does.
    ctx.deliver_at(
        "social.friend.prefs_changed",
        serde_json::json!({ "user_id": viewer.id, "friend_id": bob.id, "muted": false }),
        at - chrono::Duration::seconds(5),
    )
    .await;
    assert_eq!(names(ctx.reactors(Some(&viewer), &video, "").await).len(), 1);
    ctx.deliver_at(
        "social.friend.prefs_changed",
        serde_json::json!({ "user_id": viewer.id, "friend_id": bob.id, "muted": false }),
        at + chrono::Duration::seconds(5),
    )
    .await;
    assert_eq!(names(ctx.reactors(Some(&viewer), &video, "").await).len(), 2);

    // A block hides both ways.
    ctx.block(&carol, &viewer).await;
    assert_eq!(names(ctx.reactors(Some(&viewer), &video, "").await), [bob.handle.clone()]);
}

#[tokio::test]
async fn reactions_pop_on_the_subject_topic_without_saying_who() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let (_, neko) = ctx.custom_emoji("neko", false).await;
    let mut topic = ctx.app.listen_topic(&video.topic()).await;

    ctx.react(&bob, video.subject_ref(), emoji(&neko)).await;
    ctx.react(&bob, video.subject_ref(), dislike()).await;
    let events = drain(&mut topic).await;
    let pops: Vec<&ReactionAdded> = events
        .iter()
        .filter_map(|event| match event {
            Pushed::ReactionAdded(added) => Some(&**added),
            _ => None,
        })
        .collect();
    // The emoji pops with its picture; a dislike never does.
    assert_eq!(pops.len(), 1, "{events:?}");
    assert_eq!(pops[0].reaction.emoji, neko);
    assert!(pops[0].reaction.custom.as_option().is_some_and(|custom| custom.available));
    // The counters that follow draw custom emoji too, without anyone's own state.
    let counters: Vec<&CountersChanged> = events
        .iter()
        .filter_map(|event| match event {
            Pushed::CountersChanged(counters) => Some(&**counters),
            _ => None,
        })
        .collect();
    assert!(counters.iter().all(
        |counters| counters.reactions.rules.as_option().is_none() && counters.reactions.viewer_reactions.is_empty()
    ));
    assert!(
        counters.iter().any(|counters| counters
            .reactions
            .emoji_counts
            .iter()
            .any(|count| count.custom.as_option().is_some()))
    );
}

#[tokio::test]
async fn the_owner_hears_of_reactions_grouped_per_subject() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let (alice, bob, carol) =
        (ctx.app.user("アリス").await, ctx.app.user("ボブ").await, ctx.app.user("キャロル").await);
    let note = ctx.note(&owner, "今日のランチ");
    let (_, neko) = ctx.custom_emoji("neko", false).await;

    ctx.react(&alice, note.subject_ref(), like()).await;
    ctx.react(&bob, note.subject_ref(), emoji("🎉")).await;
    ctx.react(&carol, note.subject_ref(), emoji(&neko)).await;
    // Dislikes are never news, and do not count as people who reacted.
    ctx.react(&ctx.app.user("Dan").await, note.subject_ref(), dislike()).await;

    let notices = ctx.notifications(owner.id, "engagement.like").await;
    assert_eq!(notices.len(), 3);
    let collapse = format!("engagement.like:{}:{}", note.kind as i32, note.id);
    assert!(notices.iter().all(|notice| notice["collapse_key"] == collapse.as_str()));
    let titles: Vec<&str> = notices.iter().map(|notice| notice["title"]["ja"].as_str().unwrap()).collect();
    assert_eq!(titles[0], "アリスがいいねしたよ!");
    assert_eq!(titles[1], "ボブと他1人がリアクションしたよ! 🎉");
    assert_eq!(titles[2], format!("キャロルと他2人がリアクションしたよ! {neko}"));
    let last = &notices[2]["data"];
    assert_eq!((last["emoji"].as_str(), last["others"].as_i64()), (Some(neko.as_str()), Some(2)));
    assert!(last["emoji_image_url"].as_str().is_some_and(|url| !url.is_empty()));
}

#[tokio::test]
async fn reaction_settings_changes_are_limited() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let request = UpdateReactionSettingsRequest {
        settings: ReactionSettings { custom_emoji: true, sensitive_emoji: false, ..Default::default() }.into(),
        ..Default::default()
    };
    let mut refused = None;
    for _ in 0..40 {
        if let Err(error) = ctx
            .call::<_, UpdateReactionSettingsResponse>(
                "ReactionService",
                "UpdateReactionSettings",
                Some(&owner),
                &request,
            )
            .await
        {
            refused = Some(error.reason);
            break;
        }
    }
    assert_eq!(refused.as_deref(), Some("RATE_LIMITED"));
}

#[tokio::test]
async fn an_erased_account_takes_its_settings_and_mutes_along() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    ctx.set_reaction_settings(&owner, true, true).await;
    ctx.deliver_at(
        "notify.mute.changed",
        serde_json::json!({ "user_id": owner.id, "muted_user_id": bob.id, "muted": true }),
        chrono::Utc::now(),
    )
    .await;
    let rows = |sql: &'static str| {
        let db = ctx.app.core.db.clone();
        let owner = owner.id;
        async move { sqlx::query_scalar::<_, i64>(sql).bind(owner).fetch_one(&db).await.unwrap() }
    };
    assert_eq!(rows("SELECT count(*) FROM engagement_mutes WHERE user_id = $1").await, 1);
    ctx.deliver("identity.user.deleted", None, serde_json::json!({ "user_id": owner.id })).await;
    assert_eq!(rows("SELECT count(*) FROM reaction_settings WHERE user_id = $1").await, 0);
    assert_eq!(rows("SELECT count(*) FROM engagement_mutes WHERE user_id = $1 OR muted_user_id = $1").await, 0);
}

#[tokio::test]
async fn taking_many_emoji_back_at_once_keeps_the_lock_order() {
    // Everyone holds the same emoji in a different order and takes them all
    // back, or swaps them, at the same moment: per-emoji counts are locked in
    // name order and the statistics row last, so nobody deadlocks.
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let thread = ctx.thread(&owner, "t");
    let orders = [["🎉", "👀", "🙏"], ["🙏", "🎉", "👀"], ["👀", "🙏", "🎉"], ["🎉", "🙏", "👀"]];
    let mut people = Vec::new();
    for (index, order) in orders.iter().enumerate() {
        let person = ctx.app.user(&format!("Fan{index}")).await;
        for pick in order {
            ctx.react(&person, thread.subject_ref(), emoji(pick)).await;
        }
        ctx.react(&person, thread.subject_ref(), like()).await;
        people.push(person);
    }
    let all = unreact(thread.subject_ref(), None);
    let one = unreact(thread.subject_ref(), Some(emoji("👀")));
    let swap = set(thread.subject_ref(), dislike());
    let calls = people.iter().enumerate().flat_map(|(index, person)| {
        let first: futures::future::BoxFuture<'_, Result<serde_json::Value, _>> = match index % 2 {
            0 => Box::pin(ctx.call("ReactionService", "RemoveReaction", Some(person), &all)),
            _ => Box::pin(ctx.call("ReactionService", "RemoveReaction", Some(person), &one)),
        };
        let second: futures::future::BoxFuture<'_, Result<serde_json::Value, _>> =
            Box::pin(ctx.call("ReactionService", "SetReaction", Some(person), &swap));
        [first, second]
    });
    let results = futures::future::join_all(calls).await;
    assert!(results.iter().all(Result::is_ok), "{results:?}");

    // The counters say what the rows say.
    let summary = ctx.engagement_of(None, &thread).await;
    let reactions = summary.reactions.as_option().unwrap();
    let held: i64 = sqlx::query_scalar("SELECT count(*) FROM reactions WHERE subject_id = $1 AND kind = 3")
        .bind(thread.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(reactions.emoji_total, held);
    assert_eq!(reactions.emoji_counts.iter().map(|count| count.count).sum::<i64>(), held);
    let votes: (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE kind = 1), count(*) FILTER (WHERE kind = 2) FROM reactions WHERE subject_id = $1",
    )
    .bind(thread.id)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!((reactions.like_count, reactions.dislike_count), votes);
}

#[tokio::test]
async fn the_owner_is_not_told_about_people_they_blocked_or_muted() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let (muted, blocked, alice) =
        (ctx.app.user("Muted").await, ctx.app.user("Blocked").await, ctx.app.user("アリス").await);
    let note = ctx.note(&owner, "n");
    ctx.react(&muted, note.subject_ref(), emoji("🎉")).await;
    ctx.react(&blocked, note.subject_ref(), emoji("🎉")).await;
    ctx.deliver_at(
        "notify.mute.changed",
        serde_json::json!({ "user_id": owner.id, "muted_user_id": muted.id, "muted": true }),
        chrono::Utc::now(),
    )
    .await;
    ctx.block(&owner, &blocked).await;
    ctx.react(&alice, note.subject_ref(), like()).await;
    let notices = ctx.notifications(owner.id, "engagement.like").await;
    let last = notices.last().expect("a notification");
    assert_eq!(last["title"]["ja"], "アリスがいいねしたよ!");
    assert_eq!(last["data"]["others"], 0);
}
