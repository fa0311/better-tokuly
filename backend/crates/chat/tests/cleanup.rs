//! What the chat domain does when other domains report something: deleted
//! accounts, blocks, new friendships, uploads finishing or disappearing,
//! moderation. Also the subject resolver, the data export and the sweeps.
//!
//! No event relay runs in tests: events are delivered by calling
//! `Chat::handle_event` with a constructed event.
mod common;

use common::*;
use tokuly_core::events::Event;
use tokuly_proto::{
    chat::{chat_event::Event as Push, *},
    common::SubjectType,
    media::{AssetKind, AssetPurpose, AssetStatus},
};
use uuid::Uuid;

fn event(topic: &str, data: serde_json::Value) -> Event {
    Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id: None, occurred_at: chrono::Utc::now(), data }
}

fn media_request(conversation_id: &str, asset_ids: &[&str], text: &str) -> SendMessageRequest {
    SendMessageRequest {
        conversation_id: conversation_id.to_owned(),
        client_id: client_id(),
        text: text.to_owned(),
        attachment_asset_ids: asset_ids.iter().map(|id| id.to_string()).collect(),
        ..Default::default()
    }
}

async fn set_asset_status(ctx: &Ctx, asset_id: &str, status: AssetStatus) {
    sqlx::query("UPDATE media_assets SET status = $2, ready_at = CASE WHEN $2 = 3 THEN now() END WHERE id = $1::uuid")
        .bind(asset_id)
        .bind(status as i32)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_deleted_account_leaves_its_messages_behind() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let jiro = ctx.app.user("jiro").await;
    ctx.befriend(&taro, &jiro).await;
    let direct = ctx.direct(&taro, &hanako).await;
    let from_taro = ctx.text(&taro, &direct.id, "また明日ね").await;
    let from_hanako = ctx.text(&hanako, &direct.id, "うん、また明日").await;
    let react = AddReactionRequest { message_id: from_hanako.id.clone(), emoji: "👍".into(), ..Default::default() };
    let _: AddReactionResponse = ctx.call("Message", "AddReaction", &taro, &react).await.unwrap();
    let group = ctx.group(&taro, "三人", &[&hanako, &jiro]).await;
    ctx.text(&taro, &group.id, "グループでも").await;

    // What identity does when the grace period is over: anonymise, then announce.
    sqlx::query("UPDATE users SET deleted_at = now(), display_name = '' WHERE id = $1")
        .bind(taro.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let deleted = event("identity.user.deleted", serde_json::json!({ "user_id": taro.id }));
    let mut hanako_events = ctx.listen(&hanako).await;
    ctx.chat.handle_event(&deleted).await.unwrap();

    // The direct conversation stays readable but nobody can write in it.
    let view = ctx.conversation(&hanako, &direct.id).await;
    assert!(view.conversation.read_only);
    assert_eq!(view.conversation.member_count, 1);
    assert!(view.conversation.members.iter().any(|member| member.deleted), "the other side shows as a deleted user");
    let messages = ctx.latest(&hanako, &direct.id).await;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].id, from_taro.id);
    assert!(
        messages[0].sender.deleted && messages[0].text == "また明日ね",
        "the message stays, its sender is a placeholder"
    );
    assert!(messages[1].reactions.is_empty(), "the account's reactions are gone");
    let send = SendMessageRequest {
        conversation_id: direct.id.clone(),
        client_id: client_id(),
        text: "いる？".into(),
        ..Default::default()
    };
    assert_eq!(ctx.send(&hanako, &send).await.unwrap_err().reason, "CONVERSATION_READ_ONLY");
    let events = ctx.events(&mut hanako_events).await;
    assert!(events.iter().any(|event| matches!(event, Push::ConversationUpdated(updated) if updated.conversation.id == direct.id && updated.conversation.read_only)));

    // The group lost a member and found a new owner.
    let view = ctx.conversation(&hanako, &group.id).await;
    assert_eq!(view.conversation.member_count, 2);
    assert_eq!(view.conversation.viewer.role, MemberRole::MEMBER_ROLE_OWNER);
    let story: Vec<SystemEventType> = ctx
        .latest(&jiro, &group.id)
        .await
        .iter()
        .filter(|message| message.kind == MessageKind::MESSAGE_KIND_SYSTEM)
        .map(|message| message.system.r#type.as_known().unwrap())
        .collect();
    assert_eq!(
        story,
        vec![
            SystemEventType::SYSTEM_EVENT_TYPE_GROUP_CREATED,
            SystemEventType::SYSTEM_EVENT_TYPE_MEMBER_LEFT,
            SystemEventType::SYSTEM_EVENT_TYPE_OWNER_CHANGED
        ]
    );
    let memberships: i64 = sqlx::query_scalar("SELECT count(*) FROM chat_members WHERE user_id = $1")
        .bind(taro.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(memberships, 0);

    // Delivered again (at-least-once): nothing more happens.
    ctx.chat.handle_event(&deleted).await.unwrap();
    assert_eq!(ctx.conversation(&hanako, &group.id).await.conversation.member_count, 2);
    assert_eq!(ctx.latest(&hanako, &direct.id).await.len(), 2);
}

#[tokio::test]
async fn a_block_closes_a_direct_conversation_and_unblocking_reopens_it() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    ctx.text(&taro, &direct.id, "やあ").await;
    let asked: RequestPovResponse = ctx
        .call(
            "Pov",
            "RequestPov",
            &taro,
            &RequestPovRequest { conversation_id: direct.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();

    ctx.block(&hanako, &taro).await;
    let mut taro_events = ctx.listen(&taro).await;
    let blocked = event("social.user.blocked", serde_json::json!({ "user_id": hanako.id, "blocked_id": taro.id }));
    ctx.chat.handle_event(&blocked).await.unwrap();
    for viewer in [&taro, &hanako] {
        assert!(ctx.conversation(viewer, &direct.id).await.conversation.read_only, "read-only for both");
    }
    let events = ctx.events(&mut taro_events).await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Push::ConversationUpdated(updated) if updated.conversation.read_only))
    );
    // What was pending between the two is closed.
    assert_eq!(ctx.message(&taro, &asked.message.id).await.pov_request.state, PovState::POV_STATE_CANCELLED);
    assert!(updated_messages(&events).iter().any(|(message, _)| message.id == asked.message.id));
    ctx.chat.handle_event(&blocked).await.unwrap();

    // Conversations with other people are untouched.
    let jiro = ctx.app.user("jiro").await;
    ctx.befriend(&taro, &jiro).await;
    let other = ctx.direct(&taro, &jiro).await;
    assert!(!ctx.conversation(&taro, &other.id).await.conversation.read_only);

    ctx.unblock(&hanako, &taro).await;
    ctx.chat
        .handle_event(&event(
            "social.user.unblocked",
            serde_json::json!({ "user_id": hanako.id, "blocked_id": taro.id }),
        ))
        .await
        .unwrap();
    assert!(!ctx.conversation(&taro, &direct.id).await.conversation.read_only);
    ctx.text(&taro, &direct.id, "また話せるね").await;

    // A block between people who never talked changes nothing.
    let stranger = ctx.app.user("stranger").await;
    ctx.chat
        .handle_event(&event(
            "social.user.blocked",
            serde_json::json!({ "user_id": stranger.id, "blocked_id": taro.id }),
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn becoming_friends_settles_a_message_request() {
    let ctx = start().await;
    let stranger = ctx.app.user("stranger").await;
    let hanako = ctx.app.user("hanako").await;
    ctx.open_to_everyone(&hanako).await;
    let request = ctx.direct(&stranger, &hanako).await;
    ctx.text(&stranger, &request.id, "はじめまして").await;

    ctx.befriend(&stranger, &hanako).await;
    let mut hanako_events = ctx.listen(&hanako).await;
    let mut stranger_events = ctx.listen(&stranger).await;
    let friends = event(
        "social.friend.accepted",
        serde_json::json!({ "user_id": hanako.id, "friend_id": stranger.id, "source": 0 }),
    );
    ctx.chat.handle_event(&friends).await.unwrap();
    assert_eq!(
        ctx.conversation(&hanako, &request.id).await.conversation.viewer.request_state,
        RequestState::REQUEST_STATE_ACCEPTED
    );
    assert!(ctx
        .events(&mut hanako_events)
        .await
        .iter()
        .any(|event| matches!(event, Push::ConversationUpdated(updated) if updated.conversation.viewer.request_state == RequestState::REQUEST_STATE_ACCEPTED)));
    assert!(ctx
        .events(&mut stranger_events)
        .await
        .iter()
        .any(|event| matches!(event, Push::ConversationUpdated(updated) if updated.conversation.viewer.request_messages_left == -1)));
    ctx.chat.handle_event(&friends).await.unwrap();

    // Even before that event arrives, friends are not held to a request's limits.
    let late = ctx.app.user("late").await;
    let request = ctx.direct(&late, &hanako).await;
    ctx.befriend(&late, &hanako).await;
    for index in 0..5 {
        ctx.text(&late, &request.id, &format!("{index}通目")).await;
    }
    assert_eq!(
        ctx.conversation(&hanako, &request.id).await.conversation.viewer.request_state,
        RequestState::REQUEST_STATE_ACCEPTED
    );
}

#[tokio::test]
async fn a_video_gets_its_final_shape_when_processing_ends() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let video = ctx.upload(&taro, AssetPurpose::ASSET_PURPOSE_CHAT_VIDEO, AssetKind::ASSET_KIND_VIDEO).await;
    set_asset_status(&ctx, &video, AssetStatus::ASSET_STATUS_PROCESSING).await;
    let message = ctx.send(&taro, &media_request(&direct.id, &[&video], "")).await.unwrap();
    let message_id = message.message.id.clone();
    assert_eq!(message.message.attachments[0].status, AssetStatus::ASSET_STATUS_PROCESSING);

    set_asset_status(&ctx, &video, AssetStatus::ASSET_STATUS_READY).await;
    let mut hanako_events = ctx.listen(&hanako).await;
    let ready = event(
        "media.asset.ready",
        serde_json::json!({ "asset_id": video, "owner_id": taro.id, "purpose": 5, "kind": 2 }),
    );
    ctx.chat.handle_event(&ready).await.unwrap();
    let updates = updated_messages(&ctx.events(&mut hanako_events).await);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].1, MessageUpdateReason::MESSAGE_UPDATE_REASON_ATTACHMENT);
    assert_eq!(updates[0].0.attachments[0].status, AssetStatus::ASSET_STATUS_READY);
    assert_eq!(ctx.message(&hanako, &message_id).await.attachments[0].status, AssetStatus::ASSET_STATUS_READY);
    // Delivered twice: nothing changed, nothing is pushed.
    ctx.chat.handle_event(&ready).await.unwrap();
    assert!(ctx.events(&mut hanako_events).await.is_empty());

    // A failed upload shows as failed.
    let broken = ctx.upload(&taro, AssetPurpose::ASSET_PURPOSE_CHAT_VIDEO, AssetKind::ASSET_KIND_VIDEO).await;
    set_asset_status(&ctx, &broken, AssetStatus::ASSET_STATUS_PROCESSING).await;
    let message = ctx.send(&taro, &media_request(&direct.id, &[&broken], "")).await.unwrap();
    set_asset_status(&ctx, &broken, AssetStatus::ASSET_STATUS_FAILED).await;
    ctx.chat.handle_event(&event("media.asset.failed", serde_json::json!({ "asset_id": broken, "owner_id": taro.id, "purpose": 5, "kind": 2, "error": "CORRUPT_MEDIA" }))).await.unwrap();
    assert_eq!(ctx.message(&hanako, &message.message.id).await.attachments[0].status, AssetStatus::ASSET_STATUS_FAILED);

    // Uploads no message shows are none of chat's business.
    ctx.chat
        .handle_event(&event("media.asset.ready", serde_json::json!({ "asset_id": Uuid::now_v7() })))
        .await
        .unwrap();
}

#[tokio::test]
async fn an_upload_taken_down_leaves_the_messages_showing_it() {
    let ctx = start().await;
    let (group, owner, members) = ctx.group_of("部活", 1).await;
    let member = &members[0];
    let (kept, removed) = (ctx.photo(&owner).await, ctx.photo(&owner).await);
    let both = ctx.send(&owner, &media_request(&group.id, &[&kept, &removed], "二枚")).await.unwrap();
    let alone = ctx.send(&owner, &media_request(&group.id, &[&removed], "")).await.unwrap();
    let captioned = ctx.send(&owner, &media_request(&group.id, &[&removed], "説明つき")).await.unwrap();
    // The same upload is also the group's icon.
    let icon = ctx.upload(&owner, AssetPurpose::ASSET_PURPOSE_AVATAR, AssetKind::ASSET_KIND_IMAGE).await;
    let with_icon = UpdateGroupRequest {
        conversation_id: group.id.clone(),
        icon_asset_id: Some(icon.clone()),
        ..Default::default()
    };
    let _: UpdateGroupResponse = ctx.call("Conversation", "UpdateGroup", &owner, &with_icon).await.unwrap();

    // A takedown: media deletes the upload whatever holds it, then announces it.
    for asset in [&removed, &icon] {
        assert!(tokuly_media::assets::delete(&ctx.app.core, asset.parse().unwrap(), None).await.unwrap());
    }
    let mut member_events = ctx.listen(member).await;
    ctx.chat.handle_event(&event("media.asset.deleted", serde_json::json!({ "asset_id": removed }))).await.unwrap();
    ctx.chat.handle_event(&event("media.asset.deleted", serde_json::json!({ "asset_id": icon }))).await.unwrap();
    let events = ctx.events(&mut member_events).await;

    let after = ctx.message(member, &both.message.id).await;
    assert_eq!(
        after.attachments.iter().map(|attachment| attachment.asset_id.as_str()).collect::<Vec<_>>(),
        vec![kept.as_str()]
    );
    let emptied = ctx.message(member, &alone.message.id).await;
    assert!(emptied.deleted, "a media message with nothing left is a tombstone");
    let caption_only = ctx.message(member, &captioned.message.id).await;
    assert!(!caption_only.deleted && caption_only.attachments.is_empty() && caption_only.text == "説明つき");
    let reasons: Vec<MessageUpdateReason> = updated_messages(&events).into_iter().map(|(_, reason)| reason).collect();
    assert_eq!(
        reasons,
        vec![
            MessageUpdateReason::MESSAGE_UPDATE_REASON_ATTACHMENT,
            MessageUpdateReason::MESSAGE_UPDATE_REASON_DELETED,
            MessageUpdateReason::MESSAGE_UPDATE_REASON_ATTACHMENT
        ]
    );
    assert!(ctx.conversation(member, &group.id).await.conversation.icon.is_unset());
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Push::ConversationUpdated(updated) if updated.conversation.icon.is_unset()))
    );
    let media: ListSharedMediaResponse = ctx
        .call(
            "Message",
            "ListSharedMedia",
            member,
            &ListSharedMediaRequest { conversation_id: group.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(media.items.len(), 1);
}

#[tokio::test]
async fn moderation_takes_a_message_down() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let message = ctx.text(&taro, &direct.id, "規約違反の内容").await;
    let moderator = ctx.app.user_with_roles("moderator", &["moderator"]).await;
    let removal = |subject_type: SubjectType, subject_id: &str| {
        event(
            "admin.content.removed",
            serde_json::json!({ "subject_type": subject_type as i32, "subject_id": subject_id, "reason": "ハラスメント", "moderator_id": moderator.id }),
        )
    };

    // Other kinds of content, and messages that do not exist, are ignored.
    ctx.chat.handle_event(&removal(SubjectType::SUBJECT_TYPE_NOTE, &message.id)).await.unwrap();
    ctx.chat.handle_event(&removal(SubjectType::SUBJECT_TYPE_CHAT_MESSAGE, &Uuid::now_v7().to_string())).await.unwrap();
    assert!(!ctx.message(&hanako, &message.id).await.deleted);

    let mut hanako_events = ctx.listen(&hanako).await;
    ctx.chat.handle_event(&removal(SubjectType::SUBJECT_TYPE_CHAT_MESSAGE, &message.id)).await.unwrap();
    let after = ctx.message(&hanako, &message.id).await;
    assert!(after.deleted && after.text.is_empty());
    let updates = updated_messages(&ctx.events(&mut hanako_events).await);
    assert_eq!(updates[0].1, MessageUpdateReason::MESSAGE_UPDATE_REASON_DELETED);
    let events = ctx.app.events_by("chat.message.deleted", moderator.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["moderated"], true);
    assert_eq!(ctx.unread(&hanako).await.badge, 0, "a removed message is nothing to read");
    ctx.chat.handle_event(&removal(SubjectType::SUBJECT_TYPE_CHAT_MESSAGE, &message.id)).await.unwrap();
}

#[tokio::test]
async fn messages_are_subjects_for_the_members_who_can_see_them() {
    let ctx = start().await;
    let (group, owner, members) = ctx.group_of("部活", 1).await;
    let member = &members[0];
    let message = ctx.text(&owner, &group.id, "これは通報されるかもしれないメッセージ").await;
    let subjects = &ctx.app.core.subjects;
    let chat_message = SubjectType::SUBJECT_TYPE_CHAT_MESSAGE;

    // What a report about the message is filed with.
    let info = subjects.resolve(chat_message, &message.id, Some(member.id)).await.unwrap();
    assert_eq!(info.id.to_string(), message.id);
    assert_eq!(info.owner_id, owner.id);
    assert_eq!(info.title, "これは通報されるかもしれないメッセージ");
    assert_eq!(info.path, format!("/chat/{}?message={}", group.id, message.id));
    assert!(!info.interactions_enabled, "messages are reported, not liked or commented on");

    // Nobody else can even tell that it exists.
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(
        subjects.resolve(chat_message, &message.id, Some(outsider.id)).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND"
    );
    assert_eq!(subjects.resolve(chat_message, &message.id, None).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");
    assert_eq!(
        subjects.resolve(chat_message, "not-an-id", Some(member.id)).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND"
    );
    let newcomer = ctx.app.user("newcomer").await;
    ctx.befriend(&owner, &newcomer).await;
    let add = AddMembersRequest {
        conversation_id: group.id.clone(),
        user_ids: vec![newcomer.id.to_string()],
        ..Default::default()
    };
    let _: AddMembersResponse = ctx.call("Conversation", "AddMembers", &owner, &add).await.unwrap();
    assert_eq!(
        subjects.resolve(chat_message, &message.id, Some(newcomer.id)).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND",
        "from before they joined"
    );

    // A photo message is described and illustrated; a deleted one is gone.
    let photo = ctx.photo(&owner).await;
    let with_photo = ctx.send(&owner, &media_request(&group.id, &[&photo], "")).await.unwrap();
    let info = subjects.resolve(chat_message, &with_photo.message.id, Some(member.id)).await.unwrap();
    assert_eq!(info.title, "📷 写真を送信しました");
    assert!(info.image_url.is_some());
    let _: DeleteMessageResponse = ctx
        .call(
            "Message",
            "DeleteMessage",
            &owner,
            &DeleteMessageRequest { message_id: with_photo.message.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let info = subjects.resolve(chat_message, &with_photo.message.id, Some(member.id)).await.unwrap();
    assert_eq!(info.title, "📷 写真を送信しました");
    assert!(info.image_url.is_none(), "the upload itself is released");
}

#[tokio::test]
async fn what_was_unsent_or_edited_can_still_be_reported() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let subjects = &ctx.app.core.subjects;
    let chat_message = SubjectType::SUBJECT_TYPE_CHAT_MESSAGE;
    let delete = |message_id: &str| DeleteMessageRequest { message_id: message_id.to_owned(), ..Default::default() };

    // Unsending does not take away the recipient's way to report what they received.
    let insult = ctx.text(&taro, &direct.id, "おまえなんか大嫌いだ").await;
    let _: DeleteMessageResponse = ctx.call("Message", "DeleteMessage", &taro, &delete(&insult.id)).await.unwrap();
    assert!(ctx.message(&hanako, &insult.id).await.text.is_empty(), "gone from the conversation");
    let info = subjects.resolve(chat_message, &insult.id, Some(hanako.id)).await.unwrap();
    assert_eq!((info.owner_id, info.title.as_str()), (taro.id, "おまえなんか大嫌いだ"));
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(
        subjects.resolve(chat_message, &insult.id, Some(outsider.id)).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND",
        "only for the people it reached"
    );

    // Nor does editing it into something harmless.
    let edited = ctx.text(&taro, &direct.id, "ばか！死ね").await;
    let edit = EditMessageRequest { message_id: edited.id.clone(), text: "なんでもない".into(), ..Default::default() };
    let _: EditMessageResponse = ctx.call("Message", "EditMessage", &taro, &edit).await.unwrap();
    let info = subjects.resolve(chat_message, &edited.id, Some(hanako.id)).await.unwrap();
    assert!(info.title.contains("ばか！死ね") && info.title.contains("なんでもない"), "{}", info.title);

    // What was kept for reports is not kept forever, counted from the removal.
    sqlx::query("UPDATE chat_messages SET created_at = now() - interval '90 days' WHERE id = $1::uuid")
        .bind(&insult.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(subjects.resolve(chat_message, &insult.id, Some(hanako.id)).await.is_ok(), "removed just now");
    sqlx::query("UPDATE chat_removed_texts SET removed_at = now() - interval '31 days' WHERE message_id = $1::uuid")
        .bind(&insult.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(
        subjects.resolve(chat_message, &insult.id, Some(hanako.id)).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND"
    );
    ctx.chat.purge_removed_texts().await.unwrap();
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM chat_removed_texts WHERE message_id = $1::uuid")
        .bind(&insult.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(kept, 0);
}

#[tokio::test]
async fn the_data_export_has_a_chat_section() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    ctx.text(&taro, &direct.id, "自分のメッセージ").await;
    let theirs = ctx.text(&hanako, &direct.id, "相手のメッセージ").await;
    let react = AddReactionRequest { message_id: theirs.id.clone(), emoji: "❤️".into(), ..Default::default() };
    let _: AddReactionResponse = ctx.call("Message", "AddReaction", &taro, &react).await.unwrap();
    let _: RequestPovResponse = ctx
        .call(
            "Pov",
            "RequestPov",
            &taro,
            &RequestPovRequest { conversation_id: direct.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();

    let exporter =
        ctx.app.core.exporters.all().into_iter().find(|exporter| exporter.name() == "chat").expect("a chat exporter");
    let export = exporter.export(taro.id).await.unwrap();
    assert_eq!(export["conversations"].as_array().unwrap().len(), 1);
    assert_eq!(export["conversations"][0]["id"], direct.id);
    let messages = export["messages"].as_array().unwrap();
    let texts: Vec<&str> =
        messages.iter().filter_map(|message| message["text"].as_str()).filter(|text| !text.is_empty()).collect();
    assert_eq!(texts, vec!["自分のメッセージ"], "only what the user wrote themselves");
    assert_eq!(export["reactions"][0]["emoji"], "❤️");
    assert_eq!(export["pov_requests"][0]["requested_by_me"], true);
    assert_eq!(export["calls"], serde_json::json!([]));
}

#[tokio::test]
async fn link_previews_whose_job_was_lost_are_queued_again() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let recent = ctx.text(&taro, &direct.id, "http://localhost:8080/watch/aaaaaaaaaaa").await;
    let waiting = ctx.text(&taro, &direct.id, "http://localhost:8080/watch/bbbbbbbbbbb").await;
    let ancient = ctx.text(&taro, &direct.id, "http://localhost:8080/watch/ccccccccccc").await;
    let age = |message_id: String, interval: &'static str| {
        let db = ctx.app.core.db.clone();
        async move {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "UPDATE chat_messages SET created_at = now() - interval '{interval}' WHERE id = $1::uuid"
            )))
            .bind(message_id)
            .execute(&db)
            .await
            .unwrap();
        }
    };
    age(waiting.id.clone(), "5 minutes").await;
    age(ancient.id.clone(), "2 days").await;

    // Other tests share the database: at least the one waiting here is queued.
    assert!(ctx.chat.sweep_link_previews().await.unwrap() >= 1);
    let checked = |message_id: String| {
        let db = ctx.app.core.db.clone();
        async move {
            sqlx::query_scalar::<_, bool>("SELECT link_checked_at IS NOT NULL FROM chat_messages WHERE id = $1::uuid")
                .bind(message_id)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    assert!(checked(ancient.id.clone()).await, "links nobody looked up for a day are given up on");
    assert!(!checked(waiting.id.clone()).await && !checked(recent.id.clone()).await, "the job does the looking up");
    ctx.chat.resolve_link_preview(waiting.id.parse().unwrap()).await.unwrap();
    assert!(checked(waiting.id.clone()).await);
    assert_eq!(ctx.message(&hanako, &waiting.id).await.link_preview.embed_id, "video/bbbbbbbbbbb");
    // A message without a link, or one that does not exist, is nothing to do.
    let plain = ctx.text(&taro, &direct.id, "リンクなし").await;
    ctx.chat.resolve_link_preview(plain.id.parse().unwrap()).await.unwrap();
    ctx.chat.resolve_link_preview(Uuid::now_v7()).await.unwrap();
}
