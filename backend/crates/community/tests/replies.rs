//! `CommunityBoardService`: replies and what they notify.
mod common;

use common::*;
use tokuly_proto::{
    community::{community_event::Event as Pushed, *},
    media::AssetPurpose,
};

const SVC: &str = "CommunityBoardService";

fn list(thread: &Thread) -> ListRepliesRequest {
    ListRepliesRequest { thread_id: thread.id.clone(), ..Default::default() }
}

#[tokio::test]
async fn replies_are_numbered_and_quote_what_they_answer() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let (author, replier) =
        (ctx.member(&owner, &community, "Author").await, ctx.member(&owner, &community, "Replier").await);
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&author, &general, "質問", "これどうやるの?").await;
    let image = ctx.image(&replier, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await;
    let stamp = ctx.stamp(&replier).await;

    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    let first = ctx
        .try_reply(
            &replier,
            &CreateReplyRequest {
                thread_id: thread.short_id.clone(),
                body: "こうやるよ!\n詳しくは画像を見てね".into(),
                asset_ids: vec![image.clone()],
                stamp_id: stamp.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!((first.number, first.by_thread_author), (1, false));
    assert_eq!(first.author.id, replier.id.to_string());
    assert_eq!(first.images[0].asset_id, image);
    assert_eq!(first.stamp.id, stamp);
    assert!(first.reply_to.as_option().is_none());
    assert!(first.viewer.mine && first.viewer.can_edit && first.viewer.can_delete);
    assert!(ctx.is_held(&image, &format!("community:reply:{}", first.id)).await);
    let pushed = pushes(&mut topic).await;
    assert!(pushed.iter().any(|push| matches!(push, Pushed::ReplyCreated(created)
        if created.reply_id == first.id && created.reply_count == 1 && created.reply.number == 1)));
    let events = ctx.app.events_by("community.reply.created", replier.id).await;
    assert_eq!(events[0].data["reply_id"], first.id);

    // An answer to a reply shows what it answers.
    let second = ctx
        .try_reply(
            &author,
            &CreateReplyRequest {
                thread_id: thread.id.clone(),
                body: "ありがとう!".into(),
                reply_to_id: first.id.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!((second.number, second.by_thread_author), (2, true));
    assert_eq!(second.reply_to.reply_id, first.id);
    assert_eq!(second.reply_to.author.id, replier.id.to_string());
    assert_eq!(second.reply_to.excerpt, "こうやるよ! 詳しくは画像を見てね");
    assert!(!second.reply_to.unavailable);

    let seen = ctx.get_thread(Some(&owner), &thread.id).await.unwrap();
    assert_eq!(seen.reply_count, 2);
    assert!(seen.last_activity_at.as_option().unwrap().seconds >= seen.created_at.as_option().unwrap().seconds);

    // Oldest first by default, newest first on request, in pages.
    let numbers = |replies: &[Reply]| replies.iter().map(|reply| reply.number).collect::<Vec<_>>();
    assert_eq!(numbers(&ctx.replies(None, &thread.id).await), vec![1, 2]);
    let newest: ListRepliesResponse =
        ctx.call(SVC, "ListReplies", None, &ListRepliesRequest { newest_first: true, ..list(&thread) }).await.unwrap();
    assert_eq!(numbers(&newest.replies), vec![2, 1]);
    let page_one: ListRepliesResponse =
        ctx.call(SVC, "ListReplies", None, &ListRepliesRequest { page: page(1, ""), ..list(&thread) }).await.unwrap();
    assert!(page_one.page.has_more);
    let page_two: ListRepliesResponse = ctx
        .call(
            SVC,
            "ListReplies",
            None,
            &ListRepliesRequest { page: page(1, &page_one.page.next_cursor), ..list(&thread) },
        )
        .await
        .unwrap();
    assert_eq!((numbers(&page_one.replies), numbers(&page_two.replies)), (vec![1], vec![2]));
    assert!(!page_two.page.has_more);
    assert_eq!(ctx.get_reply(None, &second.id).await.unwrap().body, "ありがとう!");
}

#[tokio::test]
async fn writing_a_reply_is_validated() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let outsider = ctx.app.user("Outsider").await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&owner, &general, "ここ", "x").await;
    let elsewhere = ctx.thread(&owner, &general, "よそ", "x").await;
    let foreign_reply = ctx.reply(&owner, &elsewhere, "よその返信").await;
    let base = CreateReplyRequest { thread_id: thread.id.clone(), body: "本文".into(), ..Default::default() };
    for (reason, request) in [
        ("REPLY_EMPTY", CreateReplyRequest { body: "  ".into(), ..base.clone() }),
        ("BODY_TOO_LONG", CreateReplyRequest { body: "あ".repeat(2001), ..base.clone() }),
        ("REPLY_NOT_FOUND", CreateReplyRequest { reply_to_id: foreign_reply.id.clone(), ..base.clone() }),
        ("REPLY_NOT_FOUND", CreateReplyRequest { reply_to_id: uuid::Uuid::now_v7().to_string(), ..base.clone() }),
        ("STAMP_NOT_USABLE", CreateReplyRequest { stamp_id: uuid::Uuid::now_v7().to_string(), ..base.clone() }),
        ("THREAD_NOT_FOUND", CreateReplyRequest { thread_id: uuid::Uuid::now_v7().to_string(), ..base.clone() }),
    ] {
        assert_eq!(ctx.reason(SVC, "CreateReply", Some(&owner), &request).await, reason);
    }
    assert_eq!(ctx.reason(SVC, "CreateReply", Some(&outsider), &base).await, "NOT_A_MEMBER");
    assert_eq!(ctx.reason(SVC, "CreateReply", None, &base).await, "SIGN_IN_REQUIRED");
    assert_eq!(
        ctx.reason(SVC, "GetReply", None, &GetReplyRequest { reply_id: "nope".into(), ..Default::default() }).await,
        "REPLY_NOT_FOUND"
    );
    // A reply may be just a stamp.
    let stamp = ctx.stamp(&owner).await;
    let just_stamp = CreateReplyRequest { body: String::new(), stamp_id: stamp, ..base.clone() };
    assert!(ctx.try_reply(&owner, &just_stamp).await.unwrap().body.is_empty());
}

#[tokio::test]
async fn followers_hear_of_a_reply_once_until_they_read() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let (first, second) =
        (ctx.member(&owner, &community, "First").await, ctx.member(&owner, &community, "Second").await);
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&author, &general, "相談", "聞いてほしい").await;
    let replies_to = |user: &tokuly_testkit::TestUser| {
        let ctx = &ctx;
        let id = user.id;
        async move { ctx.notifications(id, "community.reply").await }
    };

    // The author follows their thread and hears about the first reply…
    let one = ctx.reply(&first, &thread, "ひとつめ").await;
    let told = replies_to(&author).await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0]["actor_id"], first.id.to_string());
    assert_eq!(told[0]["collapse_key"], format!("community.reply:{}", thread.id));
    assert!(told[0]["path"].as_str().unwrap().ends_with(&format!("?reply={}", one.id)));
    // …but not about every further one while that notice is unread.
    ctx.reply(&second, &thread, "ふたつめ").await;
    assert_eq!(replies_to(&author).await.len(), 1);
    // Whoever replied follows the thread from then on.
    assert_eq!(replies_to(&first).await.len(), 1);
    assert!(ctx.get_thread(Some(&first), &thread.id).await.unwrap().viewer.following);
    // Nobody is told about their own reply.
    assert!(replies_to(&second).await.is_empty());

    // Having read the thread, the author hears about the next reply again.
    let read =
        MarkReadRequest { target: Some(mark_read_request::Target::ThreadId(thread.id.clone())), ..Default::default() };
    let _: MarkReadResponse = ctx.call(SVC, "MarkRead", Some(&author), &read).await.unwrap();
    ctx.reply(&second, &thread, "みっつめ").await;
    assert_eq!(replies_to(&author).await.len(), 2);

    // An answer to someone's reply always reaches them, read or not.
    assert_eq!(replies_to(&first).await.len(), 1);
    ctx.try_reply(
        &second,
        &CreateReplyRequest {
            thread_id: thread.id.clone(),
            body: "それな".into(),
            reply_to_id: one.id.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(replies_to(&first).await.len(), 2);

    // Unfollowing ends it, and replying again does not follow behind one's back.
    let unfollow = SetThreadFollowRequest { thread_id: thread.id.clone(), following: false, ..Default::default() };
    let _: SetThreadFollowResponse = ctx.call(SVC, "SetThreadFollow", Some(&first), &unfollow).await.unwrap();
    ctx.reply(&first, &thread, "まだ書くけど").await;
    ctx.reply(&second, &thread, "よっつめ").await;
    assert_eq!(replies_to(&first).await.len(), 2);
    assert!(!ctx.get_thread(Some(&first), &thread.id).await.unwrap().viewer.following);
}

#[tokio::test]
async fn notification_levels_and_mentions_shape_reply_notices() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let quiet = ctx.member(&owner, &community, "Quiet").await;
    let replier = ctx.member(&owner, &community, "Replier").await;
    let general = ctx.general(&owner, &community).await;
    let level = |user: &tokuly_testkit::TestUser, level: NotificationLevel| {
        let request = UpdateMembershipRequest {
            community_id: community.id.clone(),
            notification_level: Some(level.into()),
            ..Default::default()
        };
        let ctx = &ctx;
        let user = user.clone();
        async move {
            let _: UpdateMembershipResponse =
                ctx.call("CommunityService", "UpdateMembership", Some(&user), &request).await.unwrap();
        }
    };
    level(&quiet, NotificationLevel::NOTIFICATION_LEVEL_MENTIONS).await;
    let thread = ctx.thread(&quiet, &general, "静かに", "メンションだけ").await;

    // "Mentions and announcements": replies to one's own thread stay quiet…
    ctx.reply(&replier, &thread, "返信だよ").await;
    assert!(ctx.notifications(quiet.id, "community.reply").await.is_empty());
    // …a mention gets through, once, as a mention.
    let mentioning = ctx.reply(&replier, &thread, &format!("@{} @{} 見て!", quiet.handle, author.handle)).await;
    assert_eq!(mentioning.mentions.len(), 2);
    let told = ctx.notifications(quiet.id, "community.mention").await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0]["subject"][1], mentioning.id);
    assert_eq!(ctx.notification_kinds(author.id).await, vec!["community.mention"]);

    // "None" means none.
    level(&author, NotificationLevel::NOTIFICATION_LEVEL_NONE).await;
    ctx.reply(&replier, &thread, &format!("@{} もう一回", author.handle)).await;
    assert_eq!(ctx.notifications(author.id, "community.mention").await.len(), 1);

    // Nobody hears from someone they have a block with. (In the thread of
    // a third member: the two do not see each other's threads at all.)
    level(&quiet, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    let open = ctx.thread(&owner, &general, "みんなの話", "x").await;
    ctx.block(&quiet, &replier).await;
    ctx.reply(&replier, &open, &format!("@{} ブロックされてる", quiet.handle)).await;
    assert_eq!(ctx.notifications(quiet.id, "community.mention").await.len(), 1);
    assert!(ctx.notifications(quiet.id, "community.reply").await.is_empty());
    let on_theirs =
        CreateReplyRequest { thread_id: thread.id.clone(), body: "見えないはず".into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CreateReply", Some(&replier), &on_theirs).await, "THREAD_NOT_FOUND");
}

#[tokio::test]
async fn replies_are_edited_and_deleted() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let (author, other) =
        (ctx.member(&owner, &community, "Author").await, ctx.member(&owner, &community, "Other").await);
    let moderator = ctx.moderator(&owner, &community).await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&owner, &general, "スレ", "x").await;
    let reply = ctx.reply(&author, &thread, "さいしょ").await;
    let quoting = ctx
        .try_reply(
            &other,
            &CreateReplyRequest {
                thread_id: thread.id.clone(),
                body: "引用".into(),
                reply_to_id: reply.id.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let edit = |body: &str| UpdateReplyRequest { reply_id: reply.id.clone(), body: body.into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UpdateReply", Some(&other), &edit("ちがう")).await, "PERMISSION_DENIED");
    assert_eq!(ctx.reason(SVC, "UpdateReply", Some(&moderator), &edit("ちがう")).await, "PERMISSION_DENIED");
    assert_eq!(ctx.reason(SVC, "UpdateReply", Some(&author), &edit(" ")).await, "REPLY_EMPTY");
    assert_eq!(ctx.reason(SVC, "UpdateReply", Some(&author), &edit(&"あ".repeat(2001))).await, "BODY_TOO_LONG");
    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    let edited: UpdateReplyResponse = ctx.call(SVC, "UpdateReply", Some(&author), &edit("なおした")).await.unwrap();
    assert_eq!(edited.reply.body, "なおした");
    assert!(edited.reply.edited_at.as_option().is_some());
    assert!(
        pushes(&mut topic)
            .await
            .iter()
            .any(|push| matches!(push, Pushed::ReplyUpdated(update) if update.reply.body == "なおした"))
    );
    // The quote follows the edit.
    assert_eq!(ctx.get_reply(None, &quoting.id).await.unwrap().reply_to.excerpt, "なおした");

    let delete = |reply: &Reply, reason: &str| DeleteReplyRequest {
        reply_id: reply.id.clone(),
        reason: reason.into(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(SVC, "DeleteReply", Some(&other), &delete(&reply, "いや")).await, "PERMISSION_DENIED");
    assert_eq!(ctx.reason(SVC, "DeleteReply", Some(&moderator), &delete(&reply, "")).await, "REASON_REQUIRED");
    let _: DeleteReplyResponse =
        ctx.call(SVC, "DeleteReply", Some(&moderator), &delete(&reply, "荒らし")).await.unwrap();
    assert!(pushes(&mut topic).await.iter().any(|push| matches!(push, Pushed::ReplyRemoved(removed)
        if removed.reply_id == reply.id && removed.reply_count == 1)));
    assert_eq!(ctx.get_reply(Some(&author), &reply.id).await.unwrap_err().reason, "REPLY_NOT_FOUND");
    assert_eq!(ctx.get_thread(None, &thread.id).await.unwrap().reply_count, 1);
    // What quoted it says the reply is gone; numbers keep their gaps.
    let left = ctx.replies(None, &thread.id).await;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].number, 2);
    assert!(left[0].reply_to.unavailable && left[0].reply_to.excerpt.is_empty());
    assert_eq!(ctx.notifications(author.id, "community.moderation").await[0]["body"]["ja"], "荒らし");
    let log = ctx.moderation_log(&owner, &community).await;
    let entry = log.iter().find(|entry| entry.action == ModerationAction::MODERATION_ACTION_REPLY_DELETED).unwrap();
    assert_eq!(
        (entry.reason.as_str(), entry.reply_id.as_str(), entry.target_user.id.as_str()),
        ("荒らし", reply.id.as_str(), author.id.to_string().as_str())
    );

    // An author deletes their own without a word.
    let _: DeleteReplyResponse = ctx.call(SVC, "DeleteReply", Some(&other), &delete(&quoting, "")).await.unwrap();
    assert_eq!(ctx.get_thread(None, &thread.id).await.unwrap().reply_count, 0);
    assert_eq!(ctx.reason(SVC, "DeleteReply", Some(&other), &delete(&quoting, "")).await, "REPLY_NOT_FOUND");
}
