//! `MessageService`: sending every kind of message, editing, deleting,
//! paging, searching, forwarding, reactions, pins and the media gallery.
mod common;

use common::*;
use tokuly_proto::{
    chat::{chat_event::Event as Push, list_messages_request::Position, *},
    common::{LatLng, PageRequest},
    media::{AssetKind, AssetPurpose, AssetStatus},
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

fn text_request(conversation_id: &str, text: &str) -> SendMessageRequest {
    SendMessageRequest {
        conversation_id: conversation_id.to_owned(),
        client_id: client_id(),
        text: text.to_owned(),
        ..Default::default()
    }
}

fn media_request(conversation_id: &str, asset_ids: &[&str]) -> SendMessageRequest {
    SendMessageRequest {
        conversation_id: conversation_id.to_owned(),
        client_id: client_id(),
        attachment_asset_ids: asset_ids.iter().map(|id| id.to_string()).collect(),
        ..Default::default()
    }
}

fn by_id(message_id: &str) -> GetMessageRequest {
    GetMessageRequest { message_id: message_id.to_owned(), ..Default::default() }
}

fn texts(messages: &[Message]) -> Vec<&str> {
    messages.iter().map(|message| message.text.as_str()).collect()
}

async fn sent(ctx: &Ctx, sender: &TestUser, request: &SendMessageRequest) -> Message {
    ctx.send(sender, request).await.expect("send").message.as_option().expect("message").clone()
}

#[tokio::test]
async fn a_text_message_reaches_every_device_and_notifies_the_other_side() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let mut taro_events = ctx.listen(&taro).await;
    let mut hanako_events = ctx.listen(&hanako).await;

    let request = text_request(&conversation.id, "  やっほー！\r\n元気？ ");
    let response = ctx.send(&taro, &request).await.unwrap();
    let message = response.message.as_option().unwrap();
    assert!(!response.duplicate);
    assert_eq!(message.kind, MessageKind::MESSAGE_KIND_TEXT);
    assert_eq!(message.text, "やっほー！\n元気？");
    assert_eq!(message.seq, 1);
    assert_eq!(message.sender.id, taro.id.to_string());
    assert_eq!(message.client_id, request.client_id);
    assert_eq!(message.conversation_id, conversation.id);
    assert!(message.created_at.is_set() && message.edited_at.is_unset() && !message.deleted);

    // The sender's other devices and the recipient both get the message.
    for events in [ctx.events(&mut taro_events).await, ctx.events(&mut hanako_events).await] {
        let created = created_messages(&events);
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].id, message.id);
        assert_eq!(created[0].client_id, request.client_id, "lets the sender's devices match their optimistic copy");
    }

    // One push for the recipient, collapsed per conversation, not kept in the inbox.
    let notifications = ctx.notifications(&hanako).await;
    assert_eq!(notifications.len(), 1);
    let notification = &notifications[0];
    assert_eq!(notification.kind, "chat.message");
    assert_eq!(notification.title.ja, "taro");
    assert_eq!(notification.body.ja, "やっほー！ 元気？");
    assert_eq!(notification.path, format!("/chat/{}", conversation.id));
    assert_eq!(notification.collapse_key.as_deref(), Some(format!("chat:{}", conversation.id).as_str()));
    assert_eq!(notification.actor_id, Some(taro.id));
    assert!(notification.push && !notification.inbox && !notification.urgent);
    assert!(ctx.notifications(&taro).await.is_empty(), "nobody is notified of their own message");

    // A retry with the same client_id sends nothing new.
    let mut hanako_events = ctx.listen(&hanako).await;
    let retried = ctx.send(&taro, &request).await.unwrap();
    assert!(retried.duplicate);
    assert_eq!(retried.message.id, message.id);
    assert!(ctx.events(&mut hanako_events).await.is_empty());
    assert_eq!(ctx.latest(&hanako, &conversation.id).await.len(), 1);
    assert_eq!(ctx.notifications(&hanako).await.len(), 1);
    // The same key in another conversation is another message.
    let (jiro, _) = (ctx.app.user("jiro").await, ());
    ctx.befriend(&taro, &jiro).await;
    let elsewhere = ctx.direct(&taro, &jiro).await;
    let reused = SendMessageRequest { conversation_id: elsewhere.id.clone(), ..request.clone() };
    assert!(!ctx.send(&taro, &reused).await.unwrap().duplicate);
}

#[tokio::test]
async fn message_content_is_validated() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let id = conversation.id.clone();
    let reason = |request: SendMessageRequest| {
        let ctx = &ctx;
        let taro = &taro;
        async move { ctx.send(taro, &request).await.unwrap_err().reason }
    };

    assert_eq!(reason(text_request(&id, "   ")).await, "MESSAGE_EMPTY");
    assert_eq!(reason(text_request(&id, &"あ".repeat(4001))).await, "TEXT_TOO_LONG");
    assert_eq!(
        reason(SendMessageRequest { client_id: String::new(), ..text_request(&id, "hi") }).await,
        "CLIENT_ID_INVALID"
    );
    assert_eq!(
        reason(SendMessageRequest { client_id: "x".repeat(65), ..text_request(&id, "hi") }).await,
        "CLIENT_ID_INVALID"
    );
    assert_eq!(
        reason(SendMessageRequest { conversation_id: "nope".into(), ..text_request(&id, "hi") }).await,
        "INVALID_ID"
    );

    // One kind of content per message.
    let stamp = ctx.stamp(&taro).await;
    assert_eq!(
        reason(SendMessageRequest { stamp_id: stamp.clone(), ..text_request(&id, "スタンプと文字") }).await,
        "MESSAGE_CONTENT_CONFLICT"
    );
    let photo = ctx.photo(&taro).await;
    let stamp_and_photo = SendMessageRequest { stamp_id: stamp, ..media_request(&id, &[&photo]) };
    assert_eq!(reason(stamp_and_photo).await, "MESSAGE_CONTENT_CONFLICT");
    let place =
        LocationShare { position: LatLng { lat: 35.0, lng: 139.0, ..Default::default() }.into(), ..Default::default() };
    assert_eq!(
        reason(SendMessageRequest { location: place.into(), ..text_request(&id, "ここ") }).await,
        "MESSAGE_CONTENT_CONFLICT"
    );

    // Stamps, uploads, places and answered messages must be real and usable.
    assert_eq!(
        reason(SendMessageRequest { stamp_id: Uuid::now_v7().to_string(), ..text_request(&id, "") }).await,
        "STAMP_NOT_USABLE"
    );
    let hanakos_stamp = ctx.stamp(&hanako).await;
    assert_eq!(
        reason(SendMessageRequest { stamp_id: hanakos_stamp, ..text_request(&id, "") }).await,
        "STAMP_NOT_USABLE"
    );
    assert_eq!(reason(media_request(&id, &[&Uuid::now_v7().to_string()])).await, "ASSET_NOT_FOUND");
    let hanakos_photo = ctx.photo(&hanako).await;
    assert_eq!(reason(media_request(&id, &[&hanakos_photo])).await, "ASSET_NOT_FOUND");
    let avatar = ctx.upload(&taro, AssetPurpose::ASSET_PURPOSE_AVATAR, AssetKind::ASSET_KIND_IMAGE).await;
    assert_eq!(reason(media_request(&id, &[&avatar])).await, "ASSET_WRONG_PURPOSE");
    let eleven: Vec<String> = (0..11).map(|_| Uuid::now_v7().to_string()).collect();
    assert_eq!(
        reason(media_request(&id, &eleven.iter().map(String::as_str).collect::<Vec<_>>())).await,
        "TOO_MANY_ATTACHMENTS"
    );
    let nowhere =
        LocationShare { position: LatLng { lat: 123.0, lng: 0.0, ..Default::default() }.into(), ..Default::default() };
    assert_eq!(
        reason(SendMessageRequest { location: nowhere.into(), ..text_request(&id, "") }).await,
        "LOCATION_INVALID"
    );
    assert_eq!(
        reason(SendMessageRequest { reply_to_message_id: Uuid::now_v7().to_string(), ..text_request(&id, "返信") })
            .await,
        "REPLY_NOT_FOUND"
    );

    // Uploads still being processed: only a video may be sent early.
    let unfinished = ctx.photo(&taro).await;
    sqlx::query("UPDATE media_assets SET status = $2, ready_at = NULL WHERE id = $1::uuid")
        .bind(&unfinished)
        .bind(AssetStatus::ASSET_STATUS_PROCESSING as i32)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(reason(media_request(&id, &[&unfinished])).await, "ASSET_NOT_READY");

    // Only members send, and only signed-in callers.
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(ctx.send(&outsider, &text_request(&id, "入れて")).await.unwrap_err().reason, "CONVERSATION_NOT_FOUND");
    assert_eq!(ctx.anonymous_reason("Message", "SendMessage", &text_request(&id, "hi")).await, "SIGN_IN_REQUIRED");
    assert!(ctx.latest(&taro, &id).await.is_empty(), "nothing invalid was stored");
}

#[tokio::test]
async fn text_carries_mentions_links_and_big_emoji() {
    let ctx = start().await;
    let (group, owner, members) = ctx.group_of("部活", 2).await;
    let (hanako, jiro) = (&members[0], &members[1]);
    // Hanako muted the group.
    let mute = UpdatePreferencesRequest { conversation_id: group.id.clone(), muted: Some(true), ..Default::default() };
    let _: UpdatePreferencesResponse = ctx.call("Conversation", "UpdatePreferences", hanako, &mute).await.unwrap();

    // A mention by name: resolved against the members, and it gets through a mute.
    let mentioning = sent(
        &ctx,
        &owner,
        &text_request(&group.id, &format!("@{} 明日ひま？ @nobody_here も", hanako.handle.to_uppercase())),
    )
    .await;
    assert_eq!(mentioning.mentions.len(), 1, "only members can be mentioned");
    assert_eq!(mentioning.mentions[0].user.id, hanako.id.to_string());
    assert_eq!(mentioning.mentions[0].handle, hanako.handle);
    assert!(!mentioning.mentions_everyone);
    // (Both were told when the group was created: "chat.group".)
    assert_eq!(ctx.notification_kinds(hanako).await, vec!["chat.group", "chat.mention"]);
    assert_eq!(ctx.notification_kinds(jiro).await, vec!["chat.group", "chat.message"]);
    assert_eq!(ctx.conversation(hanako, &group.id).await.conversation.viewer.unread_mention_count, 1);
    assert_eq!(ctx.conversation(jiro, &group.id).await.conversation.viewer.unread_mention_count, 0);

    // @all mentions everyone, but does not get through a mute.
    let everyone = sent(&ctx, &owner, &text_request(&group.id, "@all 集合！")).await;
    assert!(everyone.mentions_everyone && everyone.mentions.is_empty());
    assert_eq!(ctx.notification_kinds(hanako).await, vec!["chat.group", "chat.mention"], "muted: @all stays silent");
    assert_eq!(ctx.notification_kinds(jiro).await, vec!["chat.group", "chat.message", "chat.mention"]);
    assert_eq!(ctx.conversation(hanako, &group.id).await.conversation.viewer.unread_mention_count, 2);
    // Reading clears mentions along with everything else.
    assert_eq!(ctx.mark_read(hanako, &group.id).await.unread_count, 0);
    assert_eq!(ctx.conversation(hanako, &group.id).await.conversation.viewer.unread_mention_count, 0);

    // The first link gets a preview shortly after sending.
    let mut jiro_events = ctx.listen(jiro).await;
    let linking =
        sent(&ctx, &owner, &text_request(&group.id, "これ見て！http://localhost:8080/watch/dQw4w9WgXcQ。すごい")).await;
    assert!(linking.link_preview.is_unset(), "the preview arrives later");
    ctx.chat.resolve_link_preview(linking.id.parse().unwrap()).await.unwrap();
    let updates = updated_messages(&ctx.events(&mut jiro_events).await);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].1, MessageUpdateReason::MESSAGE_UPDATE_REASON_LINK_PREVIEW);
    let preview = updates[0].0.link_preview.as_option().expect("a preview");
    assert_eq!(preview.url, "http://localhost:8080/watch/dQw4w9WgXcQ");
    assert_eq!((preview.embed_provider.as_str(), preview.embed_id.as_str()), ("tokuly", "video/dQw4w9WgXcQ"));
    assert_eq!(ctx.message(jiro, &linking.id).await.link_preview.embed_id, "video/dQw4w9WgXcQ");
    // Running the job again changes nothing.
    let mut jiro_events = ctx.listen(jiro).await;
    ctx.chat.resolve_link_preview(linking.id.parse().unwrap()).await.unwrap();
    assert!(ctx.events(&mut jiro_events).await.is_empty());

    // Nothing but a few emoji: drawn large.
    assert_eq!(sent(&ctx, &owner, &text_request(&group.id, "🎉🎉")).await.big_emoji_count, 2);
    assert_eq!(sent(&ctx, &owner, &text_request(&group.id, "やった🎉")).await.big_emoji_count, 0);
}

#[tokio::test]
async fn stamps_media_and_locations_are_sent() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let id = conversation.id.clone();

    // A stamp.
    let stamp_id = ctx.stamp(&taro).await;
    let stamp = sent(&ctx, &taro, &SendMessageRequest { stamp_id: stamp_id.clone(), ..text_request(&id, "") }).await;
    assert_eq!(stamp.kind, MessageKind::MESSAGE_KIND_STAMP);
    assert_eq!(stamp.stamp.id, stamp_id);
    assert!(!stamp.stamp.image_url.is_empty());
    assert_eq!(ctx.notifications(&hanako).await.last().unwrap().body.ja, "スタンプを送信しました");

    // Photos with a caption: several uploads in one message, all held.
    let (first, second) = (ctx.photo(&taro).await, ctx.photo(&taro).await);
    let photos = sent(
        &ctx,
        &taro,
        &SendMessageRequest { text: "昨日の！".into(), ..media_request(&id, &[&first, &second, &first]) },
    )
    .await;
    assert_eq!(photos.kind, MessageKind::MESSAGE_KIND_MEDIA);
    assert_eq!(photos.text, "昨日の！");
    assert_eq!(
        photos.attachments.iter().map(|attachment| attachment.asset_id.as_str()).collect::<Vec<_>>(),
        vec![first.as_str(), second.as_str()]
    );
    assert!(
        photos
            .attachments
            .iter()
            .all(|attachment| attachment.kind == AssetKind::ASSET_KIND_IMAGE && attachment.image.is_set())
    );
    assert!(tokuly_media::assets::is_attached(&ctx.app.core, first.parse().unwrap()).await.unwrap());
    let notification = ctx.notifications(&hanako).await.pop().unwrap();
    assert_eq!(notification.body.ja, "📷 昨日の！");
    assert!(notification.image_url.is_some(), "the push shows the photo");

    // A voice message carries its waveform; a file gets a download link.
    let voice_id = ctx.upload(&taro, AssetPurpose::ASSET_PURPOSE_CHAT_AUDIO, AssetKind::ASSET_KIND_AUDIO).await;
    let voice = sent(&ctx, &taro, &media_request(&id, &[&voice_id])).await;
    assert_eq!(voice.attachments[0].kind, AssetKind::ASSET_KIND_AUDIO);
    assert!(!voice.attachments[0].waveform.is_empty());
    assert_eq!(ctx.notifications(&hanako).await.last().unwrap().body.en, "🎤 Sent a voice message");
    let file_id = ctx.upload(&taro, AssetPurpose::ASSET_PURPOSE_CHAT_FILE, AssetKind::ASSET_KIND_FILE).await;
    let file = sent(&ctx, &taro, &media_request(&id, &[&file_id])).await;
    assert_eq!(file.attachments[0].file_name, "document.pdf");
    assert!(file.attachments[0].url.starts_with("http"), "private files are served through a signed link");
    assert!(ctx.message(&hanako, &file.id).await.attachments[0].url.starts_with("http"));

    // A video may be sent while it is still transcoding.
    let video_id = ctx.upload(&taro, AssetPurpose::ASSET_PURPOSE_CHAT_VIDEO, AssetKind::ASSET_KIND_VIDEO).await;
    sqlx::query("UPDATE media_assets SET status = $2, ready_at = NULL WHERE id = $1::uuid")
        .bind(&video_id)
        .bind(AssetStatus::ASSET_STATUS_PROCESSING as i32)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let video = sent(&ctx, &taro, &media_request(&id, &[&video_id])).await;
    assert_eq!(video.attachments[0].status, AssetStatus::ASSET_STATUS_PROCESSING);

    // A place.
    let place = LocationShare {
        position: LatLng { lat: 35.658, lng: 139.7016, ..Default::default() }.into(),
        name: " 渋谷駅 ".into(),
        address: "東京都渋谷区".into(),
        ..Default::default()
    };
    let location = sent(&ctx, &taro, &SendMessageRequest { location: place.into(), ..text_request(&id, "") }).await;
    assert_eq!(location.kind, MessageKind::MESSAGE_KIND_LOCATION);
    assert_eq!(location.location.name, "渋谷駅");
    assert_eq!(location.location.address, "東京都渋谷区");
    assert!((location.location.position.lat - 35.658).abs() < 1e-9);
    let seen = ctx.message(&hanako, &location.id).await;
    assert!((seen.location.position.lng - 139.7016).abs() < 1e-9);

    // The list preview is the latest message, whoever looks.
    assert_eq!(ctx.conversation(&hanako, &id).await.conversation.last_message.id, location.id);
    assert_eq!(ctx.unread(&hanako).await.badge, 6);
}

#[tokio::test]
async fn replies_quote_the_message_they_answer() {
    let ctx = start().await;
    let (group, owner, members) = ctx.group_of("部活", 1).await;
    let member = &members[0];
    let question = ctx.text(&owner, &group.id, "明日の集合、何時にする？").await;
    let photo_id = ctx.photo(&owner).await;
    let photo = sent(&ctx, &owner, &media_request(&group.id, &[&photo_id])).await;

    let answer = sent(
        &ctx,
        member,
        &SendMessageRequest { reply_to_message_id: question.id.clone(), ..text_request(&group.id, "10時で！") },
    )
    .await;
    let quote = answer.reply_to.as_option().expect("a quote");
    assert_eq!(quote.message_id, question.id);
    assert_eq!(quote.seq, question.seq);
    assert_eq!(quote.sender.id, owner.id.to_string());
    assert_eq!(quote.kind, MessageKind::MESSAGE_KIND_TEXT);
    assert_eq!(quote.text, "明日の集合、何時にする？");
    assert!(!quote.unavailable);
    let about_photo = sent(
        &ctx,
        member,
        &SendMessageRequest { reply_to_message_id: photo.id.clone(), ..text_request(&group.id, "いい写真！") },
    )
    .await;
    assert!(about_photo.reply_to.thumbnail_url.starts_with("http"), "a media quote shows a thumbnail");

    // Someone who joins later never sees quotes of what was said before.
    let newcomer = ctx.app.user("newcomer").await;
    ctx.befriend(&owner, &newcomer).await;
    let add = AddMembersRequest {
        conversation_id: group.id.clone(),
        user_ids: vec![newcomer.id.to_string()],
        ..Default::default()
    };
    let _: AddMembersResponse = ctx.call("Conversation", "AddMembers", &owner, &add).await.unwrap();
    let mut newcomer_events = ctx.listen(&newcomer).await;
    let mut owner_events = ctx.listen(&owner).await;
    let late = sent(
        &ctx,
        member,
        &SendMessageRequest {
            reply_to_message_id: question.id.clone(), ..text_request(&group.id, "やっぱり11時")
        },
    )
    .await;
    let for_newcomer = created_messages(&ctx.events(&mut newcomer_events).await);
    assert_eq!(for_newcomer.len(), 1);
    assert!(
        for_newcomer[0].reply_to.unavailable && for_newcomer[0].reply_to.text.is_empty(),
        "the quote is withheld in the push"
    );
    let for_owner = created_messages(&ctx.events(&mut owner_events).await);
    assert_eq!(for_owner[0].reply_to.text, "明日の集合、何時にする？");
    let listed = ctx.message(&newcomer, &late.id).await;
    assert!(listed.reply_to.unavailable && listed.reply_to.text.is_empty(), "and when they load the message");

    // A reply cannot answer what the sender cannot see, nor another conversation.
    let hidden =
        SendMessageRequest { reply_to_message_id: question.id.clone(), ..text_request(&group.id, "何の話？") };
    assert_eq!(ctx.send(&newcomer, &hidden).await.unwrap_err().reason, "REPLY_NOT_FOUND");
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let elsewhere = ctx.direct(&taro, &hanako).await;
    let foreign = ctx.text(&taro, &elsewhere.id, "別の会話").await;
    let cross =
        SendMessageRequest { reply_to_message_id: foreign.id.clone(), ..text_request(&group.id, "混ざった") };
    assert_eq!(ctx.send(&owner, &cross).await.unwrap_err().reason, "REPLY_NOT_FOUND");

    // Once the answered message is deleted the quote is gone.
    let _: DeleteMessageResponse = ctx
        .call(
            "Message",
            "DeleteMessage",
            &owner,
            &DeleteMessageRequest { message_id: question.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let after = ctx.message(&owner, &answer.id).await;
    assert!(after.reply_to.unavailable && after.reply_to.text.is_empty());
    assert_eq!(after.reply_to.message_id, question.id);
}

#[tokio::test]
async fn a_block_makes_a_direct_conversation_read_only_at_once() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let message = ctx.text(&hanako, &conversation.id, "ブロックする前").await;
    ctx.block(&hanako, &taro).await;

    // Checked live: no event has been processed yet.
    for sender in [&taro, &hanako] {
        assert_eq!(
            ctx.send(sender, &text_request(&conversation.id, "届く？")).await.unwrap_err().reason,
            "CONVERSATION_READ_ONLY"
        );
    }
    let edit = EditMessageRequest { message_id: message.id.clone(), text: "書き換え".into(), ..Default::default() };
    assert_eq!(ctx.reason("Message", "EditMessage", &hanako, &edit).await, "CONVERSATION_READ_ONLY");
    let react = AddReactionRequest { message_id: message.id.clone(), emoji: "👍".into(), ..Default::default() };
    assert_eq!(ctx.reason("Message", "AddReaction", &taro, &react).await, "CONVERSATION_READ_ONLY");
    let pin = PinMessageRequest { message_id: message.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Message", "PinMessage", &taro, &pin).await, "CONVERSATION_READ_ONLY");
    assert_eq!(
        ctx.chat.post_text(conversation.id.parse().unwrap(), taro.id, "サーバーから").await.unwrap_err().reason,
        "CONVERSATION_READ_ONLY"
    );
    // What was said stays readable, and one's own messages can still be deleted.
    assert_eq!(texts(&ctx.latest(&taro, &conversation.id).await), vec!["ブロックする前"]);
    let _: DeleteMessageResponse = ctx
        .call(
            "Message",
            "DeleteMessage",
            &hanako,
            &DeleteMessageRequest { message_id: message.id, ..Default::default() },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_hello_is_the_first_thing_two_people_say() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let hello = |user: &TestUser| SendHelloRequest { user_id: user.id.to_string(), ..Default::default() };
    let mut hanako_events = ctx.listen(&hanako).await;

    let waved: SendHelloResponse = ctx.call("Message", "SendHello", &taro, &hello(&hanako)).await.unwrap();
    assert_eq!(waved.message.kind, MessageKind::MESSAGE_KIND_HELLO);
    assert_eq!(waved.conversation.kind, ConversationKind::CONVERSATION_KIND_DIRECT);
    assert_eq!(waved.conversation.last_message.id, waved.message.id);
    assert_eq!(created_messages(&ctx.events(&mut hanako_events).await)[0].kind, MessageKind::MESSAGE_KIND_HELLO);
    assert_eq!(ctx.notifications(&hanako).await[0].body.ja, "👋 やっほー！");

    // Only once, from either side.
    assert_eq!(ctx.reason("Message", "SendHello", &taro, &hello(&hanako)).await, "HELLO_NOT_FIRST");
    assert_eq!(ctx.reason("Message", "SendHello", &hanako, &hello(&taro)).await, "HELLO_NOT_FIRST");
    assert_eq!(ctx.reason("Message", "SendHello", &taro, &hello(&taro)).await, "CANNOT_MESSAGE_SELF");
    assert_eq!(ctx.anonymous_reason("Message", "SendHello", &hello(&hanako)).await, "SIGN_IN_REQUIRED");

    // The same rules as opening a conversation: strangers need to be let in,
    // and then arrive as a message request.
    let stranger = ctx.app.user("stranger").await;
    assert_eq!(ctx.reason("Message", "SendHello", &stranger, &hello(&hanako)).await, "DM_NOT_ALLOWED");
    ctx.open_to_everyone(&hanako).await;
    let requested: SendHelloResponse = ctx.call("Message", "SendHello", &stranger, &hello(&hanako)).await.unwrap();
    assert_eq!(requested.conversation.viewer.request_messages_left, 2);
    assert_eq!(ctx.notification_kinds(&hanako).await, vec!["chat.message", "chat.request"]);
}

#[tokio::test]
async fn own_messages_can_be_edited_for_a_while() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let message = ctx.text(&taro, &conversation.id, "あしたの10時に").await;
    let edit = |message_id: &str, text: &str| EditMessageRequest {
        message_id: message_id.to_owned(),
        text: text.to_owned(),
        ..Default::default()
    };

    let mut hanako_events = ctx.listen(&hanako).await;
    let edited: EditMessageResponse = ctx
        .call(
            "Message",
            "EditMessage",
            &taro,
            &edit(&message.id, "あしたの11時に！ http://localhost:8080/notes/abc123"),
        )
        .await
        .unwrap();
    assert_eq!(edited.message.text, "あしたの11時に！ http://localhost:8080/notes/abc123");
    assert!(edited.message.edited_at.is_set());
    assert_eq!(edited.message.seq, message.seq, "an edit does not move the message");
    let updates = updated_messages(&ctx.events(&mut hanako_events).await);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].1, MessageUpdateReason::MESSAGE_UPDATE_REASON_EDITED);
    assert_eq!(updates[0].0.text, edited.message.text);
    // The new link gets its preview.
    ctx.chat.resolve_link_preview(message.id.parse().unwrap()).await.unwrap();
    assert_eq!(ctx.message(&hanako, &message.id).await.link_preview.embed_id, "note/abc123");
    // Without the link, the preview goes too.
    let plain: EditMessageResponse =
        ctx.call("Message", "EditMessage", &taro, &edit(&message.id, "あしたの11時に！")).await.unwrap();
    assert!(plain.message.link_preview.is_unset());

    assert_eq!(ctx.reason("Message", "EditMessage", &hanako, &edit(&message.id, "乗っ取り")).await, "NOT_ALLOWED");
    assert_eq!(ctx.reason("Message", "EditMessage", &taro, &edit(&message.id, "  ")).await, "MESSAGE_EMPTY");
    assert_eq!(
        ctx.reason("Message", "EditMessage", &taro, &edit(&message.id, &"あ".repeat(4001))).await,
        "TEXT_TOO_LONG"
    );
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(ctx.reason("Message", "EditMessage", &outsider, &edit(&message.id, "x")).await, "MESSAGE_NOT_FOUND");
    assert_eq!(ctx.anonymous_reason("Message", "EditMessage", &edit(&message.id, "x")).await, "SIGN_IN_REQUIRED");

    // Only text and captions can be edited.
    let stamp_id = ctx.stamp(&taro).await;
    let stamp = sent(&ctx, &taro, &SendMessageRequest { stamp_id, ..text_request(&conversation.id, "") }).await;
    assert_eq!(ctx.reason("Message", "EditMessage", &taro, &edit(&stamp.id, "文字に")).await, "MESSAGE_NOT_EDITABLE");
    let photo_id = ctx.photo(&taro).await;
    let photo = sent(&ctx, &taro, &media_request(&conversation.id, &[&photo_id])).await;
    let captioned: EditMessageResponse =
        ctx.call("Message", "EditMessage", &taro, &edit(&photo.id, "キャプション")).await.unwrap();
    assert_eq!(captioned.message.text, "キャプション");
    let uncaptioned: EditMessageResponse =
        ctx.call("Message", "EditMessage", &taro, &edit(&photo.id, "")).await.unwrap();
    assert!(uncaptioned.message.text.is_empty(), "a caption may be removed");

    // The window closes 15 minutes after sending.
    sqlx::query("UPDATE chat_messages SET created_at = now() - interval '16 minutes' WHERE id = $1::uuid")
        .bind(&message.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(ctx.reason("Message", "EditMessage", &taro, &edit(&message.id, "遅すぎた")).await, "EDIT_WINDOW_CLOSED");
}

#[tokio::test]
async fn deleting_for_everyone_leaves_a_tombstone() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let photo_id = ctx.photo(&taro).await;
    let message = sent(
        &ctx,
        &taro,
        &SendMessageRequest { text: "間違えた".into(), ..media_request(&conversation.id, &[&photo_id]) },
    )
    .await;
    ctx.text(&taro, &conversation.id, "こっちが正しい").await;
    let react = AddReactionRequest { message_id: message.id.clone(), emoji: "😂".into(), ..Default::default() };
    let _: AddReactionResponse = ctx.call("Message", "AddReaction", &hanako, &react).await.unwrap();
    let _: PinMessageResponse = ctx
        .call(
            "Message",
            "PinMessage",
            &hanako,
            &PinMessageRequest { message_id: message.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(ctx.unread(&hanako).await.badge, 2);
    let delete = DeleteMessageRequest { message_id: message.id.clone(), ..Default::default() };

    assert_eq!(ctx.reason("Message", "DeleteMessage", &hanako, &delete).await, "NOT_ALLOWED");
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(ctx.reason("Message", "DeleteMessage", &outsider, &delete).await, "MESSAGE_NOT_FOUND");

    let mut hanako_events = ctx.listen(&hanako).await;
    let deleted: DeleteMessageResponse = ctx.call("Message", "DeleteMessage", &taro, &delete).await.unwrap();
    let tombstone = deleted.message.as_option().unwrap();
    assert!(tombstone.deleted);
    assert!(
        tombstone.text.is_empty()
            && tombstone.attachments.is_empty()
            && tombstone.reactions.is_empty()
            && !tombstone.pinned
    );
    assert_eq!(
        (tombstone.seq, tombstone.sender.id.as_str()),
        (message.seq, taro.id.to_string().as_str()),
        "it keeps its place and its sender"
    );
    let updates = updated_messages(&ctx.events(&mut hanako_events).await);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].1, MessageUpdateReason::MESSAGE_UPDATE_REASON_DELETED);
    assert!(updates[0].0.deleted && updates[0].0.text.is_empty());

    // Gone from the unread count, the gallery and the pins; the upload is released.
    assert_eq!(ctx.unread(&hanako).await.badge, 1);
    assert!(!tokuly_media::assets::is_attached(&ctx.app.core, photo_id.parse().unwrap()).await.unwrap());
    let stored: (String, serde_json::Value) =
        sqlx::query_as("SELECT text, attachments FROM chat_messages WHERE id = $1::uuid")
            .bind(&message.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(stored, (String::new(), serde_json::json!([])), "the content is erased, not just hidden");
    let pinned: ListPinnedMessagesResponse = ctx
        .call(
            "Message",
            "ListPinnedMessages",
            &hanako,
            &ListPinnedMessagesRequest { conversation_id: conversation.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(pinned.messages.is_empty());
    let events = ctx.app.events_by("chat.message.deleted", taro.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["message_id"], message.id);

    // Deleting twice succeeds; a tombstone accepts nothing.
    let again: DeleteMessageResponse = ctx.call("Message", "DeleteMessage", &taro, &delete).await.unwrap();
    assert!(again.message.deleted);
    assert_eq!(ctx.reason("Message", "AddReaction", &hanako, &react).await, "MESSAGE_NOT_REACTABLE");
    let edit = EditMessageRequest { message_id: message.id.clone(), text: "復活".into(), ..Default::default() };
    assert_eq!(ctx.reason("Message", "EditMessage", &taro, &edit).await, "MESSAGE_NOT_EDITABLE");
}

#[tokio::test]
async fn group_managers_delete_anyones_messages_but_not_the_story() {
    let ctx = start().await;
    let (group, owner, members) = ctx.group_of("部活", 2).await;
    let (first, second) = (&members[0], &members[1]);
    let message = ctx.text(first, &group.id, "消されるやつ").await;
    let delete = |message_id: &str| DeleteMessageRequest { message_id: message_id.to_owned(), ..Default::default() };

    assert_eq!(ctx.reason("Message", "DeleteMessage", second, &delete(&message.id)).await, "NOT_ALLOWED");
    let deleted: DeleteMessageResponse =
        ctx.call("Message", "DeleteMessage", &owner, &delete(&message.id)).await.unwrap();
    assert!(deleted.message.deleted);
    let events = ctx.app.events_by("chat.message.deleted", owner.id).await;
    assert_eq!(events[0].data["sender_id"], first.id.to_string());

    // System messages tell what happened to the group and stay.
    let system = ctx
        .latest(&owner, &group.id)
        .await
        .into_iter()
        .find(|message| message.kind == MessageKind::MESSAGE_KIND_SYSTEM)
        .unwrap();
    assert_eq!(ctx.reason("Message", "DeleteMessage", &owner, &delete(&system.id)).await, "MESSAGE_NOT_DELETABLE");
}

#[tokio::test]
async fn messages_page_in_both_directions() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let mut all = Vec::new();
    for index in 1..=25 {
        all.push(ctx.text(&taro, &conversation.id, &format!("m{index}")).await);
    }
    let list = |position: Option<Position>, limit: i32| ListMessagesRequest {
        conversation_id: conversation.id.clone(),
        limit,
        position,
        ..Default::default()
    };
    let range = |from: usize, to: usize| (from..=to).map(|index| format!("m{index}")).collect::<Vec<_>>();

    // Without a position: the newest messages, oldest first.
    let newest = ctx.messages(&hanako, &list(None, 10)).await;
    assert_eq!(texts(&newest.messages), range(16, 25));
    assert!(newest.has_older && !newest.has_newer);
    assert_eq!(newest.read_cursors.len(), 2);

    // Towards older messages.
    let older = ctx.messages(&hanako, &list(Some(Position::Before(newest.older_cursor.clone())), 10)).await;
    assert_eq!(texts(&older.messages), range(6, 15));
    assert!(older.has_older && older.has_newer);
    let oldest = ctx.messages(&hanako, &list(Some(Position::Before(older.older_cursor.clone())), 10)).await;
    assert_eq!(texts(&oldest.messages), range(1, 5));
    assert!(!oldest.has_older && oldest.has_newer);

    // Towards newer messages, e.g. catching up after a reconnect.
    let forward = ctx.messages(&hanako, &list(Some(Position::After(oldest.newer_cursor.clone())), 10)).await;
    assert_eq!(texts(&forward.messages), range(6, 15));
    assert!(forward.has_older && forward.has_newer);
    let caught_up = ctx.messages(&hanako, &list(Some(Position::After(newest.newer_cursor.clone())), 10)).await;
    assert!(caught_up.messages.is_empty() && !caught_up.has_newer);
    ctx.text(&taro, &conversation.id, "m26").await;
    let arrived = ctx.messages(&hanako, &list(Some(Position::After(caught_up.newer_cursor.clone())), 10)).await;
    assert_eq!(texts(&arrived.messages), vec!["m26"], "the cursor of an empty page still works");

    // Around a message: jumping to a reply, a search hit, a pin.
    let around = ctx.messages(&hanako, &list(Some(Position::AroundMessageId(all[11].id.clone())), 6)).await;
    assert_eq!(texts(&around.messages), range(9, 14));
    assert!(around.has_older && around.has_newer);
    let edge = ctx.messages(&hanako, &list(Some(Position::AroundMessageId(all[0].id.clone())), 6)).await;
    assert_eq!(texts(&edge.messages), range(1, 3));
    assert!(!edge.has_older);

    // Around the first unread message.
    let up_to = MarkReadRequest {
        conversation_id: conversation.id.clone(),
        message_id: all[19].id.clone(),
        ..Default::default()
    };
    let _: MarkReadResponse = ctx.call("Conversation", "MarkRead", &hanako, &up_to).await.unwrap();
    let unread = ctx.messages(&hanako, &list(Some(Position::AroundUnread(true)), 4)).await;
    assert_eq!(texts(&unread.messages), range(19, 22), "two read messages, then the unread ones");
    ctx.mark_read(&hanako, &conversation.id).await;
    let nothing_unread = ctx.messages(&hanako, &list(Some(Position::AroundUnread(true)), 4)).await;
    assert_eq!(texts(&nothing_unread.messages), range(23, 26), "the newest when everything is read");

    // Limits and bad input.
    assert_eq!(ctx.messages(&hanako, &list(None, 0)).await.messages.len(), 26, "the default page holds 30");
    assert_eq!(ctx.messages(&hanako, &list(None, 1000)).await.messages.len(), 26);
    assert_eq!(
        ctx.reason("Message", "ListMessages", &hanako, &list(Some(Position::Before("???".into())), 10)).await,
        "INVALID_CURSOR"
    );
    let unknown = list(Some(Position::AroundMessageId(Uuid::now_v7().to_string())), 10);
    assert_eq!(ctx.reason("Message", "ListMessages", &hanako, &unknown).await, "MESSAGE_NOT_FOUND");
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(ctx.reason("Message", "ListMessages", &outsider, &list(None, 10)).await, "CONVERSATION_NOT_FOUND");
    assert_eq!(ctx.anonymous_reason("Message", "ListMessages", &list(None, 10)).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason("Message", "GetMessage", &outsider, &by_id(&all[0].id)).await, "MESSAGE_NOT_FOUND");
    assert_eq!(
        ctx.reason("Message", "GetMessage", &hanako, &by_id(&Uuid::now_v7().to_string())).await,
        "MESSAGE_NOT_FOUND"
    );
}

#[tokio::test]
async fn search_finds_text_within_one_conversation() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    ctx.text(&taro, &conversation.id, "渋谷でランチしよう").await;
    ctx.text(&hanako, &conversation.id, "Shibuya LUNCH sounds great").await;
    let sale = ctx.text(&taro, &conversation.id, "セールで100%オフだって").await;
    let deleted = ctx.text(&taro, &conversation.id, "渋谷は取り消し").await;
    let _: DeleteMessageResponse = ctx
        .call("Message", "DeleteMessage", &taro, &DeleteMessageRequest { message_id: deleted.id, ..Default::default() })
        .await
        .unwrap();
    // The same words elsewhere are not found here.
    let jiro = ctx.app.user("jiro").await;
    ctx.befriend(&taro, &jiro).await;
    let elsewhere = ctx.direct(&taro, &jiro).await;
    ctx.text(&taro, &elsewhere.id, "渋谷の話は内緒").await;

    let search = |query: &str| SearchMessagesRequest {
        conversation_id: conversation.id.clone(),
        query: query.to_owned(),
        ..Default::default()
    };
    let found = |query: &'static str| {
        let ctx = &ctx;
        let hanako = &hanako;
        let request = search(query);
        async move {
            let response: SearchMessagesResponse =
                ctx.call("Message", "SearchMessages", hanako, &request).await.unwrap();
            response.messages.into_iter().map(|message| message.text).collect::<Vec<_>>()
        }
    };
    assert_eq!(found("渋谷").await, vec!["渋谷でランチしよう"], "deleted messages are not found");
    assert_eq!(found("lunch").await, vec!["Shibuya LUNCH sounds great"], "case does not matter");
    assert_eq!(found("駅").await, Vec::<String>::new());
    assert_eq!(found("%").await, vec![sale.text.clone()], "wildcards are taken literally");
    assert_eq!(found("ラ").await, vec!["渋谷でランチしよう"], "a single character is a query");

    // Newest first, in pages.
    for index in 0..3 {
        ctx.text(&taro, &conversation.id, &format!("たこ焼き{index}")).await;
    }
    let page = |cursor: &str| SearchMessagesRequest {
        page: PageRequest { limit: 2, cursor: cursor.to_owned(), ..Default::default() }.into(),
        ..search("たこ焼き")
    };
    let first: SearchMessagesResponse = ctx.call("Message", "SearchMessages", &hanako, &page("")).await.unwrap();
    assert_eq!(texts(&first.messages), vec!["たこ焼き2", "たこ焼き1"]);
    assert!(first.page.has_more);
    let second: SearchMessagesResponse =
        ctx.call("Message", "SearchMessages", &hanako, &page(&first.page.next_cursor)).await.unwrap();
    assert_eq!(texts(&second.messages), vec!["たこ焼き0"]);

    assert_eq!(ctx.reason("Message", "SearchMessages", &hanako, &search("  ")).await, "QUERY_INVALID");
    assert_eq!(ctx.reason("Message", "SearchMessages", &hanako, &search(&"a".repeat(101))).await, "QUERY_INVALID");
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(ctx.reason("Message", "SearchMessages", &outsider, &search("渋谷")).await, "CONVERSATION_NOT_FOUND");
    assert_eq!(ctx.anonymous_reason("Message", "SearchMessages", &search("渋谷")).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn messages_are_forwarded_into_other_conversations() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let source = ctx.direct(&taro, &hanako).await;
    let jiro = ctx.app.user("jiro").await;
    ctx.befriend(&taro, &jiro).await;
    let direct = ctx.direct(&taro, &jiro).await;
    let group = ctx.group(&taro, "転送先", &[&jiro]).await;

    let text = ctx.text(&hanako, &source.id, "来週のライブ行く？ http://localhost:8080/live/abc123").await;
    ctx.chat.resolve_link_preview(text.id.parse().unwrap()).await.unwrap();
    let photo_id = ctx.photo(&hanako).await;
    let photo =
        sent(&ctx, &hanako, &SendMessageRequest { text: "チケット".into(), ..media_request(&source.id, &[&photo_id]) })
            .await;

    let mut jiro_events = ctx.listen(&jiro).await;
    let forward = ForwardMessagesRequest {
        // Out of order on purpose: copies keep the order of the originals.
        message_ids: vec![photo.id.clone(), text.id.clone()],
        conversation_ids: vec![direct.id.clone(), group.id.clone()],
        client_id: client_id(),
        ..Default::default()
    };
    let forwarded: ForwardMessagesResponse = ctx.call("Message", "ForwardMessages", &taro, &forward).await.unwrap();
    assert_eq!(forwarded.messages.len(), 4);
    for copies in forwarded.messages.chunks(2) {
        assert_eq!(texts(copies), vec![text.text.as_str(), "チケット"]);
        assert!(copies.iter().all(|copy| copy.forwarded && copy.sender.id == taro.id.to_string()));
        assert_eq!(copies[0].link_preview.embed_id, "live/abc123", "a known preview travels along");
        assert_eq!(copies[1].attachments[0].asset_id, photo_id, "someone else's upload can be forwarded");
    }
    assert_eq!(forwarded.messages[0].conversation_id, direct.id);
    assert_eq!(forwarded.messages[2].conversation_id, group.id);
    assert_eq!(created_messages(&ctx.events(&mut jiro_events).await).len(), 4);
    // The upload now also belongs to the copies.
    let _: DeleteMessageResponse = ctx
        .call(
            "Message",
            "DeleteMessage",
            &hanako,
            &DeleteMessageRequest { message_id: photo.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(tokuly_media::assets::is_attached(&ctx.app.core, photo_id.parse().unwrap()).await.unwrap());

    // A retry forwards nothing twice.
    let retried: ForwardMessagesResponse = ctx
        .call(
            "Message",
            "ForwardMessages",
            &taro,
            &ForwardMessagesRequest {
                message_ids: vec![text.id.clone()],
                conversation_ids: vec![direct.id.clone()],
                ..forward.clone()
            },
        )
        .await
        .unwrap();
    assert_eq!(retried.messages[0].id, forwarded.messages[0].id);
    assert_eq!(ctx.latest(&jiro, &direct.id).await.len(), 2);

    // What cannot be forwarded, and where to.
    let request = |message_ids: Vec<String>, conversation_ids: Vec<String>| ForwardMessagesRequest {
        message_ids,
        conversation_ids,
        client_id: client_id(),
        ..Default::default()
    };
    let system = ctx
        .latest(&taro, &group.id)
        .await
        .into_iter()
        .find(|message| message.kind == MessageKind::MESSAGE_KIND_SYSTEM)
        .unwrap();
    assert_eq!(
        ctx.reason("Message", "ForwardMessages", &taro, &request(vec![system.id], vec![direct.id.clone()])).await,
        "MESSAGE_NOT_FORWARDABLE"
    );
    assert_eq!(
        ctx.reason("Message", "ForwardMessages", &taro, &request(vec![photo.id.clone()], vec![direct.id.clone()]))
            .await,
        "MESSAGE_NOT_FORWARDABLE"
    );
    assert_eq!(
        ctx.reason("Message", "ForwardMessages", &jiro, &request(vec![text.id.clone()], vec![direct.id.clone()])).await,
        "MESSAGE_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason("Message", "ForwardMessages", &hanako, &request(vec![text.id.clone()], vec![direct.id.clone()]))
            .await,
        "CONVERSATION_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason("Message", "ForwardMessages", &taro, &request(vec![], vec![direct.id.clone()])).await,
        "NOTHING_TO_FORWARD"
    );
    assert_eq!(
        ctx.reason("Message", "ForwardMessages", &taro, &request(vec![text.id.clone()], vec![])).await,
        "NOTHING_TO_FORWARD"
    );
    let many = |count: usize| (0..count).map(|_| Uuid::now_v7().to_string()).collect::<Vec<_>>();
    assert_eq!(
        ctx.reason("Message", "ForwardMessages", &taro, &request(many(11), vec![direct.id.clone()])).await,
        "TOO_MANY_MESSAGES"
    );
    assert_eq!(
        ctx.reason("Message", "ForwardMessages", &taro, &request(vec![text.id.clone()], many(11))).await,
        "TOO_MANY_CONVERSATIONS"
    );
    let long_key =
        ForwardMessagesRequest { client_id: "k".repeat(49), ..request(vec![text.id.clone()], vec![direct.id.clone()]) };
    assert_eq!(ctx.reason("Message", "ForwardMessages", &taro, &long_key).await, "CLIENT_ID_INVALID");
    assert_eq!(
        ctx.anonymous_reason("Message", "ForwardMessages", &request(vec![text.id.clone()], vec![direct.id.clone()]))
            .await,
        "SIGN_IN_REQUIRED"
    );
    // A blocked conversation takes no forwards either.
    ctx.block(&jiro, &taro).await;
    assert_eq!(
        ctx.reason("Message", "ForwardMessages", &taro, &request(vec![text.id.clone()], vec![direct.id.clone()])).await,
        "CONVERSATION_READ_ONLY"
    );
}

#[tokio::test]
async fn reactions_are_counted_per_emoji() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let message = ctx.text(&taro, &conversation.id, "合格した！").await;
    let react = |emoji: &str| AddReactionRequest {
        message_id: message.id.clone(),
        emoji: emoji.to_owned(),
        ..Default::default()
    };
    let unreact = |emoji: &str| RemoveReactionRequest {
        message_id: message.id.clone(),
        emoji: emoji.to_owned(),
        ..Default::default()
    };
    let summary = |reactions: &[ReactionCount]| {
        reactions
            .iter()
            .map(|reaction| (reaction.emoji.clone(), reaction.count, reaction.reacted_by_me))
            .collect::<Vec<_>>()
    };

    let mut taro_events = ctx.listen(&taro).await;
    let added: AddReactionResponse = ctx.call("Message", "AddReaction", &hanako, &react("🎉")).await.unwrap();
    assert_eq!(summary(&added.reactions), vec![("🎉".to_owned(), 1, true)]);
    let events = ctx.events(&mut taro_events).await;
    assert!(matches!(
        events.as_slice(),
        [Push::ReactionChanged(change)] if change.emoji == "🎉" && change.added && change.count == 1 && change.user_id == hanako.id.to_string() && change.message_id == message.id
    ));
    // The author hears about it, quietly.
    let notification = ctx.notifications(&taro).await.pop().unwrap();
    assert_eq!(notification.kind, "chat.reaction");
    assert!(notification.push && !notification.inbox);
    assert!(notification.body.ja.contains("🎉"));

    // The same emoji twice is one reaction; another person adds to the count.
    let mut taro_events = ctx.listen(&taro).await;
    let _: AddReactionResponse = ctx.call("Message", "AddReaction", &hanako, &react("🎉")).await.unwrap();
    assert!(ctx.events(&mut taro_events).await.is_empty());
    let _: AddReactionResponse = ctx.call("Message", "AddReaction", &hanako, &react("👍🏽")).await.unwrap();
    let own: AddReactionResponse = ctx.call("Message", "AddReaction", &taro, &react("🎉")).await.unwrap();
    assert_eq!(
        summary(&own.reactions),
        vec![("🎉".to_owned(), 2, true), ("👍🏽".to_owned(), 1, false)],
        "in the order the emoji first appeared"
    );
    assert_eq!(
        summary(&ctx.message(&hanako, &message.id).await.reactions),
        vec![("🎉".to_owned(), 2, true), ("👍🏽".to_owned(), 1, true)]
    );
    assert_eq!(
        ctx.notifications(&hanako).await.iter().filter(|n| n.kind == "chat.reaction").count(),
        0,
        "reacting to your own message notifies nobody"
    );

    // Who reacted, newest first, optionally one emoji only.
    let everyone: ListReactionsResponse = ctx
        .call(
            "Message",
            "ListReactions",
            &taro,
            &ListReactionsRequest { message_id: message.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let listed: Vec<(String, String)> =
        everyone.reactions.iter().map(|reaction| (reaction.user.id.clone(), reaction.emoji.clone())).collect();
    assert_eq!(
        listed,
        vec![
            (taro.id.to_string(), "🎉".to_owned()),
            (hanako.id.to_string(), "👍🏽".to_owned()),
            (hanako.id.to_string(), "🎉".to_owned())
        ]
    );
    let party: ListReactionsResponse = ctx
        .call(
            "Message",
            "ListReactions",
            &taro,
            &ListReactionsRequest { message_id: message.id.clone(), emoji: "🎉".into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(party.reactions.len(), 2);

    // Removing: only one's own, and removing nothing is fine.
    let mut taro_events = ctx.listen(&taro).await;
    let removed: RemoveReactionResponse = ctx.call("Message", "RemoveReaction", &hanako, &unreact("🎉")).await.unwrap();
    assert_eq!(
        summary(&removed.reactions),
        vec![("👍🏽".to_owned(), 1, true), ("🎉".to_owned(), 1, false)],
        "ordered by the earliest reaction still there"
    );
    assert!(
        matches!(ctx.events(&mut taro_events).await.as_slice(), [Push::ReactionChanged(change)] if !change.added && change.count == 1)
    );
    let _: RemoveReactionResponse = ctx.call("Message", "RemoveReaction", &hanako, &unreact("🎉")).await.unwrap();
    assert_eq!(ctx.message(&taro, &message.id).await.reactions.len(), 2);

    for invalid in ["", "ab", "🎉🎉", "いいね"] {
        assert_eq!(
            ctx.reason("Message", "AddReaction", &hanako, &react(invalid)).await,
            "EMOJI_INVALID",
            "{invalid:?}"
        );
    }
    assert_eq!(ctx.reason("Message", "RemoveReaction", &hanako, &unreact("x")).await, "EMOJI_INVALID");
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(ctx.reason("Message", "AddReaction", &outsider, &react("🎉")).await, "MESSAGE_NOT_FOUND");
    assert_eq!(ctx.reason("Message", "RemoveReaction", &outsider, &unreact("🎉")).await, "MESSAGE_NOT_FOUND");
    let list = ListReactionsRequest { message_id: message.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Message", "ListReactions", &outsider, &list).await, "MESSAGE_NOT_FOUND");
    assert_eq!(ctx.anonymous_reason("Message", "AddReaction", &react("🎉")).await, "SIGN_IN_REQUIRED");

    // At most twenty different emoji on one message.
    let crowded = ctx.text(&taro, &conversation.id, "人気者").await;
    let emoji = [
        "😀", "😁", "😂", "🤣", "😃", "😄", "😅", "😆", "😉", "😊", "😋", "😎", "😍", "😘", "🥰", "😗", "😙", "😚",
        "🙂", "🤗", "🤩",
    ];
    for emoji in &emoji[..20] {
        let request =
            AddReactionRequest { message_id: crowded.id.clone(), emoji: emoji.to_string(), ..Default::default() };
        let _: AddReactionResponse = ctx.call("Message", "AddReaction", &hanako, &request).await.unwrap();
    }
    let one_more =
        AddReactionRequest { message_id: crowded.id.clone(), emoji: emoji[20].to_owned(), ..Default::default() };
    assert_eq!(ctx.reason("Message", "AddReaction", &hanako, &one_more).await, "TOO_MANY_REACTIONS");
    // An emoji already on the message is always welcome.
    let joining =
        AddReactionRequest { message_id: crowded.id.clone(), emoji: emoji[0].to_owned(), ..Default::default() };
    let joined: AddReactionResponse = ctx.call("Message", "AddReaction", &taro, &joining).await.unwrap();
    assert_eq!(joined.reactions[0].count, 2);
}

#[tokio::test]
async fn a_burst_from_one_account_does_not_starve_the_connection_pool() {
    let ctx = start().await;
    let (group, owner, members) = ctx.group_of("部活", 2).await;
    let direct = ctx.direct(&owner, &members[0]).await;
    let pinned = ctx.text(&owner, &group.id, "ピン留めしてね").await;
    let mut targets = vec![pinned.id.clone()];
    for index in 0..4 {
        targets.push(ctx.text(&owner, &group.id, &format!("その{index}")).await.id);
    }

    // Several times as many requests as the test pool has connections (5),
    // all of them taking the conversation lock, from one account at once.
    let send_requests: Vec<SendMessageRequest> =
        (0..10).map(|index| text_request(&group.id, &format!("連投{index}"))).collect();
    let pin_requests: Vec<PinMessageRequest> = targets
        .iter()
        .map(|message_id| PinMessageRequest { message_id: message_id.clone(), ..Default::default() })
        .collect();
    let forward_requests: Vec<ForwardMessagesRequest> = (0..5)
        .map(|index| ForwardMessagesRequest {
            message_ids: vec![pinned.id.clone()],
            conversation_ids: vec![direct.id.clone(), group.id.clone()],
            client_id: format!("burst-{index}-{}", client_id()),
            ..Default::default()
        })
        .collect();
    let started = std::time::Instant::now();
    let sends = send_requests.iter().map(|request| ctx.send(&owner, request));
    let pins =
        pin_requests.iter().map(|request| ctx.call::<_, PinMessageResponse>("Message", "PinMessage", &owner, request));
    let forwards = forward_requests
        .iter()
        .map(|request| ctx.call::<_, ForwardMessagesResponse>("Message", "ForwardMessages", &owner, request));
    let (sends, pins, forwards) = tokio::join!(
        futures::future::join_all(sends),
        futures::future::join_all(pins),
        futures::future::join_all(forwards)
    );
    assert!(sends.iter().all(Result::is_ok), "{sends:?}");
    assert!(pins.iter().all(Result::is_ok), "{pins:?}");
    assert!(forwards.iter().all(Result::is_ok), "{forwards:?}");
    assert!(started.elapsed() < std::time::Duration::from_secs(8), "took {:?}", started.elapsed());
    // Everyone else is still served.
    ctx.text(&members[1], &group.id, "まだ使える？").await;
}

#[tokio::test]
async fn toggling_a_reaction_does_not_notify_again_and_again() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let message = ctx.text(&taro, &conversation.id, "見て見て").await;
    let react = |emoji: &str| AddReactionRequest {
        message_id: message.id.clone(),
        emoji: emoji.to_owned(),
        ..Default::default()
    };
    let unreact = RemoveReactionRequest { message_id: message.id.clone(), emoji: "😂".into(), ..Default::default() };

    // Every push counts against the author's per-minute push budget: a
    // reaction switched on and off must not drown out everybody else.
    for _ in 0..5 {
        let _: AddReactionResponse = ctx.call("Message", "AddReaction", &hanako, &react("😂")).await.unwrap();
        let _: RemoveReactionResponse = ctx.call("Message", "RemoveReaction", &hanako, &unreact).await.unwrap();
    }
    let _: AddReactionResponse = ctx.call("Message", "AddReaction", &hanako, &react("👍")).await.unwrap();
    let reactions = ctx.notifications(&taro).await.into_iter().filter(|n| n.kind == "chat.reaction").count();
    assert_eq!(reactions, 1, "one person reacting to one message is told once");

    // Someone else reacting is news again.
    let jiro = ctx.app.user("jiro").await;
    ctx.befriend(&taro, &jiro).await;
    let group = ctx.group(&taro, "三人", &[&hanako, &jiro]).await;
    let in_group = ctx.text(&taro, &group.id, "集合！").await;
    for reactor in [&hanako, &jiro] {
        let request =
            AddReactionRequest { message_id: in_group.id.clone(), emoji: "🙌".into(), ..Default::default() };
        let _: AddReactionResponse = ctx.call("Message", "AddReaction", reactor, &request).await.unwrap();
    }
    let reactions = ctx.notifications(&taro).await.into_iter().filter(|n| n.kind == "chat.reaction").count();
    assert_eq!(reactions, 3);
}

#[tokio::test]
async fn messages_are_pinned_for_everyone() {
    let ctx = start().await;
    let (group, owner, members) = ctx.group_of("部活", 1).await;
    let member = &members[0];
    let message = ctx.text(&owner, &group.id, "集合は駅前に10時").await;
    let pin = |message_id: &str| PinMessageRequest { message_id: message_id.to_owned(), ..Default::default() };
    let pinned_list = ListPinnedMessagesRequest { conversation_id: group.id.clone(), ..Default::default() };

    // Any member may pin; everyone sees the pin and a line in the story.
    let mut owner_events = ctx.listen(&owner).await;
    let pinned: PinMessageResponse = ctx.call("Message", "PinMessage", member, &pin(&message.id)).await.unwrap();
    assert!(pinned.message.pinned);
    let events = ctx.events(&mut owner_events).await;
    let updates = updated_messages(&events);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].1, MessageUpdateReason::MESSAGE_UPDATE_REASON_PINNED);
    assert!(updates[0].0.pinned);
    let story = created_messages(&events);
    assert_eq!(story[0].system.r#type, SystemEventType::SYSTEM_EVENT_TYPE_MESSAGE_PINNED);
    assert_eq!(story[0].system.message_id, message.id);
    assert_eq!(story[0].system.actor.id, member.id.to_string());

    // Pinning again changes nothing.
    let mut owner_events = ctx.listen(&owner).await;
    let _: PinMessageResponse = ctx.call("Message", "PinMessage", &owner, &pin(&message.id)).await.unwrap();
    assert!(ctx.events(&mut owner_events).await.is_empty());

    let second = ctx.text(member, &group.id, "持ち物: 水筒").await;
    let _: PinMessageResponse = ctx.call("Message", "PinMessage", &owner, &pin(&second.id)).await.unwrap();
    let listed: ListPinnedMessagesResponse =
        ctx.call("Message", "ListPinnedMessages", &owner, &pinned_list).await.unwrap();
    assert_eq!(texts(&listed.messages), vec!["持ち物: 水筒", "集合は駅前に10時"], "most recently pinned first");
    assert!(listed.messages.iter().all(|message| message.pinned));

    // Unpinning.
    let mut owner_events = ctx.listen(&owner).await;
    let unpin = UnpinMessageRequest { message_id: message.id.clone(), ..Default::default() };
    let unpinned: UnpinMessageResponse = ctx.call("Message", "UnpinMessage", member, &unpin).await.unwrap();
    assert!(!unpinned.message.pinned);
    let updates = updated_messages(&ctx.events(&mut owner_events).await);
    assert_eq!(updates[0].1, MessageUpdateReason::MESSAGE_UPDATE_REASON_UNPINNED);
    let _: UnpinMessageResponse = ctx.call("Message", "UnpinMessage", member, &unpin).await.unwrap();
    let listed: ListPinnedMessagesResponse =
        ctx.call("Message", "ListPinnedMessages", member, &pinned_list).await.unwrap();
    assert_eq!(texts(&listed.messages), vec!["持ち物: 水筒"]);

    let system = ctx
        .latest(&owner, &group.id)
        .await
        .into_iter()
        .find(|message| message.kind == MessageKind::MESSAGE_KIND_SYSTEM)
        .unwrap();
    assert_eq!(ctx.reason("Message", "PinMessage", &owner, &pin(&system.id)).await, "MESSAGE_NOT_PINNABLE");
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(ctx.reason("Message", "PinMessage", &outsider, &pin(&message.id)).await, "MESSAGE_NOT_FOUND");
    assert_eq!(ctx.reason("Message", "UnpinMessage", &outsider, &unpin).await, "MESSAGE_NOT_FOUND");
    assert_eq!(ctx.reason("Message", "ListPinnedMessages", &outsider, &pinned_list).await, "CONVERSATION_NOT_FOUND");
    assert_eq!(ctx.anonymous_reason("Message", "PinMessage", &pin(&message.id)).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn shared_media_is_listed_by_kind() {
    let ctx = start().await;
    let (group, owner, members) = ctx.group_of("部活", 1).await;
    let member = &members[0];
    let (first, second) = (ctx.photo(&owner).await, ctx.photo(&owner).await);
    let photos = sent(&ctx, &owner, &media_request(&group.id, &[&first, &second])).await;
    let voice_id = ctx.upload(member, AssetPurpose::ASSET_PURPOSE_CHAT_AUDIO, AssetKind::ASSET_KIND_AUDIO).await;
    let voice = sent(&ctx, member, &media_request(&group.id, &[&voice_id])).await;
    let file_id = ctx.upload(&owner, AssetPurpose::ASSET_PURPOSE_CHAT_FILE, AssetKind::ASSET_KIND_FILE).await;
    sent(&ctx, &owner, &media_request(&group.id, &[&file_id])).await;
    let video_id = ctx.upload(member, AssetPurpose::ASSET_PURPOSE_CHAT_VIDEO, AssetKind::ASSET_KIND_VIDEO).await;
    let video = sent(&ctx, member, &media_request(&group.id, &[&video_id])).await;

    let media = |filter: SharedMediaFilter| ListSharedMediaRequest {
        conversation_id: group.id.clone(),
        filter: filter.into(),
        ..Default::default()
    };
    let assets = |response: &ListSharedMediaResponse| {
        response.items.iter().map(|item| item.attachment.asset_id.clone()).collect::<Vec<_>>()
    };

    let everything: ListSharedMediaResponse = ctx
        .call("Message", "ListSharedMedia", &owner, &media(SharedMediaFilter::SHARED_MEDIA_FILTER_UNSPECIFIED))
        .await
        .unwrap();
    assert_eq!(
        assets(&everything),
        vec![video_id.clone(), file_id.clone(), voice_id.clone(), second.clone(), first.clone()],
        "newest first"
    );
    let visual: ListSharedMediaResponse = ctx
        .call("Message", "ListSharedMedia", &owner, &media(SharedMediaFilter::SHARED_MEDIA_FILTER_VISUAL))
        .await
        .unwrap();
    assert_eq!(assets(&visual), vec![video_id.clone(), second.clone(), first.clone()]);
    assert_eq!(
        (visual.items[0].message_id.as_str(), visual.items[0].sender_id.as_str()),
        (video.id.as_str(), member.id.to_string().as_str())
    );
    assert_eq!(visual.items[1].seq, photos.seq);
    let audio: ListSharedMediaResponse = ctx
        .call("Message", "ListSharedMedia", &owner, &media(SharedMediaFilter::SHARED_MEDIA_FILTER_AUDIO))
        .await
        .unwrap();
    assert_eq!(assets(&audio), vec![voice_id.clone()]);
    assert!(!audio.items[0].attachment.waveform.is_empty());
    let files: ListSharedMediaResponse = ctx
        .call("Message", "ListSharedMedia", &owner, &media(SharedMediaFilter::SHARED_MEDIA_FILTER_FILES))
        .await
        .unwrap();
    assert_eq!(assets(&files), vec![file_id.clone()]);
    assert!(files.items[0].attachment.url.starts_with("http"));

    // In pages.
    let page = |cursor: &str| ListSharedMediaRequest {
        page: PageRequest { limit: 2, cursor: cursor.to_owned(), ..Default::default() }.into(),
        ..media(SharedMediaFilter::SHARED_MEDIA_FILTER_VISUAL)
    };
    let first_page: ListSharedMediaResponse = ctx.call("Message", "ListSharedMedia", &owner, &page("")).await.unwrap();
    assert_eq!(assets(&first_page), vec![video_id.clone(), second.clone()]);
    let second_page: ListSharedMediaResponse =
        ctx.call("Message", "ListSharedMedia", &owner, &page(&first_page.page.next_cursor)).await.unwrap();
    assert_eq!(assets(&second_page), vec![first.clone()]);
    assert!(!second_page.page.has_more);

    // Deleted messages leave the gallery; newcomers see only what came after them.
    let _: DeleteMessageResponse = ctx
        .call("Message", "DeleteMessage", member, &DeleteMessageRequest { message_id: voice.id, ..Default::default() })
        .await
        .unwrap();
    let audio: ListSharedMediaResponse = ctx
        .call("Message", "ListSharedMedia", &owner, &media(SharedMediaFilter::SHARED_MEDIA_FILTER_AUDIO))
        .await
        .unwrap();
    assert!(audio.items.is_empty());
    let newcomer = ctx.app.user("newcomer").await;
    ctx.befriend(&owner, &newcomer).await;
    let add = AddMembersRequest {
        conversation_id: group.id.clone(),
        user_ids: vec![newcomer.id.to_string()],
        ..Default::default()
    };
    let _: AddMembersResponse = ctx.call("Conversation", "AddMembers", &owner, &add).await.unwrap();
    let seen: ListSharedMediaResponse = ctx
        .call("Message", "ListSharedMedia", &newcomer, &media(SharedMediaFilter::SHARED_MEDIA_FILTER_UNSPECIFIED))
        .await
        .unwrap();
    assert!(seen.items.is_empty());
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(
        ctx.reason("Message", "ListSharedMedia", &outsider, &media(SharedMediaFilter::SHARED_MEDIA_FILTER_UNSPECIFIED))
            .await,
        "CONVERSATION_NOT_FOUND"
    );
}

#[tokio::test]
async fn messages_feed_the_best_friend_ranking_and_stamp_statistics() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    ctx.text(&taro, &conversation.id, "おはよう").await;
    ctx.text(&taro, &conversation.id, "起きてる？").await;
    let stamp_id = ctx.stamp(&hanako).await;
    sent(&ctx, &hanako, &SendMessageRequest { stamp_id: stamp_id.clone(), ..text_request(&conversation.id, "") }).await;

    // What the social domain recorded for the pair today, per kind
    // (1 = messages, 2 = stamps).
    let (lo, hi) = if taro.id < hanako.id { (taro.id, hanako.id) } else { (hanako.id, taro.id) };
    let recorded: Vec<(i16, i64)> = sqlx::query_as(
        "SELECT kind, (lo_weight + hi_weight)::bigint FROM friend_interaction_days WHERE user_lo = $1 AND user_hi = $2 ORDER BY kind",
    )
    .bind(lo)
    .bind(hi)
    .fetch_all(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(recorded, vec![(1, 2), (2, 1)]);
    let sends: i64 = sqlx::query_scalar("SELECT count(*) FROM stamp_usage WHERE stamp_id = $1::uuid AND user_id = $2")
        .bind(&stamp_id)
        .bind(hanako.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(sends, 1, "the stamp domain counts the send");

    // In a group only the people a message addresses are reported.
    let jiro = ctx.app.user("jiro").await;
    ctx.befriend(&taro, &jiro).await;
    let group = ctx.group(&taro, "三人", &[&hanako, &jiro]).await;
    ctx.text(&taro, &group.id, "みんなおはよう").await;
    ctx.text(&taro, &group.id, &format!("@{} 今日ひま？", jiro.handle)).await;
    let (lo, hi) = if taro.id < jiro.id { (taro.id, jiro.id) } else { (jiro.id, taro.id) };
    let with_jiro: i64 = sqlx::query_scalar("SELECT COALESCE(sum(lo_weight + hi_weight), 0)::bigint FROM friend_interaction_days WHERE user_lo = $1 AND user_hi = $2")
        .bind(lo)
        .bind(hi)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(with_jiro, 1);
}
