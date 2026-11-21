//! Typing indicators and co-presence (signals from the realtime gateway),
//! and `PovService`.
mod common;

use common::*;
use redis::AsyncCommands as _;
use tokuly_core::presence::{ConversationPresenceSignal, TypingSignal};
use tokuly_proto::{
    chat::{chat_event::Event as Push, *},
    media::{AssetKind, AssetPurpose},
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

fn typing(user: &TestUser, conversation_id: &str, typing: bool) -> TypingSignal {
    TypingSignal { user_id: user.id, conversation_id: conversation_id.parse().unwrap(), typing }
}

fn presence(user: &TestUser, conversation_id: &str, present: bool) -> ConversationPresenceSignal {
    ConversationPresenceSignal { user_id: user.id, conversation_id: conversation_id.parse().unwrap(), present }
}

/// The sets of present users announced among chat events.
fn presence_sets(events: &[Push]) -> Vec<Vec<String>> {
    events
        .iter()
        .filter_map(|event| match event {
            Push::Presence(changed) => Some(changed.present_user_ids.clone()),
            _ => None,
        })
        .collect()
}

/// Makes every presence recorded for a conversation look like it ran out a
/// moment ago, as if its clients had gone silent for 75 seconds.
async fn let_presence_run_out(ctx: &Ctx, conversation_id: &str) {
    let conversation_id: Uuid = conversation_id.parse().unwrap();
    let past = chrono::Utc::now().timestamp() - 1;
    let mut conn = ctx.app.core.cache.conn();
    let key = format!("chat:present:{}", conversation_id.simple());
    let members: Vec<String> = conn.zrange(&key, 0, -1).await.unwrap();
    let due_key = format!("chat:present:due:{}", ctx.app.core.cfg.nats_prefix);
    for member in members {
        let user_id: Uuid = member.parse().unwrap();
        let _: () = conn.zadd(&key, &member, past).await.unwrap();
        let _: () = conn.zadd(&due_key, format!("{}:{}", conversation_id.simple(), user_id.simple()), past).await.unwrap();
    }
}

#[tokio::test]
async fn typing_reaches_the_other_members() {
    let ctx = start().await;
    let (group, owner, members) = ctx.group_of("部活", 2).await;
    let (hanako, jiro) = (&members[0], &members[1]);
    let mut owner_events = ctx.listen(&owner).await;
    let mut hanako_events = ctx.listen(hanako).await;

    ctx.chat.handle_typing(typing(&owner, &group.id, true)).await.unwrap();
    let received = ctx.events(&mut hanako_events).await;
    assert!(matches!(
        received.as_slice(),
        [Push::Typing(event)] if event.typing && event.user_id == owner.id.to_string() && event.conversation_id == group.id
    ));
    assert!(ctx.events(&mut owner_events).await.is_empty(), "nobody is told about their own typing");
    ctx.chat.handle_typing(typing(&owner, &group.id, false)).await.unwrap();
    assert!(matches!(ctx.events(&mut hanako_events).await.as_slice(), [Push::Typing(event)] if !event.typing));

    // The gateway forwards anything: signals from people outside are dropped.
    let outsider = ctx.app.user("outsider").await;
    ctx.chat.handle_typing(typing(&outsider, &group.id, true)).await.unwrap();
    ctx.chat.handle_typing(TypingSignal { user_id: owner.id, conversation_id: Uuid::now_v7(), typing: true }).await.unwrap();
    assert!(ctx.events(&mut hanako_events).await.is_empty());

    // People in a block with the typist hear nothing from them.
    ctx.block(jiro, &owner).await;
    let mut jiro_events = ctx.listen(jiro).await;
    ctx.chat.handle_typing(typing(&owner, &group.id, true)).await.unwrap();
    assert!(ctx.events(&mut jiro_events).await.is_empty());
    assert_eq!(ctx.events(&mut hanako_events).await.len(), 1);
}

#[tokio::test]
async fn typing_needs_an_accepted_conversation_that_can_be_written_in() {
    let ctx = start().await;
    let stranger = ctx.app.user("stranger").await;
    let hanako = ctx.app.user("hanako").await;
    ctx.open_to_everyone(&hanako).await;
    let request = ctx.direct(&stranger, &hanako).await;
    ctx.text(&stranger, &request.id, "はじめまして").await;

    // Looking at a request and starting to type must not show.
    let mut stranger_events = ctx.listen(&stranger).await;
    ctx.chat.handle_typing(typing(&hanako, &request.id, true)).await.unwrap();
    assert!(ctx.events(&mut stranger_events).await.is_empty());
    // And the sender of a request does not type at someone who has not accepted.
    let mut hanako_events = ctx.listen(&hanako).await;
    ctx.chat.handle_typing(typing(&stranger, &request.id, true)).await.unwrap();
    assert!(ctx.events(&mut hanako_events).await.is_empty());
}

#[tokio::test]
async fn co_presence_shows_who_has_the_conversation_open() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let mut taro_events = ctx.listen(&taro).await;
    let mut hanako_events = ctx.listen(&hanako).await;

    ctx.chat.handle_conversation_presence(presence(&taro, &conversation.id, true)).await.unwrap();
    for events in [ctx.events(&mut taro_events).await, ctx.events(&mut hanako_events).await] {
        assert_eq!(presence_sets(&events), vec![vec![taro.id.to_string()]], "both sides learn the complete set");
    }
    assert_eq!(ctx.conversation(&hanako, &conversation.id).await.present_user_ids, vec![taro.id.to_string()]);

    // The repeated signal of an open screen changes nothing.
    ctx.chat.handle_conversation_presence(presence(&taro, &conversation.id, true)).await.unwrap();
    assert!(ctx.events(&mut hanako_events).await.is_empty());

    // Someone watching a conversation is not notified about what they see arrive.
    ctx.text(&hanako, &conversation.id, "見てる？").await;
    assert!(ctx.notifications(&taro).await.is_empty());
    assert_eq!(ctx.unread(&taro).await.badge, 1, "it still counts as unread until the client marks it read");

    ctx.chat.handle_conversation_presence(presence(&hanako, &conversation.id, true)).await.unwrap();
    let mut both = vec![taro.id.to_string(), hanako.id.to_string()];
    both.sort_unstable();
    let mut seen = ctx.conversation(&taro, &conversation.id).await.present_user_ids;
    seen.sort_unstable();
    assert_eq!(seen, both);

    // Leaving the screen.
    let mut hanako_events = ctx.listen(&hanako).await;
    ctx.chat.handle_conversation_presence(presence(&taro, &conversation.id, false)).await.unwrap();
    assert_eq!(presence_sets(&ctx.events(&mut hanako_events).await), vec![vec![hanako.id.to_string()]]);
    ctx.chat.handle_conversation_presence(presence(&taro, &conversation.id, false)).await.unwrap();
    assert!(ctx.events(&mut hanako_events).await.is_empty(), "leaving twice says nothing");
    ctx.text(&hanako, &conversation.id, "もう見てない？").await;
    assert_eq!(ctx.notification_kinds(&taro).await, vec!["chat.message"]);

    // Signals about conversations of other people are ignored.
    let outsider = ctx.app.user("outsider").await;
    ctx.chat.handle_conversation_presence(presence(&outsider, &conversation.id, true)).await.unwrap();
    assert!(ctx.events(&mut hanako_events).await.is_empty());
    assert_eq!(ctx.conversation(&taro, &conversation.id).await.present_user_ids, vec![hanako.id.to_string()]);
}

#[tokio::test]
async fn hidden_online_status_hides_co_presence_too() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    ctx.set_privacy(&taro, |privacy| privacy.share_online_status = false).await;

    let mut hanako_events = ctx.listen(&hanako).await;
    ctx.chat.handle_conversation_presence(presence(&taro, &conversation.id, true)).await.unwrap();
    assert!(ctx.events(&mut hanako_events).await.is_empty());
    assert!(ctx.conversation(&hanako, &conversation.id).await.present_user_ids.is_empty());
    // Not shown, but still not bothered with notifications for what is on screen.
    ctx.text(&hanako, &conversation.id, "いる？").await;
    assert!(ctx.notifications(&taro).await.is_empty());
}

#[tokio::test]
async fn presence_runs_out_when_a_client_goes_silent() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    ctx.chat.handle_conversation_presence(presence(&taro, &conversation.id, true)).await.unwrap();
    assert_eq!(ctx.conversation(&hanako, &conversation.id).await.present_user_ids.len(), 1);
    // Nothing is due yet.
    let mut hanako_events = ctx.listen(&hanako).await;
    ctx.chat.sweep_presence().await.unwrap();
    assert!(ctx.events(&mut hanako_events).await.is_empty());

    // No goodbye and no repeated signal for 75 seconds: the app crashed.
    let_presence_run_out(&ctx, &conversation.id).await;
    assert!(ctx.conversation(&hanako, &conversation.id).await.present_user_ids.is_empty(), "stale entries never count");
    ctx.chat.sweep_presence().await.unwrap();
    assert_eq!(presence_sets(&ctx.events(&mut hanako_events).await), vec![Vec::<String>::new()], "the sweeper tells the others");
    ctx.text(&hanako, &conversation.id, "落ちた？").await;
    assert_eq!(ctx.notification_kinds(&taro).await, vec!["chat.message"]);

    // Coming back after having run out is announced again.
    ctx.chat.handle_conversation_presence(presence(&taro, &conversation.id, true)).await.unwrap();
    assert_eq!(presence_sets(&ctx.events(&mut hanako_events).await), vec![vec![taro.id.to_string()]]);
}

// ───────────────────────────── POV ─────────────────────────────

fn pov_request(conversation_id: &str, note: &str) -> RequestPovRequest {
    RequestPovRequest { conversation_id: conversation_id.to_owned(), note: note.to_owned(), ..Default::default() }
}

fn pov_answer(request_id: &str, asset_ids: &[&str]) -> AnswerPovRequest {
    AnswerPovRequest { request_id: request_id.to_owned(), attachment_asset_ids: asset_ids.iter().map(|id| id.to_string()).collect(), client_id: client_id(), ..Default::default() }
}

async fn ask(ctx: &Ctx, requester: &TestUser, conversation_id: &str) -> RequestPovResponse {
    ctx.call("Pov", "RequestPov", requester, &pov_request(conversation_id, "今どこ？")).await.expect("request pov")
}

#[tokio::test]
async fn a_friend_is_asked_for_their_pov_and_answers() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let mut hanako_events = ctx.listen(&hanako).await;

    let asked = ask(&ctx, &taro, &conversation.id).await;
    let request = asked.request.as_option().unwrap();
    assert_eq!(request.state, PovState::POV_STATE_OPEN);
    assert_eq!((request.requester_id.as_str(), request.target_id.as_str()), (taro.id.to_string().as_str(), hanako.id.to_string().as_str()));
    assert_eq!(request.note, "今どこ？");
    let answer_window = request.expires_at.seconds - request.created_at.seconds;
    assert!((7195..=7205).contains(&answer_window), "two hours to answer, got {answer_window}s");
    assert_eq!(asked.message.kind, MessageKind::MESSAGE_KIND_POV_REQUEST);
    assert_eq!(asked.message.pov_request.id, request.id);
    assert_eq!(request.request_message_id, asked.message.id);
    assert_eq!(created_messages(&ctx.events(&mut hanako_events).await)[0].pov_request.id, request.id);
    let notification = ctx.notifications(&hanako).await.pop().unwrap();
    assert_eq!(notification.kind, "chat.pov");
    assert!(notification.push && notification.inbox);
    assert_eq!(ctx.conversation(&hanako, &conversation.id).await.open_pov_request.id, request.id);

    // One open request per conversation, whoever asks.
    for user in [&taro, &hanako] {
        assert_eq!(ctx.reason("Pov", "RequestPov", user, &pov_request(&conversation.id, "")).await, "POV_ALREADY_OPEN");
    }

    // The answer is a media message tied to the request.
    let mut taro_events = ctx.listen(&taro).await;
    let photo = ctx.photo(&hanako).await;
    let answer = AnswerPovRequest { text: "カフェにいるよ".into(), ..pov_answer(&request.id, &[&photo]) };
    let answered: AnswerPovResponse = ctx.call("Pov", "AnswerPov", &hanako, &answer).await.unwrap();
    assert_eq!(answered.request.state, PovState::POV_STATE_ANSWERED);
    assert_eq!(answered.request.reply_message_id, answered.message.id);
    assert!(answered.request.answered_at.is_set() && !answered.request.saved);
    let lifetime = answered.request.reply_expires_at.seconds - answered.request.answered_at.seconds;
    assert!((86395..=86405).contains(&lifetime), "24 hours to save it, got {lifetime}s");
    assert_eq!(answered.message.kind, MessageKind::MESSAGE_KIND_MEDIA);
    assert_eq!(answered.message.text, "カフェにいるよ");
    assert_eq!(answered.message.attachments[0].asset_id, photo);
    let reply = answered.message.pov_reply.as_option().expect("marked as a POV answer");
    assert_eq!(reply.request_id, request.id);
    assert!(reply.expires_at.is_set() && !reply.saved && !reply.expired);

    let events = ctx.events(&mut taro_events).await;
    assert_eq!(created_messages(&events)[0].id, answered.message.id);
    let updates = updated_messages(&events);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].1, MessageUpdateReason::MESSAGE_UPDATE_REASON_POV);
    assert_eq!(updates[0].0.pov_request.state, PovState::POV_STATE_ANSWERED);
    assert_eq!(ctx.notification_kinds(&taro).await, vec!["chat.pov"]);
    assert!(ctx.conversation(&taro, &conversation.id).await.open_pov_request.is_unset());

    // A retried answer returns the stored one; a second answer is too late.
    let retried: AnswerPovResponse = ctx.call("Pov", "AnswerPov", &hanako, &answer).await.unwrap();
    assert_eq!(retried.message.id, answered.message.id);
    let another = ctx.photo(&hanako).await;
    assert_eq!(ctx.reason("Pov", "AnswerPov", &hanako, &pov_answer(&request.id, &[&another])).await, "POV_NOT_OPEN");
    // The conversation is free for the next request.
    let next = ask(&ctx, &hanako, &conversation.id).await;
    assert_eq!(next.request.target_id, taro.id.to_string());
}

#[tokio::test]
async fn pov_requests_and_answers_are_validated() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let outsider = ctx.app.user("outsider").await;

    assert_eq!(ctx.reason("Pov", "RequestPov", &taro, &pov_request(&conversation.id, &"あ".repeat(201))).await, "NOTE_TOO_LONG");
    assert_eq!(ctx.reason("Pov", "RequestPov", &outsider, &pov_request(&conversation.id, "")).await, "CONVERSATION_NOT_FOUND");
    assert_eq!(ctx.anonymous_reason("Pov", "RequestPov", &pov_request(&conversation.id, "")).await, "SIGN_IN_REQUIRED");
    let (group, owner, _) = ctx.group_of("部活", 1).await;
    assert_eq!(ctx.reason("Pov", "RequestPov", &owner, &pov_request(&group.id, "")).await, "NOT_A_DIRECT_CONVERSATION");
    // POV is between friends.
    let acquaintance = ctx.app.user("acquaintance").await;
    let loose = ctx.chat.open_direct(taro.id, acquaintance.id).await.unwrap().to_string();
    assert_eq!(ctx.reason("Pov", "RequestPov", &taro, &pov_request(&loose, "")).await, "NOT_FRIENDS");
    // Nobody asks across a block.
    let (blocker, blocked) = ctx.friends("blocker", "blocked").await;
    let cut = ctx.direct(&blocker, &blocked).await;
    ctx.block(&blocker, &blocked).await;
    assert_eq!(ctx.reason("Pov", "RequestPov", &blocked, &pov_request(&cut.id, "")).await, "CONVERSATION_READ_ONLY");

    // An upload from before the request is not "right now".
    let old_photo = ctx.photo(&hanako).await;
    let asked = ask(&ctx, &taro, &conversation.id).await;
    let request_id = asked.request.id.clone();
    assert_eq!(ctx.reason("Pov", "AnswerPov", &hanako, &pov_answer(&request_id, &[&old_photo])).await, "POV_NOT_LIVE");

    let photo = ctx.photo(&hanako).await;
    assert_eq!(ctx.reason("Pov", "AnswerPov", &taro, &pov_answer(&request_id, &[&photo])).await, "NOT_ALLOWED", "only the person asked answers");
    assert_eq!(ctx.reason("Pov", "AnswerPov", &outsider, &pov_answer(&request_id, &[&photo])).await, "POV_NOT_FOUND");
    assert_eq!(ctx.reason("Pov", "AnswerPov", &hanako, &pov_answer(&Uuid::now_v7().to_string(), &[&photo])).await, "POV_NOT_FOUND");
    assert_eq!(ctx.reason("Pov", "AnswerPov", &hanako, &pov_answer(&request_id, &[])).await, "NO_ATTACHMENTS");
    let five: Vec<String> = (0..5).map(|_| Uuid::now_v7().to_string()).collect();
    assert_eq!(ctx.reason("Pov", "AnswerPov", &hanako, &pov_answer(&request_id, &five.iter().map(String::as_str).collect::<Vec<_>>())).await, "TOO_MANY_ATTACHMENTS");
    let voice = ctx.upload(&hanako, AssetPurpose::ASSET_PURPOSE_CHAT_AUDIO, AssetKind::ASSET_KIND_AUDIO).await;
    assert_eq!(ctx.reason("Pov", "AnswerPov", &hanako, &pov_answer(&request_id, &[&voice])).await, "ASSET_WRONG_PURPOSE");
    let taros_photo = ctx.photo(&taro).await;
    assert_eq!(ctx.reason("Pov", "AnswerPov", &hanako, &pov_answer(&request_id, &[&taros_photo])).await, "ASSET_NOT_FOUND");
    let no_key = AnswerPovRequest { client_id: String::new(), ..pov_answer(&request_id, &[&photo]) };
    assert_eq!(ctx.reason("Pov", "AnswerPov", &hanako, &no_key).await, "CLIENT_ID_INVALID");
    assert_eq!(ctx.anonymous_reason("Pov", "AnswerPov", &pov_answer(&request_id, &[&photo])).await, "SIGN_IN_REQUIRED");

    // The request message itself is part of the story.
    let delete = DeleteMessageRequest { message_id: asked.message.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Message", "DeleteMessage", &taro, &delete).await, "MESSAGE_NOT_DELETABLE");
    let pin = PinMessageRequest { message_id: asked.message.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Message", "PinMessage", &taro, &pin).await, "MESSAGE_NOT_PINNABLE");
}

#[tokio::test]
async fn pov_requests_are_declined_cancelled_or_run_out() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let by_id = |request_id: &str| (DeclinePovRequest { request_id: request_id.to_owned(), ..Default::default() }, CancelPovRequest { request_id: request_id.to_owned(), ..Default::default() });

    // Declined by the person asked.
    let asked = ask(&ctx, &taro, &conversation.id).await;
    let (decline, cancel) = by_id(&asked.request.id);
    assert_eq!(ctx.reason("Pov", "DeclinePov", &taro, &decline).await, "NOT_ALLOWED");
    assert_eq!(ctx.reason("Pov", "CancelPov", &hanako, &cancel).await, "NOT_ALLOWED");
    let mut taro_events = ctx.listen(&taro).await;
    let declined: DeclinePovResponse = ctx.call("Pov", "DeclinePov", &hanako, &decline).await.unwrap();
    assert_eq!(declined.request.state, PovState::POV_STATE_DECLINED);
    let updates = updated_messages(&ctx.events(&mut taro_events).await);
    assert_eq!(updates[0].0.pov_request.state, PovState::POV_STATE_DECLINED);
    assert_eq!(ctx.reason("Pov", "DeclinePov", &hanako, &decline).await, "POV_NOT_OPEN");
    assert_eq!(ctx.reason("Pov", "CancelPov", &taro, &cancel).await, "POV_NOT_OPEN");

    // Withdrawn by the requester.
    let asked = ask(&ctx, &taro, &conversation.id).await;
    let (_, cancel) = by_id(&asked.request.id);
    let cancelled: CancelPovResponse = ctx.call("Pov", "CancelPov", &taro, &cancel).await.unwrap();
    assert_eq!(cancelled.request.state, PovState::POV_STATE_CANCELLED);
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(ctx.reason("Pov", "CancelPov", &outsider, &cancel).await, "POV_NOT_FOUND");

    // Nobody answers within two hours.
    let asked = ask(&ctx, &taro, &conversation.id).await;
    let (decline, _) = by_id(&asked.request.id);
    sqlx::query("UPDATE chat_pov_requests SET expires_at = now() - interval '1 second' WHERE id = $1::uuid").bind(&asked.request.id).execute(&ctx.app.core.db).await.unwrap();
    let photo = ctx.photo(&hanako).await;
    assert_eq!(ctx.reason("Pov", "AnswerPov", &hanako, &pov_answer(&asked.request.id, &[&photo])).await, "POV_NOT_OPEN");
    assert_eq!(ctx.reason("Pov", "DeclinePov", &hanako, &decline).await, "POV_NOT_OPEN");
    assert!(ctx.conversation(&taro, &conversation.id).await.open_pov_request.is_unset());
    let mut taro_events = ctx.listen(&taro).await;
    assert!(ctx.chat.expire_povs().await.unwrap() >= 1);
    let updates = updated_messages(&ctx.events(&mut taro_events).await);
    assert!(updates.iter().any(|(message, _)| message.id == asked.message.id && message.pov_request.state == PovState::POV_STATE_EXPIRED));

    // A request whose time is up does not block the next one, worker or not.
    let stale = ask(&ctx, &taro, &conversation.id).await;
    sqlx::query("UPDATE chat_pov_requests SET expires_at = now() - interval '1 second' WHERE id = $1::uuid").bind(&stale.request.id).execute(&ctx.app.core.db).await.unwrap();
    let fresh = ask(&ctx, &hanako, &conversation.id).await;
    assert_eq!(fresh.request.state, PovState::POV_STATE_OPEN);
    assert_eq!(ctx.message(&taro, &stale.message.id).await.pov_request.state, PovState::POV_STATE_EXPIRED);
}

#[tokio::test]
async fn an_answer_disappears_unless_the_requester_saves_it() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let save = |request_id: &str| SavePovRequest { request_id: request_id.to_owned(), ..Default::default() };
    let expire_reply = |request_id: String| {
        let db = ctx.app.core.db.clone();
        async move {
            sqlx::query("UPDATE chat_pov_requests SET reply_expires_at = now() - interval '1 second' WHERE id = $1::uuid").bind(request_id).execute(&db).await.unwrap();
        }
    };

    // Saved: kept for good, and the person who answered is told.
    let asked = ask(&ctx, &taro, &conversation.id).await;
    assert_eq!(ctx.reason("Pov", "SavePov", &taro, &save(&asked.request.id)).await, "POV_NOT_ANSWERED");
    let photo = ctx.photo(&hanako).await;
    let answered: AnswerPovResponse = ctx.call("Pov", "AnswerPov", &hanako, &pov_answer(&asked.request.id, &[&photo])).await.unwrap();
    assert_eq!(ctx.reason("Pov", "SavePov", &hanako, &save(&asked.request.id)).await, "NOT_ALLOWED", "only the requester saves");
    let mut hanako_events = ctx.listen(&hanako).await;
    let saved: SavePovResponse = ctx.call("Pov", "SavePov", &taro, &save(&asked.request.id)).await.unwrap();
    assert!(saved.request.saved && saved.request.reply_expires_at.is_unset());
    let updates = updated_messages(&ctx.events(&mut hanako_events).await);
    assert_eq!(updates.len(), 2, "the request and the answer both show it");
    assert!(updates.iter().any(|(message, _)| message.id == answered.message.id && message.pov_reply.saved && message.pov_reply.expires_at.is_unset()));
    let notification = ctx.notifications(&hanako).await.pop().unwrap();
    assert_eq!(notification.kind, "chat.pov");
    assert_eq!(notification.data["event"], "saved");
    let again: SavePovResponse = ctx.call("Pov", "SavePov", &taro, &save(&asked.request.id)).await.unwrap();
    assert!(again.request.saved);
    assert_eq!(ctx.notifications(&hanako).await.iter().filter(|n| n.data["event"] == "saved").count(), 1, "saving twice tells once");
    expire_reply(asked.request.id.clone()).await;
    ctx.chat.expire_povs().await.unwrap();
    assert_eq!(ctx.message(&taro, &answered.message.id).await.attachments.len(), 1, "a saved answer never expires");

    // Not saved: after 24 hours the content is gone for both.
    let asked = ask(&ctx, &taro, &conversation.id).await;
    let photo = ctx.photo(&hanako).await;
    let answer = AnswerPovRequest { text: "見て！".into(), ..pov_answer(&asked.request.id, &[&photo]) };
    let answered: AnswerPovResponse = ctx.call("Pov", "AnswerPov", &hanako, &answer).await.unwrap();
    // Not something to pass around or pin while it lasts.
    let forward = ForwardMessagesRequest { message_ids: vec![answered.message.id.clone()], conversation_ids: vec![conversation.id.clone()], client_id: client_id(), ..Default::default() };
    assert_eq!(ctx.reason("Message", "ForwardMessages", &taro, &forward).await, "MESSAGE_NOT_FORWARDABLE");
    let pin = PinMessageRequest { message_id: answered.message.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Message", "PinMessage", &taro, &pin).await, "MESSAGE_NOT_PINNABLE");

    expire_reply(asked.request.id.clone()).await;
    let mut taro_events = ctx.listen(&taro).await;
    assert!(ctx.chat.expire_povs().await.unwrap() >= 1);
    let updates = updated_messages(&ctx.events(&mut taro_events).await);
    assert!(updates.iter().any(|(message, reason)| message.id == answered.message.id && message.pov_reply.expired && *reason == MessageUpdateReason::MESSAGE_UPDATE_REASON_POV));
    let gone = ctx.message(&hanako, &answered.message.id).await;
    assert!(gone.attachments.is_empty() && gone.text.is_empty() && gone.pov_reply.expired && !gone.deleted);
    assert!(ctx.message(&taro, &asked.message.id).await.pov_request.reply_expired);
    assert!(!tokuly_media::assets::is_attached(&ctx.app.core, photo.parse().unwrap()).await.unwrap(), "the upload is released");
    assert_eq!(ctx.reason("Pov", "SavePov", &taro, &save(&asked.request.id)).await, "POV_REPLY_EXPIRED");
    let media: ListSharedMediaResponse =
        ctx.call("Message", "ListSharedMedia", &taro, &ListSharedMediaRequest { conversation_id: conversation.id.clone(), ..Default::default() }).await.unwrap();
    assert_eq!(media.items.len(), 1, "only the saved POV is left in the gallery");

    // Deleting an answer counts as gone as well.
    let asked = ask(&ctx, &taro, &conversation.id).await;
    let photo = ctx.photo(&hanako).await;
    let answered: AnswerPovResponse = ctx.call("Pov", "AnswerPov", &hanako, &pov_answer(&asked.request.id, &[&photo])).await.unwrap();
    let _: DeleteMessageResponse = ctx.call("Message", "DeleteMessage", &hanako, &DeleteMessageRequest { message_id: answered.message.id, ..Default::default() }).await.unwrap();
    assert_eq!(ctx.reason("Pov", "SavePov", &taro, &save(&asked.request.id)).await, "POV_REPLY_EXPIRED");
}
