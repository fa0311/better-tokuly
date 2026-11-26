//! What happens outside of requests: the hook the media server calls,
//! announcements, reactions to other domains' events, the archive
//! job, the data export. And the proof that one account cannot stall the
//! API by asking for many things at once.
mod common;

use std::time::Duration;

use axum::body::Body;
use common::*;
use futures::future::join_all;
use http_body_util::BodyExt as _;
use tokuly_core::prelude::*;
use tokuly_live::{
    LiveConfig,
    events::{AnnounceJob, AnnouncePhase, ArchiveJob},
};
use tokuly_proto::{
    common::{SubjectType, Visibility},
    live::*,
    video::NotificationLevel,
};
use tower::ServiceExt as _;

const HOOK_SECRET: &str = "test-hook-secret-0123456789";
const API_SECRET: &str = "test-api-secret-9876543210";

/// A media server that is configured (but, here, never reachable).
fn with_media_server() -> LiveConfig {
    LiveConfig {
        mediamtx_api_url: "http://127.0.0.1:9".into(),
        hook_secret: HOOK_SECRET.into(),
        mediamtx_api_secret: API_SECRET.into(),
        ..test_config()
    }
}

#[tokio::test]
async fn the_media_server_is_answered_through_the_hook() {
    let ctx = start_with(with_media_server()).await;
    let alice = ctx.app.user("alice").await;
    let friend = ctx.app.user("friend").await;
    let channel = ctx.channel(&alice, "フックch").await;
    ctx.befriend(&alice, &friend).await;
    let public = ctx.on_air(&alice, &channel, "公開").await;
    let friends_only =
        ctx.on_air_with(&alice, &channel, "友達だけ", |r| r.visibility = Visibility::VISIBILITY_FRIENDS.into()).await;
    let no_dvr = ctx.on_air_with(&alice, &channel, "巻き戻しなし", |r| r.dvr_enabled = Some(false)).await;
    let key = {
        let info: GetIngestInfoResponse = ctx
            .call(
                STREAMS,
                "GetIngestInfo",
                Some(&alice),
                &GetIngestInfoRequest { stream_id: card(&public).id.clone(), ..Default::default() },
            )
            .await
            .unwrap();
        info.ingest.as_option().unwrap().stream_key.clone()
    };
    let router = ctx.live.http_routes();
    // One question to the hook: from `peer`, with this `Authorization` header (if any).
    let send = |uri: &str, authorization: Option<String>, peer: &str, body: serde_json::Value| {
        let router = router.clone();
        let peer: std::net::IpAddr = peer.parse().unwrap();
        let mut request = http::Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .extension(axum::extract::ConnectInfo(std::net::SocketAddr::new(peer, 40_000)));
        if let Some(authorization) = authorization {
            request = request.header("authorization", authorization);
        }
        let request = request.body(Body::from(body.to_string())).unwrap();
        async move {
            let response = router.oneshot(request).await.unwrap();
            let status = response.status().as_u16();
            let _ = response.into_body().collect().await;
            status
        }
    };
    // The media server's own address, presenting `secret` as a bearer token.
    let ask = |secret: &str, body: serde_json::Value| {
        let authorization = (!secret.is_empty()).then(|| format!("Bearer {secret}"));
        send("/auth", authorization, "198.51.100.7", body)
    };
    let secret = HOOK_SECRET;
    let path = |stream: &Stream| format!("live/{}", card(stream).short_id);
    let publish = |token: &str| serde_json::json!({ "action": "publish", "path": path(&public), "protocol": "rtmp", "token": token, "ip": "203.0.113.5", "id": "c1", "query": format!("token={token}") });

    // Without the shared secret nothing is answered.
    assert_eq!(ask("wrong", publish(&key)).await, 403);
    assert_eq!(ask("", publish(&key)).await, 403);
    // The secret is read from the header only: not from the URL (which ends up in access logs),
    // and the control API's secret is not the hook's.
    assert_eq!(send(&format!("/auth?secret={secret}"), None, "198.51.100.7", publish(&key)).await, 403);
    assert_eq!(ask(API_SECRET, publish(&key)).await, 403);
    // MediaMTX sends the credentials of its `authHTTPAddress` as HTTP Basic ("mediamtx:<secret>").
    let basic = "Basic bWVkaWFtdHg6dGVzdC1ob29rLXNlY3JldC0wMTIzNDU2Nzg5".to_owned();
    assert_eq!(send("/auth", Some(basic), "198.51.100.7", publish(&key)).await, 200);
    // Publishing: the stream key and nothing else, however it arrives (RTMP token, SRT password).
    assert_eq!(ask(secret, publish(&key)).await, 200);
    assert_eq!(ask(secret, publish("wrong-key")).await, 401);
    assert_eq!(ask(secret, serde_json::json!({ "action": "publish", "path": path(&public), "protocol": "srt", "user": "tokuly", "password": key })).await, 200);
    assert_eq!(
        ask(secret, serde_json::json!({ "action": "publish", "path": "live/unknownXYZ1", "token": key })).await,
        401
    );
    assert_eq!(ask(secret, serde_json::json!({ "action": "publish", "path": "elsewhere", "token": key })).await, 401);
    // The key of one stream opens no other.
    assert_eq!(
        ask(secret, serde_json::json!({ "action": "publish", "path": path(&friends_only), "token": key })).await,
        401
    );

    // Reading: public streams on air for anyone; restricted ones with the token their page hands out.
    let read = |stream: &Stream, query: &str| serde_json::json!({ "action": "read", "path": path(stream), "protocol": "hls", "query": query });
    assert_eq!(ask(secret, read(&public, "")).await, 200);
    assert_eq!(ask(secret, read(&friends_only, "")).await, 401);
    let token = ctx
        .get_stream(Some(&friend), &card(&friends_only).id)
        .await
        .unwrap()
        .playback
        .as_option()
        .unwrap()
        .token
        .clone();
    assert!(!token.is_empty());
    assert_eq!(ask(secret, read(&friends_only, &format!("session=abc&t={token}"))).await, 200);
    // The token names who it was made for, and does not live long: the heartbeat brings the next one.
    assert_eq!(ctx.live.playback_token_viewer(&card(&friends_only).short_id, &token), Some(friend.id));
    let expires = ctx
        .get_stream(Some(&friend), &card(&friends_only).id)
        .await
        .unwrap()
        .playback
        .as_option()
        .unwrap()
        .expires_at
        .as_option()
        .map(|t| t.seconds)
        .unwrap();
    assert!(expires - now().timestamp() <= 15 * 60 + 5, "a playback token lives a quarter of an hour");
    let beat: ReportWatchingResponse = ctx
        .call(
            STREAMS,
            "ReportWatching",
            Some(&friend),
            &ReportWatchingRequest { stream_id: card(&friends_only).id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let renewed = beat.playback.as_option().expect("a heartbeat renews the playback URLs").token.clone();
    assert_eq!(ctx.live.playback_token_viewer(&card(&friends_only).short_id, &renewed), Some(friend.id));
    // The session a token opened is remembered with its viewer, so it can be cut later.
    let mut tracked = read(&friends_only, &format!("t={token}"));
    tracked["id"] = "hls-session-1".into();
    assert_eq!(ask(secret, tracked).await, 200);
    let remembered: Option<String> = redis::cmd("HGET")
        .arg(format!("live:sessions:{}", sid(&friends_only)))
        .arg("hls-session-1")
        .query_async(&mut ctx.app.core.cache.conn())
        .await
        .unwrap();
    assert_eq!(remembered, Some(friend.id.to_string()));
    assert_eq!(ask(secret, read(&friends_only, &format!("t={token}x"))).await, 401);
    // A token of one stream opens no other; a bearer token works like the query.
    assert_eq!(ask(secret, read(&no_dvr, &format!("t={token}"))).await, 200, "public anyway");
    let private =
        ctx.on_air_with(&alice, &channel, "自分だけ", |r| r.visibility = Visibility::VISIBILITY_PRIVATE.into()).await;
    assert_eq!(ask(secret, read(&private, &format!("t={token}"))).await, 401);
    assert_eq!(
        ask(
            secret,
            serde_json::json!({ "action": "read", "path": path(&friends_only), "protocol": "webrtc", "token": token })
        )
        .await,
        200
    );
    // An expired token is worth nothing.
    let expired =
        ctx.live.playback_token(&card(&friends_only).short_id, friend.id, now() - chrono::Duration::seconds(5));
    assert_eq!(ask(secret, read(&friends_only, &format!("t={expired}"))).await, 401);

    // Rewinding: only with DVR, and only inside the window.
    let dvr = |stream: &Stream, start: Time| {
        let start = start.to_rfc3339().replace(':', "%3A").replace('+', "%2B");
        serde_json::json!({ "action": "playback", "path": path(stream), "query": format!("path={}&start={start}&duration=30", path(stream)) })
    };
    // The stream went on air 45 minutes ago; its encoder was connected (and recorded) before that.
    sqlx::query("UPDATE live_streams SET started_at = now() - interval '45 minutes' WHERE id = $1")
        .bind(sid(&public))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(ask(secret, dvr(&public, now() - chrono::Duration::minutes(30))).await, 200);
    assert_eq!(
        ask(secret, dvr(&public, now() - chrono::Duration::minutes(50))).await,
        401,
        "the preview before the broadcast is inside the window but not part of it"
    );
    // A stream that is not on air yet has nothing to rewind, even for its owner's token.
    let upcoming = ctx.stream(&alice, &channel, "まだ").await;
    let owner_token =
        ctx.live.playback_token(&card(&upcoming).short_id, alice.id, now() + chrono::Duration::minutes(5));
    let mut preview = dvr(&upcoming, now() - chrono::Duration::seconds(10));
    preview["query"] = format!("{}&t={owner_token}", preview["query"].as_str().unwrap()).into();
    assert_eq!(ask(secret, preview).await, 401);
    assert_eq!(ask(secret, dvr(&public, now() - chrono::Duration::hours(2))).await, 401);
    assert_eq!(ask(secret, dvr(&no_dvr, now() - chrono::Duration::minutes(1))).await, 401);
    assert_eq!(
        ask(secret, serde_json::json!({ "action": "playback", "path": path(&public), "query": "start=yesterday" }))
            .await,
        401
    );

    // A token is worth what its viewer may do right now. The friend's token is good for minutes
    // yet, and was accepted a moment ago (so the answer is cached): once the friendship ends
    // it opens nothing, at once.
    let fresh =
        ctx.live.playback_token(&card(&friends_only).short_id, friend.id, now() + chrono::Duration::minutes(10));
    assert_eq!(ask(secret, read(&friends_only, &format!("t={fresh}"))).await, 200);
    ctx.unfriend(&alice, &friend).await;
    assert_eq!(ask(secret, read(&friends_only, &format!("t={fresh}"))).await, 401);
    assert_eq!(ask(secret, read(&friends_only, &format!("t={token}"))).await, 401);
    // The same when the stream is narrowed, and for someone who is blocked.
    ctx.befriend(&alice, &friend).await;
    assert_eq!(ask(secret, read(&friends_only, &format!("t={fresh}"))).await, 200);
    let narrow = UpdateStreamRequest {
        stream_id: card(&friends_only).id.clone(),
        visibility: Some(Visibility::VISIBILITY_PRIVATE.into()),
        ..Default::default()
    };
    let _: UpdateStreamResponse = ctx.call(STREAMS, "UpdateStream", Some(&alice), &narrow).await.unwrap();
    assert_eq!(ask(secret, read(&friends_only, &format!("t={fresh}"))).await, 401);
    let widen = UpdateStreamRequest { visibility: Some(Visibility::VISIBILITY_FRIENDS.into()), ..narrow };
    let _: UpdateStreamResponse = ctx.call(STREAMS, "UpdateStream", Some(&alice), &widen).await.unwrap();
    assert_eq!(ask(secret, read(&friends_only, &format!("t={fresh}"))).await, 200);
    ctx.block(&alice, &friend).await;
    ctx.deliver("social.", alice.id).await;
    assert_eq!(ask(secret, read(&friends_only, &format!("t={fresh}"))).await, 401);
    // Nobody's session is remembered as nobody's.
    let mut anonymous = read(&public, "");
    anonymous["id"] = "hls-session-2".into();
    assert_eq!(ask(secret, anonymous).await, 200);
    let remembered: Option<String> = redis::cmd("HGET")
        .arg(format!("live:sessions:{}", sid(&public)))
        .arg("hls-session-2")
        .query_async(&mut ctx.app.core.cache.conn())
        .await
        .unwrap();
    assert_eq!(remembered.as_deref(), Some("-"));

    // The control API belongs to the worker.
    assert_eq!(
        ask(secret, serde_json::json!({ "action": "api", "user": "tokuly", "password": API_SECRET })).await,
        200
    );
    assert_eq!(
        ask(secret, serde_json::json!({ "action": "api", "user": "tokuly", "password": secret })).await,
        401,
        "the hook secret does not open the control API"
    );
    assert_eq!(ask(secret, serde_json::json!({ "action": "api", "user": "", "password": "" })).await, 401);
    assert_eq!(ask(secret, serde_json::json!({ "action": "something-else" })).await, 401);

    // Over: nothing more to publish or read.
    ctx.end(&alice, &public).await;
    assert_eq!(ask(secret, publish(&key)).await, 401);
    assert_eq!(ask(secret, read(&public, "")).await, 401);

    // An address that keeps guessing is refused outright, even once it guesses right. Others are not.
    for _ in 0..10 {
        assert_eq!(send("/auth", Some("Bearer guess".into()), "198.51.100.99", publish(&key)).await, 403);
    }
    let right = Some(format!("Bearer {secret}"));
    assert_eq!(send("/auth", right.clone(), "198.51.100.99", read(&friends_only, "")).await, 429);
    assert_eq!(send("/auth", right, "198.51.100.7", read(&friends_only, "")).await, 401);
}

#[tokio::test]
async fn the_live_topic_follows_the_streams_visibility() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let friend = ctx.app.user("friend").await;
    let stranger = ctx.app.user("stranger").await;
    ctx.befriend(&alice, &friend).await;
    let channel = ctx.channel(&alice, "トピックch").await;
    let public = ctx.on_air(&alice, &channel, "公開").await;
    let friends_only =
        ctx.on_air_with(&alice, &channel, "友達だけ", |r| r.visibility = Visibility::VISIBILITY_FRIENDS.into()).await;
    let guards = &ctx.app.core.topic_guards;
    assert!(guards.may_subscribe(None, &public.realtime_topic).await.unwrap(), "guests read public streams");
    assert!(guards.may_subscribe(Some(friend.id), &friends_only.realtime_topic).await.unwrap());
    assert!(!guards.may_subscribe(Some(stranger.id), &friends_only.realtime_topic).await.unwrap());
    assert!(!guards.may_subscribe(None, &friends_only.realtime_topic).await.unwrap());
    assert!(!guards.may_subscribe(None, &format!("live:{}", Uuid::now_v7())).await.unwrap());
    assert!(!guards.may_subscribe(None, "live:not-an-id").await.unwrap());
}

#[tokio::test]
async fn going_on_air_is_announced_to_those_who_asked_subscribers_and_friends() {
    let ctx = start_with(LiveConfig { announce_batch_size: 2, ..test_config() }).await;
    let alice = ctx.app.user("alice").await;
    let reminded = ctx.app.user("reminded").await;
    let everything = ctx.app.user("everything").await;
    let personalized = ctx.app.user("personalized").await;
    let silent = ctx.app.user("silent").await;
    let blocked = ctx.app.user("blocked").await;
    let outsider = ctx.app.user("outsider").await;
    let channel = ctx.channel(&alice, "お知らせch").await;
    ctx.subscribe(&everything, &channel, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    ctx.subscribe(&personalized, &channel, NotificationLevel::NOTIFICATION_LEVEL_PERSONALIZED).await;
    ctx.subscribe(&silent, &channel, NotificationLevel::NOTIFICATION_LEVEL_NONE).await;
    ctx.subscribe(&reminded, &channel, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    ctx.subscribe(&blocked, &channel, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    let stream = ctx.stream(&alice, &channel, "はじまるよ").await;
    let remind = SetReminderRequest { stream_id: card(&stream).id.clone(), enabled: true, ..Default::default() };
    let _: SetReminderResponse = ctx.call(STREAMS, "SetReminder", Some(&reminded), &remind).await.unwrap();
    ctx.block(&alice, &blocked).await;
    ctx.go_live(&alice, &stream).await;

    // The event starts the announcement, once.
    assert_eq!(ctx.deliver("live.stream.started", alice.id).await, 1);
    ctx.deliver("live.stream.started", alice.id).await;
    assert!(ctx.row(&stream).await.announced_at.is_some());
    // The jobs, as the queue would run them: reminders, then subscribers in batches.
    let job = |phase, after| AnnounceJob { stream_id: sid(&stream), phase, after };
    assert_eq!(ctx.live.announce(job(AnnouncePhase::Reminders, None)).await.unwrap(), 1);
    let mut after = None;
    let mut told = 0;
    loop {
        let subscribers = tokuly_video::channels::subscriber_ids(
            &ctx.app.core,
            id(&channel.summary.id),
            tokuly_video::channels::NotifyLevel::Personalized,
        )
        .await
        .unwrap();
        let batch: Vec<Uuid> = subscribers.into_iter().filter(|id| after.is_none_or(|a| *id > a)).take(2).collect();
        told += ctx.live.announce(job(AnnouncePhase::Subscribers, after)).await.unwrap();
        match batch.last() {
            Some(last) if batch.len() == 2 => after = Some(*last),
            _ => break,
        }
    }
    assert_eq!(told, 2, "the two subscribers who were not reminded, are not silent and not blocked");

    let reminder = ctx.notifications(&reminded, "live.reminder").await;
    assert_eq!(reminder.len(), 1);
    assert!(ctx.notifications(&reminded, "live.started").await.is_empty(), "told once, by the reminder");
    assert_eq!(reminder[0].data["title"]["ja"], "お知らせch");
    assert!(reminder[0].data["body"]["ja"].as_str().unwrap().contains("はじまるよ"));
    assert_eq!(reminder[0].data["path"], format!("/live/{}", card(&stream).short_id));
    assert!(reminder[0].actor_id.is_none(), "the channel speaks, not the account behind it");
    let push_of = |events: &[tokuly_core::events::Event]| {
        events
            .iter()
            .map(|e| (e.data["inbox"].as_bool().unwrap(), e.data["push"].as_bool().unwrap()))
            .collect::<Vec<_>>()
    };
    assert_eq!(push_of(&ctx.notifications(&everything, "live.started").await), vec![(true, true)]);
    assert_eq!(push_of(&ctx.notifications(&personalized, "live.started").await), vec![(true, false)]);
    for nobody in [&silent, &blocked, &outsider, &alice] {
        assert!(ctx.notifications(nobody, "live.started").await.is_empty());
    }
    // The owner's friends see it in their feed.
    let activity = ctx.app.events_by("activity.recorded", alice.id).await;
    assert!(activity.iter().any(|e| e.data["kind"] == "live.started" && e.data["audience"] == "friends"));
    // Nothing is announced once the stream is over.
    ctx.end(&alice, &stream).await;
    assert_eq!(ctx.live.announce(job(AnnouncePhase::Subscribers, None)).await.unwrap(), 0);

    // Friends-only: friends are told (and only friends among the subscribers).
    let friend = ctx.app.user("friend").await;
    ctx.befriend(&alice, &friend).await;
    let private_party = ctx
        .on_air_with(&alice, &channel, "友達だけの配信", |r| {
            r.visibility = Visibility::VISIBILITY_FRIENDS.into()
        })
        .await;
    let job = |phase| AnnounceJob { stream_id: sid(&private_party), phase, after: None };
    assert_eq!(ctx.live.announce(job(AnnouncePhase::Subscribers)).await.unwrap(), 0);
    assert_eq!(ctx.live.announce(job(AnnouncePhase::Friends)).await.unwrap(), 1);
    let told = ctx.notifications(&friend, "live.started").await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].actor_id, Some(alice.id));
}

#[tokio::test]
async fn other_domains_events_are_followed() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "イベントch").await;
    let db = &ctx.app.core.db;

    // A takedown ends the stream and removes it; its chat stays for the moderators.
    let stream = ctx.on_air(&alice, &channel, "だめな配信").await;
    let line = ctx.say(&bob, &stream, "見てたのに").await.unwrap();
    let removed = serde_json::json!({ "subject_type": SubjectType::SUBJECT_TYPE_LIVE_STREAM as i32, "subject_id": sid(&stream), "reason": "x", "moderator_id": Uuid::now_v7() });
    ctx.deliver_event("admin.content.removed", None, removed.clone()).await;
    ctx.deliver_event("admin.content.removed", None, removed).await;
    assert_eq!(ctx.get_stream(Some(&alice), &card(&stream).id).await.unwrap_err().reason, "STREAM_NOT_FOUND");
    let row = ctx.row(&stream).await;
    assert!(row.is_deleted() && row.is_ended() && row.end_reason == EndReason::END_REASON_REMOVED as i32);
    assert_eq!(row.archive_status, tokuly_live::streams::ARCHIVE_NONE, "a stream that was taken down is not archived");
    assert!(
        ctx.app
            .core
            .subjects
            .resolve(SubjectType::SUBJECT_TYPE_LIVE_CHAT_MESSAGE, &line.id, Some(alice.id))
            .await
            .is_ok()
    );
    // A chat message that is taken down leaves the chat.
    let second = ctx.on_air(&alice, &channel, "つぎの配信").await;
    let rude = ctx.say(&bob, &second, "だめなことば").await.unwrap();
    ctx.deliver_event("admin.content.removed", None, serde_json::json!({ "subject_type": SubjectType::SUBJECT_TYPE_LIVE_CHAT_MESSAGE as i32, "subject_id": rude.id })).await;
    assert!(ctx.chat(None, &second).await.is_empty());
    // Unknown subjects and other types are ignored.
    ctx.deliver_event(
        "admin.content.removed",
        None,
        serde_json::json!({ "subject_type": SubjectType::SUBJECT_TYPE_NOTE as i32, "subject_id": sid(&second) }),
    )
    .await;
    assert!(ctx.row(&second).await.is_live());

    // A thumbnail whose upload is deleted is cleared.
    let thumbnail = ctx.thumbnail(&alice).await;
    let pictured = ctx.stream_with(&alice, &channel, "サムネあり", |r| r.thumbnail_asset_id = thumbnail.clone()).await;
    ctx.deliver_event(
        "media.asset.deleted",
        None,
        serde_json::json!({ "asset_id": thumbnail, "owner_id": alice.id, "purpose": 14, "kind": 1 }),
    )
    .await;
    assert!(!card(&ctx.get_stream(Some(&alice), &card(&pictured).id).await.unwrap()).thumbnail.is_set());

    // A block ends moderator appointments and reminders between the two.
    ctx.befriend(&alice, &bob).await;
    let add = AddModeratorRequest {
        channel_id: channel.summary.id.clone(),
        user_id: bob.id.to_string(),
        ..Default::default()
    };
    let _: AddModeratorResponse = ctx.call(CHAT, "AddModerator", Some(&alice), &add).await.unwrap();
    let _: SetReminderResponse = ctx
        .call(
            STREAMS,
            "SetReminder",
            Some(&bob),
            &SetReminderRequest { stream_id: card(&pictured).id.clone(), enabled: true, ..Default::default() },
        )
        .await
        .unwrap();
    ctx.block(&alice, &bob).await;
    ctx.deliver_event(
        "social.user.blocked",
        Some(alice.id),
        serde_json::json!({ "user_id": alice.id, "blocked_id": bob.id }),
    )
    .await;
    // The end of a friendship is followed too: it is what cuts a friends-only stream's playback
    // (proven against the real media server in `tests/mediamtx.rs`).
    ctx.deliver_event(
        "social.friend.removed",
        Some(alice.id),
        serde_json::json!({ "user_id": alice.id, "friend_id": bob.id }),
    )
    .await;
    let left: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM live_moderators WHERE user_id = $1), (SELECT count(*) FROM live_reminders WHERE user_id = $1)")
        .bind(bob.id)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(left, (0, 0));

    // Suspension follows the account as it is now, whatever order the events arrive in (checklist 10).
    sqlx::query("UPDATE users SET banned_at = now() WHERE id = $1").bind(alice.id).execute(db).await.unwrap();
    ctx.deliver_event("identity.user.unbanned", None, serde_json::json!({ "user_id": alice.id })).await;
    let row = ctx.row(&second).await;
    assert!(
        row.suspended && row.is_ended(),
        "a late `unbanned` does not reinstate a banned account, and its stream is off air"
    );
    assert_eq!(ctx.get_stream(Some(&bob), &card(&pictured).id).await.unwrap_err().reason, "STREAM_NOT_FOUND");
    assert_eq!(ctx.search_docs(&card(&pictured).id).await.pop().unwrap(), None);
    sqlx::query("UPDATE users SET banned_at = NULL WHERE id = $1").bind(alice.id).execute(db).await.unwrap();
    ctx.deliver_event("identity.user.banned", None, serde_json::json!({ "user_id": alice.id })).await;
    assert!(!ctx.row(&pictured).await.suspended);
    assert!(ctx.search_docs(&card(&pictured).id).await.pop().unwrap().is_some());

    // A deleted channel takes its streams, widgets and chat rules along.
    let carol = ctx.app.user("carol").await;
    let doomed = ctx.channel(&carol, "消えるch").await;
    let on_air = ctx.on_air(&carol, &doomed, "消える配信").await;
    let _: CreateOverlayResponse = ctx
        .call(
            OVERLAYS,
            "CreateOverlay",
            Some(&carol),
            &CreateOverlayRequest {
                channel_id: doomed.summary.id.clone(),
                kind: OverlayKind::OVERLAY_KIND_CHAT_BOX.into(),
                name: "x".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let event =
        serde_json::json!({ "channel_id": doomed.summary.id, "owner_id": carol.id, "handle": doomed.summary.handle });
    ctx.deliver_event("video.channel.deleted", None, event.clone()).await;
    ctx.deliver_event("video.channel.deleted", None, event).await;
    assert!(ctx.row(&on_air).await.is_deleted());
    let overlays: i64 = sqlx::query_scalar("SELECT count(*) FROM live_overlays WHERE owner_id = $1")
        .bind(carol.id)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(overlays, 0);
}

#[tokio::test]
async fn an_erased_account_leaves_nothing_but_bookkeeping() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "残るch").await;
    let bobs_channel = ctx.channel(&bob, "消えるch").await;
    let stream = ctx.on_air(&alice, &channel, "残る配信").await;
    let own = ctx.on_air(&bob, &bobs_channel, "消える配信").await;
    ctx.coins(&bob, 200).await;
    let bobs_line = ctx.say(&bob, &stream, "いたよ").await.unwrap();
    ctx.gift(&bob, &stream, 100, "", "gift-erase-0001").await.unwrap();
    let alices_line = ctx.say(&alice, &own, "来たよ").await.unwrap();
    let _: SetReminderResponse = ctx
        .call(
            STREAMS,
            "SetReminder",
            Some(&bob),
            &SetReminderRequest {
                stream_id: card(&ctx.stream(&alice, &channel, "予定").await).id.clone(),
                enabled: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // The export first: everything the user left here.
    let exporter = ctx.app.core.exporters.all().into_iter().find(|e| e.name() == "live").expect("the live exporter");
    let export = exporter.export(bob.id).await.unwrap();
    assert_eq!(export["streams"].as_array().unwrap().len(), 1);
    assert_eq!(export["chat_messages"].as_array().unwrap().len(), 2, "a text line and the gift's line");
    assert_eq!(export["gifts_sent"][0]["coins"], 100);
    assert_eq!(export["reminders"].as_array().unwrap().len(), 1);
    assert!(!export.to_string().contains("stream_key"));

    let event = serde_json::json!({ "user_id": bob.id });
    ctx.deliver_event("identity.user.deleted", None, event.clone()).await;
    ctx.deliver_event("identity.user.deleted", None, event).await;
    let db = &ctx.app.core.db;
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM live_streams WHERE owner_id = $1), (SELECT count(*) FROM live_chat_messages WHERE user_id = $1),
                (SELECT count(*) FROM live_reminders WHERE user_id = $1), (SELECT count(*) FROM live_gifts WHERE sender_id = $1)",
    )
    .bind(bob.id)
    .fetch_one(db)
    .await
    .unwrap();
    assert_eq!(counts, (0, 0, 0, 0), "streams erased (not kept), messages gone, gifts without the name");
    // The gift stays in the streamer's books, without its sender.
    let received: ListReceivedGiftsResponse = ctx
        .call(
            GIFTS,
            "ListReceivedGifts",
            Some(&alice),
            &ListReceivedGiftsRequest { channel_id: channel.summary.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!((received.gifts.len(), received.total_coins), (1, 100));
    assert!(!received.gifts[0].sender.is_set());
    assert!(ctx.chat(None, &stream).await.is_empty());
    assert!(ctx.row(&stream).await.is_live(), "other people's streams are untouched");
    // Saves of the erased lines go too, those by bob and those on his
    // stream; the second delivery announces nothing again.
    for line in [&bobs_line, &alices_line] {
        assert_eq!(ctx.announced(SubjectType::SUBJECT_TYPE_LIVE_CHAT_MESSAGE, &line.id).await, 1, "{}", line.text);
    }
}

/// Some of an erased account's lines can go at the same time with a stream
/// the retention sweep erases: the rest still go, each announced once.
#[tokio::test]
async fn an_account_erased_while_a_stream_with_its_lines_goes_leaves_no_line() {
    let ctx = start().await;
    let db = &ctx.app.core.db;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let erased = ctx.stream(&alice, &channel, "消える配信").await;
    let kept = ctx.stream(&alice, &channel, "残る配信").await;
    // Bob's first lines are on the stream that goes, more than a batch on the other.
    for (stream, count) in [(&erased, 10), (&kept, 1500)] {
        sqlx::query(
            "INSERT INTO live_chat_messages (stream_id, user_id, kind, text)
             SELECT $1, $2, 1, 'いるよ' FROM generate_series(1, $3) AS n",
        )
        .bind(sid(stream))
        .bind(bob.id)
        .bind(count)
        .execute(db)
        .await
        .unwrap();
    }
    let lines: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM live_chat_messages WHERE stream_id = $1")
        .bind(sid(&kept))
        .fetch_all(db)
        .await
        .unwrap();

    let mut sweep = db.begin().await.unwrap();
    sqlx::query("DELETE FROM live_streams WHERE id = $1").bind(sid(&erased)).execute(&mut *sweep).await.unwrap();
    let erase = ctx.deliver_event("identity.user.deleted", None, serde_json::json!({ "user_id": bob.id }));
    let sweep_commits = async {
        // Once the erasure waits for the lines the sweep holds.
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity
                 WHERE datname = current_database() AND wait_event_type = 'Lock' AND query LIKE '%DELETE FROM live_chat_messages%'",
            )
            .fetch_one(db)
            .await
            .unwrap();
            if waiting > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        sweep.commit().await.unwrap();
    };
    tokio::join!(erase, sweep_commits);

    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM live_chat_messages WHERE user_id = $1")
        .bind(bob.id)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(left, 0, "every line of the account goes");
    for line in &lines {
        assert_eq!(ctx.announced(SubjectType::SUBJECT_TYPE_LIVE_CHAT_MESSAGE, &line.to_string()).await, 1);
    }
}

/// One account asking for many things at once must not leave the API
/// without database connections: every transaction here reads through
/// itself, never through the pool (checklist 1). The test pool has five
/// connections; if a request held one while waiting for a second, these
/// would wait out the pool's timeout instead of finishing in moments.
#[tokio::test]
async fn one_account_cannot_stall_the_api_with_concurrent_requests() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "同時ch").await;
    let stream = ctx.on_air(&alice, &channel, "同時の配信").await;
    ctx.subscribe(&bob, &channel, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    ctx.coins(&bob, 10_000).await;
    let create: Vec<CreateStreamRequest> = (0..12)
        .map(|i| CreateStreamRequest {
            channel_id: channel.summary.id.clone(),
            title: format!("同時{i}"),
            ..Default::default()
        })
        .collect();
    let update: Vec<UpdateStreamRequest> = (0..12)
        .map(|i| UpdateStreamRequest {
            stream_id: card(&stream).id.clone(),
            title: Some(format!("改題{i}")),
            ..Default::default()
        })
        .collect();
    let overlay: Vec<CreateOverlayRequest> = (0..12)
        .map(|i| CreateOverlayRequest {
            channel_id: channel.summary.id.clone(),
            kind: OverlayKind::OVERLAY_KIND_CHAT_BOX.into(),
            name: format!("w{i}"),
            ..Default::default()
        })
        .collect();
    let moderator = AddModeratorRequest {
        channel_id: channel.summary.id.clone(),
        user_id: bob.id.to_string(),
        ..Default::default()
    };
    let gifts: Vec<String> = (0..12).map(|i| format!("gift-flood-{i:04}")).collect();
    let everything = async {
        tokio::join!(
            join_all(create.iter().map(|request| ctx.call::<_, CreateStreamResponse>(
                STREAMS,
                "CreateStream",
                Some(&alice),
                request
            ))),
            join_all(update.iter().map(|request| ctx.call::<_, UpdateStreamResponse>(
                STREAMS,
                "UpdateStream",
                Some(&alice),
                request
            ))),
            join_all(overlay.iter().map(|request| ctx.call::<_, CreateOverlayResponse>(
                OVERLAYS,
                "CreateOverlay",
                Some(&alice),
                request
            ))),
            join_all((0..12).map(|_| ctx.call::<_, AddModeratorResponse>(
                CHAT,
                "AddModerator",
                Some(&alice),
                &moderator
            ))),
            join_all(gifts.iter().map(|key| ctx.gift(&bob, &stream, 10, "", key))),
            join_all((0..8).map(|_| ctx.say(&bob, &stream, "同時に"))),
            join_all((0..12).map(|_| ctx.live.go_live(sid(&stream)))),
        )
    };
    let (created, updated, overlays, moderators, gifted, said, _) =
        tokio::time::timeout(Duration::from_secs(20), everything)
            .await
            .expect("concurrent requests of one account must not stall");
    // The limits held under concurrency: nineteen more streams fit next to the one on air.
    assert!(created.iter().all(|result| result.is_ok()));
    assert!(updated.iter().all(|result| result.is_ok()));
    assert!(overlays.iter().all(|result| result.is_ok()));
    assert!(moderators.iter().all(|result| result.is_ok()));
    assert!(gifted.iter().all(|result| result.is_ok()));
    assert!(said.iter().all(|result| result.is_ok()));
    assert_eq!(ctx.balance(&bob).await.0, 10_000 - 120);
    assert_eq!(ctx.row(&stream).await.gift_count, 12);
    let appointed: i64 = sqlx::query_scalar("SELECT count(*) FROM live_moderators WHERE channel_id = $1")
        .bind(id(&channel.summary.id))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(appointed, 1);
    assert_eq!(ctx.notifications(&bob, "live.moderator").await.len(), 1);
    // And the limit itself: twenty streams that are not over, however the requests are timed.
    let more: Vec<CreateStreamRequest> = (0..12)
        .map(|i| CreateStreamRequest {
            channel_id: channel.summary.id.clone(),
            title: format!("あふれ{i}"),
            ..Default::default()
        })
        .collect();
    let results = join_all(
        more.iter().map(|request| ctx.call::<_, CreateStreamResponse>(STREAMS, "CreateStream", Some(&alice), request)),
    )
    .await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 7);
    assert!(
        results.iter().filter_map(|result| result.as_ref().err()).all(|error| error.reason == "STREAM_LIMIT_REACHED")
    );
}

/// Ending a stream from several sides at once ends it once (checklist 6).
#[tokio::test]
async fn a_stream_ends_once_however_many_ask() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "一回ch").await;
    let stream = ctx.on_air(&alice, &channel, "一回で終わる").await;
    let end = EndStreamRequest { stream_id: card(&stream).id.clone(), ..Default::default() };
    let results =
        join_all((0..6).map(|_| ctx.call::<_, EndStreamResponse>(STREAMS, "EndStream", Some(&alice), &end))).await;
    assert!(results.iter().all(|result| result.is_ok()));
    let ended = ctx
        .app
        .events("live.stream.ended")
        .await
        .into_iter()
        .filter(|e| e.data["stream_id"] == card(&stream).id.as_str())
        .count();
    assert_eq!(ended, 1);
}

fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg").arg("-version").output().is_ok_and(|out| out.status.success())
}

/// Records `seconds` of test picture and tone into `path` as fragmented MP4.
fn record(path: &std::path::Path, seconds: u32) {
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=15",
            "-f",
            "lavfi",
            "-i",
            "sine",
        ])
        .args([
            "-t",
            &seconds.to_string(),
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-g",
            "15",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
        ])
        .args(["-movflags", "+frag_keyframe+empty_moov+default_base_moof"])
        .arg(path)
        .status()
        .expect("ffmpeg runs");
    assert!(status.success(), "ffmpeg could not record a test segment");
}

#[tokio::test]
async fn the_recording_of_an_ended_stream_becomes_a_video() {
    if !ffmpeg_available() {
        eprintln!("skipped: ffmpeg is not installed, so no recording can be made or archived");
        return;
    }
    let recordings = tempfile::tempdir().unwrap();
    let ctx =
        start_with(LiveConfig { recordings_dir: recordings.path().to_string_lossy().into_owned(), ..test_config() })
            .await;
    // Uploads need their buckets; a fresh object store has none.
    ctx.app.core.storage.ensure_buckets().await.unwrap();
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "アーカイブch").await;
    let stream = ctx.on_air_with(&alice, &channel, "のこる配信", |r| r.description = "アーカイブされます".into()).await;
    let row = ctx.row(&stream).await;
    // The media server's part: two segments, as it would have recorded them since the stream went on air.
    let dir = recordings.path().join("live").join(&row.short_id);
    std::fs::create_dir_all(&dir).unwrap();
    let started = now() - chrono::Duration::seconds(8);
    sqlx::query("UPDATE live_streams SET started_at = $2 WHERE id = $1")
        .bind(row.id)
        .bind(started)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    for (offset, seconds) in [(0i64, 4u32), (4, 3)] {
        let start = started + chrono::Duration::seconds(offset);
        record(&dir.join(format!("{}-{:06}.mp4", start.timestamp(), start.timestamp_subsec_micros())), seconds);
    }
    // A clip cut while on air belongs to the stream, later also to the archive.
    let clip: CreateClipResponse = ctx
        .call(
            CLIPS,
            "CreateClip",
            Some(&bob),
            &CreateClipRequest {
                stream_id: card(&stream).id.clone(),
                title: "いいところ".into(),
                start_seconds: 1.0,
                duration_seconds: 2.0,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let clip_id = id(&clip.clip.as_option().unwrap().id);
    ctx.live.cut_clip(tokuly_live::events::ClipJob { clip_id }).await.unwrap();
    let cut = tokuly_live::clips::find_by_id(&ctx.app.core, clip_id).await.unwrap().unwrap();
    let asset_id = cut.asset_id.unwrap_or_else(|| panic!("the clip was cut into an upload ({})", cut.error));
    // The upload belongs to the channel's owner, not to whoever cut the clip: the cutter
    // cannot attach a piece of someone's stream to their own posts.
    let upload = tokuly_media::assets::find(&ctx.app.core, asset_id).await.unwrap().unwrap();
    assert_eq!(upload.owner_id, alice.id);
    ctx.media.process_now(asset_id).await.unwrap();
    ctx.deliver_event(
        "media.asset.ready",
        None,
        serde_json::json!({ "asset_id": asset_id, "owner_id": alice.id, "purpose": 13, "kind": 2 }),
    )
    .await;
    ctx.deliver_event(
        "media.asset.ready",
        None,
        serde_json::json!({ "asset_id": asset_id, "owner_id": alice.id, "purpose": 13, "kind": 2 }),
    )
    .await;
    let ready: GetClipResponse = ctx
        .call(CLIPS, "GetClip", None, &GetClipRequest { clip_id: clip_id.to_string(), ..Default::default() })
        .await
        .unwrap();
    let ready = ready.clip.as_option().unwrap();
    assert_eq!(ready.status, ClipStatus::CLIP_STATUS_READY);
    let playback = ready.playback.as_option().expect("a ready clip plays");
    assert!(
        (1.5..=3.0).contains(&playback.duration_seconds),
        "about two seconds were cut, got {}",
        playback.duration_seconds
    );
    assert_eq!(
        ctx.app.events("live.clip.ready").await.iter().filter(|e| e.data["clip_id"] == clip_id.to_string()).count(),
        1
    );

    // The stream ends; the event queues the archive, the job makes it.
    let ended = ctx.end(&alice, &stream).await;
    assert_eq!(ended.archive_status, ArchiveStatus::ARCHIVE_STATUS_PENDING);
    assert_eq!(ctx.deliver("live.stream.ended", alice.id).await, 1);
    ctx.live.archive(ArchiveJob { stream_id: row.id }).await.unwrap();
    let archived = ctx.get_stream(Some(&bob), &card(&stream).id).await.unwrap();
    assert_eq!(archived.archive_status, ArchiveStatus::ARCHIVE_STATUS_READY);
    assert_eq!(archived.archive_video_id.len(), 11);
    let video = tokuly_video::videos::find_ref(&ctx.app.core, &archived.archive_video_id)
        .await
        .unwrap()
        .expect("the archive video");
    assert_eq!(
        (video.title.as_str(), video.description.as_str(), video.channel_id),
        ("のこる配信", "アーカイブされます", row.channel_id)
    );
    assert_eq!(
        (video.origin, video.origin_id),
        (tokuly_proto::video::VideoOrigin::VIDEO_ORIGIN_LIVE_ARCHIVE as i32, Some(row.id))
    );
    assert_eq!(video.visibility, Visibility::VISIBILITY_PUBLIC as i32);
    // The upload really is the recording: about seven seconds of video.
    ctx.media.process_now(video.asset_id).await.unwrap();
    let asset = tokuly_media::assets::find(&ctx.app.core, video.asset_id).await.unwrap().unwrap();
    let duration = asset.video().expect("a processed video").duration_seconds;
    assert!((6.0..=8.5).contains(&duration), "the archive is {duration} s long");
    // The segments are gone, the clip now also belongs to the video, and the job is safe to run again.
    assert!(!dir.exists());
    assert_eq!(tokuly_live::clips::find_by_id(&ctx.app.core, clip_id).await.unwrap().unwrap().video_id, Some(video.id));
    assert_eq!(
        ctx.app
            .events("live.stream.archived")
            .await
            .iter()
            .filter(|e| e.data["stream_id"] == row.id.to_string())
            .count(),
        1
    );
    ctx.live.archive(ArchiveJob { stream_id: row.id }).await.unwrap();
    assert_eq!(
        ctx.app
            .events("live.stream.archived")
            .await
            .iter()
            .filter(|e| e.data["stream_id"] == row.id.to_string())
            .count(),
        1
    );
    // The archive's chat replays by video (for its owner already while the video is being processed).
    let replay: ListChatReplayResponse = ctx
        .call(
            CHAT,
            "ListChatReplay",
            Some(&alice),
            &ListChatReplayRequest {
                video_id: archived.archive_video_id.clone(),
                from_offset_ms: 0,
                to_offset_ms: 60_000,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(replay.messages.is_empty());
    // The video being deleted leaves the stream without an archive.
    ctx.deliver_event("video.video.deleted", None, serde_json::json!({ "video_id": video.id, "short_id": video.short_id, "channel_id": video.channel_id, "owner_id": alice.id, "visibility": 1, "origin": 2 })).await;
    assert_eq!(
        ctx.get_stream(Some(&alice), &card(&stream).id).await.unwrap().archive_status,
        ArchiveStatus::ARCHIVE_STATUS_NONE
    );

    // A stream deleted before its archive is made leaves no video behind, and no archive
    // that would wait (and be queued again) forever.
    let gone = ctx.on_air(&alice, &channel, "消す配信").await;
    let gone_row = ctx.row(&gone).await;
    let gone_dir = recordings.path().join("live").join(&gone_row.short_id);
    std::fs::create_dir_all(&gone_dir).unwrap();
    let gone_started = gone_row.started_at.unwrap();
    record(
        &gone_dir.join(format!("{}-{:06}.mp4", gone_started.timestamp(), gone_started.timestamp_subsec_micros())),
        2,
    );
    assert_eq!(ctx.end(&alice, &gone).await.archive_status, ArchiveStatus::ARCHIVE_STATUS_PENDING);
    let _: DeleteStreamResponse = ctx
        .call(
            STREAMS,
            "DeleteStream",
            Some(&alice),
            &DeleteStreamRequest { stream_id: card(&gone).id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    ctx.live.archive(ArchiveJob { stream_id: gone_row.id }).await.unwrap();
    let after = ctx.row(&gone).await;
    assert_eq!((after.archive_status, after.archive_video_id), (tokuly_live::streams::ARCHIVE_NONE, None));
    let videos: i64 = sqlx::query_scalar("SELECT count(*) FROM videos WHERE origin_id = $1")
        .bind(gone_row.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(videos, 0, "nothing is published from a deleted stream");
    sqlx::query("UPDATE live_streams SET archive_changed_at = now() - interval '1 day' WHERE id = $1")
        .bind(gone_row.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.live.sweep_recordings().await.unwrap();
    assert_eq!(ctx.row(&gone).await.archive_status, tokuly_live::streams::ARCHIVE_NONE);
}

#[tokio::test]
async fn recordings_nobody_needs_are_swept_and_nothing_is_archived_from_nothing() {
    let recordings = tempfile::tempdir().unwrap();
    let ctx =
        start_with(LiveConfig { recordings_dir: recordings.path().to_string_lossy().into_owned(), ..test_config() })
            .await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "掃除ch").await;
    let segment = |short_id: &str| {
        let dir = recordings.path().join("live").join(short_id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{}-000000.mp4", now().timestamp())), b"not really video").unwrap();
        dir
    };
    // A stream that ended without anything usable on disk has no archive.
    let empty = ctx.on_air(&alice, &channel, "なにも録れてない").await;
    ctx.end(&alice, &empty).await;
    ctx.live.archive(ArchiveJob { stream_id: sid(&empty) }).await.unwrap();
    assert_eq!(
        ctx.get_stream(Some(&alice), &card(&empty).id).await.unwrap().archive_status,
        ArchiveStatus::ARCHIVE_STATUS_NONE
    );

    let orphan = segment("orphanAAAAA");
    let on_air = ctx.on_air(&alice, &channel, "配信中").await;
    let kept = segment(&card(&on_air).short_id);
    let no_archive = ctx.on_air_with(&alice, &channel, "残さない", |r| r.auto_archive = Some(false)).await;
    ctx.end(&alice, &no_archive).await;
    let dropped = segment(&card(&no_archive).short_id);
    sqlx::query("UPDATE live_streams SET ended_at = now() - interval '5 minutes' WHERE id = $1")
        .bind(sid(&no_archive))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let sweep = ctx.live.sweep_recordings().await.unwrap();
    assert!(sweep.removed_recordings >= 2);
    assert!(!orphan.exists(), "a recording without a stream goes");
    assert!(!dropped.exists(), "a recording that will not be archived goes");
    assert!(kept.exists(), "a stream on air keeps its recording");
    // A stream deleted long ago is erased by the sweep's purge.
    let old = ctx.stream(&alice, &channel, "むかし消した").await;
    let _: DeleteStreamResponse = ctx
        .call(
            STREAMS,
            "DeleteStream",
            Some(&alice),
            &DeleteStreamRequest { stream_id: card(&old).id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    sqlx::query("UPDATE live_streams SET deleted_at = now() - interval '40 days' WHERE id = $1")
        .bind(sid(&old))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    // A long chat, more than one batch of announcements; one line was
    // removed (and announced) before.
    let lines: Vec<(Uuid, bool)> = sqlx::query_as(
        "INSERT INTO live_chat_messages (stream_id, user_id, kind, text, deleted_at)
         SELECT $1, $2, 1, 'こんにちは', CASE WHEN n = 1 THEN now() END FROM generate_series(1, 1500) AS n
         RETURNING id, deleted_at IS NOT NULL",
    )
    .bind(sid(&old))
    .bind(alice.id)
    .fetch_all(&ctx.app.core.db)
    .await
    .unwrap();
    // Two sweeps at once (two servers): the stream is erased, and announced, once.
    let purge = || tokuly_live::streams::purge_deleted(&ctx.app.core, test_config().deleted_retention_days);
    let (one, other) = tokio::join!(purge(), purge());
    assert!(one.unwrap() + other.unwrap() >= 1);
    for (line, removed_before) in &lines {
        let expected = if *removed_before { 0 } else { 1 };
        assert_eq!(ctx.announced(SubjectType::SUBJECT_TYPE_LIVE_CHAT_MESSAGE, &line.to_string()).await, expected);
    }
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM live_streams WHERE id = $1")
        .bind(sid(&old))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
    assert_eq!(ctx.announced(SubjectType::SUBJECT_TYPE_LIVE_STREAM, &card(&old).id).await, 1);
}
