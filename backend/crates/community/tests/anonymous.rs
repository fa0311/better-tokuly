//! Anonymous channels: pseudonyms, what no response may tell, and the
//! logged way the staff finds out who wrote a post.
mod common;

use common::*;
use serde::Serialize;
use tokuly_proto::{
    common::SubjectType,
    community::{community_event::Event as Pushed, *},
    media::AssetPurpose,
};
use tokuly_testkit::TestUser;

const SVC: &str = "CommunityBoardService";
const MODERATION: &str = "CommunityModerationService";

/// A public community with an anonymous channel, an author with a name
/// that cannot be mistaken for anything else, a reader and a moderator.
struct Scene {
    owner: TestUser,
    author: TestUser,
    reader: TestUser,
    moderator: TestUser,
    community: Community,
    channel: Channel,
}

async fn scene(ctx: &Ctx) -> Scene {
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Zebediah Secretkeeper").await;
    let reader = ctx.member(&owner, &community, "Reader").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let channel = ctx.channel(&owner, &community, "ないしょ話", true).await;
    Scene { owner, author, reader, moderator, community, channel }
}

/// Fails if `response` tells who the author is, in any of the ways an
/// account can be named.
fn assert_unnamed<T: Serialize>(what: &str, response: &T, author: &TestUser) {
    for needle in [author.id.to_string(), author.id.simple().to_string(), author.handle.clone(), "Zebediah".to_owned()]
    {
        assert!(!mentions(response, &needle), "{what} names the author ({needle})");
    }
}

fn same_name(a: &Pseudonym, b: &Pseudonym) -> bool {
    a == b
}

#[tokio::test]
async fn a_pseudonym_is_stable_within_a_thread_and_differs_across_threads() {
    let ctx = start().await;
    let s = scene(&ctx).await;
    assert!(s.channel.anonymous);
    let thread = ctx.thread(&s.author, &s.channel, "ここだけの話", "実は…").await;
    let again = ctx.thread(&s.author, &s.channel, "もうひとつ", "これも…").await;
    let own_reply = ctx.reply(&s.author, &thread, "補足").await;
    let own_second = ctx.reply(&s.author, &thread, "さらに補足").await;
    let others_reply = ctx.reply(&s.reader, &thread, "わかる").await;
    let other_thread_reply = ctx.reply(&s.author, &again, "別スレでの返信").await;

    // No account anywhere; a pseudonym everywhere.
    let name = thread.pseudonym.as_option().expect("a pseudonym").clone();
    assert!(thread.author.as_option().is_none());
    assert!(!name.name.ja.is_empty() && !name.name.en.is_empty() && !name.emoji.is_empty());
    assert_eq!(name.tag.len(), 4);
    for reply in [&own_reply, &own_second, &others_reply] {
        assert!(reply.author.as_option().is_none());
        assert!(reply.pseudonym.as_option().is_some());
    }

    // Stable within the thread: the author is the same name on the thread
    // and on each of their replies, and reads as the thread's author.
    assert!(same_name(&name, &own_reply.pseudonym));
    assert!(same_name(&name, &own_second.pseudonym));
    assert!(own_reply.by_thread_author && !others_reply.by_thread_author);
    // Also when read back later, by anyone.
    let read_back = ctx.get_thread(Some(&s.reader), &thread.id).await.unwrap();
    assert!(same_name(&name, &read_back.pseudonym));
    let listed = ctx.replies(None, &thread.id).await;
    assert!(same_name(&name, &listed[0].pseudonym) && same_name(&name, &listed[1].pseudonym));

    // Someone else in the same thread is someone else.
    assert!(!same_name(&name, &others_reply.pseudonym));
    // The same author in another thread is someone else too: nothing links the two.
    let other_name = again.pseudonym.as_option().unwrap();
    assert!(!same_name(&name, other_name));
    assert_ne!(name.tag, other_name.tag);
    assert!(same_name(other_name, &other_thread_reply.pseudonym));

    // The pseudonym is what the pure function says, under the server's key.
    let expected = tokuly_community::pseudonym::pseudonym(&ctx.app.core.keys, thread.id.parse().unwrap(), s.author.id);
    assert!(same_name(&name, &expected));
}

#[tokio::test]
async fn no_member_facing_response_names_the_author() {
    let ctx = start().await;
    let s = scene(&ctx).await;
    let site_moderator = ctx.app.user_with_roles("SiteMod", &["moderator"]).await;
    let mut topic = ctx.app.listen_topic(&format!("community:{}", s.community.id)).await;
    let mut reader_frames = ctx.app.listen_user(s.reader.id).await;

    // The author writes a thread (with a poll, mentioning the reader) and
    // replies; the reader answers that reply; the author's thread becomes
    // an announcement and gets pinned.
    let request = CreateThreadRequest {
        channel_id: s.channel.id.clone(),
        title: "だれにも言えない話".into(),
        body: format!("@{} にだけ聞いてほしい", s.reader.handle),
        poll: PollInput { options: vec!["わかる".into(), "わからない".into()], ..Default::default() }.into(),
        tags: vec!["相談".into()],
        ..Default::default()
    };
    let thread = ctx.try_thread(&s.author, &request).await.unwrap();
    let authors_reply = ctx.reply(&s.author, &thread, &format!("補足です @{}", s.reader.handle)).await;
    let answer = ctx
        .try_reply(
            &s.reader,
            &CreateReplyRequest {
                thread_id: thread.id.clone(),
                body: "聞いたよ".into(),
                reply_to_id: authors_reply.id.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let pin = PinThreadRequest { thread_id: thread.id.clone(), pinned: true, ..Default::default() };
    let pinned: PinThreadResponse = ctx.call(SVC, "PinThread", Some(&s.moderator), &pin).await.unwrap();
    let announce = SetAnnouncementRequest { thread_id: thread.id.clone(), announcement: true, ..Default::default() };
    let announced: SetAnnouncementResponse =
        ctx.call(SVC, "SetAnnouncement", Some(&s.moderator), &announce).await.unwrap();
    ctx.community.run_announcement(thread.id.parse().unwrap()).await.unwrap();
    let follow = SetThreadFollowRequest { thread_id: thread.id.clone(), following: true, ..Default::default() };
    let _: SetThreadFollowResponse = ctx.call(SVC, "SetThreadFollow", Some(&s.reader), &follow).await.unwrap();

    // Everything anyone but the author can read about these posts.
    assert_unnamed("the answer to the author's reply", &answer, &s.author);
    assert_unnamed("PinThread", &pinned, &s.author);
    assert_unnamed("SetAnnouncement", &announced, &s.author);
    let list = ListThreadsRequest {
        community_id: s.community.id.clone(),
        sort: ThreadSort::THREAD_SORT_NEW.into(),
        ..Default::default()
    };
    let announcements = ListAnnouncementsRequest { community_id: s.community.id.clone(), ..Default::default() };
    for (who, viewer) in [
        ("a member", Some(&s.reader)),
        ("a moderator", Some(&s.moderator)),
        ("the owner", Some(&s.owner)),
        ("site moderation", Some(&site_moderator)),
        ("a guest", None),
    ] {
        assert_unnamed(&format!("GetThread for {who}"), &ctx.get_thread(viewer, &thread.id).await.unwrap(), &s.author);
        assert_unnamed(&format!("ListThreads for {who}"), &ctx.list_threads(viewer, &list).await, &s.author);
        assert_unnamed(&format!("ListReplies for {who}"), &ctx.replies(viewer, &thread.id).await, &s.author);
        assert_unnamed(
            &format!("GetReply for {who}"),
            &ctx.get_reply(viewer, &authors_reply.id).await.unwrap(),
            &s.author,
        );
        assert_unnamed(
            &format!("the quoting reply for {who}"),
            &ctx.get_reply(viewer, &answer.id).await.unwrap(),
            &s.author,
        );
        let listed: ListAnnouncementsResponse =
            ctx.call(SVC, "ListAnnouncements", viewer, &announcements).await.unwrap();
        assert_eq!(listed.threads.len(), 1);
        assert_unnamed(&format!("ListAnnouncements for {who}"), &listed, &s.author);
    }
    let followed: ListFollowedThreadsResponse =
        ctx.call(SVC, "ListFollowedThreads", Some(&s.reader), &ListFollowedThreadsRequest::default()).await.unwrap();
    assert_eq!(followed.threads.len(), 1);
    assert_unnamed("ListFollowedThreads", &followed, &s.author);
    let vote = VotePollRequest {
        thread_id: thread.id.clone(),
        option_ids: vec![thread.poll.options[0].id.clone()],
        ..Default::default()
    };
    let voted: VotePollResponse = ctx.call(SVC, "VotePoll", Some(&s.reader), &vote).await.unwrap();
    assert_unnamed("VotePoll", &voted, &s.author);

    // The quote of the author's reply carries the pseudonym, not the account.
    assert!(answer.reply_to.author.as_option().is_none());
    assert!(same_pseudonym(&answer.reply_to.pseudonym, &thread.pseudonym));

    // Only the author is told the posts are theirs.
    let own = ctx.get_thread(Some(&s.author), &thread.id).await.unwrap();
    assert!(own.viewer.mine && own.author.as_option().is_none());
    assert!(!ctx.get_thread(Some(&s.reader), &thread.id).await.unwrap().viewer.mine);
    assert!(ctx.replies(Some(&s.author), &thread.id).await[0].viewer.mine);

    // Realtime: the posts travel on the topic with their pseudonyms.
    let pushed = pushes(&mut topic).await;
    assert!(
        pushed.iter().any(
            |push| matches!(push, Pushed::ThreadCreated(created) if created.thread.pseudonym.as_option().is_some())
        )
    );
    assert!(
        pushed.iter().any(|push| matches!(push, Pushed::ReplyCreated(created) if created.reply_id == authors_reply.id))
    );
    assert_unnamed("the topic", &pushed, &s.author);
    assert_unnamed("pushes to the reader", &pushes(&mut reader_frames).await, &s.author);

    // Notifications: a pseudonym in the text, and no actor whose avatar would tell.
    let mut notices = Vec::new();
    for user in [&s.reader, &s.moderator, &s.owner] {
        for kind in ["community.mention", "community.reply", "community.announcement"] {
            notices.extend(ctx.notifications(user.id, kind).await);
        }
    }
    assert!(notices.iter().any(|notice| notice["kind"] == "community.mention"));
    assert!(notices.iter().any(|notice| notice["kind"] == "community.announcement"));
    for notice in &notices {
        assert_unnamed("a notification", notice, &s.author);
        if notice["kind"] != "community.reply" {
            assert!(notice["actor_id"].is_null(), "an anonymous post has no actor: {notice}");
        }
    }
    let mention = ctx.notifications(s.reader.id, "community.mention").await.remove(0);
    assert!(mention["title"]["ja"].as_str().unwrap().contains(&thread.pseudonym.name.ja));

    // The bus: no author, no actor — nothing for feeds or webhooks to show.
    for topic in ["community.thread.created", "community.reply.created"] {
        let events: Vec<_> = ctx
            .community_events(topic, &s.community.id)
            .await
            .into_iter()
            .filter(|event| event.data["anonymous"] == true)
            .collect();
        assert!(!events.is_empty());
        assert_unnamed(topic, &events, &s.author);
    }
    assert!(ctx.app.events_by("community.thread.created", s.author.id).await.is_empty());
    // Search: found by its words, owned by nobody.
    let doc = ctx.search_changes("thread", &thread.id).await.remove(0);
    assert!(doc["doc"]["owner_id"].is_null());
    assert_unnamed("the search document", &doc, &s.author);
}

fn same_pseudonym(a: &Pseudonym, b: &Pseudonym) -> bool {
    a == b
}

#[tokio::test]
async fn moderation_never_unmasks_by_accident() {
    let ctx = start().await;
    let s = scene(&ctx).await;
    ctx.mute_words(&s.moderator, &s.community, &["ひみつ"]).await;
    // One post is held by the word filter, one is deleted by a moderator.
    let held = ctx.thread(&s.author, &s.channel, "これはひみつ", "確認待ちになる").await;
    assert_eq!(held.state, PostState::POST_STATE_HIDDEN);
    let thread = ctx.thread(&s.author, &s.channel, "ふつうの話", "x").await;
    let reply = ctx.reply(&s.author, &thread, "消される返信").await;
    let delete =
        DeleteReplyRequest { reply_id: reply.id.clone(), reason: "ルール違反".into(), ..Default::default() };
    let _: DeleteReplyResponse = ctx.call(SVC, "DeleteReply", Some(&s.moderator), &delete).await.unwrap();

    // The review queue shows the post under its pseudonym.
    let queue = ctx.review_queue(&s.moderator, &s.community).await;
    assert_eq!(queue.len(), 1);
    assert_unnamed("the review queue", &queue, &s.author);
    // The log shows what was done, to a pseudonym.
    for staff in [&s.moderator, &s.owner] {
        let log = ctx.moderation_log(staff, &s.community).await;
        assert_unnamed("the moderation log", &log, &s.author);
        let deleted =
            log.iter().find(|entry| entry.action == ModerationAction::MODERATION_ACTION_REPLY_DELETED).unwrap();
        assert!(deleted.target_user.as_option().is_none());
        assert!(
            same_pseudonym(&deleted.target_pseudonym, &thread.pseudonym),
            "the log names the pseudonym of that thread"
        );
        let hidden = log.iter().find(|entry| entry.action == ModerationAction::MODERATION_ACTION_POST_HIDDEN).unwrap();
        assert!(hidden.target_user.as_option().is_none() && hidden.target_pseudonym.as_option().is_some());
    }
    // The staff's notices about the held post name nobody either.
    for notice in ctx.notifications(s.owner.id, "community.review").await {
        assert_unnamed("the review notice", &notice, &s.author);
    }
}

#[tokio::test]
async fn the_staff_looks_up_an_author_and_leaves_a_trace() {
    let ctx = start().await;
    let s = scene(&ctx).await;
    let admin = ctx.staff(&s.owner, &s.community, "Admin", MemberRole::MEMBER_ROLE_ADMIN).await;
    let thread = ctx.thread(&s.author, &s.channel, "いじめの書き込み", "ひどい内容").await;
    let reply = ctx.reply(&s.author, &thread, "さらにひどい").await;
    let reveal = |post, reason: &str| RevealAuthorRequest { post, reason: reason.into(), ..Default::default() };

    // Members cannot, and the staff must say why.
    assert_eq!(
        ctx.reason(MODERATION, "RevealAuthor", Some(&s.reader), &reveal(thread_ref(&thread.id), "知りたい")).await,
        "PERMISSION_DENIED"
    );
    assert_eq!(
        ctx.reason(MODERATION, "RevealAuthor", Some(&s.moderator), &reveal(thread_ref(&thread.id), " ")).await,
        "REASON_REQUIRED"
    );
    assert_eq!(
        ctx.reason(MODERATION, "RevealAuthor", Some(&s.moderator), &RevealAuthorRequest::default()).await,
        "POST_REQUIRED"
    );
    assert_eq!(
        ctx.reason(MODERATION, "RevealAuthor", None, &reveal(thread_ref(&thread.id), "x")).await,
        "SIGN_IN_REQUIRED"
    );

    let mut owner_frames = ctx.app.listen_user(s.owner.id).await;
    let mut moderator_frames = ctx.app.listen_user(s.moderator.id).await;
    let revealed: RevealAuthorResponse = ctx
        .call(MODERATION, "RevealAuthor", Some(&s.moderator), &reveal(thread_ref(&thread.id), "いじめの通報があった"))
        .await
        .unwrap();
    assert_eq!(revealed.author.id, s.author.id.to_string());
    assert_eq!(revealed.role, MemberRole::MEMBER_ROLE_MEMBER);
    let of_reply: RevealAuthorResponse = ctx
        .call(MODERATION, "RevealAuthor", Some(&admin), &reveal(reply_ref(&reply.id), "同じ人か確認"))
        .await
        .unwrap();
    assert_eq!(of_reply.author.id, s.author.id.to_string());

    // The lookup is in the log — who looked, at what, why — without the answer.
    let log = ctx.moderation_log(&s.owner, &s.community).await;
    let entries: Vec<_> =
        log.iter().filter(|entry| entry.action == ModerationAction::MODERATION_ACTION_AUTHOR_REVEALED).collect();
    assert_eq!(entries.len(), 2);
    let entry = entries.iter().find(|entry| entry.actor.id == s.moderator.id.to_string()).unwrap();
    assert_eq!((entry.thread_id.as_str(), entry.reason.as_str()), (thread.id.as_str(), "いじめの通報があった"));
    assert!(!entry.site_moderation);
    assert!(entry.target_user.as_option().is_none() && entry.target_pseudonym.as_option().is_some());
    assert_unnamed("the log of lookups", &log, &s.author);

    // The rest of the staff is told at once; the one who looked is not told about themselves.
    let to_owner = pushes(&mut owner_frames).await;
    let announced = to_owner.iter().find_map(|push| match push {
        Pushed::AuthorRevealed(event) if event.actor.id == s.moderator.id.to_string() => Some(event.clone()),
        _ => None,
    });
    let announced = announced.expect("the owner is told about the moderator's lookup");
    assert_eq!((announced.thread_id.as_str(), announced.reason.as_str()), (thread.id.as_str(), "いじめの通報があった"));
    assert_unnamed("the announcement to the staff", &to_owner, &s.author);
    assert!(
        !pushes(&mut moderator_frames)
            .await
            .iter()
            .any(|push| matches!(push, Pushed::AuthorRevealed(event) if event.actor.id == s.moderator.id.to_string()))
    );
    let notices = ctx.notifications(s.owner.id, "community.author_revealed").await;
    assert_eq!(notices.len(), 2);
    assert_unnamed("the notification to the staff", &notices, &s.author);
    assert_eq!(ctx.notifications(admin.id, "community.author_revealed").await.len(), 1, "only the moderator's lookup");
    assert!(ctx.notifications(s.reader.id, "community.author_revealed").await.is_empty());

    // A post that names its author has nothing to reveal.
    let general = ctx.general(&s.owner, &s.community).await;
    let named = ctx.thread(&s.author, &general, "名前つき", "x").await;
    assert_eq!(
        ctx.reason(MODERATION, "RevealAuthor", Some(&s.moderator), &reveal(thread_ref(&named.id), "x")).await,
        "POST_NOT_ANONYMOUS"
    );
}

#[tokio::test]
async fn site_moderation_can_look_up_an_author_too() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let hidden = ctx.hidden(&owner).await;
    let author = ctx.member(&owner, &hidden, "Zebediah Secretkeeper").await;
    let channel = ctx.channel(&owner, &hidden, "匿名", true).await;
    let thread = ctx.thread(&author, &channel, "通報された話", "x").await;
    let site_moderator = ctx.app.user_with_roles("SiteMod", &["moderator"]).await;
    let stranger = ctx.app.user("Stranger").await;
    let request =
        RevealAuthorRequest { post: thread_ref(&thread.id), reason: "通報の対応".into(), ..Default::default() };
    // To a stranger the hidden community's thread does not exist.
    assert_eq!(ctx.reason(MODERATION, "RevealAuthor", Some(&stranger), &request).await, "THREAD_NOT_FOUND");

    let revealed: RevealAuthorResponse =
        ctx.call(MODERATION, "RevealAuthor", Some(&site_moderator), &request).await.unwrap();
    assert_eq!(revealed.author.id, author.id.to_string());
    // The community's own staff sees that site moderation looked.
    let log = ctx.moderation_log(&owner, &hidden).await;
    let entry = log.iter().find(|entry| entry.action == ModerationAction::MODERATION_ACTION_AUTHOR_REVEALED).unwrap();
    assert!(entry.site_moderation);
    assert_eq!(entry.actor.id, site_moderator.id.to_string());
    assert_eq!(ctx.notifications(owner.id, "community.author_revealed").await.len(), 1);
    // And site moderation can read that log, though it is not staff here.
    assert_eq!(ctx.moderation_log(&site_moderator, &hidden).await.len(), log.len());
}

#[tokio::test]
async fn reports_name_nobody_and_site_moderation_asks_for_the_author() {
    let ctx = start().await;
    let s = scene(&ctx).await;
    let thread = ctx.thread(&s.author, &s.channel, "通報される", "x").await;
    let reply = ctx.reply(&s.author, &thread, "これも").await;
    // The generic domains act on a subject's owner — engagement hides a
    // subject from anyone in a block with its owner and purges it with its
    // owner's account — so an anonymous post tells them it has none.
    let subjects = &ctx.app.core.subjects;
    use SubjectType::*;
    for viewer in [Some(s.reader.id), Some(s.author.id), Some(s.moderator.id), None] {
        let as_thread = subjects.resolve(SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id, viewer).await.unwrap();
        assert!(as_thread.owner_id.is_nil());
        let as_reply = subjects.resolve(SUBJECT_TYPE_COMMUNITY_REPLY, &reply.id, viewer).await.unwrap();
        assert!(as_reply.owner_id.is_nil());
        // What is shown of the subject (title, path) says nothing about them either.
        assert!(!as_thread.title.contains("Zebediah") && !as_thread.path.contains(&s.author.handle));
        assert!(!as_reply.title.contains("Zebediah") && !as_reply.path.contains(&s.author.handle));
    }
    // Site moderation, working on a report, asks this domain who wrote it.
    let asked = |kind: SubjectType, id: &str| {
        let (ctx, id) = (&ctx, id.parse::<uuid::Uuid>().unwrap());
        async move { ctx.community.author_for_moderation(kind, id).await.unwrap() }
    };
    assert_eq!(asked(SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id).await, Some(s.author.id));
    assert_eq!(asked(SUBJECT_TYPE_COMMUNITY_REPLY, &reply.id).await, Some(s.author.id));
    assert_eq!(asked(SUBJECT_TYPE_COMMUNITY, &s.community.id).await, Some(s.owner.id));
    assert_eq!(asked(SUBJECT_TYPE_COMMUNITY_THREAD, &uuid::Uuid::now_v7().to_string()).await, None);
    assert_eq!(asked(SUBJECT_TYPE_NOTE, &thread.id).await, None);
    // A post that names its author is its author's, as everywhere else.
    let general = ctx.general(&s.owner, &s.community).await;
    let named = ctx.thread(&s.author, &general, "名前つき", "x").await;
    let as_named = subjects.resolve(SUBJECT_TYPE_COMMUNITY_THREAD, &named.id, Some(s.reader.id)).await.unwrap();
    assert_eq!(as_named.owner_id, s.author.id);
}

#[tokio::test]
async fn blocks_do_not_filter_anonymous_posts() {
    let ctx = start().await;
    let s = scene(&ctx).await;
    let general = ctx.general(&s.owner, &s.community).await;
    let anonymous = ctx.thread(&s.author, &s.channel, "匿名の話", "x").await;
    let named = ctx.thread(&s.author, &general, "名前つきの話", "x").await;
    let anonymous_reply = ctx.reply(&s.author, &anonymous, "匿名の返信").await;
    ctx.block(&s.reader, &s.author).await;

    // Hiding the anonymous posts would tell the reader who wrote them:
    // block someone, see what disappears.
    let titles = ctx.thread_titles(Some(&s.reader), &s.community).await;
    assert!(titles.contains(&"匿名の話".to_owned()));
    assert!(!titles.contains(&"名前つきの話".to_owned()));
    assert!(ctx.get_thread(Some(&s.reader), &anonymous.id).await.is_ok());
    assert_eq!(ctx.get_thread(Some(&s.reader), &named.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    assert_eq!(ctx.replies(Some(&s.reader), &anonymous.id).await.len(), 1);
    assert!(ctx.get_reply(Some(&s.reader), &anonymous_reply.id).await.is_ok());

    // Notifications behave the same with and without the block.
    let mentioning = ctx.reply(&s.author, &anonymous, &format!("@{} 読んで", s.reader.handle)).await;
    assert_eq!(ctx.notifications(s.reader.id, "community.mention").await.len(), 1);
    // And the post travels on the topic like any anonymous post.
    let mut topic = ctx.app.listen_topic(&format!("community:{}", s.community.id)).await;
    let later = ctx.reply(&s.author, &anonymous, "ブロック後の返信").await;
    assert!(pushes(&mut topic).await.iter().any(|push| matches!(push, Pushed::ReplyCreated(created)
        if created.reply_id == later.id && created.reply.as_option().is_some())));
    let _ = mentioning;

    // The other way round too: the author blocks the reader, and the reader
    // still reads the thread and writes in it like anyone — "not found"
    // would tell them who is behind the pseudonym.
    ctx.block(&s.author, &s.reader).await;
    assert!(ctx.get_thread(Some(&s.reader), &anonymous.id).await.is_ok());
    let quoting = CreateReplyRequest {
        thread_id: anonymous.id.clone(),
        body: "引用して返信".into(),
        reply_to_id: anonymous_reply.id.clone(),
        ..Default::default()
    };
    let answer = ctx.try_reply(&s.reader, &quoting).await.unwrap();
    assert!(!answer.reply_to.unavailable && answer.reply_to.excerpt == "匿名の返信");
    // And the author hears of it like of any other reply.
    assert_eq!(ctx.notifications(s.author.id, "community.reply").await.len(), 1);
}

#[tokio::test]
async fn a_channel_with_threads_keeps_its_mode() {
    let ctx = start().await;
    let s = scene(&ctx).await;
    let unmask =
        UpdateChannelRequest { channel_id: s.channel.id.clone(), anonymous: Some(false), ..Default::default() };
    // While empty, the mode can still be changed back and forth.
    let changed: UpdateChannelResponse = ctx.call(SVC, "UpdateChannel", Some(&s.owner), &unmask).await.unwrap();
    assert!(!changed.channel.anonymous);
    let mask = UpdateChannelRequest { channel_id: s.channel.id.clone(), anonymous: Some(true), ..Default::default() };
    let _: UpdateChannelResponse = ctx.call(SVC, "UpdateChannel", Some(&s.owner), &mask).await.unwrap();

    // Once people wrote under a pseudonym, nobody turns the names on.
    let thread = ctx.thread(&s.author, &s.channel, "匿名のつもりで書いた", "x").await;
    assert_eq!(ctx.reason(SVC, "UpdateChannel", Some(&s.owner), &unmask).await, "CHANNEL_NOT_EMPTY");
    assert!(ctx.get_thread(Some(&s.reader), &thread.id).await.unwrap().author.as_option().is_none());
    // Other edits still work.
    let rename = UpdateChannelRequest {
        channel_id: s.channel.id.clone(),
        name: Some("ひみつ基地".into()),
        ..Default::default()
    };
    let renamed: UpdateChannelResponse = ctx.call(SVC, "UpdateChannel", Some(&s.owner), &rename).await.unwrap();
    assert!(renamed.channel.anonymous);
}

/// Erases an account the way identity does, and tells the community domain.
async fn erase(ctx: &Ctx, user: &TestUser) {
    sqlx::query(
        "UPDATE users SET deleted_at = now(), handle = ('deleted-' || replace(id::text, '-', ''))::citext, display_name = ''
         WHERE id = $1",
    )
    .bind(user.id)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    ctx.deliver("identity.user.deleted", Some(user.id), serde_json::json!({ "user_id": user.id })).await;
}

#[tokio::test]
async fn nothing_a_member_sees_moves_with_the_authors_account() {
    let ctx = start().await;
    let s = scene(&ctx).await;
    let image = ctx.image(&s.author, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await;
    let request = CreateThreadRequest {
        channel_id: s.channel.id.clone(),
        title: "だれが書いたか分からない話".into(),
        body: "写真つき".into(),
        asset_ids: vec![image.clone()],
        poll: PollInput { options: vec!["はい".into(), "いいえ".into()], ..Default::default() }.into(),
        ..Default::default()
    };
    let thread = ctx.try_thread(&s.author, &request).await.unwrap();
    let reply = ctx.reply(&s.author, &thread, "匿名の返信").await;
    let doomed = ctx.reply(&s.author, &thread, "モデレーターに消される返信").await;
    let delete =
        DeleteReplyRequest { reply_id: doomed.id.clone(), reason: "ルール違反".into(), ..Default::default() };
    let _: DeleteReplyResponse = ctx.call(SVC, "DeleteReply", Some(&s.moderator), &delete).await.unwrap();
    for voter in [&s.author, &s.reader] {
        let vote = VotePollRequest {
            thread_id: thread.id.clone(),
            option_ids: vec![thread.poll.options[0].id.clone()],
            ..Default::default()
        };
        let _: VotePollResponse = ctx.call(SVC, "VotePoll", Some(voter), &vote).await.unwrap();
    }

    // Everything the others can read about the author's posts: the thread
    // (with its poll and picture), the replies, the list, what the staff's
    // log says about the deleted reply, and whose the subject is.
    let list = ListThreadsRequest {
        community_id: s.community.id.clone(),
        sort: ThreadSort::THREAD_SORT_NEW.into(),
        ..Default::default()
    };
    let seen = || {
        let (ctx, s, thread, reply, list) = (&ctx, &s, &thread, &reply, &list);
        async move {
            let log: Vec<_> = ctx
                .moderation_log(&s.moderator, &s.community)
                .await
                .into_iter()
                .filter(|entry| entry.action == ModerationAction::MODERATION_ACTION_REPLY_DELETED)
                .collect();
            let subject = ctx
                .app
                .core
                .subjects
                .resolve(SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id, Some(s.reader.id))
                .await
                .unwrap();
            serde_json::json!({
                "thread": ctx.get_thread(Some(&s.reader), &thread.id).await.unwrap(),
                "as a guest": ctx.get_thread(None, &thread.id).await.unwrap(),
                "as the staff": ctx.get_thread(Some(&s.moderator), &thread.id).await.unwrap(),
                "reply": ctx.get_reply(Some(&s.reader), &reply.id).await.unwrap(),
                "replies": ctx.replies(Some(&s.reader), &thread.id).await,
                "list": ctx.list_threads(Some(&s.reader), list).await,
                "log": log,
                "subject": [subject.owner_id.to_string(), subject.title, subject.path, subject.interactions_enabled.to_string()],
            })
        }
    };
    // (Read once first: reading moves the reader's own unread markers.)
    seen().await;
    let before = seen().await;
    assert_eq!(before["log"].as_array().unwrap().len(), 1);
    assert!(before["thread"]["poll"]["voterCount"] == 2 && !before["thread"]["images"].as_array().unwrap().is_empty());

    // The author leaves the community…
    let leave = LeaveCommunityRequest { community_id: s.community.id.clone(), ..Default::default() };
    let _: LeaveCommunityResponse =
        ctx.call("CommunityService", "LeaveCommunity", Some(&s.author), &leave).await.unwrap();
    assert_eq!(seen().await, before, "after the author left");
    // …is banned from it…
    let ban = BanMemberRequest {
        community_id: s.community.id.clone(),
        user_id: s.author.id.to_string(),
        reason: "別件".into(),
        ..Default::default()
    };
    let _: BanMemberResponse = ctx.call(MODERATION, "BanMember", Some(&s.owner), &ban).await.unwrap();
    assert_eq!(seen().await, before, "after the author was banned from the community");
    // …is suspended from the whole site…
    sqlx::query("UPDATE users SET banned_at = now() WHERE id = $1")
        .bind(s.author.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(seen().await, before, "after the author's account was suspended");
    // …and finally erases the account.
    erase(&ctx, &s.author).await;
    assert_eq!(seen().await, before, "after the author's account was erased");
    // The picture is still the post's (the media domain only collects
    // uploads nothing holds), and the vote is gone without a trace in the numbers.
    assert!(ctx.is_held(&image, &format!("community:thread:{}", thread.id)).await);
    let votes: i64 = sqlx::query_scalar("SELECT count(*) FROM community_poll_votes WHERE user_id = $1")
        .bind(s.author.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(votes, 0);
    // The staff can still find out whose it was: an erased account, by its ID.
    let reveal =
        RevealAuthorRequest { post: thread_ref(&thread.id), reason: "退会後の確認".into(), ..Default::default() };
    let revealed: RevealAuthorResponse =
        ctx.call(MODERATION, "RevealAuthor", Some(&s.moderator), &reveal).await.unwrap();
    assert_eq!(revealed.author.id, s.author.id.to_string());
    assert!(revealed.author.deleted);
}

#[tokio::test]
async fn the_staff_is_not_exempt_where_an_exemption_would_show() {
    let ctx = start().await;
    let s = scene(&ctx).await;
    let general = ctx.general(&s.owner, &s.community).await;
    ctx.mute_words(&s.owner, &s.community, &["だめ"]).await;

    // The word filter: a moderator's post goes through where it carries
    // their name, and is held like anyone's where it does not — a post
    // that sails past the filter would be known to be the staff's.
    let named = ctx.thread(&s.moderator, &general, "だめって言わないで", "x").await;
    assert_eq!(named.state, PostState::POST_STATE_VISIBLE);
    let held = ctx.thread(&s.moderator, &s.channel, "だめって言わないで", "x").await;
    assert_eq!(
        (held.state, held.hidden_reason),
        (PostState::POST_STATE_HIDDEN.into(), HiddenReason::HIDDEN_REASON_MUTED_WORD.into())
    );
    assert_eq!(ctx.get_thread(Some(&s.reader), &held.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");

    // Slow mode: the same.
    for channel in [&general, &s.channel] {
        let slow = SetSlowModeRequest { channel_id: channel.id.clone(), seconds: 600, ..Default::default() };
        let _: SetSlowModeResponse = ctx.call(MODERATION, "SetSlowMode", Some(&s.owner), &slow).await.unwrap();
    }
    ctx.thread(&s.moderator, &general, "名前つき1", "x").await;
    ctx.thread(&s.moderator, &general, "名前つき2", "x").await;
    let visible = ctx.thread(&s.moderator, &s.channel, "匿名1", "x").await;
    let second = CreateThreadRequest {
        channel_id: s.channel.id.clone(),
        title: "匿名2".into(),
        body: "x".into(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(SVC, "CreateThread", Some(&s.moderator), &second).await, "SLOW_MODE");
    // The owner too: one post, then the wait.
    ctx.thread(&s.owner, &s.channel, "オーナーの匿名1", "x").await;
    let again = CreateThreadRequest { title: "オーナーの匿名2".into(), ..second.clone() };
    assert_eq!(ctx.reason(SVC, "CreateThread", Some(&s.owner), &again).await, "SLOW_MODE");

    // Reports: three members hide a moderator's anonymous post, as they
    // would anyone's.
    for reporter in [&s.owner, &s.author, &s.reader] {
        ctx.report(reporter, SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, &visible.id).await;
        ctx.report(reporter, SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, &named.id).await;
    }
    assert_eq!(ctx.get_thread(Some(&s.reader), &visible.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    assert!(
        ctx.get_thread(Some(&s.reader), &named.id).await.is_ok(),
        "the staff's named posts are not hidden by reports"
    );
}

#[tokio::test]
async fn an_anonymous_post_carries_nothing_that_names_its_author() {
    let ctx = start().await;
    let s = scene(&ctx).await;
    let general = ctx.general(&s.owner, &s.community).await;
    let thread = ctx.thread(&s.reader, &s.channel, "匿名スレ", "x").await;
    let in_channel = |channel: &Channel| CreateThreadRequest {
        channel_id: channel.id.clone(),
        title: "スタンプ".into(),
        ..Default::default()
    };

    // A personal stamp is its maker's: fine under a name, refused under a pseudonym.
    let stamp = ctx.stamp(&s.author).await;
    let with_stamp = |channel: &Channel| CreateThreadRequest { stamp_id: stamp.clone(), ..in_channel(channel) };
    assert!(ctx.try_thread(&s.author, &with_stamp(&general)).await.is_ok());
    assert_eq!(ctx.reason(SVC, "CreateThread", Some(&s.author), &with_stamp(&s.channel)).await, "STAMP_NOT_ALLOWED");
    let stamp_reply =
        CreateReplyRequest { thread_id: thread.id.clone(), stamp_id: stamp.clone(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CreateReply", Some(&s.author), &stamp_reply).await, "STAMP_NOT_ALLOWED");

    // An upload that is shown somewhere under the author's name would link
    // the two posts: under a pseudonym only a fresh upload is taken.
    let used = ctx.image(&s.author, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await;
    let with_image = |channel: &Channel, asset: &str| CreateThreadRequest {
        asset_ids: vec![asset.to_owned()],
        ..in_channel(channel)
    };
    assert!(ctx.try_thread(&s.author, &with_image(&general, &used)).await.is_ok());
    assert_eq!(
        ctx.reason(SVC, "CreateThread", Some(&s.author), &with_image(&s.channel, &used)).await,
        "ASSET_ALREADY_USED"
    );
    let image_reply =
        CreateReplyRequest { thread_id: thread.id.clone(), asset_ids: vec![used.clone()], ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CreateReply", Some(&s.author), &image_reply).await, "ASSET_ALREADY_USED");
    let fresh = ctx.image(&s.author, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await;
    let posted = ctx.try_thread(&s.author, &with_image(&s.channel, &fresh)).await.unwrap();
    assert_eq!(posted.images[0].asset_id, fresh);
    assert_unnamed("a thread with a picture", &ctx.get_thread(Some(&s.reader), &posted.id).await.unwrap(), &s.author);
}

#[tokio::test]
async fn an_anonymous_channel_takes_posts_once_enough_members_can_write_in_it() {
    // The setting as shipped: five.
    let ctx = start_with(tokuly_community::CommunityConfig { anonymous_min_members: 5, ..test_config() }).await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let mut members = Vec::new();
    for name in ["A", "B", "C"] {
        members.push(ctx.member(&owner, &community, name).await);
    }
    let channel = ctx.channel(&owner, &community, "匿名", true).await;
    let post = |title: &str| CreateThreadRequest {
        channel_id: channel.id.clone(),
        title: title.into(),
        body: "x".into(),
        ..Default::default()
    };

    // Four members: whoever writes is one of three others to each reader.
    assert_eq!(ctx.reason(SVC, "CreateThread", Some(&members[0]), &post("まだ早い")).await, "TOO_FEW_MEMBERS");
    // The named channel next to it is not concerned.
    ctx.thread(&members[0], &ctx.general(&owner, &community).await, "名前つき", "x").await;
    let fifth = ctx.member(&owner, &community, "E").await;
    let thread = ctx.try_thread(&members[0], &post("5人そろった")).await.unwrap();
    assert!(thread.pseudonym.as_option().is_some());

    // When the community shrinks again the channel stops taking posts —
    // replies included — and what is there stays readable.
    let leave = LeaveCommunityRequest { community_id: community.id.clone(), ..Default::default() };
    let _: LeaveCommunityResponse = ctx.call("CommunityService", "LeaveCommunity", Some(&fifth), &leave).await.unwrap();
    let reply = CreateReplyRequest { thread_id: thread.id.clone(), body: "返信".into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CreateReply", Some(&members[1]), &reply).await, "TOO_FEW_MEMBERS");
    assert!(ctx.get_thread(Some(&members[1]), &thread.id).await.is_ok());

    // Where only the staff posts, the staff is the crowd to hide in.
    ctx.member(&owner, &community, "F").await;
    let staff_only = CreateChannelRequest {
        community_id: community.id.clone(),
        name: "運営の匿名".into(),
        anonymous: true,
        posting: PostingPolicy::POSTING_POLICY_MODERATORS.into(),
        ..Default::default()
    };
    let staff_channel: CreateChannelResponse = ctx.call(SVC, "CreateChannel", Some(&owner), &staff_only).await.unwrap();
    let from_staff = CreateThreadRequest {
        channel_id: staff_channel.channel.id.clone(),
        title: "運営より".into(),
        body: "x".into(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(SVC, "CreateThread", Some(&owner), &from_staff).await, "TOO_FEW_MEMBERS");
}

#[tokio::test]
async fn a_moderator_who_is_part_of_it_does_not_look_up_the_author() {
    let ctx = start().await;
    let s = scene(&ctx).await;
    let site_moderator = ctx.app.user_with_roles("SiteMod", &["moderator"]).await;
    let reveal = |post, reason: &str| RevealAuthorRequest { post, reason: reason.into(), ..Default::default() };
    let refused = |post| {
        let (ctx, s, request) = (&ctx, &s, reveal(post, "だれが書いたか知りたい"));
        async move { ctx.reason(MODERATION, "RevealAuthor", Some(&s.moderator), &request).await }
    };

    // A post about the moderator: they are mentioned in it.
    let about =
        ctx.thread(&s.author, &s.channel, "モデレーターへの不満", &format!("@{} はひどい", s.moderator.handle)).await;
    assert_eq!(refused(thread_ref(&about.id)).await, "CONFLICT_OF_INTEREST");
    // A reply in the moderator's own thread, and one answering their reply.
    let theirs = ctx.thread(&s.moderator, &s.channel, "モデレーターの匿名スレ", "x").await;
    let in_theirs = ctx.reply(&s.author, &theirs, "このスレ主はおかしい").await;
    assert_eq!(refused(reply_ref(&in_theirs.id)).await, "CONFLICT_OF_INTEREST");
    let neutral = ctx.thread(&s.reader, &s.channel, "関係ないスレ", "x").await;
    let moderators_reply = ctx.reply(&s.moderator, &neutral, "モデレーターの返信").await;
    let answering = CreateReplyRequest {
        thread_id: neutral.id.clone(),
        body: "それは違う".into(),
        reply_to_id: moderators_reply.id.clone(),
        ..Default::default()
    };
    let answer = ctx.try_reply(&s.author, &answering).await.unwrap();
    assert_eq!(refused(reply_ref(&answer.id)).await, "CONFLICT_OF_INTEREST");
    // A post the moderator reported themselves.
    let reported = ctx.thread(&s.author, &s.channel, "モデレーターが通報したスレ", "x").await;
    ctx.report(&s.moderator, SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, &reported.id).await;
    assert_eq!(refused(thread_ref(&reported.id)).await, "CONFLICT_OF_INTEREST");
    // Their own post: nothing to look up, and nothing to log.
    assert_eq!(refused(thread_ref(&theirs.id)).await, "CONFLICT_OF_INTEREST");

    // Site moderation asks the same question about a subject it knows by ID
    // (the server bridges it to the admin domain), and gets the same answers.
    let involved = |kind: SubjectType, id: &str, who: &TestUser| {
        let (ctx, id, who) = (&ctx, id.parse::<uuid::Uuid>().unwrap(), who.id);
        async move { ctx.community.is_party_for_moderation(kind, id, who).await.unwrap() }
    };
    const THREAD: SubjectType = SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD;
    const REPLY: SubjectType = SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY;
    assert!(involved(THREAD, &about.id, &s.moderator).await, "mentioned");
    assert!(involved(REPLY, &in_theirs.id, &s.moderator).await, "their thread");
    assert!(involved(REPLY, &answer.id, &s.moderator).await, "answered");
    assert!(involved(THREAD, &reported.id, &s.moderator).await, "reported it");
    assert!(involved(THREAD, &about.id, &s.author).await, "wrote it");
    assert!(!involved(THREAD, &about.id, &site_moderator).await, "site moderation has no part in it");
    assert!(!involved(THREAD, &about.id, &s.reader).await);
    // Not a post, or not one that exists: nobody is involved.
    assert!(!involved(SubjectType::SUBJECT_TYPE_COMMUNITY, &s.community.id, &s.moderator).await);
    assert!(!involved(THREAD, &uuid::Uuid::now_v7().to_string(), &s.moderator).await);

    // None of the refused lookups left a trace or told anyone anything.
    let lookups = || {
        let (ctx, s) = (&ctx, &s);
        async move {
            let log = ctx.moderation_log(&s.owner, &s.community).await;
            log.iter().filter(|entry| entry.action == ModerationAction::MODERATION_ACTION_AUTHOR_REVEALED).count()
        }
    };
    assert_eq!(lookups().await, 0);
    assert!(ctx.notifications(s.owner.id, "community.author_revealed").await.is_empty());

    // Someone who is not part of it decides: another member of the staff, or site moderation.
    let by_owner: RevealAuthorResponse = ctx
        .call(MODERATION, "RevealAuthor", Some(&s.owner), &reveal(thread_ref(&about.id), "モデレーターへの中傷の通報"))
        .await
        .unwrap();
    assert_eq!(by_owner.author.id, s.author.id.to_string());
    let by_site: RevealAuthorResponse = ctx
        .call(MODERATION, "RevealAuthor", Some(&site_moderator), &reveal(reply_ref(&answer.id), "通報の対応"))
        .await
        .unwrap();
    assert_eq!(by_site.author.id, s.author.id.to_string());
    assert_eq!(lookups().await, 2);
    // And a post the moderator has nothing to do with is theirs to look up as before.
    let unrelated = ctx.thread(&s.author, &s.channel, "関係のない悪口", "x").await;
    let looked: RevealAuthorResponse = ctx
        .call(MODERATION, "RevealAuthor", Some(&s.moderator), &reveal(thread_ref(&unrelated.id), "通報があった"))
        .await
        .unwrap();
    assert_eq!(looked.author.id, s.author.id.to_string());
}

#[tokio::test]
async fn the_conflict_rule_covers_everything_a_pseudonym_wrote_in_the_thread() {
    let ctx = start().await;
    let s = scene(&ctx).await;
    let reveal = |post, user: &TestUser| {
        let (ctx, user) = (&ctx, user.clone());
        let request =
            RevealAuthorRequest { post, reason: "だれが書いたか知りたい".into(), ..Default::default() };
        async move { ctx.call::<_, RevealAuthorResponse>(MODERATION, "RevealAuthor", Some(&user), &request).await }
    };

    // The author attacks the moderator in one reply and writes harmless
    // ones around it. Every one of them carries the same pseudonym, so
    // looking up a harmless one would unmask the attacker just the same.
    let thread = ctx.thread(&s.reader, &s.channel, "ふつうのスレ", "x").await;
    let harmless = ctx.reply(&s.author, &thread, "いい天気ですね").await;
    let attack = ctx.reply(&s.author, &thread, &format!("@{} は最低のモデレーター", s.moderator.handle)).await;
    let later = ctx.reply(&s.author, &thread, "ではまた").await;
    assert!(harmless.pseudonym.as_option() == attack.pseudonym.as_option());
    for post in [&attack, &harmless, &later] {
        assert_eq!(reveal(reply_ref(&post.id), &s.moderator).await.unwrap_err().reason, "CONFLICT_OF_INTEREST");
    }
    // Someone else's reply in the same thread is not that author's business.
    let bystander = ctx.reply(&s.owner, &thread, "横から失礼").await;
    assert_eq!(reveal(reply_ref(&bystander.id), &s.moderator).await.unwrap().author.id, s.owner.id.to_string());

    // The same through a report: the moderator reported the thread, and
    // may not look up the replies its author wrote under it either.
    let reported = ctx.thread(&s.author, &s.channel, "通報されるスレ", "x").await;
    let under = ctx.reply(&s.author, &reported, "同じ人の返信").await;
    ctx.report(&s.moderator, SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, &reported.id).await;
    assert_eq!(reveal(reply_ref(&under.id), &s.moderator).await.unwrap_err().reason, "CONFLICT_OF_INTEREST");

    // And a moderator who took part in an anonymous thread is one of its
    // voices: they do not look up the others, the thread's author included.
    let debate = ctx.thread(&s.author, &s.channel, "議論のスレ", "x").await;
    let opponent = ctx.reply(&s.reader, &debate, "反対です").await;
    ctx.reply(&s.moderator, &debate, "モデレーターも匿名で参加").await;
    assert_eq!(reveal(thread_ref(&debate.id), &s.moderator).await.unwrap_err().reason, "CONFLICT_OF_INTEREST");
    assert_eq!(reveal(reply_ref(&opponent.id), &s.moderator).await.unwrap_err().reason, "CONFLICT_OF_INTEREST");

    // Someone on the staff who stayed out of all of it decides.
    let admin = ctx.staff(&s.owner, &s.community, "Admin", MemberRole::MEMBER_ROLE_ADMIN).await;
    assert_eq!(reveal(reply_ref(&harmless.id), &admin).await.unwrap().author.id, s.author.id.to_string());
    assert_eq!(reveal(thread_ref(&debate.id), &admin).await.unwrap().author.id, s.author.id.to_string());
    // (The owner wrote in the first thread, and so is a voice in it too.)
    assert_eq!(reveal(reply_ref(&harmless.id), &s.owner).await.unwrap_err().reason, "CONFLICT_OF_INTEREST");
}
