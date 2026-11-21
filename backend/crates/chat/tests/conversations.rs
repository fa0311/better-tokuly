//! `ConversationService`: direct conversations and message requests, groups
//! and their members, the conversation list, private preferences, read state.
mod common;

use common::*;
use tokuly_chat::ChatConfig;
use tokuly_proto::{
    chat::{chat_event::Event as Push, *},
    identity::Audience,
    media::{AssetKind, AssetPurpose},
};
use uuid::Uuid;

fn conversation_request(conversation_id: &str) -> GetConversationRequest {
    GetConversationRequest { conversation_id: conversation_id.to_owned(), ..Default::default() }
}

fn ids(conversations: &[Conversation]) -> Vec<String> {
    conversations.iter().map(|conversation| conversation.id.clone()).collect()
}

#[tokio::test]
async fn capabilities_describe_the_server() {
    let ctx = start().await;
    let user = ctx.app.user("taro").await;
    let response: GetCapabilitiesResponse =
        ctx.call("Conversation", "GetCapabilities", &user, &GetCapabilitiesRequest::default()).await.unwrap();
    let capabilities = response.capabilities.as_option().unwrap();
    assert_eq!(capabilities.max_text_length, 4000);
    assert_eq!(capabilities.max_attachments, 10);
    assert_eq!(capabilities.max_group_members, 100);
    assert_eq!(capabilities.request_message_limit, 3);
    assert_eq!(capabilities.pov_answer_seconds, 7200);
    assert_eq!(capabilities.pov_reply_seconds, 86400);
    assert!(!capabilities.calls_enabled && !capabilities.assist_enabled, "neither is configured in this test");
    assert_eq!(
        ctx.anonymous_reason("Conversation", "GetCapabilities", &GetCapabilitiesRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
}

#[tokio::test]
async fn friends_share_one_direct_conversation() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;

    let opened = ctx.direct(&taro, &hanako).await;
    assert_eq!(opened.kind, ConversationKind::CONVERSATION_KIND_DIRECT);
    assert_eq!(opened.member_count, 2);
    assert!(opened.name.is_empty() && !opened.read_only);
    let mut members: Vec<&str> = opened.members.iter().map(|member| member.id.as_str()).collect();
    members.sort_unstable();
    let mut expected = vec![taro.id.to_string(), hanako.id.to_string()];
    expected.sort_unstable();
    assert_eq!(members, expected);
    let viewer = opened.viewer.as_option().unwrap();
    assert_eq!(viewer.request_state, RequestState::REQUEST_STATE_ACCEPTED);
    assert_eq!(viewer.request_messages_left, -1, "friends talk without a request");
    assert!(opened.last_message.is_unset());

    // The same conversation from the other side, and when asked again.
    assert_eq!(ctx.direct(&hanako, &taro).await.id, opened.id);
    assert_eq!(ctx.direct(&taro, &hanako).await.id, opened.id);
    let created = ctx.app.events_by("chat.conversation.created", taro.id).await;
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].data["conversation_id"], opened.id);
}

#[tokio::test]
async fn strangers_get_in_only_when_the_recipient_allows_it() {
    let ctx = start().await;
    let taro = ctx.app.user("taro").await;
    let hanako = ctx.app.user("hanako").await;
    let open =
        |user: &tokuly_testkit::TestUser| OpenDirectRequest { user_id: user.id.to_string(), ..Default::default() };

    // By default only friends may message someone.
    assert_eq!(ctx.reason("Conversation", "OpenDirect", &taro, &open(&hanako)).await, "DM_NOT_ALLOWED");
    assert_eq!(ctx.reason("Conversation", "OpenDirect", &taro, &open(&taro)).await, "CANNOT_MESSAGE_SELF");
    let nobody = OpenDirectRequest { user_id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.reason("Conversation", "OpenDirect", &taro, &nobody).await, "USER_NOT_FOUND");
    let garbage = OpenDirectRequest { user_id: "not-an-id".into(), ..Default::default() };
    assert_eq!(ctx.reason("Conversation", "OpenDirect", &taro, &garbage).await, "INVALID_ID");
    assert_eq!(ctx.anonymous_reason("Conversation", "OpenDirect", &open(&hanako)).await, "SIGN_IN_REQUIRED");

    ctx.open_to_everyone(&hanako).await;
    let opened = ctx.direct(&taro, &hanako).await;
    let viewer = opened.viewer.as_option().unwrap();
    assert_eq!(viewer.request_state, RequestState::REQUEST_STATE_ACCEPTED);
    assert_eq!(viewer.request_messages_left, 3, "a stranger's messages are a request with an allowance");
    let hanako_view = ctx.conversation(&hanako, &opened.id).await;
    assert_eq!(hanako_view.conversation.viewer.request_state, RequestState::REQUEST_STATE_PENDING);

    // Nobody can be forbidden from messaging everyone, and a block hides a user entirely.
    let jiro = ctx.app.user("jiro").await;
    ctx.set_privacy(&jiro, |privacy| privacy.direct_messages_from = Audience::AUDIENCE_NOBODY.into()).await;
    assert_eq!(ctx.reason("Conversation", "OpenDirect", &taro, &open(&jiro)).await, "DM_NOT_ALLOWED");
    let saburo = ctx.app.user("saburo").await;
    ctx.open_to_everyone(&saburo).await;
    ctx.block(&saburo, &taro).await;
    assert_eq!(ctx.reason("Conversation", "OpenDirect", &taro, &open(&saburo)).await, "USER_NOT_FOUND");
    assert_eq!(ctx.reason("Conversation", "OpenDirect", &saburo, &open(&taro)).await, "USER_NOT_FOUND");
}

#[tokio::test]
async fn a_message_request_waits_until_it_is_accepted() {
    let ctx = start().await;
    let stranger = ctx.app.user("stranger").await;
    let hanako = ctx.app.user("hanako").await;
    ctx.open_to_everyone(&hanako).await;
    let conversation = ctx.direct(&stranger, &hanako).await;

    // Only plain content, and only a few messages, before the request is accepted.
    let photo = ctx.photo(&stranger).await;
    let with_photo = SendMessageRequest {
        conversation_id: conversation.id.clone(),
        client_id: client_id(),
        attachment_asset_ids: vec![photo],
        ..Default::default()
    };
    assert_eq!(ctx.send(&stranger, &with_photo).await.unwrap_err().reason, "REQUEST_PENDING");
    ctx.text(&stranger, &conversation.id, "はじめまして！").await;
    ctx.text(&stranger, &conversation.id, "友達の友達です").await;
    assert_eq!(ctx.conversation(&stranger, &conversation.id).await.conversation.viewer.request_messages_left, 1);

    // The request announces itself once and stays out of the inbox and the badge.
    assert_eq!(ctx.notification_kinds(&hanako).await, vec!["chat.request"]);
    assert!(ctx.list(&hanako, ConversationFilter::CONVERSATION_FILTER_INBOX).await.conversations.is_empty());
    let requests = ctx.list(&hanako, ConversationFilter::CONVERSATION_FILTER_REQUESTS).await;
    assert_eq!(ids(&requests.conversations), vec![conversation.id.clone()]);
    assert_eq!(requests.conversations[0].viewer.unread_count, 2);
    let unread = requests.unread.as_option().unwrap();
    assert_eq!((unread.badge, unread.pending_requests), (0, 1));

    // Looking at a request tells its sender nothing.
    let mut stranger_events = ctx.listen(&stranger).await;
    ctx.mark_read(&hanako, &conversation.id).await;
    assert!(ctx.events(&mut stranger_events).await.is_empty(), "no read receipt for a request");

    ctx.text(&stranger, &conversation.id, "三通目").await;
    let fourth = SendMessageRequest {
        conversation_id: conversation.id.clone(),
        client_id: client_id(),
        text: "四通目".into(),
        ..Default::default()
    };
    assert_eq!(ctx.send(&stranger, &fourth).await.unwrap_err().reason, "REQUEST_PENDING");

    // Accepting moves it to the inbox and lifts the sender's allowance.
    let mut stranger_events = ctx.listen(&stranger).await;
    let respond =
        RespondToRequestRequest { conversation_id: conversation.id.clone(), accept: true, ..Default::default() };
    let accepted: RespondToRequestResponse =
        ctx.call("Conversation", "RespondToRequest", &hanako, &respond).await.unwrap();
    assert_eq!(accepted.conversation.viewer.request_state, RequestState::REQUEST_STATE_ACCEPTED);
    let updates: Vec<i32> = ctx
        .events(&mut stranger_events)
        .await
        .into_iter()
        .filter_map(|event| match event {
            Push::ConversationUpdated(updated) => Some(updated.conversation.viewer.request_messages_left),
            _ => None,
        })
        .collect();
    assert_eq!(updates, vec![-1], "the sender learns that the request was accepted");
    assert_eq!(
        ids(&ctx.list(&hanako, ConversationFilter::CONVERSATION_FILTER_INBOX).await.conversations),
        vec![conversation.id.clone()]
    );
    ctx.send(&stranger, &fourth).await.expect("no allowance after acceptance");
    assert_eq!(ctx.reason("Conversation", "RespondToRequest", &hanako, &respond).await, "NO_PENDING_REQUEST");
    assert_eq!(ctx.reason("Conversation", "RespondToRequest", &stranger, &respond).await, "NO_PENDING_REQUEST");
}

#[tokio::test]
async fn declining_a_request_is_silent_and_answering_one_accepts_it() {
    let ctx = start().await;
    let hanako = ctx.app.user("hanako").await;
    ctx.open_to_everyone(&hanako).await;

    // Declined: gone for the recipient, nothing ever reaches them again, and
    // the sender cannot tell (the allowance simply runs out).
    let pest = ctx.app.user("pest").await;
    let conversation = ctx.direct(&pest, &hanako).await;
    ctx.text(&pest, &conversation.id, "ねえねえ").await;
    let mut hanako_events = ctx.listen(&hanako).await;
    let mut pest_events = ctx.listen(&pest).await;
    let decline =
        RespondToRequestRequest { conversation_id: conversation.id.clone(), accept: false, ..Default::default() };
    let declined: RespondToRequestResponse =
        ctx.call("Conversation", "RespondToRequest", &hanako, &decline).await.unwrap();
    assert!(declined.conversation.is_unset());
    let removed: Vec<_> = ctx.events(&mut hanako_events).await;
    assert!(
        matches!(removed.as_slice(), [Push::ConversationRemoved(event)] if event.reason == ConversationRemovalReason::CONVERSATION_REMOVAL_REASON_DECLINED),
        "the recipient's other devices drop the request"
    );
    assert!(ctx.events(&mut pest_events).await.is_empty(), "the sender is not told");

    let mut hanako_events = ctx.listen(&hanako).await;
    ctx.text(&pest, &conversation.id, "無視しないで").await;
    assert!(ctx.events(&mut hanako_events).await.is_empty(), "messages after a decline go unnoticed");
    assert!(ctx.list(&hanako, ConversationFilter::CONVERSATION_FILTER_REQUESTS).await.conversations.is_empty());
    assert_eq!(ctx.unread(&hanako).await.pending_requests, 0);
    assert_eq!(ctx.notification_kinds(&hanako).await, vec!["chat.request"], "only the very first message notified");
    ctx.text(&pest, &conversation.id, "三通目").await;
    let more = SendMessageRequest {
        conversation_id: conversation.id.clone(),
        client_id: client_id(),
        text: "四通目".into(),
        ..Default::default()
    };
    assert_eq!(ctx.send(&pest, &more).await.unwrap_err().reason, "REQUEST_PENDING");

    // Answered: replying to a request accepts it.
    let nice = ctx.app.user("nice").await;
    let conversation = ctx.direct(&nice, &hanako).await;
    ctx.text(&nice, &conversation.id, "こんにちは").await;
    ctx.text(&hanako, &conversation.id, "こんにちは！").await;
    assert_eq!(
        ctx.conversation(&hanako, &conversation.id).await.conversation.viewer.request_state,
        RequestState::REQUEST_STATE_ACCEPTED
    );
    assert_eq!(ctx.conversation(&nice, &conversation.id).await.conversation.viewer.request_messages_left, -1);
}

#[tokio::test]
async fn groups_are_created_with_friends() {
    let ctx = start().await;
    let owner = ctx.app.user("owner").await;
    let (hanako, jiro) = (ctx.app.user("hanako").await, ctx.app.user("jiro").await);
    ctx.befriend(&owner, &hanako).await;
    ctx.befriend(&owner, &jiro).await;
    let mut hanako_events = ctx.listen(&hanako).await;

    let avatar = ctx.upload(&owner, AssetPurpose::ASSET_PURPOSE_AVATAR, AssetKind::ASSET_KIND_IMAGE).await;
    let request = CreateGroupRequest {
        name: "  放課後メンバー ".into(),
        member_ids: vec![hanako.id.to_string(), jiro.id.to_string(), hanako.id.to_string(), owner.id.to_string()],
        icon_asset_id: avatar.clone(),
        ..Default::default()
    };
    let created: CreateGroupResponse = ctx.call("Conversation", "CreateGroup", &owner, &request).await.unwrap();
    let group = created.conversation.as_option().unwrap();
    assert_eq!(group.kind, ConversationKind::CONVERSATION_KIND_GROUP);
    assert_eq!(group.name, "放課後メンバー");
    assert_eq!(group.member_count, 3, "duplicates and the creator themselves are not counted twice");
    assert_eq!(group.icon.asset_id, avatar);
    assert_eq!(group.viewer.role, MemberRole::MEMBER_ROLE_OWNER);
    assert_eq!(group.members[0].id, owner.id.to_string(), "the avatar collage starts with the longest standing member");
    assert!(
        tokuly_media::assets::is_attached(&ctx.app.core, avatar.parse().unwrap()).await.unwrap(),
        "the icon upload is held"
    );

    // The story starts with a system message, which everyone receives.
    let system = group.last_message.system.as_option().expect("a system message");
    assert_eq!(system.r#type, SystemEventType::SYSTEM_EVENT_TYPE_GROUP_CREATED);
    assert_eq!(system.actor.id, owner.id.to_string());
    assert_eq!(system.targets.len(), 2);
    assert_eq!(system.text, "放課後メンバー");
    let received = created_messages(&ctx.events(&mut hanako_events).await);
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].kind, MessageKind::MESSAGE_KIND_SYSTEM);
    assert!(received[0].sender.is_unset());

    // Members are told they were added; a system message is nothing to read.
    assert_eq!(ctx.notification_kinds(&hanako).await, vec!["chat.group"]);
    let hanako_view = ctx.conversation(&hanako, &group.id).await;
    assert_eq!(hanako_view.conversation.viewer.role, MemberRole::MEMBER_ROLE_MEMBER);
    assert_eq!(hanako_view.conversation.viewer.unread_count, 0);
    assert_eq!(ctx.app.events_by("chat.member.joined", owner.id).await.len(), 2);
}

#[tokio::test]
async fn group_creation_is_validated() {
    let mut cfg = ChatConfig::default();
    cfg.max_group_members = 3;
    let ctx = start_with(cfg).await;
    let owner = ctx.app.user("owner").await;
    let friend = ctx.app.user("friend").await;
    ctx.befriend(&owner, &friend).await;
    let stranger = ctx.app.user("stranger").await;
    let group = |name: &str, members: &[&tokuly_testkit::TestUser]| CreateGroupRequest {
        name: name.to_owned(),
        member_ids: members.iter().map(|member| member.id.to_string()).collect(),
        ..Default::default()
    };

    assert_eq!(ctx.reason("Conversation", "CreateGroup", &owner, &group("  ", &[&friend])).await, "GROUP_NAME_INVALID");
    assert_eq!(
        ctx.reason("Conversation", "CreateGroup", &owner, &group(&"あ".repeat(51), &[&friend])).await,
        "GROUP_NAME_INVALID"
    );
    assert_eq!(ctx.reason("Conversation", "CreateGroup", &owner, &group("ひとり", &[])).await, "NO_MEMBERS");
    assert_eq!(ctx.reason("Conversation", "CreateGroup", &owner, &group("ひとり", &[&owner])).await, "NO_MEMBERS");
    assert_eq!(
        ctx.reason("Conversation", "CreateGroup", &owner, &group("知らない人", &[&friend, &stranger])).await,
        "NOT_FRIENDS"
    );
    assert_eq!(
        ctx.anonymous_reason("Conversation", "CreateGroup", &group("だれ", &[&friend])).await,
        "SIGN_IN_REQUIRED"
    );

    // Three members at most in this test: the creator and two others.
    let (second, third) = (ctx.app.user("second").await, ctx.app.user("third").await);
    ctx.befriend(&owner, &second).await;
    ctx.befriend(&owner, &third).await;
    assert_eq!(
        ctx.reason("Conversation", "CreateGroup", &owner, &group("多すぎ", &[&friend, &second, &third])).await,
        "TOO_MANY_MEMBERS"
    );

    // The icon must be an avatar upload of the caller.
    let chat_photo = ctx.photo(&owner).await;
    let wrong_purpose = CreateGroupRequest { icon_asset_id: chat_photo, ..group("アイコン", &[&friend]) };
    assert_eq!(ctx.reason("Conversation", "CreateGroup", &owner, &wrong_purpose).await, "ASSET_WRONG_PURPOSE");
    let someone_elses = ctx.upload(&friend, AssetPurpose::ASSET_PURPOSE_AVATAR, AssetKind::ASSET_KIND_IMAGE).await;
    let not_mine = CreateGroupRequest { icon_asset_id: someone_elses, ..group("アイコン", &[&friend]) };
    assert_eq!(ctx.reason("Conversation", "CreateGroup", &owner, &not_mine).await, "ASSET_NOT_FOUND");

    let created = ctx.group(&owner, "ちょうど", &[&friend, &second]).await;
    assert_eq!(created.member_count, 3);
    // Full: nobody else fits.
    let add = AddMembersRequest {
        conversation_id: created.id.clone(),
        user_ids: vec![third.id.to_string()],
        ..Default::default()
    };
    assert_eq!(ctx.reason("Conversation", "AddMembers", &owner, &add).await, "TOO_MANY_MEMBERS");
}

#[tokio::test]
async fn conversations_exist_only_for_their_members() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let outsider = ctx.app.user("outsider").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let id = conversation.id.clone();

    assert_eq!(
        ctx.reason("Conversation", "GetConversation", &outsider, &conversation_request(&id)).await,
        "CONVERSATION_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason("Conversation", "GetConversation", &taro, &conversation_request(&Uuid::now_v7().to_string())).await,
        "CONVERSATION_NOT_FOUND"
    );
    assert_eq!(
        ctx.anonymous_reason("Conversation", "GetConversation", &conversation_request(&id)).await,
        "SIGN_IN_REQUIRED"
    );
    let members = ListMembersRequest { conversation_id: id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Conversation", "ListMembers", &outsider, &members).await, "CONVERSATION_NOT_FOUND");
    let read = MarkReadRequest { conversation_id: id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Conversation", "MarkRead", &outsider, &read).await, "CONVERSATION_NOT_FOUND");
    let preferences =
        UpdatePreferencesRequest { conversation_id: id.clone(), pinned: Some(true), ..Default::default() };
    assert_eq!(
        ctx.reason("Conversation", "UpdatePreferences", &outsider, &preferences).await,
        "CONVERSATION_NOT_FOUND"
    );
    let hide = HideConversationRequest { conversation_id: id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Conversation", "HideConversation", &outsider, &hide).await, "CONVERSATION_NOT_FOUND");
    let respond = RespondToRequestRequest { conversation_id: id.clone(), accept: true, ..Default::default() };
    assert_eq!(ctx.reason("Conversation", "RespondToRequest", &outsider, &respond).await, "CONVERSATION_NOT_FOUND");
    assert_eq!(
        ctx.anonymous_reason("Conversation", "ListConversations", &ListConversationsRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
    assert_eq!(
        ctx.anonymous_reason("Conversation", "GetUnreadCounts", &GetUnreadCountsRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
    assert!(ctx.list(&outsider, ConversationFilter::CONVERSATION_FILTER_UNSPECIFIED).await.conversations.is_empty());
}

#[tokio::test]
async fn the_list_shows_pinned_conversations_first_then_the_latest() {
    let ctx = start().await;
    let me = ctx.app.user("me").await;
    let mut conversations = Vec::new();
    for name in ["first", "second", "third"] {
        let friend = ctx.app.user(name).await;
        ctx.befriend(&me, &friend).await;
        let conversation = ctx.direct(&me, &friend).await;
        ctx.text(&friend, &conversation.id, &format!("{name} より")).await;
        conversations.push((conversation.id, friend));
    }
    // A conversation nobody wrote in is not listed.
    let quiet = ctx.app.user("quiet").await;
    ctx.befriend(&me, &quiet).await;
    ctx.direct(&me, &quiet).await;

    let inbox = ctx.list(&me, ConversationFilter::CONVERSATION_FILTER_UNSPECIFIED).await;
    let order: Vec<String> = conversations.iter().rev().map(|(id, _)| id.clone()).collect();
    assert_eq!(ids(&inbox.conversations), order, "latest activity first");
    let newest = &inbox.conversations[0];
    assert_eq!(newest.last_message.text, "third より");
    assert_eq!(newest.last_message.sender.id, conversations[2].1.id.to_string(), "the preview says who wrote it");
    assert_eq!(newest.viewer.unread_count, 1);
    let unread = inbox.unread.as_option().unwrap();
    assert_eq!((unread.badge, unread.unread_conversations), (3, 3));

    // My own reply shows as the preview ("あなた: …" on the client) and clears what was unread.
    ctx.text(&me, &conversations[0].0, "了解！").await;
    let inbox = ctx.list(&me, ConversationFilter::CONVERSATION_FILTER_INBOX).await;
    assert_eq!(inbox.conversations[0].id, conversations[0].0);
    assert_eq!(inbox.conversations[0].last_message.sender.id, me.id.to_string());
    assert_eq!(inbox.conversations[0].viewer.unread_count, 0);
    assert_eq!(inbox.unread.badge, 2);

    // Pinned conversations lead, whatever happens later.
    let pin = UpdatePreferencesRequest {
        conversation_id: conversations[1].0.clone(),
        pinned: Some(true),
        ..Default::default()
    };
    let pinned: UpdatePreferencesResponse = ctx.call("Conversation", "UpdatePreferences", &me, &pin).await.unwrap();
    assert!(pinned.conversation.viewer.pinned);
    ctx.text(&conversations[2].1, &conversations[2].0, "もう一通").await;
    let inbox = ctx.list(&me, ConversationFilter::CONVERSATION_FILTER_INBOX).await;
    assert_eq!(
        ids(&inbox.conversations),
        vec![conversations[1].0.clone(), conversations[2].0.clone(), conversations[0].0.clone()]
    );

    // Pages follow the same order.
    let first_page: ListConversationsResponse = ctx
        .call(
            "Conversation",
            "ListConversations",
            &me,
            &ListConversationsRequest {
                page: tokuly_proto::common::PageRequest { limit: 2, ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(ids(&first_page.conversations), vec![conversations[1].0.clone(), conversations[2].0.clone()]);
    assert!(first_page.page.has_more);
    let next = tokuly_proto::common::PageRequest {
        limit: 2,
        cursor: first_page.page.next_cursor.clone(),
        ..Default::default()
    };
    let second_page: ListConversationsResponse = ctx
        .call(
            "Conversation",
            "ListConversations",
            &me,
            &ListConversationsRequest { page: next.into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(ids(&second_page.conversations), vec![conversations[0].0.clone()]);
    assert!(!second_page.page.has_more);
    let bad_cursor = tokuly_proto::common::PageRequest { cursor: "???".into(), ..Default::default() };
    assert_eq!(
        ctx.reason(
            "Conversation",
            "ListConversations",
            &me,
            &ListConversationsRequest { page: bad_cursor.into(), ..Default::default() }
        )
        .await,
        "INVALID_CURSOR"
    );
}

#[tokio::test]
async fn groups_are_renamed_and_get_icons_by_those_who_manage_them() {
    let ctx = start().await;
    let (group, owner, members) = ctx.group_of("旧名", 2).await;
    let member = &members[0];
    let rename = |name: &str| UpdateGroupRequest {
        conversation_id: group.id.clone(),
        name: Some(name.to_owned()),
        ..Default::default()
    };

    assert_eq!(ctx.reason("Conversation", "UpdateGroup", member, &rename("乗っ取り")).await, "NOT_ALLOWED");
    assert_eq!(ctx.reason("Conversation", "UpdateGroup", &owner, &rename("")).await, "GROUP_NAME_INVALID");
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(ctx.reason("Conversation", "UpdateGroup", &outsider, &rename("x")).await, "CONVERSATION_NOT_FOUND");

    let mut member_events = ctx.listen(member).await;
    let renamed: UpdateGroupResponse =
        ctx.call("Conversation", "UpdateGroup", &owner, &rename("新しい名前")).await.unwrap();
    assert_eq!(renamed.conversation.name, "新しい名前");
    let events = ctx.events(&mut member_events).await;
    assert!(events.iter().any(|event| matches!(event, Push::ConversationUpdated(updated) if updated.conversation.name == "新しい名前" && updated.conversation.viewer.is_unset())));
    let system = created_messages(&events);
    assert_eq!(system.len(), 1);
    assert_eq!(system[0].system.r#type, SystemEventType::SYSTEM_EVENT_TYPE_GROUP_RENAMED);
    assert_eq!(system[0].system.text, "新しい名前");

    // The same name again changes nothing and says nothing.
    let mut member_events = ctx.listen(member).await;
    let _: UpdateGroupResponse = ctx.call("Conversation", "UpdateGroup", &owner, &rename("新しい名前")).await.unwrap();
    assert!(ctx.events(&mut member_events).await.is_empty());

    // An admin may change the icon; removing it releases the upload.
    let promote = SetMemberRoleRequest {
        conversation_id: group.id.clone(),
        user_id: member.id.to_string(),
        role: MemberRole::MEMBER_ROLE_ADMIN.into(),
        ..Default::default()
    };
    let _: SetMemberRoleResponse = ctx.call("Conversation", "SetMemberRole", &owner, &promote).await.unwrap();
    let avatar = ctx.upload(member, AssetPurpose::ASSET_PURPOSE_AVATAR, AssetKind::ASSET_KIND_IMAGE).await;
    let with_icon = UpdateGroupRequest {
        conversation_id: group.id.clone(),
        icon_asset_id: Some(avatar.clone()),
        ..Default::default()
    };
    let updated: UpdateGroupResponse = ctx.call("Conversation", "UpdateGroup", member, &with_icon).await.unwrap();
    assert_eq!(updated.conversation.icon.asset_id, avatar);
    assert_eq!(updated.conversation.last_message.system.r#type, SystemEventType::SYSTEM_EVENT_TYPE_ICON_CHANGED);
    let avatar_id: Uuid = avatar.parse().unwrap();
    assert!(tokuly_media::assets::is_attached(&ctx.app.core, avatar_id).await.unwrap());
    let without_icon = UpdateGroupRequest {
        conversation_id: group.id.clone(),
        icon_asset_id: Some(String::new()),
        ..Default::default()
    };
    let updated: UpdateGroupResponse = ctx.call("Conversation", "UpdateGroup", &owner, &without_icon).await.unwrap();
    assert!(updated.conversation.icon.is_unset());
    assert!(!tokuly_media::assets::is_attached(&ctx.app.core, avatar_id).await.unwrap());

    // Direct conversations have nothing to rename.
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let rename_direct = UpdateGroupRequest { conversation_id: direct.id, name: Some("x".into()), ..Default::default() };
    assert_eq!(ctx.reason("Conversation", "UpdateGroup", &taro, &rename_direct).await, "NOT_A_GROUP");
}

#[tokio::test]
async fn friends_are_added_and_see_the_conversation_from_then_on() {
    let ctx = start().await;
    let (group, owner, members) = ctx.group_of("部活", 1).await;
    let member = &members[0];
    let before = ctx.text(&owner, &group.id, "入る前の話").await;

    // Any member may add their own friends, and only those.
    let newcomer = ctx.app.user("newcomer").await;
    let add = |users: &[&tokuly_testkit::TestUser]| AddMembersRequest {
        conversation_id: group.id.clone(),
        user_ids: users.iter().map(|user| user.id.to_string()).collect(),
        ..Default::default()
    };
    assert_eq!(ctx.reason("Conversation", "AddMembers", member, &add(&[&newcomer])).await, "NOT_FRIENDS");
    assert_eq!(ctx.reason("Conversation", "AddMembers", member, &add(&[])).await, "NO_MEMBERS");
    assert_eq!(ctx.reason("Conversation", "AddMembers", &newcomer, &add(&[&newcomer])).await, "CONVERSATION_NOT_FOUND");
    ctx.befriend(member, &newcomer).await;

    let mut newcomer_events = ctx.listen(&newcomer).await;
    let added: AddMembersResponse =
        ctx.call("Conversation", "AddMembers", member, &add(&[&newcomer, &owner])).await.unwrap();
    assert_eq!(added.conversation.member_count, 3);
    assert_eq!(added.added.len(), 1, "people who are already members are skipped");
    assert_eq!(added.added[0].user.id, newcomer.id.to_string());
    assert_eq!(added.added[0].added_by_id, member.id.to_string());
    let again: AddMembersResponse = ctx.call("Conversation", "AddMembers", member, &add(&[&newcomer])).await.unwrap();
    assert!(again.added.is_empty());

    // The newcomer is told, with their own view of the conversation.
    let events = ctx.events(&mut newcomer_events).await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Push::ConversationUpdated(updated) if updated.conversation.viewer.is_set()))
    );
    let system = created_messages(&events);
    assert_eq!(system[0].system.r#type, SystemEventType::SYSTEM_EVENT_TYPE_MEMBERS_ADDED);
    assert_eq!(ctx.notification_kinds(&newcomer).await, vec!["chat.group"]);

    // History starts at the moment they joined.
    let view = ctx.conversation(&newcomer, &group.id).await;
    assert_eq!(view.conversation.viewer.unread_count, 0);
    assert_eq!(view.conversation.viewer.history_from_seq, before.seq);
    let visible = ctx.latest(&newcomer, &group.id).await;
    assert_eq!(visible.len(), 1, "only the message announcing them");
    assert_eq!(visible[0].kind, MessageKind::MESSAGE_KIND_SYSTEM);
    let old = GetMessageRequest { message_id: before.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Message", "GetMessage", &newcomer, &old).await, "MESSAGE_NOT_FOUND");
    assert_eq!(ctx.latest(&owner, &group.id).await.len(), 3, "old members keep the whole history");
}

#[tokio::test]
async fn members_are_removed_by_those_above_them() {
    let ctx = start().await;
    let (group, owner, members) = ctx.group_of("部活", 3).await;
    let (admin, plain, other) = (&members[0], &members[1], &members[2]);
    let role = |user: &tokuly_testkit::TestUser, role: MemberRole| SetMemberRoleRequest {
        conversation_id: group.id.clone(),
        user_id: user.id.to_string(),
        role: role.into(),
        ..Default::default()
    };
    let _: SetMemberRoleResponse =
        ctx.call("Conversation", "SetMemberRole", &owner, &role(admin, MemberRole::MEMBER_ROLE_ADMIN)).await.unwrap();
    let remove = |user: &tokuly_testkit::TestUser| RemoveMemberRequest {
        conversation_id: group.id.clone(),
        user_id: user.id.to_string(),
        ..Default::default()
    };

    assert_eq!(ctx.reason("Conversation", "RemoveMember", plain, &remove(other)).await, "NOT_ALLOWED");
    assert_eq!(ctx.reason("Conversation", "RemoveMember", admin, &remove(&owner)).await, "NOT_ALLOWED");
    assert_eq!(ctx.reason("Conversation", "RemoveMember", admin, &remove(admin)).await, "CANNOT_REMOVE_SELF");
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(ctx.reason("Conversation", "RemoveMember", &owner, &remove(&outsider)).await, "MEMBER_NOT_FOUND");
    assert_eq!(ctx.reason("Conversation", "RemoveMember", &outsider, &remove(plain)).await, "CONVERSATION_NOT_FOUND");

    // An admin removes a plain member.
    let mut plain_events = ctx.listen(plain).await;
    let mut other_events = ctx.listen(other).await;
    let removed: RemoveMemberResponse = ctx.call("Conversation", "RemoveMember", admin, &remove(plain)).await.unwrap();
    assert_eq!(removed.conversation.member_count, 3);
    assert!(matches!(
        ctx.events(&mut plain_events).await.as_slice(),
        [Push::ConversationRemoved(event)] if event.reason == ConversationRemovalReason::CONVERSATION_REMOVAL_REASON_REMOVED
    ));
    let story = created_messages(&ctx.events(&mut other_events).await);
    assert_eq!(story[0].system.r#type, SystemEventType::SYSTEM_EVENT_TYPE_MEMBER_REMOVED);
    assert_eq!(story[0].system.actor.id, admin.id.to_string());
    assert_eq!(story[0].system.targets[0].id, plain.id.to_string());
    assert_eq!(
        ctx.reason("Conversation", "GetConversation", plain, &conversation_request(&group.id)).await,
        "CONVERSATION_NOT_FOUND"
    );
    assert!(ctx.list(plain, ConversationFilter::CONVERSATION_FILTER_INBOX).await.conversations.is_empty());
    let left = ctx.app.events("chat.member.left").await;
    assert!(
        left.iter()
            .any(|event| event.data["user_id"] == plain.id.to_string()
                && event.data["removed_by"] == admin.id.to_string())
    );

    // The owner removes an admin.
    let _: RemoveMemberResponse = ctx.call("Conversation", "RemoveMember", &owner, &remove(admin)).await.unwrap();
    assert_eq!(ctx.conversation(&owner, &group.id).await.conversation.member_count, 2);
}

#[tokio::test]
async fn roles_change_and_the_group_can_be_handed_over() {
    let ctx = start().await;
    let (group, owner, members) = ctx.group_of("部活", 2).await;
    let (first, second) = (&members[0], &members[1]);
    let role = |user: &tokuly_testkit::TestUser, role: MemberRole| SetMemberRoleRequest {
        conversation_id: group.id.clone(),
        user_id: user.id.to_string(),
        role: role.into(),
        ..Default::default()
    };

    assert_eq!(
        ctx.reason("Conversation", "SetMemberRole", first, &role(second, MemberRole::MEMBER_ROLE_ADMIN)).await,
        "NOT_ALLOWED"
    );
    assert_eq!(
        ctx.reason("Conversation", "SetMemberRole", &owner, &role(&owner, MemberRole::MEMBER_ROLE_ADMIN)).await,
        "CANNOT_CHANGE_OWN_ROLE"
    );
    assert_eq!(
        ctx.reason("Conversation", "SetMemberRole", &owner, &role(first, MemberRole::MEMBER_ROLE_UNSPECIFIED)).await,
        "ROLE_INVALID"
    );
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(
        ctx.reason("Conversation", "SetMemberRole", &owner, &role(&outsider, MemberRole::MEMBER_ROLE_ADMIN)).await,
        "MEMBER_NOT_FOUND"
    );

    let mut first_events = ctx.listen(first).await;
    let promoted: SetMemberRoleResponse =
        ctx.call("Conversation", "SetMemberRole", &owner, &role(first, MemberRole::MEMBER_ROLE_ADMIN)).await.unwrap();
    assert_eq!(promoted.member.role, MemberRole::MEMBER_ROLE_ADMIN);
    let events = ctx.events(&mut first_events).await;
    assert!(events.iter().any(|event| matches!(event, Push::ConversationUpdated(updated) if updated.conversation.viewer.role == MemberRole::MEMBER_ROLE_ADMIN)));
    assert_eq!(created_messages(&events)[0].system.r#type, SystemEventType::SYSTEM_EVENT_TYPE_ADMIN_ADDED);

    // Owner first, then admins, then members in the order they joined.
    let listed: ListMembersResponse = ctx
        .call(
            "Conversation",
            "ListMembers",
            second,
            &ListMembersRequest { conversation_id: group.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let order: Vec<(String, MemberRole)> =
        listed.members.iter().map(|member| (member.user.id.clone(), member.role.as_known().unwrap())).collect();
    assert_eq!(
        order,
        vec![
            (owner.id.to_string(), MemberRole::MEMBER_ROLE_OWNER),
            (first.id.to_string(), MemberRole::MEMBER_ROLE_ADMIN),
            (second.id.to_string(), MemberRole::MEMBER_ROLE_MEMBER),
        ]
    );
    let paged = ListMembersRequest {
        conversation_id: group.id.clone(),
        page: tokuly_proto::common::PageRequest { limit: 2, ..Default::default() }.into(),
        ..Default::default()
    };
    let first_page: ListMembersResponse = ctx.call("Conversation", "ListMembers", second, &paged).await.unwrap();
    assert_eq!(first_page.members.len(), 2);
    let rest = ListMembersRequest {
        page: tokuly_proto::common::PageRequest {
            limit: 2,
            cursor: first_page.page.next_cursor.clone(),
            ..Default::default()
        }
        .into(),
        ..paged
    };
    let second_page: ListMembersResponse = ctx.call("Conversation", "ListMembers", second, &rest).await.unwrap();
    assert_eq!(second_page.members.len(), 1);
    assert_eq!(second_page.members[0].user.id, second.id.to_string());

    // Handing the group over makes the old owner an admin.
    let handed: SetMemberRoleResponse =
        ctx.call("Conversation", "SetMemberRole", &owner, &role(second, MemberRole::MEMBER_ROLE_OWNER)).await.unwrap();
    assert_eq!(handed.member.role, MemberRole::MEMBER_ROLE_OWNER);
    assert_eq!(ctx.conversation(&owner, &group.id).await.conversation.viewer.role, MemberRole::MEMBER_ROLE_ADMIN);
    assert_eq!(
        ctx.conversation(&owner, &group.id).await.conversation.last_message.system.r#type,
        SystemEventType::SYSTEM_EVENT_TYPE_OWNER_CHANGED
    );
    assert_eq!(
        ctx.reason("Conversation", "SetMemberRole", &owner, &role(first, MemberRole::MEMBER_ROLE_MEMBER)).await,
        "NOT_ALLOWED"
    );
    let demoted: SetMemberRoleResponse =
        ctx.call("Conversation", "SetMemberRole", second, &role(first, MemberRole::MEMBER_ROLE_MEMBER)).await.unwrap();
    assert_eq!(demoted.member.role, MemberRole::MEMBER_ROLE_MEMBER);
}

#[tokio::test]
async fn leaving_passes_the_group_on_and_an_empty_group_is_deleted() {
    let ctx = start().await;
    let (group, owner, members) = ctx.group_of("部活", 2).await;
    let (first, second) = (&members[0], &members[1]);
    let leave = LeaveConversationRequest { conversation_id: group.id.clone(), ..Default::default() };
    let photo = ctx.photo(&owner).await;
    let with_photo = SendMessageRequest {
        conversation_id: group.id.clone(),
        client_id: client_id(),
        attachment_asset_ids: vec![photo.clone()],
        ..Default::default()
    };
    ctx.send(&owner, &with_photo).await.unwrap();

    // The owner leaves: the longest standing member inherits the group.
    let mut owner_events = ctx.listen(&owner).await;
    let mut first_events = ctx.listen(first).await;
    let _: LeaveConversationResponse = ctx.call("Conversation", "LeaveConversation", &owner, &leave).await.unwrap();
    assert!(matches!(
        ctx.events(&mut owner_events).await.as_slice(),
        [Push::ConversationRemoved(event)] if event.reason == ConversationRemovalReason::CONVERSATION_REMOVAL_REASON_LEFT
    ));
    let events = ctx.events(&mut first_events).await;
    let story: Vec<SystemEventType> =
        created_messages(&events).iter().map(|message| message.system.r#type.as_known().unwrap()).collect();
    assert_eq!(
        story,
        vec![SystemEventType::SYSTEM_EVENT_TYPE_MEMBER_LEFT, SystemEventType::SYSTEM_EVENT_TYPE_OWNER_CHANGED]
    );
    let view = ctx.conversation(first, &group.id).await;
    assert_eq!(view.conversation.viewer.role, MemberRole::MEMBER_ROLE_OWNER);
    assert_eq!(view.conversation.member_count, 2);
    assert_eq!(ctx.reason("Conversation", "LeaveConversation", &owner, &leave).await, "CONVERSATION_NOT_FOUND");

    // Everyone leaves: the group and what it held are gone.
    let _: LeaveConversationResponse = ctx.call("Conversation", "LeaveConversation", first, &leave).await.unwrap();
    assert_eq!(ctx.conversation(second, &group.id).await.conversation.viewer.role, MemberRole::MEMBER_ROLE_OWNER);
    let _: LeaveConversationResponse = ctx.call("Conversation", "LeaveConversation", second, &leave).await.unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM chat_conversations WHERE id = $1::uuid")
        .bind(&group.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    assert!(
        !tokuly_media::assets::is_attached(&ctx.app.core, photo.parse().unwrap()).await.unwrap(),
        "uploads of deleted messages are released"
    );

    // A direct conversation cannot be left, only hidden.
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let leave_direct = LeaveConversationRequest { conversation_id: direct.id, ..Default::default() };
    assert_eq!(ctx.reason("Conversation", "LeaveConversation", &taro, &leave_direct).await, "NOT_A_GROUP");
}

#[tokio::test]
async fn preferences_are_private_to_each_member() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let preferences = |change: UpdatePreferencesRequest| UpdatePreferencesRequest {
        conversation_id: conversation.id.clone(),
        ..change
    };

    // Mute: unread still counts, but nothing notifies and the badge ignores it.
    let mut taro_events = ctx.listen(&taro).await;
    let muted: UpdatePreferencesResponse = ctx
        .call(
            "Conversation",
            "UpdatePreferences",
            &taro,
            &preferences(UpdatePreferencesRequest { muted: Some(true), ..Default::default() }),
        )
        .await
        .unwrap();
    assert!(muted.conversation.viewer.muted && muted.conversation.viewer.muted_until.is_unset());
    assert!(
        ctx.events(&mut taro_events)
            .await
            .iter()
            .any(|event| matches!(event, Push::ConversationUpdated(updated) if updated.conversation.viewer.muted)),
        "the caller's other devices follow"
    );
    ctx.text(&hanako, &conversation.id, "聞こえる？").await;
    assert!(ctx.notifications(&taro).await.is_empty());
    let unread = ctx.unread(&taro).await;
    assert_eq!((unread.badge, unread.unread_conversations), (0, 1));
    assert!(
        !ctx.conversation(&hanako, &conversation.id).await.conversation.viewer.muted,
        "the other side has its own settings"
    );

    // A timed mute must end in the future.
    let past = tokuly_core::time::ts(chrono::Utc::now() - chrono::Duration::minutes(1));
    let invalid = preferences(UpdatePreferencesRequest { muted: Some(true), muted_until: past, ..Default::default() });
    assert_eq!(ctx.reason("Conversation", "UpdatePreferences", &taro, &invalid).await, "MUTE_UNTIL_INVALID");
    let later = tokuly_core::time::ts(chrono::Utc::now() + chrono::Duration::hours(8));
    let timed: UpdatePreferencesResponse = ctx
        .call(
            "Conversation",
            "UpdatePreferences",
            &taro,
            &preferences(UpdatePreferencesRequest { muted: Some(true), muted_until: later, ..Default::default() }),
        )
        .await
        .unwrap();
    assert!(timed.conversation.viewer.muted && timed.conversation.viewer.muted_until.is_set());
    let unmuted: UpdatePreferencesResponse = ctx
        .call(
            "Conversation",
            "UpdatePreferences",
            &taro,
            &preferences(UpdatePreferencesRequest { muted: Some(false), ..Default::default() }),
        )
        .await
        .unwrap();
    assert!(!unmuted.conversation.viewer.muted);
    ctx.text(&hanako, &conversation.id, "今度は？").await;
    assert_eq!(ctx.notification_kinds(&taro).await, vec!["chat.message"]);
    assert_eq!(ctx.unread(&taro).await.badge, 2);

    // Archive: out of the inbox, into the archive, with its own unread total.
    let archived: UpdatePreferencesResponse = ctx
        .call(
            "Conversation",
            "UpdatePreferences",
            &taro,
            &preferences(UpdatePreferencesRequest { archived: Some(true), ..Default::default() }),
        )
        .await
        .unwrap();
    assert!(archived.conversation.viewer.archived);
    assert!(ctx.list(&taro, ConversationFilter::CONVERSATION_FILTER_INBOX).await.conversations.is_empty());
    assert_eq!(
        ids(&ctx.list(&taro, ConversationFilter::CONVERSATION_FILTER_ARCHIVED).await.conversations),
        vec![conversation.id.clone()]
    );
    let unread = ctx.unread(&taro).await;
    assert_eq!((unread.badge, unread.archived_unread), (0, 2));
    // Pinning brings it back.
    let pinned: UpdatePreferencesResponse = ctx
        .call(
            "Conversation",
            "UpdatePreferences",
            &taro,
            &preferences(UpdatePreferencesRequest { pinned: Some(true), ..Default::default() }),
        )
        .await
        .unwrap();
    assert!(pinned.conversation.viewer.pinned && !pinned.conversation.viewer.archived);
}

#[tokio::test]
async fn at_most_twenty_conversations_can_be_pinned() {
    let ctx = start().await;
    let me = ctx.app.user("me").await;
    let mut conversation_ids = Vec::new();
    for index in 0..21 {
        let other = ctx.app.user(&format!("other{index}")).await;
        conversation_ids.push(ctx.chat.open_direct(me.id, other.id).await.unwrap().to_string());
    }
    let pin = |conversation_id: &str, pinned: bool| UpdatePreferencesRequest {
        conversation_id: conversation_id.to_owned(),
        pinned: Some(pinned),
        ..Default::default()
    };
    for conversation_id in &conversation_ids[..20] {
        let _: UpdatePreferencesResponse =
            ctx.call("Conversation", "UpdatePreferences", &me, &pin(conversation_id, true)).await.unwrap();
    }
    assert_eq!(
        ctx.reason("Conversation", "UpdatePreferences", &me, &pin(&conversation_ids[20], true)).await,
        "TOO_MANY_PINNED"
    );
    // Pinning a pinned one again is fine, and unpinning makes room.
    let _: UpdatePreferencesResponse =
        ctx.call("Conversation", "UpdatePreferences", &me, &pin(&conversation_ids[0], true)).await.unwrap();
    let _: UpdatePreferencesResponse =
        ctx.call("Conversation", "UpdatePreferences", &me, &pin(&conversation_ids[0], false)).await.unwrap();
    let _: UpdatePreferencesResponse =
        ctx.call("Conversation", "UpdatePreferences", &me, &pin(&conversation_ids[20], true)).await.unwrap();
}

#[tokio::test]
async fn hiding_takes_a_conversation_out_until_the_next_message() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let old = ctx.text(&hanako, &conversation.id, "古い話").await;

    let mut taro_events = ctx.listen(&taro).await;
    let mut hanako_events = ctx.listen(&hanako).await;
    let hide = HideConversationRequest { conversation_id: conversation.id.clone(), ..Default::default() };
    let _: HideConversationResponse = ctx.call("Conversation", "HideConversation", &taro, &hide).await.unwrap();
    assert!(matches!(
        ctx.events(&mut taro_events).await.as_slice(),
        [Push::ConversationRemoved(event)] if event.reason == ConversationRemovalReason::CONVERSATION_REMOVAL_REASON_HIDDEN
    ));
    assert!(ctx.events(&mut hanako_events).await.is_empty(), "hiding is nobody else's business");
    assert!(ctx.list(&taro, ConversationFilter::CONVERSATION_FILTER_INBOX).await.conversations.is_empty());
    assert_eq!(ctx.unread(&taro).await.badge, 0);
    assert_eq!(
        ids(&ctx.list(&hanako, ConversationFilter::CONVERSATION_FILTER_INBOX).await.conversations),
        vec![conversation.id.clone()]
    );

    // The next message brings it back, history included.
    ctx.text(&hanako, &conversation.id, "新しい話").await;
    let inbox = ctx.list(&taro, ConversationFilter::CONVERSATION_FILTER_INBOX).await;
    assert_eq!(ids(&inbox.conversations), vec![conversation.id.clone()]);
    assert_eq!(inbox.conversations[0].viewer.unread_count, 1);
    assert_eq!(ctx.latest(&taro, &conversation.id).await.len(), 2);

    // Clearing the history forgets everything so far, for the caller only.
    let clear =
        HideConversationRequest { conversation_id: conversation.id.clone(), clear_history: true, ..Default::default() };
    let _: HideConversationResponse = ctx.call("Conversation", "HideConversation", &taro, &clear).await.unwrap();
    assert!(ctx.latest(&taro, &conversation.id).await.is_empty());
    let gone = GetMessageRequest { message_id: old.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Message", "GetMessage", &taro, &gone).await, "MESSAGE_NOT_FOUND");
    assert_eq!(ctx.latest(&hanako, &conversation.id).await.len(), 2);
    assert!(ctx.conversation(&taro, &conversation.id).await.conversation.last_message.is_unset());
    let fresh = ctx.text(&hanako, &conversation.id, "まっさら").await;
    let visible = ctx.latest(&taro, &conversation.id).await;
    assert_eq!(visible.iter().map(|message| message.id.as_str()).collect::<Vec<_>>(), vec![fresh.id.as_str()]);
    assert_eq!(ctx.conversation(&taro, &conversation.id).await.conversation.viewer.unread_count, 1);
}

#[tokio::test]
async fn reading_moves_the_cursor_forward_and_clears_what_was_unread() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let first = ctx.text(&hanako, &conversation.id, "一").await;
    let second = ctx.text(&hanako, &conversation.id, "二").await;
    let third = ctx.text(&hanako, &conversation.id, "三").await;
    assert_eq!(ctx.unread(&taro).await.badge, 3);
    assert_eq!(ctx.unread(&hanako).await.badge, 0, "your own messages are never unread");

    // Read up to the second message.
    let mut hanako_events = ctx.listen(&hanako).await;
    let mut taro_events = ctx.listen(&taro).await;
    let up_to = |message: &Message| MarkReadRequest {
        conversation_id: conversation.id.clone(),
        message_id: message.id.clone(),
        ..Default::default()
    };
    let read: MarkReadResponse = ctx.call("Conversation", "MarkRead", &taro, &up_to(&second)).await.unwrap();
    assert_eq!((read.read_seq, read.unread_count, read.unread.badge), (second.seq, 1, 1));
    for events in [ctx.events(&mut hanako_events).await, ctx.events(&mut taro_events).await] {
        assert!(
            matches!(events.as_slice(), [Push::ReadReceipt(receipt)] if receipt.user_id == taro.id.to_string() && receipt.read_seq == second.seq),
            "the sender and the reader's other devices learn about it"
        );
    }

    // The cursor never moves back.
    let back: MarkReadResponse = ctx.call("Conversation", "MarkRead", &taro, &up_to(&first)).await.unwrap();
    assert_eq!((back.read_seq, back.unread_count), (second.seq, 1));
    let all = ctx.mark_read(&taro, &conversation.id).await;
    assert_eq!((all.read_seq, all.unread_count, all.unread.badge), (third.seq, 0, 0));

    // What the others see: cursors on the chat screen, readers of one message.
    let screen = ctx.conversation(&hanako, &conversation.id).await;
    let cursor = screen.read_cursors.iter().find(|cursor| cursor.user_id == taro.id.to_string()).unwrap();
    assert_eq!(cursor.read_seq, third.seq);
    assert!(cursor.read_at.is_set());
    assert_eq!(ctx.message(&hanako, &third.id).await.read_count, 1);
    let receipts: ListReadReceiptsResponse = ctx
        .call(
            "Message",
            "ListReadReceipts",
            &hanako,
            &ListReadReceiptsRequest { message_id: third.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(receipts.receipts.len(), 1);
    assert_eq!(receipts.receipts[0].user.id, taro.id.to_string());
    assert_eq!(receipts.unread_member_count, 0);

    // A message of another conversation is not a position in this one.
    let (other, friend) = ctx.friends("other", "friend").await;
    let elsewhere = ctx.direct(&other, &friend).await;
    let foreign = ctx.text(&other, &elsewhere.id, "別の話").await;
    assert_eq!(ctx.reason("Conversation", "MarkRead", &taro, &up_to(&foreign)).await, "MESSAGE_NOT_FOUND");
}

#[tokio::test]
async fn groups_count_readers() {
    let ctx = start().await;
    let (group, owner, members) = ctx.group_of("部活", 3).await;
    let message = ctx.text(&owner, &group.id, "集合！").await;
    assert_eq!(message.read_count, 0);
    ctx.mark_read(&members[0], &group.id).await;
    ctx.mark_read(&members[1], &group.id).await;

    assert_eq!(ctx.message(&owner, &message.id).await.read_count, 2);
    let receipts: ListReadReceiptsResponse = ctx
        .call(
            "Message",
            "ListReadReceipts",
            &owner,
            &ListReadReceiptsRequest { message_id: message.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let readers: Vec<&str> = receipts.receipts.iter().map(|receipt| receipt.user.id.as_str()).collect();
    assert_eq!(
        readers,
        vec![members[1].id.to_string().as_str(), members[0].id.to_string().as_str()],
        "most recent reader first"
    );
    assert_eq!(receipts.unread_member_count, 1);
    let outsider = ctx.app.user("outsider").await;
    let request = ListReadReceiptsRequest { message_id: message.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Message", "ListReadReceipts", &outsider, &request).await, "MESSAGE_NOT_FOUND");
}

#[tokio::test]
async fn the_server_can_open_conversations_and_post_in_them() {
    let ctx = start().await;
    let (shop, customer) = (ctx.app.user("shop").await, ctx.app.user("customer").await);

    // No friendship and no privacy check: both simply have a conversation.
    let conversation_id = ctx.chat.open_direct(shop.id, customer.id).await.unwrap();
    assert_eq!(ctx.chat.open_direct(customer.id, shop.id).await.unwrap(), conversation_id, "one conversation per pair");
    let view = ctx.conversation(&customer, &conversation_id.to_string()).await;
    assert_eq!(view.conversation.viewer.request_state, RequestState::REQUEST_STATE_ACCEPTED);

    let mut customer_events = ctx.listen(&customer).await;
    let message_id = ctx.chat.post_text(conversation_id, shop.id, "ご購入ありがとうございます！").await.unwrap();
    let received = created_messages(&ctx.events(&mut customer_events).await);
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].id, message_id.to_string());
    assert_eq!(received[0].sender.id, shop.id.to_string());
    assert_eq!(ctx.notification_kinds(&customer).await, vec!["chat.message"]);
    assert_eq!(ctx.unread(&customer).await.badge, 1);

    // The sender must be a member, and there must be something to say.
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(
        ctx.chat.post_text(conversation_id, outsider.id, "hi").await.unwrap_err().reason,
        "CONVERSATION_NOT_FOUND"
    );
    assert_eq!(ctx.chat.post_text(conversation_id, shop.id, "   ").await.unwrap_err().reason, "MESSAGE_EMPTY");
    assert_eq!(ctx.chat.open_direct(shop.id, shop.id).await.unwrap_err().reason, "CANNOT_MESSAGE_SELF");
    assert_eq!(ctx.chat.open_direct(shop.id, Uuid::now_v7()).await.unwrap_err().reason, "USER_NOT_FOUND");

    // A request that was waiting is settled by the server's say-so.
    let stranger = ctx.app.user("stranger").await;
    ctx.open_to_everyone(&customer).await;
    let requested = ctx.direct(&stranger, &customer).await;
    assert_eq!(ctx.chat.open_direct(customer.id, stranger.id).await.unwrap().to_string(), requested.id);
    assert_eq!(
        ctx.conversation(&customer, &requested.id).await.conversation.viewer.request_state,
        RequestState::REQUEST_STATE_ACCEPTED
    );
}
