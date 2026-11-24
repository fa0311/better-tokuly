//! `WatchPartyService`: synchronized playback, seats, guests, chat, and who
//! may listen to a party's realtime topic.
mod common;

use std::sync::Arc;

use async_trait::async_trait;
use common::*;
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::{StampRef, Visibility},
    video::*,
};
use tokuly_testkit::{RpcError, TestUser};
use tokuly_video::{PartyStamps, VideoConfig, party};

async fn create(ctx: &Ctx, host: &TestUser, video: &Video) -> PartySession {
    let request = CreatePartyRequest { video_id: card(video).id.clone(), ..Default::default() };
    let response: CreatePartyResponse =
        ctx.call(PARTIES, "CreateParty", Some(host), &request).await.expect("create party");
    response.session.as_option().cloned().unwrap()
}

async fn try_join(
    ctx: &Ctx,
    user: Option<&TestUser>,
    code: &str,
    guest_name: &str,
    guest_token: &str,
) -> Result<PartySession, RpcError> {
    let request = JoinPartyRequest {
        code: code.to_owned(),
        guest_name: guest_name.to_owned(),
        guest_token: guest_token.to_owned(),
        ..Default::default()
    };
    let response: JoinPartyResponse = ctx.call(PARTIES, "JoinParty", user, &request).await?;
    Ok(response.session.as_option().cloned().unwrap())
}

async fn join(ctx: &Ctx, user: &TestUser, session: &PartySession) -> PartySession {
    try_join(ctx, Some(user), &session.party.code, "", "").await.expect("join party")
}

async fn join_as_guest(ctx: &Ctx, session: &PartySession, name: &str) -> PartySession {
    try_join(ctx, None, &session.party.code, name, "").await.expect("join party as a guest")
}

async fn members(ctx: &Ctx, viewer: &TestUser, party_id: &str) -> Vec<(String, PartyRole)> {
    let response: GetPartyResponse = ctx
        .call(
            PARTIES,
            "GetParty",
            Some(viewer),
            &GetPartyRequest { party_id: party_id.to_owned(), ..Default::default() },
        )
        .await
        .expect("get party");
    response.session.members.iter().map(|member| (member.name.clone(), member.role.as_known().unwrap())).collect()
}

async fn leave(ctx: &Ctx, user: &TestUser, party_id: &str) -> Result<LeavePartyResponse, RpcError> {
    let request = LeavePartyRequest { party_id: party_id.to_owned(), ..Default::default() };
    ctx.call(PARTIES, "LeaveParty", Some(user), &request).await
}

async fn may_listen(ctx: &Ctx, subscriber: Option<&TestUser>, topic: &str) -> bool {
    ctx.app.core.topic_guards.may_subscribe(subscriber.map(|user| user.id), topic).await.expect("topic guard")
}

async fn say(
    ctx: &Ctx,
    user: Option<&TestUser>,
    party_id: &str,
    guest_token: &str,
    text: &str,
) -> Result<PartyMessage, RpcError> {
    let request = SendPartyMessageRequest {
        party_id: party_id.to_owned(),
        guest_token: guest_token.to_owned(),
        text: text.to_owned(),
        ..Default::default()
    };
    let response: SendPartyMessageResponse = ctx.call(PARTIES, "SendPartyMessage", user, &request).await?;
    Ok(response.message.as_option().cloned().unwrap())
}

/// A public video on a new channel of `owner`.
async fn some_video(ctx: &Ctx, owner: &TestUser, title: &str) -> Video {
    let channel = ctx.channel(owner, "ch").await;
    ctx.public_video(owner, &channel, title).await
}

#[tokio::test]
async fn a_host_starts_a_party_and_people_join_by_code() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let video = some_video(&ctx, &alice, "みんなで見よう").await;

    let request =
        CreatePartyRequest { video_id: card(&video).short_id.clone(), position_seconds: 3.0, ..Default::default() };
    let response: CreatePartyResponse = ctx.call(PARTIES, "CreateParty", Some(&alice), &request).await.unwrap();
    let session = response.session.as_option().cloned().unwrap();
    let party_id = session.party.id.clone();
    assert_eq!(session.party.code.len(), 10);
    assert_eq!(session.party.invite_url, format!("http://localhost:8080/party/{}", session.party.code));
    assert_eq!(session.party.video.as_option().unwrap().id, card(&video).id);
    assert_eq!((session.party.member_count, session.party.max_members, session.party.allow_guests), (1, 100, true));
    let playback = session.party.playback.as_option().unwrap();
    assert_eq!((playback.playing, playback.position_seconds, playback.rate, playback.sequence), (false, 3.0, 1.0, 1));
    assert_eq!((session.self_.role, session.self_.is_guest), (PartyRole::PARTY_ROLE_HOST.into(), false));
    assert_eq!(session.self_.user.as_option().unwrap().id, alice.id.to_string());
    assert_eq!(session.realtime_topic, format!("party:{party_id}"));
    assert!(session.server_time.as_option().is_some() && session.guest_token.is_empty());

    // Only members may follow the topic: knowing the party ID is not enough.
    assert!(may_listen(&ctx, Some(&alice), &session.realtime_topic).await);
    assert!(!may_listen(&ctx, Some(&bob), &session.realtime_topic).await);
    assert!(!may_listen(&ctx, None, &session.realtime_topic).await);

    // The invite page shows the party before joining.
    let preview: PreviewPartyResponse = ctx
        .call(
            PARTIES,
            "PreviewParty",
            None,
            &PreviewPartyRequest { code: session.party.code.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!((preview.party.id.as_str(), preview.host.name.as_str()), (party_id.as_str(), "alice"));
    assert_eq!(
        ctx.reason(
            PARTIES,
            "PreviewParty",
            None,
            &PreviewPartyRequest { code: "nosuchcode".into(), ..Default::default() }
        )
        .await,
        "PARTY_NOT_FOUND"
    );

    // Joining: everyone present is told.
    let mut topic = ctx.app.listen_topic(&session.realtime_topic).await;
    let joined = join(&ctx, &bob, &session).await;
    assert_eq!(
        (joined.self_.role, joined.members.len(), joined.party.member_count),
        (PartyRole::PARTY_ROLE_MEMBER.into(), 2, 2)
    );
    assert_eq!(joined.members[0].name, "alice", "the host comes first");
    match next_push(&mut topic).await {
        Push::PartyMemberJoined(member) => {
            assert_eq!((member.name.as_str(), member.id.as_str()), ("bob", joined.self_.id.as_str()))
        }
        other => panic!("unexpected push {other:?}"),
    }
    assert!(may_listen(&ctx, Some(&bob), &session.realtime_topic).await);
    // Joining again (a reload) keeps the seat and announces nothing.
    let again = join(&ctx, &bob, &session).await;
    assert_eq!((again.self_.id.as_str(), again.party.member_count), (joined.self_.id.as_str(), 2));
    assert!(pushes(topic.drain(QUIET).await).is_empty());

    // Only those inside see the party itself.
    assert_eq!(members(&ctx, &bob, &party_id).await.len(), 2);
    let get = GetPartyRequest { party_id: party_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(PARTIES, "GetParty", Some(&carol), &get).await, "NOT_A_PARTY_MEMBER");
    assert_eq!(ctx.reason(PARTIES, "GetParty", None, &get).await, "NOT_A_PARTY_MEMBER");
    let unknown = GetPartyRequest { party_id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.reason(PARTIES, "GetParty", Some(&bob), &unknown).await, "PARTY_NOT_FOUND");
    assert_eq!(try_join(&ctx, Some(&carol), "nosuchcode", "", "").await.unwrap_err().reason, "PARTY_NOT_FOUND");

    // Heartbeats keep the seat and carry the state.
    let beat: HeartbeatPartyResponse = ctx
        .call(
            PARTIES,
            "HeartbeatParty",
            Some(&bob),
            &HeartbeatPartyRequest { party_id: party_id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!((beat.member_count, beat.playback.position_seconds), (2, 3.0));
    assert_eq!(
        ctx.reason(
            PARTIES,
            "HeartbeatParty",
            Some(&carol),
            &HeartbeatPartyRequest { party_id: party_id.clone(), ..Default::default() }
        )
        .await,
        "NOT_A_PARTY_MEMBER"
    );
}

#[tokio::test]
async fn parties_need_a_video_the_host_can_play() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let minor = ctx.app.user("minor").await;
    sqlx::query("UPDATE users SET birthdate = current_date - interval '15 years' WHERE id = $1")
        .bind(minor.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.public_video(&alice, &channel, "ok").await;
    let private = ctx.published(&alice, &channel, "private", Visibility::VISIBILITY_PRIVATE).await;
    let adult_only =
        ctx.published_with(&alice, &channel, "R18", Visibility::VISIBILITY_PUBLIC, |r| r.age_restricted = true).await;

    let request = |video: &Video| CreatePartyRequest { video_id: card(video).id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(PARTIES, "CreateParty", Some(&minor), &request(&private)).await, "VIDEO_NOT_FOUND");
    assert_eq!(ctx.reason(PARTIES, "CreateParty", Some(&minor), &request(&adult_only)).await, "PLAYBACK_BLOCKED");
    assert_eq!(ctx.reason(PARTIES, "CreateParty", None, &request(&video)).await, "SIGN_IN_REQUIRED");
    let beyond = CreatePartyRequest { position_seconds: 500.0, ..request(&video) };
    assert_eq!(ctx.reason(PARTIES, "CreateParty", Some(&minor), &beyond).await, "PLAYBACK_INVALID");
    let no_list = CreatePartyRequest { playlist_id: "nosuchlist1".into(), ..request(&video) };
    assert_eq!(ctx.reason(PARTIES, "CreateParty", Some(&minor), &no_list).await, "PLAYLIST_NOT_FOUND");

    // A party can go through a playlist.
    let playlist = CreatePlaylistRequest {
        title: "連続再生".into(),
        visibility: Visibility::VISIBILITY_UNLISTED.into(),
        video_id: card(&video).id.clone(),
        ..Default::default()
    };
    let playlist: CreatePlaylistResponse =
        ctx.call(PLAYLISTS, "CreatePlaylist", Some(&alice), &playlist).await.unwrap();
    let with_list =
        CreatePartyRequest { playlist_id: playlist.playlist.id.clone(), allow_guests: Some(false), ..request(&video) };
    let response: CreatePartyResponse = ctx.call(PARTIES, "CreateParty", Some(&minor), &with_list).await.unwrap();
    assert_eq!(response.session.party.playlist_id, playlist.playlist.short_id);
    assert!(!response.session.party.allow_guests);
    // An adult-only video keeps minors and guests out of someone else's party as well.
    let session = create(&ctx, &alice, &adult_only).await;
    assert_eq!(try_join(&ctx, Some(&minor), &session.party.code, "", "").await.unwrap_err().reason, "PLAYBACK_BLOCKED");
    assert_eq!(try_join(&ctx, None, &session.party.code, "ゲスト", "").await.unwrap_err().reason, "PLAYBACK_BLOCKED");
}

#[tokio::test]
async fn limits_apply_to_hosting_and_to_seats() {
    let ctx = start_with(VideoConfig { party_max_hosted: 1, party_max_members: 2, ..test_config() }).await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let video = some_video(&ctx, &alice, "v").await;
    let session = create(&ctx, &alice, &video).await;
    let second = CreatePartyRequest { video_id: card(&video).id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(PARTIES, "CreateParty", Some(&alice), &second).await, "PARTY_LIMIT_REACHED");

    join(&ctx, &bob, &session).await;
    assert_eq!(try_join(&ctx, Some(&carol), &session.party.code, "", "").await.unwrap_err().reason, "PARTY_FULL");
    assert_eq!(try_join(&ctx, None, &session.party.code, "ゲスト", "").await.unwrap_err().reason, "PARTY_FULL");
    // Those already inside can come back; a freed seat can be taken.
    assert!(try_join(&ctx, Some(&bob), &session.party.code, "", "").await.is_ok());
    let _: LeavePartyResponse = ctx
        .call(
            PARTIES,
            "LeaveParty",
            Some(&bob),
            &LeavePartyRequest { party_id: session.party.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(try_join(&ctx, Some(&carol), &session.party.code, "", "").await.is_ok());

    // Ending a party frees the host to start another.
    let _: EndPartyResponse = ctx
        .call(
            PARTIES,
            "EndParty",
            Some(&alice),
            &EndPartyRequest { party_id: session.party.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(ctx.call::<_, CreatePartyResponse>(PARTIES, "CreateParty", Some(&alice), &second).await.is_ok());
}

#[tokio::test]
async fn guests_join_by_link_and_listen_on_a_topic_of_their_own() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let video = some_video(&ctx, &alice, "ゲスト歓迎").await;
    let session = create(&ctx, &alice, &video).await;
    let party_id = session.party.id.clone();

    assert_eq!(try_join(&ctx, None, &session.party.code, "  ", "").await.unwrap_err().reason, "GUEST_NAME_INVALID");
    assert_eq!(
        try_join(&ctx, None, &session.party.code, &"g".repeat(25), "").await.unwrap_err().reason,
        "GUEST_NAME_INVALID"
    );
    let mut member_topic = ctx.app.listen_topic(&session.realtime_topic).await;
    let guest = join_as_guest(&ctx, &session, " ゲスト太郎 ").await;
    assert_eq!(
        (guest.self_.name.as_str(), guest.self_.is_guest, guest.self_.role),
        ("ゲスト太郎", true, PartyRole::PARTY_ROLE_MEMBER.into())
    );
    assert!(guest.self_.user.as_option().is_none() && !guest.guest_token.is_empty());
    assert!(matches!(next_push(&mut member_topic).await, Push::PartyMemberJoined(member) if member.is_guest));

    // The guest's topic carries a secret of their seat; the plain party topic stays closed to them.
    assert!(
        guest.realtime_topic.starts_with(&format!("party:{party_id}:"))
            && guest.realtime_topic.len() > session.realtime_topic.len() + 10
    );
    assert!(may_listen(&ctx, None, &guest.realtime_topic).await);
    assert!(!may_listen(&ctx, None, &format!("party:{party_id}")).await);
    assert!(!may_listen(&ctx, None, &format!("party:{party_id}:guessedkey")).await);
    assert!(!may_listen(&ctx, None, &format!("party:{party_id}:a:b")).await);
    assert!(!may_listen(&ctx, None, "party:not-a-uuid").await);
    // A second guest gets a different topic.
    let other = join_as_guest(&ctx, &session, "ゲスト花子").await;
    assert_ne!(other.realtime_topic, guest.realtime_topic);
    assert!(
        matches!(next_push(&mut member_topic).await, Push::PartyMemberJoined(member) if member.name == "ゲスト花子")
    );

    // Guests act with their token: get, heartbeat, chat. Events reach every topic.
    let get =
        GetPartyRequest { party_id: party_id.clone(), guest_token: guest.guest_token.clone(), ..Default::default() };
    let seen: GetPartyResponse = ctx.call(PARTIES, "GetParty", None, &get).await.unwrap();
    assert_eq!((seen.session.self_.id.as_str(), seen.session.members.len()), (guest.self_.id.as_str(), 3));
    assert!(seen.session.guest_token.is_empty(), "the token is handed out once, on joining");
    let wrong =
        GetPartyRequest { party_id: party_id.clone(), guest_token: "not-my-token".into(), ..Default::default() };
    assert_eq!(ctx.reason(PARTIES, "GetParty", None, &wrong).await, "NOT_A_PARTY_MEMBER");
    let beat = HeartbeatPartyRequest {
        party_id: party_id.clone(),
        guest_token: guest.guest_token.clone(),
        ..Default::default()
    };
    let beat: HeartbeatPartyResponse = ctx.call(PARTIES, "HeartbeatParty", None, &beat).await.unwrap();
    assert_eq!(beat.member_count, 3);
    let mut guest_topic = ctx.app.listen_topic(&guest.realtime_topic).await;
    let mut other_topic = ctx.app.listen_topic(&other.realtime_topic).await;
    let message = say(&ctx, None, &party_id, &guest.guest_token, "こんにちは!").await.unwrap();
    assert_eq!(
        (message.text.as_str(), message.sender.name.as_str(), message.sender.is_guest),
        ("こんにちは!", "ゲスト太郎", true)
    );
    for listener in [&mut member_topic, &mut guest_topic, &mut other_topic] {
        assert!(matches!(next_push(listener).await, Push::PartyMessage(pushed) if pushed.id == message.id));
    }

    // Rejoining with the token (a reload) keeps the seat.
    let back = try_join(&ctx, None, &session.party.code, "別の名前", &guest.guest_token).await.unwrap();
    assert_eq!(
        (back.self_.id.as_str(), back.self_.name.as_str(), back.realtime_topic.as_str()),
        (guest.self_.id.as_str(), "ゲスト太郎", guest.realtime_topic.as_str())
    );

    // Guests watch and chat; they do not run the party or send stamps.
    let control = SetPartyPlaybackRequest { party_id: party_id.clone(), playing: true, ..Default::default() };
    assert_eq!(ctx.reason(PARTIES, "SetPartyPlayback", None, &control).await, "SIGN_IN_REQUIRED");
    let promote = SetPartyMemberRoleRequest {
        party_id: party_id.clone(),
        member_id: guest.self_.id.clone(),
        role: PartyRole::PARTY_ROLE_COHOST.into(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(PARTIES, "SetPartyMemberRole", Some(&alice), &promote).await, "GUESTS_CANNOT_HOST");
    let stamp = SendPartyMessageRequest {
        party_id: party_id.clone(),
        guest_token: guest.guest_token.clone(),
        stamp_id: Uuid::now_v7().to_string(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(PARTIES, "SendPartyMessage", None, &stamp).await, "STAMP_NOT_USABLE");

    // The host can close the door: nobody new comes in as a guest, the ones inside stay.
    let close = UpdatePartyRequest { party_id: party_id.clone(), allow_guests: Some(false), ..Default::default() };
    assert_eq!(ctx.reason(PARTIES, "UpdateParty", Some(&bob), &close).await, "NOT_A_PARTY_MEMBER");
    let response: UpdatePartyResponse = ctx.call(PARTIES, "UpdateParty", Some(&alice), &close).await.unwrap();
    assert!(!response.party.allow_guests);
    assert!(matches!(next_push(&mut guest_topic).await, Push::PartyUpdated(updated) if !updated.allow_guests));
    assert_eq!(
        try_join(&ctx, None, &session.party.code, "遅刻ゲスト", "").await.unwrap_err().reason,
        "PARTY_GUESTS_NOT_ALLOWED"
    );
    assert!(say(&ctx, None, &party_id, &guest.guest_token, "まだいるよ").await.is_ok());
    assert!(try_join(&ctx, Some(&bob), &session.party.code, "", "").await.is_ok(), "accounts still join");

    // A guest who leaves is cut off from the topic at once.
    let leave =
        LeavePartyRequest { party_id: party_id.clone(), guest_token: guest.guest_token.clone(), ..Default::default() };
    let _: LeavePartyResponse = ctx.call(PARTIES, "LeaveParty", None, &leave).await.unwrap();
    assert!(!may_listen(&ctx, None, &guest.realtime_topic).await);
    assert_eq!(
        say(&ctx, None, &party_id, &guest.guest_token, "もういない").await.unwrap_err().reason,
        "NOT_A_PARTY_MEMBER"
    );
    let _: LeavePartyResponse = ctx.call(PARTIES, "LeaveParty", None, &leave).await.unwrap();
}

#[tokio::test]
async fn friends_only_videos_keep_strangers_and_guests_out() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let friend = ctx.app.user("friend").await;
    let stranger = ctx.app.user("stranger").await;
    let blocked = ctx.app.user("blocked").await;
    ctx.befriend(&alice, &friend).await;
    ctx.block(&alice, &blocked).await;
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.published(&alice, &channel, "フレンド限定", Visibility::VISIBILITY_FRIENDS).await;
    let session = create(&ctx, &alice, &video).await;

    assert!(try_join(&ctx, Some(&friend), &session.party.code, "", "").await.is_ok());
    assert_eq!(
        try_join(&ctx, Some(&stranger), &session.party.code, "", "").await.unwrap_err().reason,
        "VIDEO_NOT_FOUND"
    );
    assert_eq!(try_join(&ctx, None, &session.party.code, "ゲスト", "").await.unwrap_err().reason, "VIDEO_NOT_FOUND");
    // The invite page does not show a video the visitor may not watch.
    let preview = PreviewPartyRequest { code: session.party.code.clone(), ..Default::default() };
    let seen: PreviewPartyResponse = ctx.call(PARTIES, "PreviewParty", Some(&stranger), &preview).await.unwrap();
    assert!(seen.party.video.as_option().is_none());
    let seen: PreviewPartyResponse = ctx.call(PARTIES, "PreviewParty", Some(&friend), &preview).await.unwrap();
    assert!(seen.party.video.as_option().is_some());
    // Someone the host blocked finds no party at all.
    let public = some_video(&ctx, &friend, "公開").await;
    let open = create(&ctx, &alice, &public).await;
    assert_eq!(try_join(&ctx, Some(&blocked), &open.party.code, "", "").await.unwrap_err().reason, "PARTY_NOT_FOUND");
    assert_eq!(
        ctx.reason(
            PARTIES,
            "PreviewParty",
            Some(&blocked),
            &PreviewPartyRequest { code: open.party.code.clone(), ..Default::default() }
        )
        .await,
        "PARTY_NOT_FOUND"
    );
}

#[tokio::test]
async fn the_host_and_co_hosts_control_playback() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let channel = ctx.channel(&alice, "ch").await;
    let video = ctx.public_video(&alice, &channel, "一本目").await;
    let next = ctx.public_video(&alice, &channel, "二本目").await;
    let private = ctx
        .published(&carol, &ctx.channel(&carol, "carol").await, "carol private", Visibility::VISIBILITY_PRIVATE)
        .await;
    let session = create(&ctx, &alice, &video).await;
    let party_id = session.party.id.clone();
    let seat = join(&ctx, &bob, &session).await;
    let mut topic = ctx.app.listen_topic(&session.realtime_topic).await;

    let set = |playing: bool, position: f64, rate: f64| SetPartyPlaybackRequest {
        party_id: party_id.clone(),
        playing,
        position_seconds: position,
        rate,
        ..Default::default()
    };
    let before = chrono::Utc::now();
    let response: SetPartyPlaybackResponse =
        ctx.call(PARTIES, "SetPartyPlayback", Some(&alice), &set(true, 5.0, 1.5)).await.unwrap();
    let playback = response.playback.as_option().cloned().unwrap();
    assert_eq!((playback.playing, playback.position_seconds, playback.rate, playback.sequence), (true, 5.0, 1.5, 2));
    assert_eq!(playback.changed_by_member_id, session.self_.id);
    // Server timestamps let clients compensate for latency.
    let stamped = from_ts(playback.updated_at.as_option().unwrap());
    let server_time = from_ts(response.server_time.as_option().unwrap());
    assert!(stamped >= before - chrono::Duration::seconds(1) && stamped <= server_time);
    match next_push(&mut topic).await {
        Push::PartyPlayback(pushed) => {
            assert_eq!((pushed.sequence, pushed.position_seconds, pushed.playing), (2, 5.0, true))
        }
        other => panic!("unexpected push {other:?}"),
    }
    // A rate of 0 keeps the speed.
    let response: SetPartyPlaybackResponse =
        ctx.call(PARTIES, "SetPartyPlayback", Some(&alice), &set(false, 6.0, 0.0)).await.unwrap();
    assert_eq!((response.playback.rate, response.playback.sequence, response.playback.playing), (1.5, 3, false));
    assert!(matches!(next_push(&mut topic).await, Push::PartyPlayback(_)));

    assert_eq!(
        ctx.reason(PARTIES, "SetPartyPlayback", Some(&alice), &set(true, 14.0, 1.0)).await,
        "PLAYBACK_INVALID",
        "the video is 12 s long"
    );
    assert_eq!(ctx.reason(PARTIES, "SetPartyPlayback", Some(&alice), &set(true, -1.0, 1.0)).await, "PLAYBACK_INVALID");
    assert_eq!(ctx.reason(PARTIES, "SetPartyPlayback", Some(&alice), &set(true, 1.0, 5.0)).await, "PLAYBACK_INVALID");
    assert_eq!(ctx.reason(PARTIES, "SetPartyPlayback", Some(&bob), &set(true, 1.0, 1.0)).await, "NOT_PARTY_HOST");
    assert_eq!(ctx.reason(PARTIES, "SetPartyPlayback", Some(&carol), &set(true, 1.0, 1.0)).await, "NOT_A_PARTY_MEMBER");
    let elsewhere = SetPartyPlaybackRequest { party_id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.reason(PARTIES, "SetPartyPlayback", Some(&alice), &elsewhere).await, "PARTY_NOT_FOUND");

    // A co-host may control playback, too.
    let role = |member_id: &str, role: PartyRole| SetPartyMemberRoleRequest {
        party_id: party_id.clone(),
        member_id: member_id.to_owned(),
        role: role.into(),
        ..Default::default()
    };
    let response: SetPartyMemberRoleResponse = ctx
        .call(PARTIES, "SetPartyMemberRole", Some(&alice), &role(&seat.self_.id, PartyRole::PARTY_ROLE_COHOST))
        .await
        .unwrap();
    assert_eq!(
        response.members.iter().map(|m| m.role.as_known().unwrap()).collect::<Vec<_>>(),
        vec![PartyRole::PARTY_ROLE_HOST, PartyRole::PARTY_ROLE_COHOST]
    );
    assert!(
        matches!(next_push(&mut topic).await, Push::PartyMemberUpdated(member) if member.role == PartyRole::PARTY_ROLE_COHOST && member.name == "bob")
    );
    let response: SetPartyPlaybackResponse =
        ctx.call(PARTIES, "SetPartyPlayback", Some(&bob), &set(true, 7.0, 1.0)).await.unwrap();
    assert_eq!(response.playback.changed_by_member_id, seat.self_.id);
    assert!(matches!(next_push(&mut topic).await, Push::PartyPlayback(_)));

    // Switching videos restarts playback, paused at the beginning.
    let switch = |video: &Video| SetPartyVideoRequest {
        party_id: party_id.clone(),
        video_id: card(video).id.clone(),
        ..Default::default()
    };
    let response: SetPartyVideoResponse = ctx.call(PARTIES, "SetPartyVideo", Some(&bob), &switch(&next)).await.unwrap();
    let switched = response.party.as_option().unwrap();
    assert_eq!(switched.video.as_option().unwrap().title, "二本目");
    let playback = switched.playback.as_option().unwrap();
    assert_eq!((playback.playing, playback.position_seconds, playback.rate, playback.sequence), (false, 0.0, 1.0, 5));
    match next_push(&mut topic).await {
        Push::PartyUpdated(updated) => assert_eq!(updated.video.as_option().unwrap().id, card(&next).id),
        other => panic!("unexpected push {other:?}"),
    }
    assert_eq!(ctx.reason(PARTIES, "SetPartyVideo", Some(&alice), &switch(&private)).await, "VIDEO_NOT_FOUND");
    let _: SetPartyMemberRoleResponse = ctx
        .call(PARTIES, "SetPartyMemberRole", Some(&alice), &role(&seat.self_.id, PartyRole::PARTY_ROLE_MEMBER))
        .await
        .unwrap();
    assert_eq!(ctx.reason(PARTIES, "SetPartyVideo", Some(&bob), &switch(&video)).await, "NOT_PARTY_HOST");
    assert_eq!(ctx.reason(PARTIES, "SetPartyVideo", None, &switch(&video)).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn hosting_is_handed_over_and_the_party_ends_with_its_last_member() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let video = some_video(&ctx, &alice, "v").await;
    let session = create(&ctx, &alice, &video).await;
    let party_id = session.party.id.clone();
    let bob_seat = join(&ctx, &bob, &session).await;
    let carol_seat = join(&ctx, &carol, &session).await;
    let guest = join_as_guest(&ctx, &session, "ゲスト").await;
    let role = |member_id: &str, role: PartyRole| SetPartyMemberRoleRequest {
        party_id: party_id.clone(),
        member_id: member_id.to_owned(),
        role: role.into(),
        ..Default::default()
    };

    // What the host cannot do with roles.
    assert_eq!(
        ctx.reason(
            PARTIES,
            "SetPartyMemberRole",
            Some(&bob),
            &role(&carol_seat.self_.id, PartyRole::PARTY_ROLE_COHOST)
        )
        .await,
        "NOT_PARTY_HOST"
    );
    assert_eq!(
        ctx.reason(PARTIES, "SetPartyMemberRole", Some(&alice), &role(&session.self_.id, PartyRole::PARTY_ROLE_MEMBER))
            .await,
        "ROLE_INVALID"
    );
    assert_eq!(
        ctx.reason(
            PARTIES,
            "SetPartyMemberRole",
            Some(&alice),
            &role(&bob_seat.self_.id, PartyRole::PARTY_ROLE_UNSPECIFIED)
        )
        .await,
        "ROLE_INVALID"
    );
    assert_eq!(
        ctx.reason(
            PARTIES,
            "SetPartyMemberRole",
            Some(&alice),
            &role(&Uuid::now_v7().to_string(), PartyRole::PARTY_ROLE_COHOST)
        )
        .await,
        "PARTY_MEMBER_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(PARTIES, "SetPartyMemberRole", Some(&alice), &role(&guest.self_.id, PartyRole::PARTY_ROLE_HOST))
            .await,
        "GUESTS_CANNOT_HOST"
    );

    // Handing the party over: the old host stays as a co-host.
    let response: SetPartyMemberRoleResponse = ctx
        .call(PARTIES, "SetPartyMemberRole", Some(&alice), &role(&carol_seat.self_.id, PartyRole::PARTY_ROLE_HOST))
        .await
        .unwrap();
    let roles: Vec<(String, PartyRole)> =
        response.members.iter().map(|m| (m.name.clone(), m.role.as_known().unwrap())).collect();
    assert_eq!(
        roles[..2],
        [("carol".to_owned(), PartyRole::PARTY_ROLE_HOST), ("alice".to_owned(), PartyRole::PARTY_ROLE_COHOST)]
    );
    assert_eq!(roles.iter().filter(|(_, role)| *role == PartyRole::PARTY_ROLE_HOST).count(), 1);
    assert_eq!(
        ctx.reason(
            PARTIES,
            "EndParty",
            Some(&alice),
            &EndPartyRequest { party_id: party_id.clone(), ..Default::default() }
        )
        .await,
        "NOT_PARTY_HOST"
    );

    // When the host leaves, the longest-present co-host takes over.
    let mut topic = ctx.app.listen_topic(&session.realtime_topic).await;
    leave(&ctx, &carol, &party_id).await.unwrap();
    match next_push(&mut topic).await {
        Push::PartyMemberLeft(left) => {
            assert_eq!(
                (left.member_id.as_str(), left.reason, left.member_count),
                (carol_seat.self_.id.as_str(), PartyLeaveReason::PARTY_LEAVE_REASON_LEFT.into(), 3)
            )
        }
        other => panic!("unexpected push {other:?}"),
    }
    assert!(
        matches!(next_push(&mut topic).await, Push::PartyMemberUpdated(member) if member.name == "alice" && member.role == PartyRole::PARTY_ROLE_HOST)
    );
    // Without a co-host, the longest-present member does.
    leave(&ctx, &alice, &party_id).await.unwrap();
    assert_eq!(
        members(&ctx, &bob, &party_id).await,
        vec![("bob".to_owned(), PartyRole::PARTY_ROLE_HOST), ("ゲスト".to_owned(), PartyRole::PARTY_ROLE_MEMBER)]
    );
    leave(&ctx, &carol, &party_id).await.unwrap(); // not in the party any more: nothing happens

    // Guests cannot keep a party going on their own.
    let mut guest_topic = ctx.app.listen_topic(&guest.realtime_topic).await;
    leave(&ctx, &bob, &party_id).await.unwrap();
    let pushed = pushes(guest_topic.drain(QUIET).await);
    assert!(
        matches!(pushed.last(), Some(Push::PartyEnded(ended)) if ended.reason == PartyEndReason::PARTY_END_REASON_EMPTY),
        "{pushed:?}"
    );
    assert_eq!(try_join(&ctx, Some(&alice), &session.party.code, "", "").await.unwrap_err().reason, "PARTY_NOT_FOUND");
    assert!(!may_listen(&ctx, None, &guest.realtime_topic).await, "an ended party has no audience");
    assert_eq!(leave(&ctx, &bob, &party_id).await.unwrap_err().reason, "PARTY_NOT_FOUND");
}

#[tokio::test]
async fn removed_members_stay_out() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let video = some_video(&ctx, &alice, "v").await;
    let session = create(&ctx, &alice, &video).await;
    let party_id = session.party.id.clone();
    let bob_seat = join(&ctx, &bob, &session).await;
    let carol_seat = join(&ctx, &carol, &session).await;
    let guest = join_as_guest(&ctx, &session, "荒らし").await;
    let remove = |member_id: &str| RemovePartyMemberRequest {
        party_id: party_id.clone(),
        member_id: member_id.to_owned(),
        ..Default::default()
    };
    let role = SetPartyMemberRoleRequest {
        party_id: party_id.clone(),
        member_id: bob_seat.self_.id.clone(),
        role: PartyRole::PARTY_ROLE_COHOST.into(),
        ..Default::default()
    };
    let _: SetPartyMemberRoleResponse = ctx.call(PARTIES, "SetPartyMemberRole", Some(&alice), &role).await.unwrap();

    // Members remove nobody; co-hosts remove members but not the host; nobody removes themselves.
    assert_eq!(
        ctx.reason(PARTIES, "RemovePartyMember", Some(&carol), &remove(&bob_seat.self_.id)).await,
        "NOT_PARTY_HOST"
    );
    assert_eq!(
        ctx.reason(PARTIES, "RemovePartyMember", Some(&bob), &remove(&session.self_.id)).await,
        "NOT_PARTY_HOST"
    );
    assert_eq!(
        ctx.reason(PARTIES, "RemovePartyMember", Some(&bob), &remove(&bob_seat.self_.id)).await,
        "CANNOT_REMOVE_SELF"
    );
    assert_eq!(
        ctx.reason(PARTIES, "RemovePartyMember", Some(&alice), &remove(&Uuid::now_v7().to_string())).await,
        "PARTY_MEMBER_NOT_FOUND"
    );
    assert_eq!(ctx.reason(PARTIES, "RemovePartyMember", None, &remove(&carol_seat.self_.id)).await, "SIGN_IN_REQUIRED");

    // The removed guest hears about it on their own topic, and then nothing more.
    let mut guest_topic = ctx.app.listen_topic(&guest.realtime_topic).await;
    let mut topic = ctx.app.listen_topic(&session.realtime_topic).await;
    let _: RemovePartyMemberResponse =
        ctx.call(PARTIES, "RemovePartyMember", Some(&bob), &remove(&guest.self_.id)).await.unwrap();
    for listener in [&mut guest_topic, &mut topic] {
        match next_push(listener).await {
            Push::PartyMemberLeft(left) => assert_eq!(
                (left.member_id.as_str(), left.reason),
                (guest.self_.id.as_str(), PartyLeaveReason::PARTY_LEAVE_REASON_REMOVED.into())
            ),
            other => panic!("unexpected push {other:?}"),
        }
    }
    say(&ctx, Some(&alice), &party_id, "", "静かになったね").await.unwrap();
    assert!(matches!(next_push(&mut topic).await, Push::PartyMessage(_)));
    assert!(pushes(guest_topic.drain(QUIET).await).is_empty(), "nothing is published to a removed guest's topic");
    assert!(!may_listen(&ctx, None, &guest.realtime_topic).await);
    // Neither the old token nor a fresh guest from the same address gets back in.
    assert_eq!(
        try_join(&ctx, None, &session.party.code, "荒らし", &guest.guest_token).await.unwrap_err().reason,
        "PARTY_BANNED"
    );
    assert_eq!(try_join(&ctx, None, &session.party.code, "別人です", "").await.unwrap_err().reason, "PARTY_BANNED");

    // A removed account cannot rejoin either; the host may remove a co-host.
    let _: RemovePartyMemberResponse =
        ctx.call(PARTIES, "RemovePartyMember", Some(&alice), &remove(&bob_seat.self_.id)).await.unwrap();
    assert_eq!(try_join(&ctx, Some(&bob), &session.party.code, "", "").await.unwrap_err().reason, "PARTY_BANNED");
    assert!(!may_listen(&ctx, Some(&bob), &session.realtime_topic).await);
    assert_eq!(members(&ctx, &alice, &party_id).await.len(), 2);
}

/// Stands in for the stamp domain: one stamp everyone may send.
struct OneStamp(Uuid);

#[async_trait]
impl PartyStamps for OneStamp {
    async fn send_stamp(&self, _user_id: Uuid, stamp_id: Uuid) -> AppResult<StampRef> {
        if stamp_id == self.0 {
            Ok(StampRef {
                id: stamp_id.to_string(),
                name: "いいね".into(),
                image_url: "https://example.com/stamp.webp".into(),
                ..Default::default()
            })
        } else {
            Err(AppError::precondition("STAMP_NOT_USABLE", "you cannot send this stamp"))
        }
    }
}

#[tokio::test]
async fn party_chat_carries_text_and_stamps() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let video = some_video(&ctx, &alice, "v").await;
    let session = create(&ctx, &alice, &video).await;
    let party_id = session.party.id.clone();
    join(&ctx, &bob, &session).await;

    let mut topic = ctx.app.listen_topic(&session.realtime_topic).await;
    let message = say(&ctx, Some(&bob), &party_id, "", "  ここ好き!  ").await.unwrap();
    assert_eq!((message.text.as_str(), message.sender.name.as_str()), ("ここ好き!", "bob"));
    assert!(message.sent_at.as_option().is_some() && message.stamp.as_option().is_none());
    assert!(
        matches!(next_push(&mut topic).await, Push::PartyMessage(pushed) if pushed.id == message.id && pushed.text == "ここ好き!")
    );
    assert!(say(&ctx, Some(&alice), &party_id, "", &"あ".repeat(500)).await.is_ok());

    assert_eq!(say(&ctx, Some(&bob), &party_id, "", "   ").await.unwrap_err().reason, "MESSAGE_EMPTY");
    assert_eq!(say(&ctx, Some(&bob), &party_id, "", &"あ".repeat(501)).await.unwrap_err().reason, "MESSAGE_TOO_LONG");
    assert_eq!(say(&ctx, Some(&carol), &party_id, "", "入れて").await.unwrap_err().reason, "NOT_A_PARTY_MEMBER");
    assert_eq!(say(&ctx, None, &party_id, "", "匿名").await.unwrap_err().reason, "NOT_A_PARTY_MEMBER");
    assert_eq!(
        say(&ctx, Some(&bob), &Uuid::now_v7().to_string(), "", "どこ?").await.unwrap_err().reason,
        "PARTY_NOT_FOUND"
    );

    // Stamps come from the stamp domain, which the server plugs in. Without it they are refused.
    let stamp_id = Uuid::now_v7();
    let stamp = |stamp_id: Uuid, text: &str| SendPartyMessageRequest {
        party_id: party_id.clone(),
        stamp_id: stamp_id.to_string(),
        text: text.to_owned(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(PARTIES, "SendPartyMessage", Some(&bob), &stamp(stamp_id, "")).await, "STAMP_NOT_USABLE");
    ctx.video.set_party_stamps(Arc::new(OneStamp(stamp_id)));
    let response: SendPartyMessageResponse =
        ctx.call(PARTIES, "SendPartyMessage", Some(&bob), &stamp(stamp_id, "")).await.unwrap();
    assert_eq!(
        (response.message.stamp.as_option().unwrap().name.as_str(), response.message.text.as_str()),
        ("いいね", "")
    );
    let response: SendPartyMessageResponse =
        ctx.call(PARTIES, "SendPartyMessage", Some(&bob), &stamp(stamp_id, "スタンプつき")).await.unwrap();
    assert_eq!(response.message.text, "スタンプつき");
    assert_eq!(
        ctx.reason(PARTIES, "SendPartyMessage", Some(&bob), &stamp(Uuid::now_v7(), "")).await,
        "STAMP_NOT_USABLE"
    );
    let malformed =
        SendPartyMessageRequest { party_id: party_id.clone(), stamp_id: "nope".into(), ..Default::default() };
    assert_eq!(ctx.reason(PARTIES, "SendPartyMessage", Some(&bob), &malformed).await, "STAMP_NOT_USABLE");

    // Someone who joins later gets the recent messages, oldest first.
    let late = join(&ctx, &carol, &session).await;
    let texts: Vec<&str> = late.recent_messages.iter().map(|message| message.text.as_str()).collect();
    assert_eq!(texts.len(), 4);
    assert_eq!((texts[0], texts[3]), ("ここ好き!", "スタンプつき"));
    assert_eq!(late.recent_messages[2].stamp.as_option().unwrap().id, stamp_id.to_string());
}

#[tokio::test]
async fn friends_can_be_invited() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let friend = ctx.app.user("friend").await;
    let inside = ctx.app.user("inside").await;
    let stranger = ctx.app.user("stranger").await;
    ctx.befriend(&alice, &friend).await;
    ctx.befriend(&alice, &inside).await;
    let video = some_video(&ctx, &alice, "招待するよ").await;
    let session = create(&ctx, &alice, &video).await;
    join(&ctx, &inside, &session).await;

    let invite = |ids: Vec<String>| InviteToPartyRequest {
        party_id: session.party.id.clone(),
        user_ids: ids,
        ..Default::default()
    };
    let _: InviteToPartyResponse = ctx
        .call(
            PARTIES,
            "InviteToParty",
            Some(&alice),
            &invite(vec![friend.id.to_string(), inside.id.to_string(), friend.id.to_string()]),
        )
        .await
        .unwrap();
    // (Alice's friend requests are in the outbox, too.)
    let invitations: Vec<_> = ctx
        .app
        .events_by("notify.requested", alice.id)
        .await
        .into_iter()
        .filter(|event| event.data["kind"] == "video.party_invite")
        .collect();
    assert_eq!(invitations.len(), 1, "one per friend, and none for those already inside");
    let invitation = &invitations[0].data;
    assert_eq!(
        (invitation["kind"].as_str(), invitation["user_id"].as_str()),
        (Some("video.party_invite"), Some(friend.id.to_string().as_str()))
    );
    assert_eq!(invitation["path"], format!("/party/{}", session.party.code));
    assert!(invitation["body"]["ja"].as_str().unwrap().contains("招待するよ"));

    assert_eq!(
        ctx.reason(PARTIES, "InviteToParty", Some(&alice), &invite(vec![stranger.id.to_string()])).await,
        "NOT_FRIENDS"
    );
    assert_eq!(
        ctx.reason(
            PARTIES,
            "InviteToParty",
            Some(&alice),
            &invite((0..21).map(|_| Uuid::now_v7().to_string()).collect())
        )
        .await,
        "TOO_MANY_INVITES"
    );
    assert_eq!(ctx.reason(PARTIES, "InviteToParty", Some(&alice), &invite(vec!["nope".into()])).await, "INVALID_ID");
    assert_eq!(
        ctx.reason(PARTIES, "InviteToParty", Some(&friend), &invite(vec![alice.id.to_string()])).await,
        "NOT_A_PARTY_MEMBER"
    );
    assert_eq!(ctx.reason(PARTIES, "InviteToParty", None, &invite(vec![])).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn ending_a_party_closes_it_for_everyone() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let video = some_video(&ctx, &alice, "v").await;
    let session = create(&ctx, &alice, &video).await;
    let party_id = session.party.id.clone();
    join(&ctx, &bob, &session).await;
    let guest = join_as_guest(&ctx, &session, "ゲスト").await;

    let end = EndPartyRequest { party_id: party_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(PARTIES, "EndParty", Some(&bob), &end).await, "NOT_PARTY_HOST");
    assert_eq!(ctx.reason(PARTIES, "EndParty", None, &end).await, "SIGN_IN_REQUIRED");
    let mut topic = ctx.app.listen_topic(&session.realtime_topic).await;
    let mut guest_topic = ctx.app.listen_topic(&guest.realtime_topic).await;
    let _: EndPartyResponse = ctx.call(PARTIES, "EndParty", Some(&alice), &end).await.unwrap();
    for listener in [&mut topic, &mut guest_topic] {
        match next_push(listener).await {
            Push::PartyEnded(ended) => assert_eq!(
                (ended.party_id.as_str(), ended.reason),
                (party_id.as_str(), PartyEndReason::PARTY_END_REASON_ENDED_BY_HOST.into())
            ),
            other => panic!("unexpected push {other:?}"),
        }
    }

    // Those who were in it can still look at it; nothing else works.
    let seen: GetPartyResponse = ctx
        .call(PARTIES, "GetParty", Some(&bob), &GetPartyRequest { party_id: party_id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert!(seen.session.party.ended_at.as_option().is_some());
    assert_eq!(ctx.reason(PARTIES, "EndParty", Some(&alice), &end).await, "PARTY_NOT_FOUND");
    assert_eq!(say(&ctx, Some(&bob), &party_id, "", "まだ?").await.unwrap_err().reason, "PARTY_NOT_FOUND");
    assert_eq!(
        ctx.reason(
            PARTIES,
            "HeartbeatParty",
            Some(&bob),
            &HeartbeatPartyRequest { party_id: party_id.clone(), ..Default::default() }
        )
        .await,
        "PARTY_NOT_FOUND"
    );
    assert_eq!(try_join(&ctx, Some(&bob), &session.party.code, "", "").await.unwrap_err().reason, "PARTY_NOT_FOUND");
    assert_eq!(
        ctx.reason(
            PARTIES,
            "PreviewParty",
            None,
            &PreviewPartyRequest { code: session.party.code.clone(), ..Default::default() }
        )
        .await,
        "PARTY_NOT_FOUND"
    );
    assert!(!may_listen(&ctx, Some(&bob), &session.realtime_topic).await);
}

#[tokio::test]
async fn a_party_ends_when_its_video_goes_away() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let deleted = ctx.public_video(&alice, &channel, "消える").await;
    let unpublished = ctx.public_video(&alice, &channel, "下書きに戻る").await;
    let hidden = ctx.public_video(&alice, &channel, "非公開になる").await;

    for (video, how) in [(&deleted, "delete"), (&unpublished, "unpublish"), (&hidden, "hide")] {
        let session = create(&ctx, &bob, video).await;
        let mut topic = ctx.app.listen_topic(&session.realtime_topic).await;
        let video_id = card(video).id.clone();
        match how {
            "delete" => {
                let _: DeleteVideoResponse = ctx
                    .call(VIDEOS, "DeleteVideo", Some(&alice), &DeleteVideoRequest { video_id, ..Default::default() })
                    .await
                    .unwrap();
            }
            "unpublish" => {
                let _: UnpublishVideoResponse = ctx
                    .call(
                        VIDEOS,
                        "UnpublishVideo",
                        Some(&alice),
                        &UnpublishVideoRequest { video_id, ..Default::default() },
                    )
                    .await
                    .unwrap();
            }
            _ => {
                let request = UpdateVideoRequest {
                    video_id,
                    visibility: Some(Visibility::VISIBILITY_PRIVATE.into()),
                    ..Default::default()
                };
                let _: UpdateVideoResponse = ctx.call(VIDEOS, "UpdateVideo", Some(&alice), &request).await.unwrap();
            }
        }
        match next_push(&mut topic).await {
            Push::PartyEnded(ended) => {
                assert_eq!(ended.reason, PartyEndReason::PARTY_END_REASON_VIDEO_UNAVAILABLE, "{how}")
            }
            other => panic!("{how}: unexpected push {other:?}"),
        }
        assert_eq!(
            try_join(&ctx, Some(&alice), &session.party.code, "", "").await.unwrap_err().reason,
            "PARTY_NOT_FOUND",
            "{how}"
        );
    }
}

#[tokio::test]
async fn the_worker_drops_silent_members_and_ends_old_parties() {
    let ctx = start().await;
    let db = &ctx.app.core.db;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let video = some_video(&ctx, &alice, "v").await;
    let session = create(&ctx, &alice, &video).await;
    let party_id = session.party.id.clone();
    let bob_seat = join(&ctx, &bob, &session).await;
    join(&ctx, &carol, &session).await;
    let silent = |seat_id: &str| {
        sqlx::query("UPDATE watch_party_members SET last_seen_at = now() - interval '2 minutes' WHERE id = $1")
            .bind(id(seat_id))
            .execute(db)
    };

    // Bob's client went away; the others keep sending heartbeats.
    silent(&bob_seat.self_.id).await.unwrap();
    let mut topic = ctx.app.listen_topic(&session.realtime_topic).await;
    let swept = party::sweep(&ctx.app.core, 60, 12).await.unwrap();
    assert!(swept.timed_out >= 1);
    match next_push(&mut topic).await {
        Push::PartyMemberLeft(left) => assert_eq!(
            (left.member_id.as_str(), left.reason, left.member_count),
            (bob_seat.self_.id.as_str(), PartyLeaveReason::PARTY_LEAVE_REASON_TIMED_OUT.into(), 2)
        ),
        other => panic!("unexpected push {other:?}"),
    }
    assert_eq!(members(&ctx, &alice, &party_id).await.len(), 2);
    // Timing out is not a ban: bob can come back.
    assert!(try_join(&ctx, Some(&bob), &session.party.code, "", "").await.is_ok());

    // A silent host is replaced like one who left.
    silent(&session.self_.id).await.unwrap();
    party::sweep(&ctx.app.core, 60, 12).await.unwrap();
    assert_eq!(members(&ctx, &carol, &party_id).await[0], ("carol".to_owned(), PartyRole::PARTY_ROLE_HOST));

    // Parties do not run forever, and ended ones are deleted a day later.
    sqlx::query("UPDATE watch_parties SET created_at = now() - interval '13 hours' WHERE id = $1")
        .bind(id(&party_id))
        .execute(db)
        .await
        .unwrap();
    let mut topic = ctx.app.listen_topic(&session.realtime_topic).await;
    assert!(party::sweep(&ctx.app.core, 60, 12).await.unwrap().expired >= 1);
    assert!(
        matches!(next_push(&mut topic).await, Push::PartyEnded(ended) if ended.reason == PartyEndReason::PARTY_END_REASON_EXPIRED)
    );
    sqlx::query("UPDATE watch_parties SET ended_at = now() - interval '25 hours' WHERE id = $1")
        .bind(id(&party_id))
        .execute(db)
        .await
        .unwrap();
    assert!(party::sweep(&ctx.app.core, 60, 12).await.unwrap().purged >= 1);
    let get = GetPartyRequest { party_id: party_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(PARTIES, "GetParty", Some(&carol), &get).await, "PARTY_NOT_FOUND");
}

#[tokio::test]
async fn a_removed_account_does_not_come_back_as_a_guest() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let video = some_video(&ctx, &alice, "movie night").await;
    let session = create(&ctx, &alice, &video).await;
    let bob_seat = join(&ctx, &bob, &session).await;

    let remove = RemovePartyMemberRequest {
        party_id: session.party.id.clone(),
        member_id: bob_seat.self_.id.clone(),
        ..Default::default()
    };
    let _: RemovePartyMemberResponse = ctx.call(PARTIES, "RemovePartyMember", Some(&alice), &remove).await.unwrap();
    assert_eq!(try_join(&ctx, Some(&bob), &session.party.code, "", "").await.unwrap_err().reason, "PARTY_BANNED");
    // Signed out, from the same address.
    assert_eq!(
        try_join(&ctx, None, &session.party.code, "ぼぶじゃないよ", "").await.unwrap_err().reason,
        "PARTY_BANNED"
    );
}
