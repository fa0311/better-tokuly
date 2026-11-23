//! `LiveChatService` and `LivePollService`: chat, its moderation, polls.
mod common;

use common::*;
use futures::future::join_all;
use tokuly_core::prelude::*;
use tokuly_proto::{common::SubjectType, live::*, video::NotificationLevel};

#[tokio::test]
async fn members_write_guests_read_and_the_streamer_speaks_as_the_channel() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "チャットch").await;
    let stream = ctx.on_air(&alice, &channel, "おしゃべり").await;
    let mut topic = ctx.app.listen_topic(&stream.realtime_topic).await;

    let hello = ctx.say(&bob, &stream, "  こんにちは!\n初見です ").await.unwrap();
    assert_eq!(hello.text, "こんにちは! 初見です");
    assert_eq!(
        (hello.kind, hello.role),
        (ChatMessageKind::CHAT_MESSAGE_KIND_TEXT.into(), ChatRole::CHAT_ROLE_VIEWER.into())
    );
    assert_eq!(hello.author.as_option().unwrap().user.as_option().unwrap().id, bob.id.to_string());
    let pushed = wait_for(&mut topic, |push| match push {
        Push::ChatMessage(message) => Some(message),
        _ => None,
    })
    .await;
    assert_eq!(pushed.id, hello.id);

    // The streamer writes as the channel: the account behind it is not shown.
    let reply = ctx.say(&alice, &stream, "いらっしゃい!").await.unwrap();
    assert_eq!(reply.role, ChatRole::CHAT_ROLE_OWNER);
    let author = reply.author.as_option().unwrap();
    assert!(!author.user.is_set());
    assert_eq!(author.channel.as_option().unwrap().id, channel.summary.id);
    assert!(!serde_json::to_string(&reply).unwrap().contains(&alice.id.to_string()));

    // A stamp the sender may use.
    let stamp_id = ctx.stamp(&bob).await;
    let request = SendChatMessageRequest {
        stream_id: card(&stream).id.clone(),
        stamp_id: stamp_id.clone(),
        ..Default::default()
    };
    let sent: SendChatMessageResponse = ctx.call(CHAT, "SendChatMessage", Some(&bob), &request).await.unwrap();
    let stamp = sent.message.as_option().unwrap();
    assert_eq!(stamp.kind, ChatMessageKind::CHAT_MESSAGE_KIND_STAMP);
    assert_eq!(stamp.stamp.as_option().unwrap().id, stamp_id);
    // Someone else's personal stamp is not theirs to send.
    assert_eq!(ctx.reason(CHAT, "SendChatMessage", Some(&alice), &request).await, "STAMP_NOT_USABLE");
    let both = SendChatMessageRequest { text: "と".into(), ..request.clone() };
    assert_eq!(ctx.reason(CHAT, "SendChatMessage", Some(&bob), &both).await, "MESSAGE_INVALID");

    // Guests read (newest first, paged) but do not write.
    let guest = SendChatMessageRequest { stream_id: card(&stream).id.clone(), text: "hi".into(), ..Default::default() };
    assert_eq!(ctx.reason(CHAT, "SendChatMessage", None, &guest).await, "SIGN_IN_REQUIRED");
    let list = |limit, cursor: &str| ListChatMessagesRequest {
        stream_id: card(&stream).short_id.clone(),
        page: page(limit, cursor),
        ..Default::default()
    };
    let first: ListChatMessagesResponse = ctx.call(CHAT, "ListChatMessages", None, &list(2, "")).await.unwrap();
    assert_eq!(
        first.messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec![stamp.id.as_str(), reply.id.as_str()]
    );
    let info = first.page.as_option().unwrap();
    assert!(info.has_more);
    let rest: ListChatMessagesResponse =
        ctx.call(CHAT, "ListChatMessages", None, &list(2, &info.next_cursor)).await.unwrap();
    assert_eq!(rest.messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec![hello.id.as_str()]);
    assert_eq!(ctx.row(&stream).await.chat_message_count, 3);

    for (text, reason) in [("", "MESSAGE_INVALID"), ("   ", "MESSAGE_INVALID")] {
        let bad =
            SendChatMessageRequest { stream_id: card(&stream).id.clone(), text: text.into(), ..Default::default() };
        assert_eq!(ctx.reason(CHAT, "SendChatMessage", Some(&bob), &bad).await, reason);
    }
    assert_eq!(ctx.say(&bob, &stream, &"あ".repeat(201)).await.unwrap_err().reason, "MESSAGE_INVALID");
    // Once the stream is over the chat is closed.
    ctx.end(&alice, &stream).await;
    assert_eq!(ctx.say(&bob, &stream, "おつかれ").await.unwrap_err().reason, "STREAM_ENDED");
    let unknown =
        SendChatMessageRequest { stream_id: Uuid::now_v7().to_string(), text: "?".into(), ..Default::default() };
    assert_eq!(ctx.reason(CHAT, "SendChatMessage", Some(&bob), &unknown).await, "STREAM_NOT_FOUND");
}

#[tokio::test]
async fn chat_rules_decide_who_may_write_what_and_how_fast() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let moderator = ctx.app.user("mod").await;
    let channel = ctx.channel(&alice, "ルールch").await;
    ctx.subscribe(&carol, &channel, NotificationLevel::NOTIFICATION_LEVEL_PERSONALIZED).await;
    ctx.subscribe(&moderator, &channel, NotificationLevel::NOTIFICATION_LEVEL_PERSONALIZED).await;
    let stream = ctx.on_air(&alice, &channel, "ルールの配信").await;
    let settings = |fill: fn(&mut ChatSettings)| {
        let mut chat = ChatSettings { enabled: true, stamps_enabled: true, ..Default::default() };
        fill(&mut chat);
        UpdateChatSettingsRequest { stream_id: card(&stream).id.clone(), chat: Some(chat).into(), ..Default::default() }
    };
    let add = AddModeratorRequest {
        channel_id: channel.summary.id.clone(),
        user_id: moderator.id.to_string(),
        ..Default::default()
    };
    let _: AddModeratorResponse = ctx.call(CHAT, "AddModerator", Some(&alice), &add).await.unwrap();

    // The rules of the chat are the streamer's: not a viewer's, and not a moderator's either
    // (a moderator acts on messages and people, and cannot close the chat on its owner).
    assert_eq!(ctx.reason(CHAT, "UpdateChatSettings", Some(&bob), &settings(|_| {})).await, "NOT_THE_STREAMER");
    assert_eq!(
        ctx.reason(CHAT, "UpdateChatSettings", Some(&moderator), &settings(|c| c.enabled = false)).await,
        "NOT_THE_STREAMER"
    );
    assert_eq!(
        ctx.reason(CHAT, "UpdateChatSettings", Some(&alice), &settings(|c| c.slow_mode_seconds = 301)).await,
        "CHAT_SETTINGS_INVALID"
    );
    let mut topic = ctx.app.listen_topic(&stream.realtime_topic).await;

    // Subscribers only.
    let _: UpdateChatSettingsResponse = ctx
        .call(
            CHAT,
            "UpdateChatSettings",
            Some(&alice),
            &settings(|c| c.audience = ChatAudience::CHAT_AUDIENCE_SUBSCRIBERS.into()),
        )
        .await
        .unwrap();
    let changed = wait_for(&mut topic, |push| match push {
        Push::ChatSettingsChanged(change) => Some(change),
        _ => None,
    })
    .await;
    assert_eq!(changed.chat.as_option().unwrap().audience, ChatAudience::CHAT_AUDIENCE_SUBSCRIBERS);
    assert_eq!(ctx.say(&bob, &stream, "登録してない").await.unwrap_err().reason, "SUBSCRIBERS_ONLY");
    assert_eq!(ctx.say(&carol, &stream, "登録者だよ").await.unwrap().role, ChatRole::CHAT_ROLE_SUBSCRIBER);
    let page = ctx.get_stream(Some(&bob), &card(&stream).id).await.unwrap();
    let state = page.viewer.as_option().unwrap();
    assert_eq!((state.can_chat, state.chat_block_reason.as_str()), (false, "SUBSCRIBERS_ONLY"));

    // Members only: accounts older than the asked age (the test accounts are seconds old).
    let _: UpdateChatSettingsResponse = ctx
        .call(
            CHAT,
            "UpdateChatSettings",
            Some(&alice),
            &settings(|c| c.audience = ChatAudience::CHAT_AUDIENCE_MEMBERS.into()),
        )
        .await
        .unwrap();
    assert_eq!(ctx.say(&carol, &stream, "新参です").await.unwrap_err().reason, "MEMBERS_ONLY");
    sqlx::query("UPDATE users SET created_at = now() - interval '3 days' WHERE id = $1")
        .bind(carol.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.say(&carol, &stream, "古参です").await.unwrap();
    // The streamer and moderators are never locked out.
    assert_eq!(ctx.say(&moderator, &stream, "モデです").await.unwrap().role, ChatRole::CHAT_ROLE_MODERATOR);

    // A closed chat.
    let _: UpdateChatSettingsResponse =
        ctx.call(CHAT, "UpdateChatSettings", Some(&alice), &settings(|c| c.enabled = false)).await.unwrap();
    assert_eq!(ctx.say(&bob, &stream, "しーん").await.unwrap_err().reason, "CHAT_DISABLED");
    ctx.say(&alice, &stream, "お知らせ").await.unwrap();

    // Links and stamps off, slow mode on.
    let strict = settings(|c| {
        c.link_policy = LinkPolicy::LINK_POLICY_BLOCK.into();
        c.stamps_enabled = false;
        c.slow_mode_seconds = 30;
    });
    let _: UpdateChatSettingsResponse = ctx.call(CHAT, "UpdateChatSettings", Some(&alice), &strict).await.unwrap();
    assert_eq!(ctx.say(&bob, &stream, "見て https://example.com").await.unwrap_err().reason, "LINKS_NOT_ALLOWED");
    ctx.say(&moderator, &stream, "公式は https://example.com").await.unwrap();
    let stamp = SendChatMessageRequest {
        stream_id: card(&stream).id.clone(),
        stamp_id: ctx.stamp(&bob).await,
        ..Default::default()
    };
    assert_eq!(ctx.reason(CHAT, "SendChatMessage", Some(&bob), &stamp).await, "STAMPS_DISABLED");
    ctx.say(&bob, &stream, "一通目").await.unwrap();
    let error = ctx.say(&bob, &stream, "二通目").await.unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("resource_exhausted", "SLOW_MODE"));
    // A refused message does not use up the slot; moderators are not slowed.
    ctx.say(&moderator, &stream, "一").await.unwrap();
    ctx.say(&moderator, &stream, "二").await.unwrap();

    // Banned words of the channel.
    let words = SetBannedWordsRequest {
        channel_id: channel.summary.id.clone(),
        words: vec![" ネタバレ ".into(), "Spoiler".into()],
        ..Default::default()
    };
    assert_eq!(ctx.reason(CHAT, "SetBannedWords", Some(&moderator), &words).await, "CHANNEL_NOT_FOUND");
    let stored: SetBannedWordsResponse = ctx.call(CHAT, "SetBannedWords", Some(&alice), &words).await.unwrap();
    assert_eq!(stored.words, vec!["ネタバレ", "spoiler"]);
    let get = GetBannedWordsRequest { channel_id: channel.summary.id.clone(), ..Default::default() };
    let read: GetBannedWordsResponse = ctx.call(CHAT, "GetBannedWords", Some(&moderator), &get).await.unwrap();
    assert_eq!(read.words.len(), 2);
    assert_eq!(ctx.reason(CHAT, "GetBannedWords", Some(&bob), &get).await, "NOT_A_MODERATOR");
    assert_eq!(ctx.say(&carol, &stream, "それSPOILERだよ").await.unwrap_err().reason, "MESSAGE_BLOCKED");
    let too_long = SetBannedWordsRequest {
        channel_id: channel.summary.id.clone(),
        words: vec!["x".repeat(31)],
        ..Default::default()
    };
    assert_eq!(ctx.reason(CHAT, "SetBannedWords", Some(&alice), &too_long).await, "BANNED_WORDS_INVALID");

    // Writing too fast is refused whoever it is.
    let _: UpdateChatSettingsResponse =
        ctx.call(CHAT, "UpdateChatSettings", Some(&alice), &settings(|_| {})).await.unwrap();
    let dave = ctx.app.user("dave").await;
    let mut refused = 0;
    for i in 0..10 {
        if let Err(error) = ctx.say(&dave, &stream, &format!("連投{i}")).await {
            assert_eq!(error.reason, "RATE_LIMITED");
            refused += 1;
        }
    }
    assert_eq!(refused, 2);
}

#[tokio::test]
async fn moderators_delete_pin_and_restrict() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let troll = ctx.app.user("troll").await;
    let moderator = ctx.app.user("mod").await;
    let stranger = ctx.app.user("stranger").await;
    let channel = ctx.channel(&alice, "モデch").await;
    let stream = ctx.on_air(&alice, &channel, "モデの配信").await;
    let channel_id = channel.summary.id.clone();

    // Moderators are chosen among subscribers and friends, and are told.
    let add = |user: &tokuly_testkit::TestUser| AddModeratorRequest {
        channel_id: channel_id.clone(),
        user_id: user.id.to_string(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(CHAT, "AddModerator", Some(&alice), &add(&stranger)).await, "MODERATOR_NOT_ELIGIBLE");
    assert_eq!(ctx.reason(CHAT, "AddModerator", Some(&alice), &add(&alice)).await, "CANNOT_MODERATE_SELF");
    assert_eq!(ctx.reason(CHAT, "AddModerator", Some(&bob), &add(&moderator)).await, "CHANNEL_NOT_FOUND");
    ctx.befriend(&alice, &moderator).await;
    let appointed: AddModeratorResponse = ctx.call(CHAT, "AddModerator", Some(&alice), &add(&moderator)).await.unwrap();
    assert_eq!(appointed.moderator.as_option().unwrap().user.as_option().unwrap().id, moderator.id.to_string());
    let _: AddModeratorResponse = ctx.call(CHAT, "AddModerator", Some(&alice), &add(&moderator)).await.unwrap();
    let notices = ctx.notifications(&moderator, "live.moderator").await;
    assert_eq!(notices.len(), 1, "appointing twice notifies once");
    assert!(notices[0].actor_id.is_none(), "the channel speaks, not the account behind it");
    let list = ListModeratorsRequest { channel_id: channel_id.clone(), ..Default::default() };
    let moderators: ListModeratorsResponse = ctx.call(CHAT, "ListModerators", Some(&moderator), &list).await.unwrap();
    assert_eq!(moderators.moderators.len(), 1);
    assert_eq!(ctx.reason(CHAT, "ListModerators", Some(&bob), &list).await, "NOT_A_MODERATOR");
    assert!(
        ctx.get_stream(Some(&moderator), &card(&stream).id).await.unwrap().viewer.as_option().unwrap().is_moderator
    );

    // Deleting: authors their own, moderators anything.
    let mine = ctx.say(&bob, &stream, "消すやつ").await.unwrap();
    let rude = ctx.say(&troll, &stream, "あらし1").await.unwrap();
    let delete =
        |message: &ChatMessage| DeleteChatMessageRequest { message_id: message.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(CHAT, "DeleteChatMessage", Some(&bob), &delete(&rude)).await, "NOT_A_MODERATOR");
    assert_eq!(ctx.reason(CHAT, "DeleteChatMessage", None, &delete(&rude)).await, "SIGN_IN_REQUIRED");
    let mut topic = ctx.app.listen_topic(&stream.realtime_topic).await;
    let _: DeleteChatMessageResponse = ctx.call(CHAT, "DeleteChatMessage", Some(&bob), &delete(&mine)).await.unwrap();
    let gone = wait_for(&mut topic, |push| match push {
        Push::ChatMessageDeleted(deleted) => Some(deleted),
        _ => None,
    })
    .await;
    assert_eq!(gone.message_id, mine.id);
    assert_eq!(ctx.reason(CHAT, "DeleteChatMessage", Some(&bob), &delete(&mine)).await, "MESSAGE_NOT_FOUND");
    let _: DeleteChatMessageResponse =
        ctx.call(CHAT, "DeleteChatMessage", Some(&moderator), &delete(&rude)).await.unwrap();
    assert!(ctx.chat(None, &stream).await.is_empty());
    // What the streamer wrote is not a moderator's to remove; the streamer removes their own.
    let hosts = ctx.say(&alice, &stream, "配信者のことば").await.unwrap();
    assert_eq!(
        ctx.reason(CHAT, "DeleteChatMessage", Some(&moderator), &delete(&hosts)).await,
        "CANNOT_MODERATE_STREAMER"
    );
    let _: DeleteChatMessageResponse =
        ctx.call(CHAT, "DeleteChatMessage", Some(&alice), &delete(&hosts)).await.unwrap();

    // Pinning.
    let good = ctx.say(&bob, &stream, "名言").await.unwrap();
    let pin = PinChatMessageRequest { message_id: good.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(CHAT, "PinChatMessage", Some(&bob), &pin).await, "NOT_A_MODERATOR");
    let pinned: PinChatMessageResponse = ctx.call(CHAT, "PinChatMessage", Some(&moderator), &pin).await.unwrap();
    assert_eq!(pinned.message.as_option().unwrap().id, good.id);
    let change = wait_for(&mut topic, |push| match push {
        Push::ChatPinChanged(change) => Some(change),
        _ => None,
    })
    .await;
    assert_eq!(change.message.as_option().unwrap().id, good.id);
    assert_eq!(ctx.get_stream(None, &card(&stream).id).await.unwrap().pinned_message.as_option().unwrap().id, good.id);
    let unpin = UnpinChatMessageRequest { stream_id: card(&stream).id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(CHAT, "UnpinChatMessage", Some(&bob), &unpin).await, "NOT_A_MODERATOR");
    let _: UnpinChatMessageResponse = ctx.call(CHAT, "UnpinChatMessage", Some(&alice), &unpin).await.unwrap();
    assert!(!ctx.get_stream(None, &card(&stream).id).await.unwrap().pinned_message.is_set());
    // A pinned message that is deleted is unpinned too.
    let _: PinChatMessageResponse = ctx.call(CHAT, "PinChatMessage", Some(&alice), &pin).await.unwrap();
    let _: DeleteChatMessageResponse = ctx.call(CHAT, "DeleteChatMessage", Some(&alice), &delete(&good)).await.unwrap();
    assert!(!ctx.get_stream(None, &card(&stream).id).await.unwrap().pinned_message.is_set());

    // Timeouts and bans.
    ctx.say(&troll, &stream, "あらし2").await.unwrap();
    assert_eq!(ctx.restrict(&bob, &channel, &troll, 60, false).await.unwrap_err().reason, "NOT_A_MODERATOR");
    assert_eq!(ctx.restrict(&moderator, &channel, &troll, 5, false).await.unwrap_err().reason, "RESTRICTION_INVALID");
    assert_eq!(ctx.restrict(&moderator, &channel, &alice, 60, false).await.unwrap_err().reason, "CANNOT_RESTRICT");
    assert_eq!(ctx.restrict(&moderator, &channel, &moderator, 60, false).await.unwrap_err().reason, "CANNOT_RESTRICT");
    let mut troll_feed = ctx.app.listen_user(troll.id).await;
    let timeout = ctx.restrict(&moderator, &channel, &troll, 600, true).await.unwrap();
    assert!(timeout.expires_at.is_set());
    assert_eq!(timeout.moderator.as_option().unwrap().id, moderator.id.to_string());
    let told = wait_for(&mut troll_feed, |push| match push {
        Push::ChatRestrictionChanged(change) => Some(change),
        _ => None,
    })
    .await;
    assert!(told.restricted && told.expires_at.is_set());
    let cleared = wait_for(&mut topic, |push| match push {
        Push::ChatUserCleared(cleared) => Some(cleared),
        _ => None,
    })
    .await;
    assert_eq!(cleared.user_id, troll.id.to_string());
    assert!(ctx.chat(None, &stream).await.is_empty(), "their messages were removed with them");
    assert_eq!(ctx.say(&troll, &stream, "まだいる").await.unwrap_err().reason, "CHAT_TIMED_OUT");
    let state = ctx.get_stream(Some(&troll), &card(&stream).id).await.unwrap();
    let state = state.viewer.as_option().unwrap();
    assert!(!state.can_chat && state.chat_block_reason == "CHAT_TIMED_OUT" && state.chat_blocked_until.is_set());
    // A ban replaces the timeout; the streamer's acts are not signed with their account.
    let ban = ctx.restrict(&alice, &channel, &troll, 0, false).await.unwrap();
    assert!(!ban.expires_at.is_set() && !ban.moderator.is_set());
    assert_eq!(ctx.say(&troll, &stream, "まだ?").await.unwrap_err().reason, "CHAT_BANNED");
    let listing = ListChatRestrictionsRequest { channel_id: channel_id.clone(), ..Default::default() };
    let restrictions: ListChatRestrictionsResponse =
        ctx.call(CHAT, "ListChatRestrictions", Some(&moderator), &listing).await.unwrap();
    assert_eq!(restrictions.restrictions.len(), 1);
    assert_eq!(restrictions.restrictions[0].user.as_option().unwrap().id, troll.id.to_string());
    assert_eq!(ctx.reason(CHAT, "ListChatRestrictions", Some(&bob), &listing).await, "NOT_A_MODERATOR");
    let lift = UnrestrictChatUserRequest {
        channel_id: channel_id.clone(),
        user_id: troll.id.to_string(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(CHAT, "UnrestrictChatUser", Some(&bob), &lift).await, "NOT_A_MODERATOR");
    let _: UnrestrictChatUserResponse = ctx.call(CHAT, "UnrestrictChatUser", Some(&moderator), &lift).await.unwrap();
    ctx.say(&troll, &stream, "ごめんなさい").await.unwrap();

    // Only the streamer restricts a moderator, and that ends the appointment.
    ctx.befriend(&alice, &bob).await;
    let _: AddModeratorResponse = ctx.call(CHAT, "AddModerator", Some(&alice), &add(&bob)).await.unwrap();
    assert_eq!(ctx.restrict(&moderator, &channel, &bob, 60, false).await.unwrap_err().reason, "CANNOT_RESTRICT");
    ctx.restrict(&alice, &channel, &bob, 60, false).await.unwrap();
    let moderators: ListModeratorsResponse = ctx.call(CHAT, "ListModerators", Some(&alice), &list).await.unwrap();
    assert_eq!(moderators.moderators.len(), 1);
    // A moderator may step down; nobody else dismisses.
    let remove = RemoveModeratorRequest {
        channel_id: channel_id.clone(),
        user_id: moderator.id.to_string(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(CHAT, "RemoveModerator", Some(&troll), &remove).await, "CHANNEL_NOT_FOUND");
    let _: RemoveModeratorResponse = ctx.call(CHAT, "RemoveModerator", Some(&moderator), &remove).await.unwrap();
    assert_eq!(ctx.reason(CHAT, "ListModerators", Some(&moderator), &list).await, "NOT_A_MODERATOR");
}

/// What was sent stays reportable: deleting a message, being timed out or
/// the stream being deleted does not take the handle away (checklist 4).
#[tokio::test]
async fn removed_messages_and_deleted_streams_can_still_be_reported() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let troll = ctx.app.user("troll").await;
    let channel = ctx.channel(&alice, "通報ch").await;
    let stream = ctx.on_air(&alice, &channel, "通報の配信").await;
    let subjects = &ctx.app.core.subjects;
    let message_type = SubjectType::SUBJECT_TYPE_LIVE_CHAT_MESSAGE;

    let insult = ctx.say(&troll, &stream, "ひどいことば").await.unwrap();
    let resolved = subjects.resolve(message_type, &insult.id, Some(bob.id)).await.unwrap();
    assert_eq!((resolved.owner_id, resolved.title.as_str()), (troll.id, "ひどいことば"));
    assert_eq!(resolved.path, format!("/live/{}", card(&stream).short_id));
    assert!(!resolved.interactions_enabled);
    // The author deletes it: gone from the chat, still there to be reported.
    let delete = DeleteChatMessageRequest { message_id: insult.id.clone(), ..Default::default() };
    let _: DeleteChatMessageResponse = ctx.call(CHAT, "DeleteChatMessage", Some(&troll), &delete).await.unwrap();
    assert!(ctx.chat(Some(&bob), &stream).await.is_empty());
    assert_eq!(subjects.resolve(message_type, &insult.id, Some(bob.id)).await.unwrap().owner_id, troll.id);
    // A timeout that clears their messages: the same.
    let second = ctx.say(&troll, &stream, "二つ目").await.unwrap();
    ctx.restrict(&alice, &channel, &troll, 600, true).await.unwrap();
    assert_eq!(subjects.resolve(message_type, &second.id, Some(bob.id)).await.unwrap().owner_id, troll.id);
    // The streamer deletes the whole stream: the stream and its chat stay reportable while they are kept.
    let streamer_line = ctx.say(&alice, &stream, "配信者のことば").await.unwrap();
    let delete = DeleteStreamRequest { stream_id: card(&stream).id.clone(), ..Default::default() };
    let _: DeleteStreamResponse = ctx.call(STREAMS, "DeleteStream", Some(&alice), &delete).await.unwrap();
    assert_eq!(ctx.get_stream(Some(&bob), &card(&stream).id).await.unwrap_err().reason, "STREAM_NOT_FOUND");
    assert_eq!(subjects.resolve(message_type, &streamer_line.id, Some(bob.id)).await.unwrap().owner_id, alice.id);
    let gone = subjects.resolve(SubjectType::SUBJECT_TYPE_LIVE_STREAM, &card(&stream).id, Some(bob.id)).await.unwrap();
    assert_eq!(gone.owner_id, alice.id);
    assert!(!gone.interactions_enabled, "a deleted stream takes no more likes or comments");
    // Someone the streamer blocked does not see either.
    ctx.block(&alice, &troll).await;
    assert_eq!(
        subjects.resolve(message_type, &streamer_line.id, Some(troll.id)).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND"
    );
    // The retention clock runs from the deletion, not from the stream's creation.
    sqlx::query("UPDATE live_streams SET created_at = now() - interval '90 days' WHERE id = $1")
        .bind(sid(&stream))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    tokuly_live::streams::purge_deleted(&ctx.app.core, 30).await.unwrap();
    assert!(
        subjects.resolve(message_type, &streamer_line.id, Some(bob.id)).await.is_ok(),
        "an old stream deleted today is kept"
    );
    sqlx::query("UPDATE live_streams SET deleted_at = now() - interval '31 days' WHERE id = $1")
        .bind(sid(&stream))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    tokuly_live::streams::purge_deleted(&ctx.app.core, 30).await.unwrap();
    assert_eq!(
        subjects.resolve(message_type, &streamer_line.id, Some(bob.id)).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND"
    );
}

/// A ban and a message of the banned user at the same moment: afterwards
/// no message of theirs is left in the chat, whichever came first (checklist 6).
#[tokio::test]
async fn a_ban_and_a_message_cannot_pass_each_other() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "競争ch").await;
    let stream = ctx.on_air(&alice, &channel, "競争の配信").await;
    for round in 0..8 {
        let troll = ctx.app.user("troll").await;
        // Every other round the troll is a moderator: being one is no way around the ban.
        if round % 2 == 1 {
            ctx.befriend(&alice, &troll).await;
            let add = AddModeratorRequest {
                channel_id: channel.summary.id.clone(),
                user_id: troll.id.to_string(),
                ..Default::default()
            };
            let _: AddModeratorResponse = ctx.call(CHAT, "AddModerator", Some(&alice), &add).await.unwrap();
        }
        let text = format!("すべりこみ{round}");
        let (said, banned) =
            tokio::join!(ctx.say(&troll, &stream, &text), ctx.restrict(&alice, &channel, &troll, 0, true));
        banned.unwrap();
        if let Err(error) = &said {
            assert_eq!(error.reason, "CHAT_BANNED");
        }
        let left: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM live_chat_messages WHERE stream_id = $1 AND user_id = $2 AND deleted_at IS NULL",
        )
        .bind(sid(&stream))
        .bind(troll.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
        assert_eq!(left, 0, "round {round}: the message was {}", if said.is_ok() { "sent" } else { "refused" });
    }
}

#[tokio::test]
async fn blocked_people_are_kept_apart_in_chat() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let channel = ctx.channel(&alice, "ブロックch").await;
    let stream = ctx.on_air(&alice, &channel, "ブロックの配信").await;
    ctx.say(&bob, &stream, "ぼぶです").await.unwrap();
    ctx.say(&carol, &stream, "きゃろるです").await.unwrap();
    // Carol blocks Bob: each reads the chat without the other.
    ctx.block(&carol, &bob).await;
    let texts = |messages: Vec<ChatMessage>| messages.into_iter().map(|m| m.text).collect::<Vec<_>>();
    assert_eq!(texts(ctx.chat(Some(&carol), &stream).await), vec!["きゃろるです"]);
    assert_eq!(texts(ctx.chat(Some(&bob), &stream).await), vec!["ぼぶです"]);
    assert_eq!(ctx.chat(None, &stream).await.len(), 2);
    // The streamer blocks Bob: the stream is gone for him, chat included.
    ctx.block(&alice, &bob).await;
    assert_eq!(ctx.say(&bob, &stream, "あれ?").await.unwrap_err().reason, "STREAM_NOT_FOUND");
    let list = ListChatMessagesRequest { stream_id: card(&stream).id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(CHAT, "ListChatMessages", Some(&bob), &list).await, "STREAM_NOT_FOUND");
}

#[tokio::test]
async fn chat_replay_follows_the_media_timeline() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "リプレイch").await;
    let stream = ctx.on_air(&alice, &channel, "リプレイの配信").await;
    let stream_id = sid(&stream);
    let db = &ctx.app.core.db;
    // On air for 100 s, encoder away for 50 s, back since 30 s.
    sqlx::query("UPDATE live_streams SET started_at = now() - interval '180 seconds' WHERE id = $1")
        .bind(stream_id)
        .execute(db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO live_ingest_sessions (stream_id, started_at, ended_at) VALUES
             ($1, now() - interval '200 seconds', now() - interval '80 seconds'),
             ($1, now() - interval '30 seconds', NULL)",
    )
    .bind(stream_id)
    .execute(db)
    .await
    .unwrap();
    let now_line = ctx.say(&bob, &stream, "いま").await.unwrap();
    // 100 s of the first connection after going on air, plus 30 s of the second.
    assert!((129_000..=133_000).contains(&now_line.offset_ms), "offset {}", now_line.offset_ms);
    let early = ctx.say(&bob, &stream, "むかし").await.unwrap();
    sqlx::query("UPDATE live_chat_messages SET offset_ms = 5000 WHERE id = $1")
        .bind(id(&early.id))
        .execute(db)
        .await
        .unwrap();

    let replay = |from, to| ListChatReplayRequest {
        stream_id: card(&stream).id.clone(),
        from_offset_ms: from,
        to_offset_ms: to,
        ..Default::default()
    };
    let window: ListChatReplayResponse = ctx.call(CHAT, "ListChatReplay", None, &replay(0, 60_000)).await.unwrap();
    assert_eq!(window.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), vec!["むかし"]);
    assert!(!window.truncated);
    let later: ListChatReplayResponse =
        ctx.call(CHAT, "ListChatReplay", None, &replay(100_000, 200_000)).await.unwrap();
    assert_eq!(later.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), vec!["いま"]);
    for (from, to) in [(-1, 10), (10, 10), (0, 600_001)] {
        assert_eq!(ctx.reason(CHAT, "ListChatReplay", None, &replay(from, to)).await, "REPLAY_RANGE_INVALID");
    }
    // By the archive video: whoever may watch the video reads the chat that went with it.
    let asset = tokuly_media::testing::ready_video(
        &ctx.app.core,
        alice.id,
        tokuly_proto::media::AssetPurpose::ASSET_PURPOSE_LIVE_ARCHIVE,
    )
    .await
    .unwrap();
    let video_id = tokuly_video::videos::create_from_asset(
        &ctx.app.core,
        tokuly_video::videos::NewVideo {
            visibility: tokuly_proto::common::Visibility::VISIBILITY_PUBLIC,
            origin: tokuly_proto::video::VideoOrigin::VIDEO_ORIGIN_LIVE_ARCHIVE,
            origin_id: Some(stream_id),
            publish: true,
            ..tokuly_video::videos::NewVideo::new(id(&channel.summary.id), asset.id, "アーカイブ")
        },
    )
    .await
    .unwrap();
    let by_video = ListChatReplayRequest {
        video_id: video_id.to_string(),
        from_offset_ms: 0,
        to_offset_ms: 60_000,
        ..Default::default()
    };
    let window: ListChatReplayResponse = ctx.call(CHAT, "ListChatReplay", None, &by_video).await.unwrap();
    assert_eq!(window.messages.len(), 1);
    let unknown = ListChatReplayRequest {
        video_id: Uuid::now_v7().to_string(),
        from_offset_ms: 0,
        to_offset_ms: 10,
        ..Default::default()
    };
    assert_eq!(ctx.reason(CHAT, "ListChatReplay", None, &unknown).await, "STREAM_NOT_FOUND");
}

#[tokio::test]
async fn polls_take_one_vote_per_viewer_and_close_by_themselves() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let channel = ctx.channel(&alice, "投票ch").await;
    let stream = ctx.on_air(&alice, &channel, "投票の配信").await;
    let create = |question: &str, options: &[&str], seconds| CreatePollRequest {
        stream_id: card(&stream).id.clone(),
        question: question.to_owned(),
        options: options.iter().map(|o| (*o).to_owned()).collect(),
        duration_seconds: seconds,
        ..Default::default()
    };
    assert_eq!(
        ctx.reason(POLLS, "CreatePoll", Some(&bob), &create("好き?", &["はい", "いいえ"], 60)).await,
        "NOT_A_MODERATOR"
    );
    assert_eq!(ctx.reason(POLLS, "CreatePoll", Some(&alice), &create("好き?", &["はい"], 60)).await, "POLL_INVALID");
    assert_eq!(
        ctx.reason(POLLS, "CreatePoll", Some(&alice), &create("好き?", &["はい", "いいえ"], 5)).await,
        "POLL_INVALID"
    );
    let mut topic = ctx.app.listen_topic(&stream.realtime_topic).await;
    let created: CreatePollResponse = ctx
        .call(POLLS, "CreatePoll", Some(&alice), &create("次のゲームは?", &["マイクラ", "スプラ", "雑談"], 60))
        .await
        .unwrap();
    let poll = created.poll.as_option().unwrap().clone();
    assert_eq!((poll.options.len(), poll.total_votes), (3, 0));
    let opened = wait_for(&mut topic, |push| match push {
        Push::PollUpdated(poll) => Some(poll),
        _ => None,
    })
    .await;
    assert_eq!(opened.id, poll.id);
    assert_eq!(
        ctx.reason(POLLS, "CreatePoll", Some(&alice), &create("もう一つ", &["a", "b"], 60)).await,
        "POLL_ALREADY_OPEN"
    );
    assert_eq!(ctx.get_stream(None, &card(&stream).id).await.unwrap().active_poll.as_option().unwrap().id, poll.id);

    let vote = |option| VotePollRequest { poll_id: poll.id.clone(), option, ..Default::default() };
    assert_eq!(ctx.reason(POLLS, "VotePoll", None, &vote(1)).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(POLLS, "VotePoll", Some(&bob), &vote(4)).await, "POLL_OPTION_INVALID");
    assert_eq!(ctx.reason(POLLS, "VotePoll", Some(&bob), &vote(0)).await, "POLL_OPTION_INVALID");
    let voted: VotePollResponse = ctx.call(POLLS, "VotePoll", Some(&bob), &vote(2)).await.unwrap();
    let after = voted.poll.as_option().unwrap();
    assert_eq!((after.viewer_choice, after.total_votes, after.options[1].vote_count), (2, 1, 1));
    assert_eq!(ctx.reason(POLLS, "VotePoll", Some(&bob), &vote(1)).await, "ALREADY_VOTED");
    // The same viewer voting many times at once still has one vote (checklist 6).
    let requests: Vec<VotePollRequest> = (0..6).map(|i| vote(1 + i % 3)).collect();
    let votes = join_all(
        requests.iter().map(|request| ctx.call::<_, VotePollResponse>(POLLS, "VotePoll", Some(&carol), request)),
    )
    .await;
    assert_eq!(votes.iter().filter(|result| result.is_ok()).count(), 1);
    let listed: ListPollsResponse = ctx
        .call(
            POLLS,
            "ListPolls",
            Some(&bob),
            &ListPollsRequest { stream_id: card(&stream).id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!((listed.polls.len(), listed.polls[0].total_votes, listed.polls[0].viewer_choice), (1, 2, 2));
    let guest: ListPollsResponse = ctx
        .call(POLLS, "ListPolls", None, &ListPollsRequest { stream_id: card(&stream).id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(guest.polls[0].viewer_choice, 0);

    // Closing: by a moderator, or by the clock.
    let end = EndPollRequest { poll_id: poll.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(POLLS, "EndPoll", Some(&bob), &end).await, "NOT_A_MODERATOR");
    let ended: EndPollResponse = ctx.call(POLLS, "EndPoll", Some(&alice), &end).await.unwrap();
    assert!(ended.poll.as_option().unwrap().ended_at.is_set());
    assert_eq!(ctx.reason(POLLS, "VotePoll", Some(&alice), &vote(1)).await, "POLL_CLOSED");
    let second: CreatePollResponse =
        ctx.call(POLLS, "CreatePoll", Some(&alice), &create("二回目", &["a", "b"], 60)).await.unwrap();
    let second = second.poll.as_option().unwrap();
    sqlx::query("UPDATE live_polls SET ends_at = now() - interval '1 second' WHERE id = $1")
        .bind(id(&second.id))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(ctx.live.tick_polls().await.unwrap() >= 1);
    assert!(!ctx.get_stream(None, &card(&stream).id).await.unwrap().active_poll.is_set());
    // Two polls opened at the same moment: one wins.
    let requests: Vec<CreatePollRequest> = (0..4).map(|i| create(&format!("同時{i}"), &["a", "b"], 60)).collect();
    let both = join_all(
        requests.iter().map(|request| ctx.call::<_, CreatePollResponse>(POLLS, "CreatePoll", Some(&alice), request)),
    )
    .await;
    assert_eq!(both.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(both.iter().filter_map(|result| result.as_ref().err()).all(|error| error.reason == "POLL_ALREADY_OPEN"));
    // The stream ending closes the open poll.
    ctx.end(&alice, &stream).await;
    let open: i64 = sqlx::query_scalar("SELECT count(*) FROM live_polls WHERE stream_id = $1 AND ended_at IS NULL")
        .bind(sid(&stream))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(open, 0);
    assert_eq!(ctx.reason(POLLS, "CreatePoll", Some(&alice), &create("おわり", &["a", "b"], 60)).await, "STREAM_ENDED");
}

/// The IDs of the chat lines among what a listener got within `QUIET` after the last one.
async fn lines(listener: &mut tokuly_testkit::FrameListener) -> Vec<String> {
    pushes(listener.drain(QUIET).await)
        .into_iter()
        .filter_map(|push| match push {
            Push::ChatMessage(message) => Some(message.id),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_chat_line_does_not_reach_whom_its_author_blocked_or_was_blocked_by() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let dave = ctx.app.user("dave").await;
    let erin = ctx.app.user("erin").await;
    let channel = ctx.channel(&alice, "ブロックch").await;
    let stream = ctx.on_air(&alice, &channel, "みんなの配信").await;
    let guards = &ctx.app.core.topic_guards;

    // Signed-in viewers follow the stream's topic (the gateway asks the guard,
    // which remembers them); the guest topic is for anonymous connections only.
    let guest_topic = ctx.get_stream(None, &card(&stream).id).await.unwrap().guest_topic;
    assert_eq!(guest_topic, format!("{}:guests", stream.realtime_topic));
    assert!(ctx.get_stream(Some(&erin), &card(&stream).id).await.unwrap().guest_topic.is_empty());
    for viewer in [&carol, &dave, &erin] {
        assert!(guards.may_subscribe(Some(viewer.id), &stream.realtime_topic).await.unwrap());
        assert!(!guards.may_subscribe(Some(viewer.id), &guest_topic).await.unwrap());
    }
    assert!(guards.may_subscribe(None, &guest_topic).await.unwrap());
    assert!(!guards.may_subscribe(None, &format!("live:{}:other", card(&stream).id)).await.unwrap());

    let mut topic = ctx.app.listen_topic(&stream.realtime_topic).await;
    let mut guests = ctx.app.listen_topic(&guest_topic).await;
    let mut devices = Vec::new();
    for user in [&alice, &bob, &carol, &dave, &erin] {
        devices.push(ctx.app.listen_user(user.id).await);
    }

    // Without blocks, a line goes to the topic.
    let before = ctx.say(&bob, &stream, "やあ").await.unwrap();
    assert_eq!(lines(&mut topic).await, [before.id.clone()]);
    assert!(lines(&mut guests).await.is_empty());

    // Carol blocks Bob, Bob blocks Dave: Bob's lines leave the topic. Whoever else follows
    // the stream gets them on their devices, and anonymous viewers on the guest topic.
    ctx.block(&carol, &bob).await;
    ctx.block(&bob, &dave).await;
    let after = ctx.say(&bob, &stream, "こんにちは").await.unwrap();
    assert!(lines(&mut topic).await.is_empty(), "the topic does not carry it");
    assert_eq!(lines(&mut guests).await, [after.id.clone()]);
    let [to_alice, to_bob, to_carol, to_dave, to_erin] = &mut devices[..] else { unreachable!() };
    assert_eq!(lines(to_alice).await, [after.id.clone()], "the streamer");
    assert_eq!(lines(to_bob).await, [after.id.clone()], "the author");
    assert_eq!(lines(to_erin).await, [after.id.clone()], "a viewer with no block");
    assert!(lines(to_carol).await.is_empty(), "who blocked the author");
    assert!(lines(to_dave).await.is_empty(), "whom the author blocked");

    // Everyone else's lines still go to the topic.
    let other = ctx.say(&erin, &stream, "どうも").await.unwrap();
    assert_eq!(lines(&mut topic).await, [other.id]);
    assert!(lines(to_erin).await.is_empty());
}
