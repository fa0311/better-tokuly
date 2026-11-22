//! `CommunityBoardService`: channels, threads, the list orders, pins,
//! locks, announcements, follows, read markers and polls.
mod common;

use common::*;
use tokuly_community::CommunityConfig;
use tokuly_proto::{
    common::SubjectType,
    community::{community_event::Event as Pushed, *},
    media::AssetPurpose,
};
use tokuly_testkit::TestUser;

const SVC: &str = "CommunityBoardService";

async fn log_actions(ctx: &Ctx, staff: &TestUser, community: &Community) -> Vec<ModerationAction> {
    ctx.moderation_log(staff, community).await.into_iter().filter_map(|entry| entry.action.as_known()).collect()
}

#[tokio::test]
async fn channels_are_run_by_admins() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let create = CreateChannelRequest {
        community_id: community.id.clone(),
        name: " お知らせ ".into(),
        description: "運営からのお知らせ".into(),
        emoji: "📢".into(),
        posting: PostingPolicy::POSTING_POLICY_MODERATORS.into(),
        ..Default::default()
    };
    for user in [&member, &moderator] {
        assert_eq!(ctx.reason(SVC, "CreateChannel", Some(user), &create).await, "PERMISSION_DENIED");
    }
    for (reason, request) in [
        ("CHANNEL_NAME_INVALID", CreateChannelRequest { name: " ".into(), ..create.clone() }),
        ("CHANNEL_NAME_INVALID", CreateChannelRequest { name: "あ".repeat(31), ..create.clone() }),
        ("DESCRIPTION_TOO_LONG", CreateChannelRequest { description: "あ".repeat(201), ..create.clone() }),
        ("EMOJI_INVALID", CreateChannelRequest { emoji: "ab".into(), ..create.clone() }),
    ] {
        assert_eq!(ctx.reason(SVC, "CreateChannel", Some(&owner), &request).await, reason);
    }

    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    let response: CreateChannelResponse = ctx.call(SVC, "CreateChannel", Some(&owner), &create).await.unwrap();
    let news = response.channel.as_option().unwrap().clone();
    assert_eq!((news.name.as_str(), news.position, news.emoji.as_str()), ("お知らせ", 1, "📢"));
    assert!(pushes(&mut topic).await.iter().any(|push| matches!(push, Pushed::ChannelsChanged(_))));

    // Only moderators post in it; everyone reads.
    let as_member = ctx.channels(Some(&member), &community).await;
    assert_eq!(as_member.len(), 2);
    assert!(as_member[0].can_post && !as_member[1].can_post);
    assert!(ctx.channels(Some(&moderator), &community).await[1].can_post);
    let post =
        CreateThreadRequest { channel_id: news.id.clone(), title: "t".into(), body: "b".into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CreateThread", Some(&member), &post).await, "POSTING_RESTRICTED");
    let official = ctx.try_thread(&moderator, &post).await.unwrap();
    let reply = CreateReplyRequest { thread_id: official.id.clone(), body: "質問".into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CreateReply", Some(&member), &reply).await, "POSTING_RESTRICTED");

    // Edits change what is sent and leave the rest.
    let update = UpdateChannelRequest {
        channel_id: news.id.clone(),
        name: Some("ニュース".into()),
        posting: Some(PostingPolicy::POSTING_POLICY_EVERYONE.into()),
        ..Default::default()
    };
    assert_eq!(ctx.reason(SVC, "UpdateChannel", Some(&moderator), &update).await, "PERMISSION_DENIED");
    let updated: UpdateChannelResponse = ctx.call(SVC, "UpdateChannel", Some(&owner), &update).await.unwrap();
    assert_eq!((updated.channel.name.as_str(), updated.channel.emoji.as_str()), ("ニュース", "📢"));
    assert_eq!(updated.channel.posting, PostingPolicy::POSTING_POLICY_EVERYONE);

    // The order is whatever the admins say, naming every channel once.
    let general = as_member[0].clone();
    let reorder = |ids: Vec<String>| ReorderChannelsRequest {
        community_id: community.id.clone(),
        channel_ids: ids,
        ..Default::default()
    };
    let swapped: ReorderChannelsResponse = ctx
        .call(SVC, "ReorderChannels", Some(&owner), &reorder(vec![news.id.clone(), general.id.clone()]))
        .await
        .unwrap();
    assert_eq!(
        swapped.channels.iter().map(|channel| channel.id.clone()).collect::<Vec<_>>(),
        vec![news.id.clone(), general.id.clone()]
    );
    for bad in [vec![news.id.clone()], vec![news.id.clone(), news.id.clone()], vec![news.id.clone(), "nope".into()]] {
        assert_eq!(ctx.reason(SVC, "ReorderChannels", Some(&owner), &reorder(bad)).await, "CHANNEL_ORDER_MISMATCH");
    }
    assert_eq!(
        ctx.reason(SVC, "ReorderChannels", Some(&member), &reorder(vec![general.id.clone(), news.id.clone()])).await,
        "PERMISSION_DENIED"
    );

    // Deleting a channel takes its threads along; the last channel stays.
    let delete = |channel: &Channel| DeleteChannelRequest { channel_id: channel.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "DeleteChannel", Some(&moderator), &delete(&news)).await, "PERMISSION_DENIED");
    let _: DeleteChannelResponse = ctx.call(SVC, "DeleteChannel", Some(&owner), &delete(&news)).await.unwrap();
    assert_eq!(ctx.get_thread(Some(&owner), &official.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    assert_eq!(ctx.get(Some(&owner), &community.id).await.unwrap().thread_count, 0);
    assert_eq!(ctx.reason(SVC, "DeleteChannel", Some(&owner), &delete(&news)).await, "CHANNEL_NOT_FOUND");
    assert_eq!(ctx.reason(SVC, "DeleteChannel", Some(&owner), &delete(&general)).await, "LAST_CHANNEL");
    let actions = log_actions(&ctx, &owner, &community).await;
    for action in [
        ModerationAction::MODERATION_ACTION_CHANNEL_CREATED,
        ModerationAction::MODERATION_ACTION_CHANNEL_UPDATED,
        ModerationAction::MODERATION_ACTION_CHANNEL_DELETED,
    ] {
        assert!(actions.contains(&action), "{action:?} is logged");
    }
}

#[tokio::test]
async fn a_community_has_only_so_many_channels() {
    let ctx = start_with(CommunityConfig { max_channels: 2, ..test_config() }).await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    ctx.channel(&owner, &community, "second", false).await;
    let third = CreateChannelRequest { community_id: community.id.clone(), name: "third".into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CreateChannel", Some(&owner), &third).await, "TOO_MANY_CHANNELS");
}

#[tokio::test]
async fn threads_carry_text_images_a_stamp_and_tags() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let friend = ctx.member(&owner, &community, "Friend").await;
    let outsider = ctx.app.user("Outsider").await;
    let general = ctx.general(&owner, &community).await;
    let images = vec![
        ctx.image(&author, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await,
        ctx.image(&author, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE).await,
    ];
    let stamp = ctx.stamp(&author).await;
    let request = CreateThreadRequest {
        channel_id: general.id.clone(),
        title: "  ランクいこう  ".into(),
        body: format!("今夜どう? @{} @{} @{}", friend.handle, outsider.handle, author.handle),
        asset_ids: images.clone(),
        stamp_id: stamp.clone(),
        tags: vec!["#Ranked".into(), "夜".into()],
        ..Default::default()
    };
    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    let thread = ctx.try_thread(&author, &request).await.unwrap();

    assert_eq!(thread.title, "ランクいこう");
    assert_eq!(thread.short_id.len(), 11);
    assert_eq!(thread.author.id, author.id.to_string());
    assert!(thread.pseudonym.as_option().is_none());
    assert_eq!(thread.images.iter().map(|image| image.asset_id.clone()).collect::<Vec<_>>(), images);
    assert_eq!(thread.stamp.id, stamp);
    assert_eq!(thread.tags, vec!["ranked", "夜"]);
    assert_eq!(thread.state, PostState::POST_STATE_VISIBLE);
    assert_eq!(thread.path, format!("/c/{}/t/{}", community.handle, thread.short_id));
    // Only members can be mentioned, and nobody mentions themselves.
    assert_eq!(
        thread.mentions.iter().map(|mention| mention.user_id.clone()).collect::<Vec<_>>(),
        vec![friend.id.to_string()]
    );
    let viewer = thread.viewer.as_option().unwrap();
    assert!(viewer.mine && viewer.following && viewer.can_edit && viewer.can_delete && viewer.can_reply);
    assert!(!viewer.unread && !viewer.can_moderate);

    // What follows a post: holds on the uploads, counters, an event, a
    // search document, a push, a word to the mentioned member.
    for image in &images {
        assert!(ctx.is_held(image, &format!("community:thread:{}", thread.id)).await);
    }
    assert_eq!(ctx.channels(Some(&owner), &community).await[0].thread_count, 1);
    assert_eq!(ctx.get(None, &community.id).await.unwrap().thread_count, 1);
    let events = ctx.app.events_by("community.thread.created", author.id).await;
    assert_eq!(events[0].data["thread_id"], thread.id);
    assert_eq!(events[0].data["author_id"], author.id.to_string());
    let doc = &ctx.search_changes("thread", &thread.id).await[0]["doc"];
    assert_eq!(
        (doc["title"].as_str(), doc["owner_id"].as_str()),
        (Some("ランクいこう"), Some(author.id.to_string().as_str()))
    );
    let pushed = pushes(&mut topic).await;
    assert!(pushed.iter().any(|push| matches!(push, Pushed::ThreadCreated(created)
        if created.thread_id == thread.id && created.thread.title == "ランクいこう" && created.thread.viewer.as_option().is_none())));
    let told = ctx.notifications(friend.id, "community.mention").await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0]["actor_id"], author.id.to_string());
    assert!(ctx.notification_kinds(outsider.id).await.is_empty());

    // Both IDs lead to it, for anyone: the community is public.
    assert_eq!(ctx.get_thread(None, &thread.short_id).await.unwrap().id, thread.id);
    let seen = ctx.get_thread(Some(&friend), &thread.id).await.unwrap();
    assert!(seen.viewer.unread && !seen.viewer.mine && !seen.viewer.can_edit && seen.viewer.can_reply);
    assert!(!ctx.get_thread(Some(&outsider), &thread.id).await.unwrap().viewer.can_reply);
}

#[tokio::test]
async fn writing_a_thread_is_validated() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let outsider = ctx.app.user("Outsider").await;
    let general = ctx.general(&owner, &community).await;
    let base = CreateThreadRequest {
        channel_id: general.id.clone(),
        title: "題".into(),
        body: "本文".into(),
        ..Default::default()
    };
    let foreign = ctx.image(&outsider, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await;
    let wrong = ctx.image(&author, AssetPurpose::ASSET_PURPOSE_AVATAR).await;
    let mut five = Vec::new();
    for _ in 0..5 {
        five.push(ctx.image(&author, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await);
    }
    let poll =
        |options: &[&str]| PollInput { options: options.iter().map(|o| o.to_string()).collect(), ..Default::default() };
    let cases: Vec<(&str, CreateThreadRequest)> = vec![
        ("TITLE_INVALID", CreateThreadRequest { title: " ".into(), ..base.clone() }),
        ("TITLE_INVALID", CreateThreadRequest { title: "あ".repeat(101), ..base.clone() }),
        ("BODY_TOO_LONG", CreateThreadRequest { body: "あ".repeat(5001), ..base.clone() }),
        ("THREAD_EMPTY", CreateThreadRequest { body: String::new(), ..base.clone() }),
        ("TOO_MANY_IMAGES", CreateThreadRequest { asset_ids: five, ..base.clone() }),
        ("ASSET_NOT_FOUND", CreateThreadRequest { asset_ids: vec![foreign], ..base.clone() }),
        ("ASSET_WRONG_PURPOSE", CreateThreadRequest { asset_ids: vec![wrong], ..base.clone() }),
        ("INVALID_ID", CreateThreadRequest { asset_ids: vec!["x".into()], ..base.clone() }),
        ("STAMP_NOT_USABLE", CreateThreadRequest { stamp_id: uuid::Uuid::now_v7().to_string(), ..base.clone() }),
        ("TOO_MANY_TAGS", CreateThreadRequest { tags: (0..6).map(|i| format!("t{i}")).collect(), ..base.clone() }),
        ("TAG_INVALID", CreateThreadRequest { tags: vec!["a b".into()], ..base.clone() }),
        ("POLL_INVALID", CreateThreadRequest { poll: poll(&["only"]).into(), ..base.clone() }),
        ("POLL_INVALID", CreateThreadRequest { poll: poll(&["same", "same"]).into(), ..base.clone() }),
        ("CHANNEL_NOT_FOUND", CreateThreadRequest { channel_id: uuid::Uuid::now_v7().to_string(), ..base.clone() }),
        ("CHANNEL_NOT_FOUND", CreateThreadRequest { channel_id: "general".into(), ..base.clone() }),
    ];
    for (reason, request) in cases {
        assert_eq!(ctx.reason(SVC, "CreateThread", Some(&author), &request).await, reason);
    }
    // Reading a public community is open; posting takes a membership.
    assert_eq!(ctx.reason(SVC, "CreateThread", Some(&outsider), &base).await, "NOT_A_MEMBER");
    assert_eq!(ctx.reason(SVC, "CreateThread", None, &base).await, "SIGN_IN_REQUIRED");
    // A thread may be just a title and a poll.
    let just_poll =
        CreateThreadRequest { body: String::new(), poll: poll(&["はい", "いいえ"]).into(), ..base.clone() };
    assert!(ctx.try_thread(&author, &just_poll).await.unwrap().poll.as_option().is_some());
}

#[tokio::test]
async fn boards_are_read_as_the_community_allows() {
    let ctx = start().await;
    let (owner, outsider) = (ctx.app.user("Owner").await, ctx.app.user("Outsider").await);
    let restricted = ctx.restricted(&owner).await;
    let hidden = ctx.hidden(&owner).await;
    let in_restricted = ctx.thread(&owner, &ctx.general(&owner, &restricted).await, "中の話", "メンバーだけ").await;
    let in_hidden = ctx.thread(&owner, &ctx.general(&owner, &hidden).await, "秘密", "ひみつ").await;
    let list = |community: &Community| ListThreadsRequest { community_id: community.id.clone(), ..Default::default() };

    for viewer in [Some(&outsider), None] {
        assert_eq!(ctx.get_thread(viewer, &in_restricted.id).await.unwrap_err().reason, "NOT_A_MEMBER");
        assert_eq!(ctx.reason(SVC, "ListThreads", viewer, &list(&restricted)).await, "NOT_A_MEMBER");
        assert_eq!(
            ctx.reason(
                SVC,
                "ListReplies",
                viewer,
                &ListRepliesRequest { thread_id: in_restricted.id.clone(), ..Default::default() }
            )
            .await,
            "NOT_A_MEMBER"
        );
        // A hidden community's threads do not exist for outsiders.
        assert_eq!(ctx.get_thread(viewer, &in_hidden.short_id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
        assert_eq!(ctx.reason(SVC, "ListThreads", viewer, &list(&hidden)).await, "COMMUNITY_NOT_FOUND");
    }
    assert_eq!(ctx.get_thread(Some(&owner), &in_hidden.short_id).await.unwrap().id, in_hidden.id);
    assert_eq!(ctx.get_thread(Some(&owner), "no-such-thread").await.unwrap_err().reason, "THREAD_NOT_FOUND");
    // Threads of a community that is not public stay out of search.
    assert!(ctx.search_changes("thread", &in_restricted.id).await[0]["doc"].is_null());
}

#[tokio::test]
async fn an_edit_shows_and_tells_new_mentions() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let (first, second) =
        (ctx.member(&owner, &community, "First").await, ctx.member(&owner, &community, "Second").await);
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&author, &general, "古い題", &format!("@{}", first.handle)).await;
    assert!(thread.edited_at.as_option().is_none());

    let update = UpdateThreadRequest {
        thread_id: thread.id.clone(),
        title: Some("新しい題".into()),
        body: Some(format!("@{} @{}", first.handle, second.handle)),
        tags: StringList { values: vec!["edited".into()], ..Default::default() }.into(),
        ..Default::default()
    };
    for user in [&first, &owner] {
        assert_eq!(ctx.reason(SVC, "UpdateThread", Some(user), &update).await, "PERMISSION_DENIED");
    }
    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    let updated: UpdateThreadResponse = ctx.call(SVC, "UpdateThread", Some(&author), &update).await.unwrap();
    let updated = updated.thread.as_option().unwrap();
    assert_eq!((updated.title.as_str(), updated.tags.clone()), ("新しい題", vec!["edited".to_owned()]));
    assert!(updated.edited_at.as_option().is_some());
    assert_eq!(updated.mentions.len(), 2);
    assert!(
        pushes(&mut topic)
            .await
            .iter()
            .any(|push| matches!(push, Pushed::ThreadUpdated(update) if update.thread.title == "新しい題"))
    );
    // Whoever was told before is not told again.
    assert_eq!(ctx.notifications(first.id, "community.mention").await.len(), 1);
    assert_eq!(ctx.notifications(second.id, "community.mention").await.len(), 1);

    // Only what is sent changes; a thread cannot be edited into nothing.
    let title_only =
        UpdateThreadRequest { thread_id: thread.id.clone(), title: Some("題だけ".into()), ..Default::default() };
    let kept: UpdateThreadResponse = ctx.call(SVC, "UpdateThread", Some(&author), &title_only).await.unwrap();
    assert_eq!(kept.thread.body, updated.body);
    let emptied = UpdateThreadRequest { thread_id: thread.id.clone(), body: Some(" ".into()), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UpdateThread", Some(&author), &emptied).await, "THREAD_EMPTY");
    let untitled =
        UpdateThreadRequest { thread_id: thread.id.clone(), title: Some(String::new()), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UpdateThread", Some(&author), &untitled).await, "TITLE_INVALID");
}

#[tokio::test]
async fn threads_are_deleted_by_their_author_or_with_a_reason() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let member = ctx.member(&owner, &community, "Member").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let general = ctx.general(&owner, &community).await;
    let image = ctx.image(&author, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await;
    let reply_image = ctx.image(&member, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await;
    let with_image = CreateThreadRequest {
        channel_id: general.id.clone(),
        title: "消される".into(),
        asset_ids: vec![image.clone()],
        ..Default::default()
    };
    let doomed = ctx.try_thread(&author, &with_image).await.unwrap();
    let reply = ctx
        .try_reply(
            &member,
            &CreateReplyRequest {
                thread_id: doomed.id.clone(),
                asset_ids: vec![reply_image.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let own = ctx.thread(&author, &general, "自分で消す", "さよなら").await;
    let delete = |thread: &Thread, reason: &str| DeleteThreadRequest {
        thread_id: thread.id.clone(),
        reason: reason.into(),
        ..Default::default()
    };

    assert_eq!(ctx.reason(SVC, "DeleteThread", Some(&member), &delete(&doomed, "いや")).await, "PERMISSION_DENIED");
    assert_eq!(ctx.reason(SVC, "DeleteThread", Some(&moderator), &delete(&doomed, " ")).await, "REASON_REQUIRED");
    assert_eq!(
        ctx.reason(SVC, "DeleteThread", Some(&moderator), &delete(&doomed, &"あ".repeat(301))).await,
        "REASON_TOO_LONG"
    );

    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    let _: DeleteThreadResponse =
        ctx.call(SVC, "DeleteThread", Some(&moderator), &delete(&doomed, "ルール違反")).await.unwrap();
    assert!(
        pushes(&mut topic)
            .await
            .iter()
            .any(|push| matches!(push, Pushed::ThreadRemoved(removed) if removed.thread_id == doomed.id))
    );
    assert_eq!(ctx.get_thread(Some(&author), &doomed.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    assert_eq!(ctx.get_reply(Some(&member), &reply.id).await.unwrap_err().reason, "REPLY_NOT_FOUND");
    assert_eq!(
        ctx.reason(SVC, "DeleteThread", Some(&moderator), &delete(&doomed, "もう一回")).await,
        "THREAD_NOT_FOUND"
    );
    // Search forgets it at once. Its uploads (and its replies') stay on
    // file for as long as the removed thread is retained (see `retention.rs`).
    assert!(ctx.is_held(&image, &format!("community:thread:{}", doomed.id)).await);
    assert!(ctx.is_held(&reply_image, &format!("community:reply:{}", reply.id)).await);
    assert!(ctx.search_changes("thread", &doomed.id).await.last().unwrap()["doc"].is_null());
    // The author is told why; the log keeps the moderator's reason.
    let told = ctx.notifications(author.id, "community.moderation").await;
    assert_eq!(
        (told[0]["data"]["change"].as_str(), told[0]["body"]["ja"].as_str()),
        (Some("removed"), Some("ルール違反"))
    );
    let log = ctx.moderation_log(&owner, &community).await;
    let entry = log.iter().find(|entry| entry.action == ModerationAction::MODERATION_ACTION_THREAD_DELETED).unwrap();
    assert_eq!(entry.reason, "ルール違反");
    assert_eq!(entry.actor.id, moderator.id.to_string());
    assert_eq!(entry.target_user.id, author.id.to_string());

    // An author needs no reason, and no log entry is written.
    let _: DeleteThreadResponse = ctx.call(SVC, "DeleteThread", Some(&author), &delete(&own, "")).await.unwrap();
    assert_eq!(ctx.get(None, &community.id).await.unwrap().thread_count, 0);
    assert_eq!(
        ctx.moderation_log(&owner, &community)
            .await
            .iter()
            .filter(|entry| entry.action == ModerationAction::MODERATION_ACTION_THREAD_DELETED)
            .count(),
        1
    );
}

#[tokio::test]
async fn threads_list_hot_new_and_top() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let general = ctx.general(&owner, &community).await;
    let other = ctx.channel(&owner, &community, "other", false).await;
    let a = ctx.thread(&owner, &general, "A", "first").await;
    let b = ctx.thread(&owner, &general, "B", "second").await;
    let c = ctx
        .try_thread(
            &owner,
            &CreateThreadRequest {
                channel_id: other.id.clone(),
                title: "C".into(),
                body: "third".into(),
                tags: vec!["special".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // B is liked five times, A gets a reply, C nothing.
    ctx.app
        .core
        .subjects
        .counter_changed(SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, b.id.parse().unwrap(), "like_count", 5)
        .await
        .unwrap();
    ctx.reply(&owner, &a, "返信").await;
    ctx.community.refresh_hot_scores().await.unwrap();

    let list = |sort: ThreadSort| ListThreadsRequest {
        community_id: community.id.clone(),
        sort: sort.into(),
        ..Default::default()
    };
    let titles =
        |response: &ListThreadsResponse| response.threads.iter().map(|thread| thread.title.clone()).collect::<Vec<_>>();
    assert_eq!(titles(&ctx.list_threads(None, &list(ThreadSort::THREAD_SORT_NEW)).await), vec!["C", "B", "A"]);
    assert_eq!(titles(&ctx.list_threads(None, &list(ThreadSort::THREAD_SORT_TOP)).await), vec!["B", "A", "C"]);
    assert_eq!(titles(&ctx.list_threads(None, &list(ThreadSort::THREAD_SORT_HOT)).await), vec!["B", "A", "C"]);
    // Hot is the default.
    assert_eq!(titles(&ctx.list_threads(None, &list(ThreadSort::THREAD_SORT_UNSPECIFIED)).await), vec!["B", "A", "C"]);
    let liked = &ctx.list_threads(None, &list(ThreadSort::THREAD_SORT_TOP)).await.threads[0];
    assert_eq!((liked.like_count, liked.reply_count), (5, 0));

    // Every order pages through all threads exactly once.
    for sort in [ThreadSort::THREAD_SORT_NEW, ThreadSort::THREAD_SORT_TOP, ThreadSort::THREAD_SORT_HOT] {
        let first = ctx.list_threads(None, &ListThreadsRequest { page: page(2, ""), ..list(sort) }).await;
        assert!(first.page.has_more);
        let second =
            ctx.list_threads(None, &ListThreadsRequest { page: page(2, &first.page.next_cursor), ..list(sort) }).await;
        assert!(!second.page.has_more);
        let mut seen = titles(&first);
        seen.extend(titles(&second));
        seen.sort();
        // "Hot" scores fade with time: when they are recomputed between two
        // pages (another test's timer here), the thread at the page's edge
        // shows up on both. Nothing is ever left out.
        seen.dedup();
        assert_eq!(seen, vec!["A", "B", "C"], "{sort:?}");
    }

    // Filters: one channel, one tag, a window for "top".
    let in_other = ListThreadsRequest { channel_id: other.id.clone(), ..list(ThreadSort::THREAD_SORT_NEW) };
    assert_eq!(titles(&ctx.list_threads(None, &in_other).await), vec!["C"]);
    let tagged = ListThreadsRequest { tag: "#Special".into(), ..list(ThreadSort::THREAD_SORT_NEW) };
    assert_eq!(titles(&ctx.list_threads(None, &tagged).await), vec!["C"]);
    sqlx::query("UPDATE community_threads SET created_at = now() - interval '2 days' WHERE id = $1")
        .bind(b.id.parse::<uuid::Uuid>().unwrap())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let today = ListThreadsRequest { window: TopWindow::TOP_WINDOW_DAY.into(), ..list(ThreadSort::THREAD_SORT_TOP) };
    assert_eq!(titles(&ctx.list_threads(None, &today).await), vec!["A", "C"]);
    let week = ListThreadsRequest { window: TopWindow::TOP_WINDOW_WEEK.into(), ..list(ThreadSort::THREAD_SORT_TOP) };
    assert_eq!(titles(&ctx.list_threads(None, &week).await), vec!["B", "A", "C"]);

    let elsewhere = ctx.public(&owner).await;
    let foreign_channel = ListThreadsRequest {
        channel_id: ctx.general(&owner, &elsewhere).await.id,
        ..list(ThreadSort::THREAD_SORT_NEW)
    };
    assert_eq!(ctx.reason(SVC, "ListThreads", None, &foreign_channel).await, "CHANNEL_NOT_FOUND");
    let bad_tag = ListThreadsRequest { tag: "a b".into(), ..list(ThreadSort::THREAD_SORT_NEW) };
    assert_eq!(ctx.reason(SVC, "ListThreads", None, &bad_tag).await, "TAG_INVALID");
    let bad_sort = serde_json::json!({ "communityId": community.id, "sort": 99 });
    assert_eq!(ctx.reason(SVC, "ListThreads", None, &bad_sort).await, "SORT_INVALID");
    let _ = c;
}

#[tokio::test]
async fn hot_scores_fade_with_age() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let general = ctx.general(&owner, &community).await;
    let old = ctx.thread(&owner, &general, "old but liked", "x").await;
    let fresh = ctx.thread(&owner, &general, "fresh", "x").await;
    ctx.app
        .core
        .subjects
        .counter_changed(SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, old.id.parse().unwrap(), "like_count", 3)
        .await
        .unwrap();
    let list = ListThreadsRequest { community_id: community.id.clone(), ..Default::default() };
    assert_eq!(ctx.list_threads(None, &list).await.threads[0].title, "old but liked");

    // Three likes do not carry a thread for a week…
    sqlx::query("UPDATE community_threads SET created_at = now() - interval '7 days', last_activity_at = now() - interval '7 days' WHERE id = $1")
        .bind(old.id.parse::<uuid::Uuid>().unwrap())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(ctx.community.refresh_hot_scores().await.unwrap() >= 2);
    assert_eq!(ctx.list_threads(None, &list).await.threads[0].title, "fresh");
    // …and after a month without activity it has no score at all.
    sqlx::query("UPDATE community_threads SET last_activity_at = now() - interval '40 days' WHERE id = $1")
        .bind(old.id.parse::<uuid::Uuid>().unwrap())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.community.refresh_hot_scores().await.unwrap();
    let score: f64 = sqlx::query_scalar("SELECT hot_score FROM community_threads WHERE id = $1")
        .bind(old.id.parse::<uuid::Uuid>().unwrap())
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(score, 0.0);
    let _ = fresh;
}

#[tokio::test]
async fn pinned_threads_come_first_and_separately() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let general = ctx.general(&owner, &community).await;
    let rules = ctx.thread(&member, &general, "ルール", "読んでね").await;
    let chat = ctx.thread(&member, &general, "雑談", "わいわい").await;
    let pin =
        |thread: &Thread, pinned: bool| PinThreadRequest { thread_id: thread.id.clone(), pinned, ..Default::default() };
    assert_eq!(ctx.reason(SVC, "PinThread", Some(&member), &pin(&rules, true)).await, "PERMISSION_DENIED");
    let pinned: PinThreadResponse = ctx.call(SVC, "PinThread", Some(&moderator), &pin(&rules, true)).await.unwrap();
    assert!(pinned.thread.pinned && pinned.thread.viewer.can_moderate);

    let list = ListThreadsRequest {
        community_id: community.id.clone(),
        sort: ThreadSort::THREAD_SORT_NEW.into(),
        page: page(1, ""),
        ..Default::default()
    };
    let first = ctx.list_threads(Some(&member), &list).await;
    assert_eq!(first.pinned.iter().map(|thread| thread.id.clone()).collect::<Vec<_>>(), vec![rules.id.clone()]);
    assert_eq!(first.threads.iter().map(|thread| thread.id.clone()).collect::<Vec<_>>(), vec![chat.id.clone()]);
    assert!(!first.page.has_more, "pinned threads are not part of the paged list");

    // A channel has room for five pins.
    let mut extra = Vec::new();
    for i in 0..5 {
        extra.push(ctx.thread(&member, &general, &format!("pin {i}"), "x").await);
    }
    for thread in &extra[..4] {
        let _: PinThreadResponse = ctx.call(SVC, "PinThread", Some(&moderator), &pin(thread, true)).await.unwrap();
    }
    assert_eq!(ctx.reason(SVC, "PinThread", Some(&moderator), &pin(&extra[4], true)).await, "TOO_MANY_PINNED");
    // Pinning what is pinned changes nothing; unpinning frees a place.
    let _: PinThreadResponse = ctx.call(SVC, "PinThread", Some(&moderator), &pin(&rules, true)).await.unwrap();
    let unpinned: PinThreadResponse = ctx.call(SVC, "PinThread", Some(&moderator), &pin(&rules, false)).await.unwrap();
    assert!(!unpinned.thread.pinned);
    let _: PinThreadResponse = ctx.call(SVC, "PinThread", Some(&moderator), &pin(&extra[4], true)).await.unwrap();
    let actions = log_actions(&ctx, &owner, &community).await;
    assert_eq!(
        actions.iter().filter(|action| **action == ModerationAction::MODERATION_ACTION_THREAD_PINNED).count(),
        6
    );
    assert_eq!(
        actions.iter().filter(|action| **action == ModerationAction::MODERATION_ACTION_THREAD_UNPINNED).count(),
        1
    );
}

#[tokio::test]
async fn a_locked_thread_takes_no_replies_and_no_edits() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&author, &general, "議論", "どうぞ").await;
    let reply = ctx.reply(&author, &thread, "先に書いた").await;
    let lock = |locked: bool| LockThreadRequest { thread_id: thread.id.clone(), locked, ..Default::default() };
    assert_eq!(ctx.reason(SVC, "LockThread", Some(&author), &lock(true)).await, "PERMISSION_DENIED");
    let locked: LockThreadResponse = ctx.call(SVC, "LockThread", Some(&owner), &lock(true)).await.unwrap();
    assert!(locked.thread.locked && !locked.thread.viewer.can_reply);

    let more =
        CreateReplyRequest { thread_id: thread.id.clone(), body: "まだ言いたい".into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CreateReply", Some(&author), &more).await, "THREAD_LOCKED");
    let edit = UpdateThreadRequest { thread_id: thread.id.clone(), title: Some("編集".into()), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UpdateThread", Some(&author), &edit).await, "THREAD_LOCKED");
    let edit_reply = UpdateReplyRequest { reply_id: reply.id.clone(), body: "編集".into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UpdateReply", Some(&author), &edit_reply).await, "THREAD_LOCKED");
    let seen = ctx.get_thread(Some(&author), &thread.id).await.unwrap();
    assert!(!seen.viewer.can_edit && !seen.viewer.can_reply && seen.viewer.can_delete);

    let _: LockThreadResponse = ctx.call(SVC, "LockThread", Some(&owner), &lock(false)).await.unwrap();
    ctx.try_reply(&author, &more).await.unwrap();
    let actions = log_actions(&ctx, &owner, &community).await;
    assert!(actions.contains(&ModerationAction::MODERATION_ACTION_THREAD_LOCKED));
    assert!(actions.contains(&ModerationAction::MODERATION_ACTION_THREAD_UNLOCKED));
}

#[tokio::test]
async fn an_announcement_reaches_members_per_their_level() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let (all, mentions_only, none) = (
        ctx.member(&owner, &community, "All").await,
        ctx.member(&owner, &community, "MentionsOnly").await,
        ctx.member(&owner, &community, "None").await,
    );
    let moderator = ctx.moderator(&owner, &community).await;
    for (user, level) in [
        (&mentions_only, NotificationLevel::NOTIFICATION_LEVEL_MENTIONS),
        (&none, NotificationLevel::NOTIFICATION_LEVEL_NONE),
    ] {
        let request = UpdateMembershipRequest {
            community_id: community.id.clone(),
            notification_level: Some(level.into()),
            ..Default::default()
        };
        let _: UpdateMembershipResponse =
            ctx.call("CommunityService", "UpdateMembership", Some(user), &request).await.unwrap();
    }
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&author, &general, "文化祭のお知らせ", "来週です").await;
    let announce = |announcement: bool| SetAnnouncementRequest {
        thread_id: thread.id.clone(),
        announcement,
        ..Default::default()
    };
    assert_eq!(ctx.reason(SVC, "SetAnnouncement", Some(&author), &announce(true)).await, "PERMISSION_DENIED");

    let announced: SetAnnouncementResponse =
        ctx.call(SVC, "SetAnnouncement", Some(&moderator), &announce(true)).await.unwrap();
    assert!(announced.thread.announcement);
    // The members are told by a job; run it as the worker would.
    let thread_id: uuid::Uuid = thread.id.parse().unwrap();
    ctx.community.run_announcement(thread_id).await.unwrap();
    for user in [&all, &mentions_only, &owner, &moderator] {
        let told = ctx.notifications(user.id, "community.announcement").await;
        assert_eq!(told.len(), 1, "{} is told once", user.handle);
        assert_eq!(told[0]["body"]["ja"], "文化祭のお知らせ");
        assert_eq!(told[0]["actor_id"], author.id.to_string());
    }
    assert!(ctx.notifications(none.id, "community.announcement").await.is_empty());
    assert!(ctx.notifications(author.id, "community.announcement").await.is_empty(), "the author knows");

    // Running the job again, or taking the mark back and setting it again, tells nobody twice.
    ctx.community.run_announcement(thread_id).await.unwrap();
    let _: SetAnnouncementResponse =
        ctx.call(SVC, "SetAnnouncement", Some(&moderator), &announce(false)).await.unwrap();
    let _: SetAnnouncementResponse = ctx.call(SVC, "SetAnnouncement", Some(&moderator), &announce(true)).await.unwrap();
    ctx.community.run_announcement(thread_id).await.unwrap();
    assert_eq!(ctx.notifications(all.id, "community.announcement").await.len(), 1);

    let list = ListAnnouncementsRequest { community_id: community.id.clone(), ..Default::default() };
    let listed: ListAnnouncementsResponse = ctx.call(SVC, "ListAnnouncements", None, &list).await.unwrap();
    assert_eq!(listed.threads.iter().map(|thread| thread.id.clone()).collect::<Vec<_>>(), vec![thread.id.clone()]);
    let actions = log_actions(&ctx, &owner, &community).await;
    assert_eq!(
        actions.iter().filter(|action| **action == ModerationAction::MODERATION_ACTION_ANNOUNCEMENT_SET).count(),
        2
    );
    assert!(actions.contains(&ModerationAction::MODERATION_ACTION_ANNOUNCEMENT_CLEARED));
}

#[tokio::test]
async fn unread_markers_follow_what_a_member_has_read() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let reader = ctx.member(&owner, &community, "Reader").await;
    let outsider = ctx.app.user("Outsider").await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&owner, &general, "新着", "読んでね").await;
    let mark = |target: mark_read_request::Target| MarkReadRequest { target: Some(target), ..Default::default() };
    use mark_read_request::Target;

    // New to the reader: the thread, its channel and the community say so.
    assert!(ctx.get_thread(Some(&reader), &thread.id).await.unwrap().viewer.unread);
    assert!(ctx.channels(Some(&reader), &community).await[0].has_unread);
    assert!(ctx.viewer(&reader, &community.id).await.has_unread);
    // Not to its author.
    assert!(!ctx.get_thread(Some(&owner), &thread.id).await.unwrap().viewer.unread);
    assert!(!ctx.viewer(&owner, &community.id).await.has_unread);

    let _: MarkReadResponse =
        ctx.call(SVC, "MarkRead", Some(&reader), &mark(Target::ThreadId(thread.id.clone()))).await.unwrap();
    assert!(!ctx.get_thread(Some(&reader), &thread.id).await.unwrap().viewer.unread);
    assert!(!ctx.channels(Some(&reader), &community).await[0].has_unread);
    assert!(!ctx.viewer(&reader, &community.id).await.has_unread);

    // Replies make it unread again, and are counted.
    ctx.reply(&owner, &thread, "ひとつめ").await;
    ctx.reply(&owner, &thread, "ふたつめ").await;
    let seen = ctx.get_thread(Some(&reader), &thread.id).await.unwrap();
    assert!(seen.viewer.unread);
    assert_eq!(seen.viewer.unread_reply_count, 2);
    // Reading the channel covers every thread in it; reading the community everything.
    let _: MarkReadResponse =
        ctx.call(SVC, "MarkRead", Some(&reader), &mark(Target::ChannelId(general.id.clone()))).await.unwrap();
    assert!(!ctx.get_thread(Some(&reader), &thread.id).await.unwrap().viewer.unread);
    ctx.thread(&owner, &general, "もうひとつ", "x").await;
    assert!(ctx.viewer(&reader, &community.id).await.has_unread);
    let _: MarkReadResponse =
        ctx.call(SVC, "MarkRead", Some(&reader), &mark(Target::CommunityId(community.id.clone()))).await.unwrap();
    assert!(!ctx.viewer(&reader, &community.id).await.has_unread);
    assert!(!ctx.channels(Some(&reader), &community).await[0].has_unread);

    assert_eq!(ctx.reason(SVC, "MarkRead", Some(&reader), &MarkReadRequest::default()).await, "TARGET_REQUIRED");
    assert_eq!(
        ctx.reason(SVC, "MarkRead", Some(&outsider), &mark(Target::ThreadId(thread.id.clone()))).await,
        "NOT_A_MEMBER"
    );
    assert_eq!(
        ctx.reason(SVC, "MarkRead", Some(&outsider), &mark(Target::CommunityId(community.id.clone()))).await,
        "NOT_A_MEMBER"
    );
    // Nothing is unread for someone who is not a member.
    assert!(!ctx.get_thread(Some(&outsider), &thread.id).await.unwrap().viewer.unread);
}

#[tokio::test]
async fn followed_threads_are_listed_across_communities() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let (one, two) = (ctx.public(&owner).await, ctx.public(&owner).await);
    let reader = ctx.app.user("Reader").await;
    ctx.join(&reader, &one).await;
    ctx.join(&reader, &two).await;
    let first = ctx.thread(&owner, &ctx.general(&owner, &one).await, "one", "x").await;
    let second = ctx.thread(&owner, &ctx.general(&owner, &two).await, "two", "x").await;
    let follow = |thread: &Thread, following: bool| SetThreadFollowRequest {
        thread_id: thread.id.clone(),
        following,
        ..Default::default()
    };
    let outsider = ctx.app.user("Outsider").await;
    assert_eq!(ctx.reason(SVC, "SetThreadFollow", Some(&outsider), &follow(&first, true)).await, "NOT_A_MEMBER");
    for thread in [&first, &second] {
        let response: SetThreadFollowResponse =
            ctx.call(SVC, "SetThreadFollow", Some(&reader), &follow(thread, true)).await.unwrap();
        assert!(response.following);
    }
    assert!(ctx.get_thread(Some(&reader), &first.id).await.unwrap().viewer.following);

    let followed = |request: ListFollowedThreadsRequest| {
        let ctx = &ctx;
        let reader = &reader;
        async move {
            let response: ListFollowedThreadsResponse =
                ctx.call(SVC, "ListFollowedThreads", Some(reader), &request).await.unwrap();
            response.threads.into_iter().map(|thread| thread.title).collect::<Vec<_>>()
        }
    };
    assert_eq!(followed(ListFollowedThreadsRequest::default()).await, vec!["two", "one"]);
    assert_eq!(
        followed(ListFollowedThreadsRequest { community_id: one.id.clone(), ..Default::default() }).await,
        vec!["one"]
    );
    // Only what has something new.
    let read =
        MarkReadRequest { target: Some(mark_read_request::Target::ThreadId(second.id.clone())), ..Default::default() };
    let _: MarkReadResponse = ctx.call(SVC, "MarkRead", Some(&reader), &read).await.unwrap();
    assert_eq!(followed(ListFollowedThreadsRequest { unread_only: true, ..Default::default() }).await, vec!["one"]);

    let _: SetThreadFollowResponse =
        ctx.call(SVC, "SetThreadFollow", Some(&reader), &follow(&first, false)).await.unwrap();
    assert_eq!(followed(ListFollowedThreadsRequest::default()).await, vec!["two"]);
    assert_eq!(
        ctx.reason(SVC, "ListFollowedThreads", None, &ListFollowedThreadsRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
}

#[tokio::test]
async fn polls_show_results_after_voting() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let (voter, other) = (ctx.member(&owner, &community, "Voter").await, ctx.member(&owner, &community, "Other").await);
    let outsider = ctx.app.user("Outsider").await;
    let general = ctx.general(&owner, &community).await;
    let request = CreateThreadRequest {
        channel_id: general.id.clone(),
        title: "お昼どうする?".into(),
        poll: PollInput {
            options: vec!["ラーメン".into(), "カレー".into(), "うどん".into()], ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    let thread = ctx.try_thread(&author, &request).await.unwrap();
    let options: Vec<String> = thread.poll.options.iter().map(|option| option.id.clone()).collect();
    assert_eq!(
        thread.poll.options.iter().map(|option| option.text.as_str()).collect::<Vec<_>>(),
        vec!["ラーメン", "カレー", "うどん"]
    );
    let vote = |ids: &[&String]| VotePollRequest {
        thread_id: thread.id.clone(),
        option_ids: ids.iter().map(|id| id.to_string()).collect(),
        ..Default::default()
    };

    let _: VotePollResponse = ctx.call(SVC, "VotePoll", Some(&other), &vote(&[&options[0]])).await.unwrap();
    // Before voting: how many voted, not what for.
    let unseen = ctx.get_thread(Some(&voter), &thread.id).await.unwrap();
    assert!(!unseen.poll.results_visible);
    assert_eq!(unseen.poll.voter_count, 1);
    assert!(unseen.poll.options.iter().all(|option| option.vote_count == 0));
    assert!(!ctx.get_thread(None, &thread.id).await.unwrap().poll.results_visible);
    // The author always sees how it stands.
    let authors = ctx.get_thread(Some(&author), &thread.id).await.unwrap();
    assert!(authors.poll.results_visible);
    assert_eq!(authors.poll.options[0].vote_count, 1);

    let voted: VotePollResponse = ctx.call(SVC, "VotePoll", Some(&voter), &vote(&[&options[1]])).await.unwrap();
    assert!(voted.poll.results_visible);
    assert_eq!(voted.poll.my_option_ids, vec![options[1].clone()]);
    assert_eq!(voted.poll.options.iter().map(|option| option.vote_count).collect::<Vec<_>>(), vec![1, 1, 0]);
    assert_eq!(voted.poll.voter_count, 2);
    // A new vote replaces the old one; no options takes it back.
    let changed: VotePollResponse = ctx.call(SVC, "VotePoll", Some(&voter), &vote(&[&options[0]])).await.unwrap();
    assert_eq!(changed.poll.options.iter().map(|option| option.vote_count).collect::<Vec<_>>(), vec![2, 0, 0]);
    assert_eq!(changed.poll.voter_count, 2);
    let retracted: VotePollResponse = ctx.call(SVC, "VotePoll", Some(&voter), &vote(&[])).await.unwrap();
    assert_eq!(retracted.poll.voter_count, 1);
    assert!(!retracted.poll.results_visible && retracted.poll.my_option_ids.is_empty());

    assert_eq!(ctx.reason(SVC, "VotePoll", Some(&voter), &vote(&[&options[0], &options[1]])).await, "SINGLE_CHOICE");
    assert_eq!(
        ctx.reason(SVC, "VotePoll", Some(&voter), &vote(&[&uuid::Uuid::now_v7().to_string()])).await,
        "OPTION_NOT_FOUND"
    );
    assert_eq!(ctx.reason(SVC, "VotePoll", Some(&outsider), &vote(&[&options[0]])).await, "NOT_A_MEMBER");
    let plain = ctx.thread(&author, &general, "投票なし", "x").await;
    let no_poll = VotePollRequest { thread_id: plain.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "VotePoll", Some(&voter), &no_poll).await, "POLL_NOT_FOUND");

    // The author (or the staff) ends it; then results are for everyone and votes are over.
    let close = ClosePollRequest { thread_id: thread.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "ClosePoll", Some(&voter), &close).await, "PERMISSION_DENIED");
    assert_eq!(
        ctx.reason(
            SVC,
            "ClosePoll",
            Some(&author),
            &ClosePollRequest { thread_id: plain.id.clone(), ..Default::default() }
        )
        .await,
        "POLL_NOT_FOUND"
    );
    let closed: ClosePollResponse = ctx.call(SVC, "ClosePoll", Some(&author), &close).await.unwrap();
    assert!(closed.poll.closed);
    assert!(ctx.get_thread(None, &thread.id).await.unwrap().poll.results_visible);
    assert_eq!(ctx.reason(SVC, "VotePoll", Some(&voter), &vote(&[&options[0]])).await, "POLL_CLOSED");
}

#[tokio::test]
async fn polls_can_take_several_choices_and_a_deadline() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let voter = ctx.member(&owner, &community, "Voter").await;
    let general = ctx.general(&owner, &community).await;
    let request = CreateThreadRequest {
        channel_id: general.id.clone(),
        title: "行ける日は?".into(),
        poll: PollInput {
            options: vec!["土".into(), "日".into(), "月".into()],
            multiple_choice: true,
            closes_at: ts(now() + chrono::Duration::hours(1)),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    let thread = ctx.try_thread(&owner, &request).await.unwrap();
    assert!(thread.poll.multiple_choice && !thread.poll.closed && thread.poll.closes_at.as_option().is_some());
    let options: Vec<String> = thread.poll.options.iter().map(|option| option.id.clone()).collect();
    let vote = VotePollRequest {
        thread_id: thread.id.clone(),
        option_ids: vec![options[0].clone(), options[2].clone(), options[0].clone()],
        ..Default::default()
    };
    let voted: VotePollResponse = ctx.call(SVC, "VotePoll", Some(&voter), &vote).await.unwrap();
    assert_eq!(voted.poll.options.iter().map(|option| option.vote_count).collect::<Vec<_>>(), vec![1, 0, 1]);
    assert_eq!(voted.poll.voter_count, 1);

    // Past the deadline the poll is closed without anyone closing it.
    sqlx::query("UPDATE community_polls SET closes_at = now() - interval '1 minute' WHERE thread_id = $1")
        .bind(thread.id.parse::<uuid::Uuid>().unwrap())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(ctx.reason(SVC, "VotePoll", Some(&voter), &vote).await, "POLL_CLOSED");
    let seen = ctx.get_thread(None, &thread.id).await.unwrap();
    assert!(seen.poll.closed && seen.poll.results_visible);
}

#[tokio::test]
async fn slow_mode_spaces_out_a_members_posts() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let general = ctx.general(&owner, &community).await;
    let slow = |seconds: i32| SetSlowModeRequest { channel_id: general.id.clone(), seconds, ..Default::default() };
    assert_eq!(
        ctx.reason("CommunityModerationService", "SetSlowMode", Some(&member), &slow(60)).await,
        "PERMISSION_DENIED"
    );
    assert_eq!(
        ctx.reason("CommunityModerationService", "SetSlowMode", Some(&moderator), &slow(21_601)).await,
        "DURATION_INVALID"
    );
    assert_eq!(
        ctx.reason("CommunityModerationService", "SetSlowMode", Some(&moderator), &slow(-1)).await,
        "DURATION_INVALID"
    );
    let set: SetSlowModeResponse =
        ctx.call("CommunityModerationService", "SetSlowMode", Some(&moderator), &slow(60)).await.unwrap();
    assert_eq!(set.channel.slow_mode_seconds, 60);

    // A post that is refused does not use up the member's turn.
    let empty = CreateThreadRequest { channel_id: general.id.clone(), title: "空".into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CreateThread", Some(&member), &empty).await, "THREAD_EMPTY");
    let thread = ctx.thread(&member, &general, "ひとつめ", "x").await;
    let again = CreateThreadRequest {
        channel_id: general.id.clone(),
        title: "ふたつめ".into(),
        body: "x".into(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(SVC, "CreateThread", Some(&member), &again).await, "SLOW_MODE");
    let reply = CreateReplyRequest { thread_id: thread.id.clone(), body: "返信も".into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CreateReply", Some(&member), &reply).await, "SLOW_MODE");
    // The staff is not slowed down.
    ctx.thread(&moderator, &general, "a", "x").await;
    ctx.thread(&moderator, &general, "b", "x").await;

    let _: SetSlowModeResponse =
        ctx.call("CommunityModerationService", "SetSlowMode", Some(&moderator), &slow(0)).await.unwrap();
    ctx.try_thread(&member, &again).await.unwrap();
    let log = ctx.moderation_log(&owner, &community).await;
    assert_eq!(
        log.iter().filter(|entry| entry.action == ModerationAction::MODERATION_ACTION_SLOW_MODE_CHANGED).count(),
        2
    );
}

#[tokio::test]
async fn posting_is_limited_per_minute() {
    let ctx = start_with(CommunityConfig { posts_per_minute: 2, ..test_config() }).await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&owner, &general, "1", "x").await;
    ctx.reply(&owner, &thread, "2").await;
    let third = CreateReplyRequest { thread_id: thread.id.clone(), body: "3".into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CreateReply", Some(&owner), &third).await, "RATE_LIMITED");
}

#[tokio::test]
async fn a_block_hides_two_members_posts_from_each_other() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let (alice, bob) = (ctx.member(&owner, &community, "Alice").await, ctx.member(&owner, &community, "Bob").await);
    let general = ctx.general(&owner, &community).await;
    let by_alice = ctx.thread(&alice, &general, "alice's", "x").await;
    let by_owner = ctx.thread(&owner, &general, "owner's", "x").await;
    let alices_reply = ctx.reply(&alice, &by_owner, "from alice").await;
    let quoting = ctx
        .try_reply(
            &owner,
            &CreateReplyRequest {
                thread_id: by_owner.id.clone(),
                body: "to alice".into(),
                reply_to_id: alices_reply.id.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    ctx.block(&bob, &alice).await;

    // Bob no longer sees what Alice wrote — lists, direct links, quotes.
    assert_eq!(ctx.thread_titles(Some(&bob), &community).await, vec!["owner's"]);
    assert_eq!(ctx.get_thread(Some(&bob), &by_alice.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    let replies = ctx.replies(Some(&bob), &by_owner.id).await;
    assert_eq!(replies.iter().map(|reply| reply.id.clone()).collect::<Vec<_>>(), vec![quoting.id.clone()]);
    assert!(replies[0].reply_to.unavailable && replies[0].reply_to.excerpt.is_empty());
    assert_eq!(ctx.get_reply(Some(&bob), &alices_reply.id).await.unwrap_err().reason, "REPLY_NOT_FOUND");
    let answer = CreateReplyRequest {
        thread_id: by_owner.id.clone(),
        body: "x".into(),
        reply_to_id: alices_reply.id.clone(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(SVC, "CreateReply", Some(&bob), &answer).await, "REPLY_NOT_FOUND");
    // And the other way round.
    let by_bob = ctx.thread(&bob, &general, "bob's", "x").await;
    assert!(!ctx.thread_titles(Some(&alice), &community).await.contains(&"bob's".to_owned()));
    assert_eq!(ctx.get_thread(Some(&alice), &by_bob.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    // Everyone else sees everything.
    assert_eq!(ctx.thread_titles(Some(&owner), &community).await, vec!["bob's", "owner's", "alice's"]);
    assert_eq!(ctx.replies(None, &by_owner.id).await.len(), 2);

    // On the topic, a post of someone with a block travels as an ID only:
    // whoever follows the topic fetches it, and the fetch applies their blocks.
    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    let later = ctx.thread(&alice, &general, "after the block", "x").await;
    let pushed = pushes(&mut topic).await;
    assert!(pushed.iter().any(|push| matches!(push, Pushed::ThreadCreated(created)
        if created.thread_id == later.id && created.thread.as_option().is_none())));
}
