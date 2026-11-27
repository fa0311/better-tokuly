//! `CallService`, against the LiveKit server of the development
//! infrastructure (`TOKULY_LIVEKIT_*`, exported by `tools/dev/infra.sh env`).
//!
//! Nobody connects any media in these tests: they exercise the call state
//! machine, the tokens and the room-service calls.
mod common;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use common::*;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use tokuly_chat::{ChatConfig, livekit::Claims};
use tokuly_proto::chat::{chat_event::Event as Push, *};
use tokuly_testkit::TestUser;
use uuid::Uuid;

/// Chat with calls on the development LiveKit server (and no AI assistant).
fn livekit_config() -> ChatConfig {
    let cfg = ChatConfig::from_env();
    assert!(
        cfg.calls_enabled(),
        "these tests need the LiveKit settings of the dev infrastructure: eval \"$(tools/dev/infra.sh env)\""
    );
    ChatConfig { anthropic_api_key: String::new(), ..cfg }
}

fn start_request(conversation_id: &str, video: bool) -> StartCallRequest {
    StartCallRequest { conversation_id: conversation_id.to_owned(), video, ..Default::default() }
}

async fn start_call(ctx: &Ctx, caller: &TestUser, conversation_id: &str, video: bool) -> StartCallResponse {
    ctx.call("Call", "StartCall", caller, &start_request(conversation_id, video)).await.expect("start call")
}

async fn join(ctx: &Ctx, user: &TestUser, call_id: &str) -> JoinCallResponse {
    ctx.call("Call", "JoinCall", user, &JoinCallRequest { call_id: call_id.to_owned(), ..Default::default() })
        .await
        .expect("join call")
}

async fn leave(ctx: &Ctx, user: &TestUser, call_id: &str) -> Call {
    let response: LeaveCallResponse = ctx
        .call("Call", "LeaveCall", user, &LeaveCallRequest { call_id: call_id.to_owned(), ..Default::default() })
        .await
        .expect("leave call");
    response.call.as_option().expect("call").clone()
}

async fn decline(ctx: &Ctx, user: &TestUser, call_id: &str) -> Call {
    let response: DeclineCallResponse = ctx
        .call("Call", "DeclineCall", user, &DeclineCallRequest { call_id: call_id.to_owned(), ..Default::default() })
        .await
        .expect("decline call");
    response.call.as_option().expect("call").clone()
}

async fn get_call(ctx: &Ctx, user: &TestUser, call_id: &str) -> Call {
    let response: GetCallResponse = ctx
        .call("Call", "GetCall", user, &GetCallRequest { call_id: call_id.to_owned(), ..Default::default() })
        .await
        .expect("get call");
    response.call.as_option().expect("call").clone()
}

fn state_of(call: &Call, user: &TestUser) -> Option<CallParticipantState> {
    call.participants
        .iter()
        .find(|participant| participant.user.id == user.id.to_string())
        .and_then(|participant| participant.state.as_known())
}

/// The calls announced among chat events, in order.
fn call_changes(events: &[Push]) -> Vec<Call> {
    events
        .iter()
        .filter_map(|event| match event {
            Push::Call(changed) => changed.call.as_option().cloned(),
            _ => None,
        })
        .collect()
}

/// Makes a call's ringing time look like it is up.
async fn let_ringing_run_out(ctx: &Ctx, call_id: &str) {
    sqlx::query("UPDATE chat_calls SET ring_expires_at = now() - interval '1 second' WHERE id = $1::uuid")
        .bind(call_id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn calls_are_disabled_without_livekit() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let call_id = Uuid::now_v7().to_string();

    let error = ctx
        .call::<_, StartCallResponse>("Call", "StartCall", &taro, &start_request(&direct.id, false))
        .await
        .unwrap_err();
    assert_eq!((error.reason.as_str(), error.code.as_str()), ("CALLS_DISABLED", "unimplemented"));
    assert_eq!(
        ctx.reason("Call", "JoinCall", &taro, &JoinCallRequest { call_id: call_id.clone(), ..Default::default() })
            .await,
        "CALLS_DISABLED"
    );
    assert_eq!(
        ctx.reason(
            "Call",
            "DeclineCall",
            &taro,
            &DeclineCallRequest { call_id: call_id.clone(), ..Default::default() }
        )
        .await,
        "CALLS_DISABLED"
    );
    assert_eq!(
        ctx.reason("Call", "LeaveCall", &taro, &LeaveCallRequest { call_id: call_id.clone(), ..Default::default() })
            .await,
        "CALLS_DISABLED"
    );
    // The history needs no media server.
    let history: ListCallsResponse = ctx.call("Call", "ListCalls", &taro, &ListCallsRequest::default()).await.unwrap();
    assert!(history.calls.is_empty());
    assert_eq!(
        ctx.reason("Call", "GetCall", &taro, &GetCallRequest { call_id, ..Default::default() }).await,
        "CALL_NOT_FOUND"
    );
    assert_eq!(ctx.anonymous_reason("Call", "ListCalls", &ListCallsRequest::default()).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn livekit_accepts_the_tokens_minted_here() {
    let cfg = livekit_config();
    let ctx = start_with(cfg.clone()).await;
    let livekit = ctx.chat.livekit().expect("calls are configured");
    let room = format!("tokuly-test-{}", Uuid::now_v7().simple());

    // The room service, called through LiveKit's Twirp HTTP API with our own tokens.
    let created = livekit.create_room(&room, 4).await.expect("create room");
    assert_eq!(created.name, room);
    assert!(!created.sid.is_empty());
    assert_eq!(livekit.list_rooms(std::slice::from_ref(&room)).await.expect("list rooms"), vec![room.clone()]);
    assert!(livekit.list_participants(&room).await.expect("list participants").is_empty(), "nobody connected");
    livekit.remove_participant(&room, "nobody").await.expect("removing someone who is not there is fine");
    livekit.delete_room(&room).await.expect("delete room");
    assert!(livekit.list_rooms(std::slice::from_ref(&room)).await.unwrap().is_empty());
    assert!(livekit.list_participants(&room).await.unwrap().is_empty(), "nobody is in a room that is gone");
    livekit.delete_room(&room).await.expect("deleting twice is fine");

    // A token signed with another secret is turned away.
    let forged = ChatConfig { livekit_api_secret: "not-the-secret-livekit-was-started-with".into(), ..cfg };
    let impostor = tokuly_chat::livekit::LiveKit::from_config(&forged).unwrap().unwrap();
    let refused = impostor.create_room(&room, 4).await.unwrap_err();
    assert!(refused.is_unauthorized(), "{refused}");
}

#[tokio::test]
async fn a_direct_call_rings_is_answered_and_ends() {
    let cfg = livekit_config();
    let ctx = start_with(cfg.clone()).await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let mut hanako_events = ctx.listen(&hanako).await;

    let started = start_call(&ctx, &taro, &direct.id, false).await;
    let call = started.call.as_option().unwrap();
    assert_eq!(call.state, CallState::CALL_STATE_RINGING);
    assert_eq!(call.started_by.id, taro.id.to_string());
    assert!(!call.video && call.answered_at.is_unset() && call.ended_at.is_unset());
    assert_eq!(state_of(call, &taro), Some(CallParticipantState::CALL_PARTICIPANT_STATE_JOINED));
    assert_eq!(state_of(call, &hanako), Some(CallParticipantState::CALL_PARTICIPANT_STATE_RINGING));
    let ring_time = call.ring_expires_at.seconds - call.started_at.seconds;
    assert!((43..=47).contains(&ring_time), "rings for 45 seconds, got {ring_time}");

    // The caller gets what a LiveKit client needs to connect.
    let credentials = started.credentials.as_option().unwrap();
    assert_eq!(credentials.url, cfg.livekit_public_url);
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_issuer(&[cfg.livekit_api_key.as_str()]);
    let claims = jsonwebtoken::decode::<Claims>(
        &credentials.token,
        &DecodingKey::from_secret(cfg.livekit_api_secret.as_bytes()).unwrap(),
        &validation,
    )
    .unwrap()
    .claims;
    assert_eq!(claims.sub, taro.id.to_string(), "the participant identity is the user");
    assert_eq!(claims.name, "taro");
    assert_eq!(claims.video.room, credentials.room);
    assert!(claims.video.room_join && !claims.video.room_admin && !claims.video.room_create);
    assert!(credentials.expires_at.seconds > chrono::Utc::now().timestamp());
    let livekit = ctx.chat.livekit().unwrap();
    assert_eq!(
        livekit.list_rooms(std::slice::from_ref(&credentials.room)).await.unwrap().len(),
        1,
        "the room is prepared"
    );

    // The callee: a call entry in the conversation, a ringing event, an urgent push.
    let events = ctx.events(&mut hanako_events).await;
    let entry = &created_messages(&events)[0];
    assert_eq!(entry.kind, MessageKind::MESSAGE_KIND_CALL);
    assert_eq!(entry.id, call.message_id);
    assert_eq!(
        (entry.call.call_id.as_str(), entry.call.state.as_known()),
        (call.id.as_str(), Some(CallState::CALL_STATE_RINGING))
    );
    let ringing = call_changes(&events);
    assert_eq!(ringing.len(), 1);
    assert_eq!(state_of(&ringing[0], &hanako), Some(CallParticipantState::CALL_PARTICIPANT_STATE_RINGING));
    let notification = ctx.notifications(&hanako).await.pop().unwrap();
    assert_eq!(notification.kind, "chat.call");
    assert!(
        notification.urgent && notification.push && !notification.inbox,
        "a ringing call breaks through quiet hours"
    );
    assert_eq!(notification.data["call_id"], call.id);
    let screen = ctx.conversation(&hanako, &direct.id).await;
    assert_eq!(screen.conversation.active_call_id, call.id);
    assert_eq!(screen.active_call.id, call.id);
    assert_eq!(screen.conversation.viewer.unread_count, 1);

    // One live call per conversation.
    for user in [&taro, &hanako] {
        assert_eq!(ctx.reason("Call", "StartCall", user, &start_request(&direct.id, true)).await, "CALL_IN_PROGRESS");
    }

    // Answered.
    let mut taro_events = ctx.listen(&taro).await;
    let joined = join(&ctx, &hanako, &call.id).await;
    assert_eq!(joined.call.state, CallState::CALL_STATE_ACTIVE);
    assert!(joined.call.answered_at.is_set());
    assert_eq!(state_of(&joined.call, &hanako), Some(CallParticipantState::CALL_PARTICIPANT_STATE_JOINED));
    assert_eq!(joined.credentials.room, credentials.room);
    let events = ctx.events(&mut taro_events).await;
    assert_eq!(call_changes(&events)[0].state, CallState::CALL_STATE_ACTIVE);
    let updates = updated_messages(&events);
    assert_eq!(updates[0].1, MessageUpdateReason::MESSAGE_UPDATE_REASON_CALL);
    assert_eq!(updates[0].0.call.state, CallState::CALL_STATE_ACTIVE);
    assert_eq!(
        ctx.reason(
            "Call",
            "DeclineCall",
            &hanako,
            &DeclineCallRequest { call_id: call.id.clone(), ..Default::default() }
        )
        .await,
        "ALREADY_JOINED"
    );
    // Joining again (another device, a reconnect) hands out fresh credentials.
    assert!(!join(&ctx, &hanako, &call.id).await.credentials.token.is_empty());

    // They talk for three minutes, then one side hangs up.
    sqlx::query("UPDATE chat_calls SET answered_at = now() - interval '3 minutes' WHERE id = $1::uuid")
        .bind(&call.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    sqlx::query("UPDATE chat_call_participants SET joined_at = now() - interval '3 minutes' WHERE call_id = $1::uuid")
        .bind(&call.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let mut hanako_events = ctx.listen(&hanako).await;
    let ended = leave(&ctx, &taro, &call.id).await;
    assert_eq!(
        (ended.state.as_known(), ended.end_reason.as_known()),
        (Some(CallState::CALL_STATE_ENDED), Some(CallEndReason::CALL_END_REASON_COMPLETED))
    );
    assert!((178..=185).contains(&ended.duration_seconds), "about three minutes, got {}s", ended.duration_seconds);
    assert!(
        ended
            .participants
            .iter()
            .all(|participant| participant.state == CallParticipantState::CALL_PARTICIPANT_STATE_LEFT)
    );
    let events = ctx.events(&mut hanako_events).await;
    assert_eq!(call_changes(&events)[0].state, CallState::CALL_STATE_ENDED);
    let summary = &updated_messages(&events)[0].0.call;
    assert_eq!(summary.end_reason, CallEndReason::CALL_END_REASON_COMPLETED);
    assert!(summary.duration_seconds >= 178, "the timeline entry shows how long the call was");
    assert!(
        livekit.list_rooms(std::slice::from_ref(&credentials.room)).await.unwrap().is_empty(),
        "the room is closed"
    );
    assert!(ctx.conversation(&taro, &direct.id).await.conversation.active_call_id.is_empty());
    let finished = ctx.app.events_by("chat.call.ended", taro.id).await;
    assert_eq!(finished[0].data["end_reason"], CallEndReason::CALL_END_REASON_COMPLETED as i32);

    // Minutes on the phone count towards the best-friend ranking (kind 6).
    let (lo, hi) = if taro.id < hanako.id { (taro.id, hanako.id) } else { (hanako.id, taro.id) };
    let minutes: i64 = sqlx::query_scalar(
        "SELECT COALESCE(sum(lo_weight + hi_weight), 0)::bigint FROM friend_interaction_days WHERE user_lo = $1 AND user_hi = $2 AND kind = 6",
    )
    .bind(lo)
    .bind(hi)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert!((3..=4).contains(&minutes), "{minutes} minutes");

    // An ended call is history.
    assert_eq!(leave(&ctx, &hanako, &call.id).await.state, CallState::CALL_STATE_ENDED, "hanging up twice is fine");
    assert_eq!(
        ctx.reason("Call", "JoinCall", &hanako, &JoinCallRequest { call_id: call.id.clone(), ..Default::default() })
            .await,
        "CALL_ENDED"
    );
    assert_eq!(
        ctx.reason(
            "Call",
            "DeclineCall",
            &hanako,
            &DeclineCallRequest { call_id: call.id.clone(), ..Default::default() }
        )
        .await,
        "CALL_ENDED"
    );
    for user in [&taro, &hanako] {
        let history: ListCallsResponse =
            ctx.call("Call", "ListCalls", user, &ListCallsRequest::default()).await.unwrap();
        assert_eq!(history.calls.iter().map(|call| call.id.as_str()).collect::<Vec<_>>(), vec![call.id.as_str()]);
    }
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(
        ctx.reason("Call", "GetCall", &outsider, &GetCallRequest { call_id: call.id.clone(), ..Default::default() })
            .await,
        "CALL_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason("Call", "JoinCall", &outsider, &JoinCallRequest { call_id: call.id.clone(), ..Default::default() })
            .await,
        "CALL_NOT_FOUND"
    );
    // And the conversation is free for the next call.
    let next = start_call(&ctx, &hanako, &direct.id, true).await;
    assert!(next.call.video);
    leave(&ctx, &hanako, &next.call.id).await;
}

#[tokio::test]
async fn an_unanswered_call_is_declined_cancelled_or_missed() {
    let ctx = start_with(livekit_config()).await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let outcome = |call: &Call| (call.state.as_known().unwrap(), call.end_reason.as_known().unwrap());

    // Declined by the callee.
    let call_id = start_call(&ctx, &taro, &direct.id, false).await.call.id.clone();
    let mut taro_events = ctx.listen(&taro).await;
    let declined = decline(&ctx, &hanako, &call_id).await;
    assert_eq!(outcome(&declined), (CallState::CALL_STATE_ENDED, CallEndReason::CALL_END_REASON_DECLINED));
    assert_eq!(state_of(&declined, &hanako), Some(CallParticipantState::CALL_PARTICIPANT_STATE_DECLINED));
    assert_eq!(declined.duration_seconds, 0);
    let events = ctx.events(&mut taro_events).await;
    assert_eq!(call_changes(&events)[0].end_reason, CallEndReason::CALL_END_REASON_DECLINED);
    assert_eq!(updated_messages(&events)[0].0.call.end_reason, CallEndReason::CALL_END_REASON_DECLINED);
    assert_eq!(ctx.notifications(&hanako).await.len(), 1, "declining a call is not missing it");

    // Cancelled by the caller before anyone answered: a missed call for the callee.
    let call_id = start_call(&ctx, &taro, &direct.id, true).await.call.id.clone();
    let cancelled = leave(&ctx, &taro, &call_id).await;
    assert_eq!(outcome(&cancelled), (CallState::CALL_STATE_ENDED, CallEndReason::CALL_END_REASON_CANCELLED));
    assert_eq!(state_of(&cancelled, &hanako), Some(CallParticipantState::CALL_PARTICIPANT_STATE_MISSED));
    let notifications = ctx.notifications(&hanako).await;
    let (rang, missed) = (&notifications[notifications.len() - 2], &notifications[notifications.len() - 1]);
    assert!(rang.urgent && !missed.urgent && missed.inbox, "the missed call stays in the inbox");
    assert_eq!(missed.collapse_key, rang.collapse_key, "and replaces the ringing notification");
    assert_eq!(missed.data["missed"], true);
    let missed_only: ListCallsResponse = ctx
        .call("Call", "ListCalls", &hanako, &ListCallsRequest { missed_only: true, ..Default::default() })
        .await
        .unwrap();
    assert_eq!(missed_only.calls.iter().map(|call| call.id.as_str()).collect::<Vec<_>>(), vec![call_id.as_str()]);
    let missed_by_caller: ListCallsResponse = ctx
        .call("Call", "ListCalls", &taro, &ListCallsRequest { missed_only: true, ..Default::default() })
        .await
        .unwrap();
    assert!(missed_by_caller.calls.is_empty());

    // Nobody answers in time.
    let call_id = start_call(&ctx, &taro, &direct.id, false).await.call.id.clone();
    let sweeping = SWEEPS.lock().await;
    assert_eq!(ctx.chat.expire_ringing_calls().await.unwrap(), 0, "still ringing");
    let mut taro_events = ctx.listen(&taro).await;
    let_ringing_run_out(&ctx, &call_id).await;
    assert_eq!(ctx.chat.expire_ringing_calls().await.unwrap(), 1);
    drop(sweeping);
    let missed = get_call(&ctx, &taro, &call_id).await;
    assert_eq!(outcome(&missed), (CallState::CALL_STATE_ENDED, CallEndReason::CALL_END_REASON_MISSED));
    assert_eq!(state_of(&missed, &hanako), Some(CallParticipantState::CALL_PARTICIPANT_STATE_MISSED));
    let events = ctx.events(&mut taro_events).await;
    assert_eq!(call_changes(&events)[0].end_reason, CallEndReason::CALL_END_REASON_MISSED);
    assert_eq!(ctx.notifications(&hanako).await.last().unwrap().data["missed"], true);
    assert_eq!(
        ctx.message(&hanako, &missed.message_id).await.call.end_reason,
        CallEndReason::CALL_END_REASON_MISSED,
        "the timeline shows a missed call"
    );

    // Hanging up on a call that is ringing for you declines it.
    let call_id = start_call(&ctx, &taro, &direct.id, false).await.call.id.clone();
    let refused = leave(&ctx, &hanako, &call_id).await;
    assert_eq!(outcome(&refused), (CallState::CALL_STATE_ENDED, CallEndReason::CALL_END_REASON_DECLINED));
}

#[tokio::test]
async fn calls_respect_privacy_requests_and_blocks() {
    let ctx = start_with(livekit_config()).await;
    let taro = ctx.app.user("taro").await;

    // Not friends, in a conversation the server opened: the callee's "who may call me" decides.
    let acquaintance = ctx.app.user("acquaintance").await;
    let loose = ctx.chat.open_direct(taro.id, acquaintance.id).await.unwrap().to_string();
    assert_eq!(ctx.reason("Call", "StartCall", &taro, &start_request(&loose, false)).await, "CALL_NOT_ALLOWED");
    ctx.open_to_everyone(&acquaintance).await;
    let allowed = start_call(&ctx, &taro, &loose, false).await;
    leave(&ctx, &taro, &allowed.call.id).await;

    // A message request must be accepted before anyone calls, in either direction.
    let hanako = ctx.app.user("hanako").await;
    ctx.open_to_everyone(&hanako).await;
    ctx.open_to_everyone(&taro).await;
    let request = ctx.direct(&taro, &hanako).await;
    ctx.text(&taro, &request.id, "はじめまして").await;
    assert_eq!(ctx.reason("Call", "StartCall", &taro, &start_request(&request.id, false)).await, "REQUEST_PENDING");
    assert_eq!(ctx.reason("Call", "StartCall", &hanako, &start_request(&request.id, false)).await, "REQUEST_PENDING");

    // A block ends a ringing call and forbids new ones.
    let (caller, callee) = ctx.friends("caller", "callee").await;
    let direct = ctx.direct(&caller, &callee).await;
    let ringing = start_call(&ctx, &caller, &direct.id, false).await;
    ctx.block(&callee, &caller).await;
    let blocked = tokuly_core::events::Event {
        id: Uuid::now_v7(),
        topic: "social.user.blocked".into(),
        actor_id: Some(callee.id),
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({ "user_id": callee.id, "blocked_id": caller.id }),
    };
    ctx.chat.handle_event(&blocked).await.unwrap();
    let failed = get_call(&ctx, &caller, &ringing.call.id).await;
    assert_eq!(
        (failed.state.as_known(), failed.end_reason.as_known()),
        (Some(CallState::CALL_STATE_ENDED), Some(CallEndReason::CALL_END_REASON_FAILED))
    );
    assert_eq!(
        ctx.reason("Call", "StartCall", &caller, &start_request(&direct.id, false)).await,
        "CONVERSATION_READ_ONLY"
    );

    let outsider = ctx.app.user("outsider").await;
    assert_eq!(
        ctx.reason("Call", "StartCall", &outsider, &start_request(&direct.id, false)).await,
        "CONVERSATION_NOT_FOUND"
    );
    assert_eq!(ctx.anonymous_reason("Call", "StartCall", &start_request(&direct.id, false)).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn a_group_call_stays_open_while_someone_is_in_it() {
    let ctx = start_with(livekit_config()).await;
    let (group, owner, members) = ctx.group_of("部活", 3).await;
    let (first, second, third) = (&members[0], &members[1], &members[2]);
    // One member muted the group: rung in the app, but not pushed.
    let mute = UpdatePreferencesRequest { conversation_id: group.id.clone(), muted: Some(true), ..Default::default() };
    let _: UpdatePreferencesResponse = ctx.call("Conversation", "UpdatePreferences", third, &mute).await.unwrap();

    let started = start_call(&ctx, &owner, &group.id, true).await;
    let call_id = started.call.id.clone();
    for member in &members {
        assert_eq!(state_of(&started.call, member), Some(CallParticipantState::CALL_PARTICIPANT_STATE_RINGING));
    }
    let call_notifications = |user: &TestUser| {
        let ctx = &ctx;
        let user = user.clone();
        async move {
            ctx.notifications(&user)
                .await
                .into_iter()
                .filter(|notification| notification.kind == "chat.call")
                .collect::<Vec<_>>()
        }
    };
    assert!(call_notifications(first).await[0].urgent);
    assert!(call_notifications(third).await.is_empty());

    // One answers, one declines: the call goes on.
    assert_eq!(join(&ctx, first, &call_id).await.call.state, CallState::CALL_STATE_ACTIVE);
    let declined = decline(&ctx, second, &call_id).await;
    assert_eq!(declined.state, CallState::CALL_STATE_ACTIVE);
    assert_eq!(state_of(&declined, second), Some(CallParticipantState::CALL_PARTICIPANT_STATE_DECLINED));
    assert_eq!(decline(&ctx, second, &call_id).await.state, CallState::CALL_STATE_ACTIVE, "declining twice is fine");

    // The ringing time is up: whoever did not answer missed it, but may still join.
    let sweeping = SWEEPS.lock().await;
    let_ringing_run_out(&ctx, &call_id).await;
    assert_eq!(ctx.chat.expire_ringing_calls().await.unwrap(), 1);
    let ringing_over = get_call(&ctx, &owner, &call_id).await;
    assert_eq!(ringing_over.state, CallState::CALL_STATE_ACTIVE);
    assert_eq!(state_of(&ringing_over, third), Some(CallParticipantState::CALL_PARTICIPANT_STATE_MISSED));
    assert_eq!(ctx.chat.expire_ringing_calls().await.unwrap(), 0, "nothing left to time out");
    drop(sweeping);

    // The caller may leave; latecomers and people who declined may still come in.
    assert_eq!(leave(&ctx, &owner, &call_id).await.state, CallState::CALL_STATE_ACTIVE);
    assert_eq!(
        state_of(&join(&ctx, third, &call_id).await.call, third),
        Some(CallParticipantState::CALL_PARTICIPANT_STATE_JOINED)
    );
    assert_eq!(
        state_of(&join(&ctx, second, &call_id).await.call, second),
        Some(CallParticipantState::CALL_PARTICIPANT_STATE_JOINED)
    );
    assert_eq!(leave(&ctx, first, &call_id).await.state, CallState::CALL_STATE_ACTIVE);
    assert_eq!(leave(&ctx, second, &call_id).await.state, CallState::CALL_STATE_ACTIVE);
    // The last one out ends it.
    let ended = leave(&ctx, third, &call_id).await;
    assert_eq!(
        (ended.state.as_known(), ended.end_reason.as_known()),
        (Some(CallState::CALL_STATE_ENDED), Some(CallEndReason::CALL_END_REASON_COMPLETED))
    );

    // Everyone who was rung declines: there is nobody to wait for.
    let call_id = start_call(&ctx, &owner, &group.id, false).await.call.id.clone();
    decline(&ctx, first, &call_id).await;
    decline(&ctx, second, &call_id).await;
    let refused = decline(&ctx, third, &call_id).await;
    assert_eq!(
        (refused.state.as_known(), refused.end_reason.as_known()),
        (Some(CallState::CALL_STATE_ENDED), Some(CallEndReason::CALL_END_REASON_DECLINED))
    );

    // Someone removed from the group is out of its call; members added during it may join.
    let call_id = start_call(&ctx, &owner, &group.id, false).await.call.id.clone();
    join(&ctx, first, &call_id).await;
    let remove =
        RemoveMemberRequest { conversation_id: group.id.clone(), user_id: first.id.to_string(), ..Default::default() };
    let _: RemoveMemberResponse = ctx.call("Conversation", "RemoveMember", &owner, &remove).await.unwrap();
    let after_removal = get_call(&ctx, &owner, &call_id).await;
    assert_eq!(state_of(&after_removal, first), Some(CallParticipantState::CALL_PARTICIPANT_STATE_LEFT));
    assert_eq!(after_removal.state, CallState::CALL_STATE_ACTIVE);
    assert_eq!(
        ctx.reason("Call", "JoinCall", first, &JoinCallRequest { call_id: call_id.clone(), ..Default::default() })
            .await,
        "CALL_NOT_FOUND"
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
        state_of(&join(&ctx, &newcomer, &call_id).await.call, &newcomer),
        Some(CallParticipantState::CALL_PARTICIPANT_STATE_JOINED)
    );
    leave(&ctx, &newcomer, &call_id).await;
    assert_eq!(leave(&ctx, &owner, &call_id).await.state, CallState::CALL_STATE_ENDED);
}

#[tokio::test]
async fn big_groups_are_told_about_a_call_without_being_rung() {
    let ctx = start_with(livekit_config()).await;
    let (group, owner, members) = ctx.group_of("学年", 8).await;
    assert_eq!(group.member_count, 9);

    let mut member_events = ctx.listen(&members[0]).await;
    let started = start_call(&ctx, &owner, &group.id, false).await;
    assert_eq!(started.call.participants.len(), 1, "nobody is rung in a group this large");
    let notification =
        ctx.notifications(&members[0]).await.into_iter().find(|notification| notification.kind == "chat.call").unwrap();
    assert!(!notification.urgent, "an ordinary notification, not a ringing phone");
    assert_eq!(notification.data["ringing"], false);
    // Everyone still sees the call and can join from the conversation.
    assert_eq!(call_changes(&ctx.events(&mut member_events).await).len(), 1);
    assert_eq!(ctx.conversation(&members[3], &group.id).await.active_call.id, started.call.id);
    let joined = join(&ctx, &members[3], &started.call.id).await;
    assert_eq!(joined.call.state, CallState::CALL_STATE_ACTIVE);
    leave(&ctx, &members[3], &started.call.id).await;
    let ended = leave(&ctx, &owner, &started.call.id).await;
    assert_eq!(ended.end_reason, CallEndReason::CALL_END_REASON_COMPLETED);
    assert!(
        ended
            .participants
            .iter()
            .all(|participant| participant.state != CallParticipantState::CALL_PARTICIPANT_STATE_MISSED),
        "nobody missed anything"
    );
}

#[tokio::test]
async fn participants_who_vanished_from_livekit_are_dropped() {
    let ctx = start_with(livekit_config()).await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let call_id = start_call(&ctx, &taro, &direct.id, false).await.call.id.clone();
    join(&ctx, &hanako, &call_id).await;

    // Nobody in this test connects to LiveKit. For a minute that is normal:
    // clients need a moment to connect.
    ctx.chat.reconcile_calls().await.unwrap();
    assert_eq!(get_call(&ctx, &taro, &call_id).await.state, CallState::CALL_STATE_ACTIVE);

    // Two minutes later both are still marked as joined but neither is in the room.
    let _sweeping = SWEEPS.lock().await;
    sqlx::query("UPDATE chat_calls SET created_at = now() - interval '2 minutes', answered_at = now() - interval '2 minutes' WHERE id = $1::uuid")
        .bind(&call_id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    sqlx::query("UPDATE chat_call_participants SET joined_at = now() - interval '2 minutes' WHERE call_id = $1::uuid")
        .bind(&call_id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let mut hanako_events = ctx.listen(&hanako).await;
    assert!(ctx.chat.reconcile_calls().await.unwrap() >= 1);
    let ended = get_call(&ctx, &taro, &call_id).await;
    assert_eq!(
        (ended.state.as_known(), ended.end_reason.as_known()),
        (Some(CallState::CALL_STATE_ENDED), Some(CallEndReason::CALL_END_REASON_COMPLETED))
    );
    assert!(
        ended
            .participants
            .iter()
            .all(|participant| participant.state == CallParticipantState::CALL_PARTICIPANT_STATE_LEFT)
    );
    assert!(
        call_changes(&ctx.events(&mut hanako_events).await)
            .iter()
            .any(|call| call.id == call_id && call.state == CallState::CALL_STATE_ENDED)
    );
    assert!(ctx.conversation(&taro, &direct.id).await.active_call.is_unset());
}

/// A stand-in for LiveKit's room service: records what the server asks of
/// it and reports whoever a test says is connected to a room.
#[derive(Clone, Default)]
struct RoomService {
    calls: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    connected: Arc<Mutex<HashMap<String, Vec<String>>>>,
}

impl RoomService {
    /// The identities removed from `room` so far, in order.
    fn removed_from(&self, room: &str) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(method, body)| method == "RemoveParticipant" && body["room"] == room)
            .map(|(_, body)| body["identity"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    fn connect(&self, room: &str, identities: &[&str]) {
        self.connected.lock().unwrap().insert(room.to_owned(), identities.iter().map(|id| id.to_string()).collect());
    }
}

async fn room_service(
    State(stub): State<RoomService>,
    Path(method): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    stub.calls.lock().unwrap().push((method.clone(), body.clone()));
    let room = body["room"].as_str().or(body["name"].as_str()).unwrap_or_default().to_owned();
    Json(match method.as_str() {
        "CreateRoom" => serde_json::json!({ "sid": "RM_stub", "name": room }),
        "ListParticipants" => {
            let connected = stub.connected.lock().unwrap().get(&room).cloned().unwrap_or_default();
            let participants: Vec<_> =
                connected.iter().map(|identity| serde_json::json!({ "identity": identity })).collect();
            serde_json::json!({ "participants": participants })
        }
        _ => serde_json::json!({}),
    })
}

/// A chat domain whose calls run on the [`RoomService`] stub.
async fn start_with_room_service() -> (Ctx, RoomService) {
    let stub = RoomService::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let address = listener.local_addr().unwrap();
    let router =
        Router::new().route("/twirp/livekit.RoomService/{method}", post(room_service)).with_state(stub.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.expect("stub server") });
    let cfg = ChatConfig {
        livekit_url: format!("http://{address}"),
        livekit_public_url: "ws://livekit.test".into(),
        livekit_api_key: "stubkey".into(),
        livekit_api_secret: "a-stub-secret-of-at-least-thirty-two-characters".into(),
        ..test_config()
    };
    (start_with(cfg).await, stub)
}

#[tokio::test]
async fn every_join_makes_sure_the_room_exists() {
    let (ctx, livekit) = start_with_room_service().await;
    let (group, owner, members) = ctx.group_of("部活", 2).await;
    let created = |room: &str| {
        livekit
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(method, body)| method == "CreateRoom" && body["name"] == room)
            .count()
    };
    let started = start_call(&ctx, &owner, &group.id, false).await;
    let (call_id, room) = (started.call.id.clone(), started.credentials.room.clone());
    assert_eq!(created(&room), 1);
    // LiveKit may have closed the room since (it makes none on its own):
    // answering late and reconnecting each ask for it again.
    join(&ctx, &members[0], &call_id).await;
    assert_eq!(created(&room), 2);
    join(&ctx, &members[0], &call_id).await;
    assert_eq!(created(&room), 3);
}

#[tokio::test]
async fn nobody_stays_connected_to_a_call_they_are_no_longer_part_of() {
    let (ctx, livekit) = start_with_room_service().await;
    let (group, owner, members) = ctx.group_of("部活", 2).await;
    let (first, second) = (&members[0], &members[1]);
    let started = start_call(&ctx, &owner, &group.id, false).await;
    let (call_id, room) = (started.call.id.clone(), started.credentials.room.clone());
    join(&ctx, first, &call_id).await;
    join(&ctx, second, &call_id).await;

    // A join token cannot be taken back: whoever loses their place in the
    // call is disconnected from its room.
    let remove =
        RemoveMemberRequest { conversation_id: group.id.clone(), user_id: first.id.to_string(), ..Default::default() };
    let _: RemoveMemberResponse = ctx.call("Conversation", "RemoveMember", &owner, &remove).await.unwrap();
    assert_eq!(livekit.removed_from(&room), vec![first.id.to_string()]);
    assert_eq!(leave(&ctx, second, &call_id).await.state, CallState::CALL_STATE_ACTIVE);
    assert_eq!(livekit.removed_from(&room), vec![first.id.to_string(), second.id.to_string()]);

    // Connecting again with the old token does not last: whoever is in the
    // room without being in the call is put out at the next reconciliation.
    let _sweeping = SWEEPS.lock().await;
    sqlx::query("UPDATE chat_calls SET created_at = now() - interval '2 minutes' WHERE id = $1::uuid")
        .bind(&call_id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    livekit.connect(&room, &[&owner.id.to_string(), &first.id.to_string(), "intruder"]);
    ctx.chat.reconcile_calls().await.unwrap();
    let removed = livekit.removed_from(&room);
    assert_eq!(removed.len(), 4, "{removed:?}");
    assert!(removed[2..].contains(&first.id.to_string()) && removed[2..].contains(&"intruder".to_owned()));
    let call = get_call(&ctx, &owner, &call_id).await;
    assert_eq!(call.state, CallState::CALL_STATE_ACTIVE, "the caller is still talking");
    assert_eq!(state_of(&call, &owner), Some(CallParticipantState::CALL_PARTICIPANT_STATE_JOINED));
}

#[tokio::test]
async fn a_removed_member_does_not_watch_the_call_go_on() {
    let (ctx, _livekit) = start_with_room_service().await;
    let (group, owner, members) = ctx.group_of("部活", 2).await;
    let (first, second) = (&members[0], &members[1]);
    let call_id = start_call(&ctx, &owner, &group.id, false).await.call.id.clone();
    join(&ctx, first, &call_id).await;
    let remove =
        RemoveMemberRequest { conversation_id: group.id.clone(), user_id: first.id.to_string(), ..Default::default() };
    let _: RemoveMemberResponse = ctx.call("Conversation", "RemoveMember", &owner, &remove).await.unwrap();

    // Who joins the group's call from now on is none of their business.
    join(&ctx, second, &call_id).await;
    let get = GetCallRequest { call_id: call_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Call", "GetCall", first, &get).await, "CALL_NOT_FOUND");
    let history: ListCallsResponse = ctx.call("Call", "ListCalls", first, &ListCallsRequest::default()).await.unwrap();
    assert!(history.calls.is_empty(), "the live call is not in their history yet");

    // Once it is over it is part of their history like any call they were in.
    leave(&ctx, second, &call_id).await;
    assert_eq!(leave(&ctx, &owner, &call_id).await.state, CallState::CALL_STATE_ENDED);
    let history: ListCallsResponse = ctx.call("Call", "ListCalls", first, &ListCallsRequest::default()).await.unwrap();
    assert_eq!(history.calls.iter().map(|call| call.id.clone()).collect::<Vec<_>>(), vec![call_id.clone()]);
    assert_eq!(get_call(&ctx, first, &call_id).await.state, CallState::CALL_STATE_ENDED);
}

#[tokio::test]
async fn a_member_removed_while_joining_does_not_stay_in_the_call() {
    let (ctx, _livekit) = start_with_room_service().await;
    let mut pairs = Vec::new();
    for _ in 0..20 {
        let (group, owner, members) = ctx.group_of("部活", 1).await;
        let call_id = start_call(&ctx, &owner, &group.id, false).await.call.id.clone();
        pairs.push((group, owner, members.into_iter().next().unwrap(), call_id));
    }

    // Each member joins while the owner removes them, at the same moment.
    let ctx = &ctx;
    let races = pairs.iter().map(|(group, owner, member, call_id)| async move {
        let join_request = JoinCallRequest { call_id: call_id.clone(), ..Default::default() };
        let remove_request = RemoveMemberRequest {
            conversation_id: group.id.clone(),
            user_id: member.id.to_string(),
            ..Default::default()
        };
        let join = ctx.call::<_, JoinCallResponse>("Call", "JoinCall", member, &join_request);
        let remove = ctx.call::<_, RemoveMemberResponse>("Conversation", "RemoveMember", owner, &remove_request);
        let (_, removed) = tokio::join!(join, remove);
        removed.expect("the removal goes through");
    });
    futures::future::join_all(races).await;

    for (_, owner, member, call_id) in &pairs {
        let call = get_call(ctx, owner, call_id).await;
        assert_ne!(
            state_of(&call, member),
            Some(CallParticipantState::CALL_PARTICIPANT_STATE_JOINED),
            "a non-member is in the call"
        );
    }

    // Should one slip through anyway, the reconciliation takes them out.
    let (_, owner, member, call_id) = &pairs[0];
    let _sweeping = SWEEPS.lock().await;
    sqlx::query(
        "INSERT INTO chat_call_participants (call_id, user_id, state, joined_at, rung) VALUES ($1::uuid, $2, 2, now(), false)
         ON CONFLICT (call_id, user_id) DO UPDATE SET state = 2, joined_at = now()",
    )
    .bind(call_id)
    .bind(member.id)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    sqlx::query("UPDATE chat_calls SET created_at = now() - interval '2 minutes' WHERE id = $1::uuid")
        .bind(call_id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.chat.reconcile_calls().await.unwrap();
    let call = get_call(ctx, owner, call_id).await;
    assert_eq!(state_of(&call, member), Some(CallParticipantState::CALL_PARTICIPANT_STATE_LEFT));
}

#[tokio::test]
async fn an_erased_account_leaves_no_trace_in_the_calls_of_a_group() {
    let (ctx, _livekit) = start_with_room_service().await;
    let (group, owner, members) = ctx.group_of("部活", 2).await;
    let (first, second) = (&members[0], &members[1]);
    let ended = start_call(&ctx, &owner, &group.id, false).await.call.id.clone();
    join(&ctx, first, &ended).await;
    leave(&ctx, first, &ended).await;
    assert_eq!(leave(&ctx, &owner, &ended).await.state, CallState::CALL_STATE_ENDED);
    let live = start_call(&ctx, &owner, &group.id, false).await.call.id.clone();
    join(&ctx, first, &live).await;

    // What identity does when the grace period is over: anonymise, then announce.
    sqlx::query("UPDATE users SET deleted_at = now(), display_name = '' WHERE id = $1")
        .bind(first.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let deleted = tokuly_core::events::Event {
        id: Uuid::now_v7(),
        topic: "identity.user.deleted".into(),
        actor_id: None,
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({ "user_id": first.id }),
    };
    ctx.chat.handle_event(&deleted).await.unwrap();

    for call_id in [&ended, &live] {
        let call = get_call(&ctx, &owner, call_id).await;
        assert_eq!(state_of(&call, first), None, "not among the participants any more");
        assert!(state_of(&call, &owner).is_some());
    }
    assert_eq!(get_call(&ctx, &owner, &live).await.state, CallState::CALL_STATE_ACTIVE, "the others talk on");
    join(&ctx, second, &live).await;
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM chat_call_participants WHERE user_id = $1")
        .bind(first.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(rows, 0);

    // Delivered again (at-least-once): nothing changes.
    ctx.chat.handle_event(&deleted).await.unwrap();
    assert_eq!(get_call(&ctx, &owner, &live).await.state, CallState::CALL_STATE_ACTIVE);
}
