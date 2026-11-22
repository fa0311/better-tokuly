//! A room's life: created on the spot or scheduled, started, updated,
//! cancelled, ended, deleted — and who gets to see and hear of it.
mod common;

use common::*;
use tokuly_core::events::Event;
use tokuly_proto::spaces::*;
use tokuly_spaces::SpacesConfig;
use tokuly_testkit::TestUser;
use uuid::Uuid;

fn id_request(space_id: &str) -> GetSpaceRequest {
    GetSpaceRequest { space_id: space_id.to_owned(), ..Default::default() }
}

/// The `spaces.room.started` event of one room.
async fn started_event(ctx: &Ctx, host: &TestUser, space_id: &str) -> Event {
    ctx.app
        .events_by("spaces.room.started", host.id)
        .await
        .into_iter()
        .find(|event| event.data["space_id"] == space_id)
        .expect("the room announced its start")
}

#[tokio::test]
async fn the_config_says_what_the_server_offers() {
    let ctx = start().await;
    let response: GetSpacesConfigResponse =
        ctx.anonymous("Space", "GetSpacesConfig", &GetSpacesConfigRequest::default()).await.expect("no sign-in needed");
    let config = response.config.as_option().unwrap();
    assert!(config.audio_enabled);
    assert_eq!((config.max_speakers, config.max_cohosts, config.max_listeners), (10, 2, 3000));
    assert_eq!((config.max_title_length, config.max_tags, config.max_schedule_days), (80, 5, 30));
    assert_eq!((config.host_grace_seconds, config.max_hangouts, config.max_hangout_members), (180, 10, 50));
}

#[tokio::test]
async fn an_instant_room_goes_live_with_the_host_inside() {
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let request = CreateSpaceRequest {
        title: "  放課後トーク  ".into(),
        tags: vec!["#雑談".into(), "Music".into(), "music".into()],
        language: "ja-JP".into(),
        audience: SpaceAudience::SPACE_AUDIENCE_PUBLIC.into(),
        ..Default::default()
    };
    let response: CreateSpaceResponse = ctx.call("Space", "CreateSpace", &host, &request).await.unwrap();
    let space = response.space.as_option().unwrap();
    assert_eq!(space.title, "放課後トーク");
    assert_eq!(space.tags, ["雑談", "music"]);
    assert_eq!(space.language, "ja");
    assert_eq!(space.state, SpaceState::SPACE_STATE_LIVE);
    assert_eq!(space.audience, SpaceAudience::SPACE_AUDIENCE_PUBLIC);
    assert_eq!(space.host.id, host.id.to_string());
    assert!(space.started_at.is_set() && space.scheduled_at.is_unset() && space.ended_at.is_unset());
    assert_eq!((space.listener_count, space.speaker_count, space.peak_listeners, space.total_listeners), (1, 1, 1, 1));
    assert_eq!(space.max_speakers, 10);
    assert_eq!(space.stage.len(), 1);
    assert_eq!(space.stage[0].user.id, host.id.to_string());
    assert_eq!(space.stage[0].role, SpaceRole::SPACE_ROLE_HOST);
    let viewer = space.viewer.as_option().unwrap();
    assert!(viewer.inside && !viewer.banned && !viewer.hand_raised);
    assert_eq!(viewer.role, SpaceRole::SPACE_ROLE_HOST);

    // What a LiveKit client connects with: the host may talk.
    let credentials = response.credentials.as_option().unwrap();
    assert_eq!(credentials.url, ctx.livekit().public_url().unwrap());
    assert_eq!(credentials.room, format!("tokuly-space-{}", Uuid::parse_str(&space.id).unwrap().simple()));
    assert!(credentials.can_publish);
    assert!(credentials.expires_at.seconds > chrono::Utc::now().timestamp());
    let claims = ctx.claims(credentials);
    assert_eq!(claims.sub, host.id.to_string(), "the participant identity is the user");
    assert_eq!(claims.name, "taro");
    assert_eq!(claims.metadata, r#"{"role":"host"}"#);
    assert_eq!(claims.video.room, credentials.room);
    assert!(claims.video.room_join && !claims.video.room_admin && !claims.video.room_create);
    assert_eq!((claims.video.can_publish, claims.video.can_subscribe), (Some(true), Some(true)));
    assert_eq!(claims.video.can_publish_sources, ["microphone"]);
    assert_eq!(
        ctx.livekit().list_rooms(std::slice::from_ref(&credentials.room)).await.unwrap().len(),
        1,
        "the room is prepared"
    );

    // The other domains hear of it: an event, an activity entry, a search document.
    let started = started_event(&ctx, &host, &space.id).await;
    assert_eq!(started.data["host_id"], host.id.to_string());
    assert_eq!((started.data["audience"].as_i64(), started.data["scheduled"].as_bool()), (Some(1), Some(false)));
    assert_eq!(started.data["title"], "放課後トーク");
    let activity = ctx.activities(&host).await.pop().unwrap();
    assert_eq!(activity.kind, "spaces.started");
    assert_eq!(activity.path, format!("/spaces/{}", space.id));
    let indexed = ctx.search_changes(&space.id).await.pop().unwrap();
    let doc = indexed.doc.expect("a public room is searchable");
    assert_eq!((doc.kind.as_str(), doc.title.as_str(), doc.public), ("space", "放課後トーク", true));
    assert_eq!(doc.tags, ["雑談", "music"]);

    // A public room starting tells nobody by default.
    let friend = ctx.app.user("hanako").await;
    ctx.befriend(&host, &friend).await;
    ctx.spaces.handle_event(&started).await.unwrap();
    assert!(ctx.notifications(&friend).await.is_empty());
}

#[tokio::test]
async fn rooms_are_validated() {
    let ctx = start_with(SpacesConfig { max_scheduled: 2, ..Default::default() }).await;
    let host = ctx.app.user("taro").await;
    let stranger = ctx.app.user("jiro").await;
    let create = |change: fn(&mut CreateSpaceRequest)| {
        let mut request = CreateSpaceRequest { title: "ルーム".into(), ..Default::default() };
        change(&mut request);
        request
    };
    let later = |seconds: i64| tokuly_core::time::ts(chrono::Utc::now() + chrono::Duration::seconds(seconds));

    assert_eq!(ctx.reason("Space", "CreateSpace", &host, &create(|r| r.title = "   ".into())).await, "INVALID_TITLE");
    assert_eq!(
        ctx.reason("Space", "CreateSpace", &host, &create(|r| r.title = "あ".repeat(81))).await,
        "INVALID_TITLE"
    );
    assert_eq!(
        ctx.reason("Space", "CreateSpace", &host, &create(|r| r.tags = vec!["two words".into()])).await,
        "INVALID_TAGS"
    );
    assert_eq!(
        ctx.reason("Space", "CreateSpace", &host, &create(|r| r.tags = (0..6).map(|n| format!("t{n}")).collect()))
            .await,
        "INVALID_TAGS"
    );
    assert_eq!(
        ctx.reason("Space", "CreateSpace", &host, &create(|r| r.language = "nihongo".into())).await,
        "INVALID_LANGUAGE"
    );
    let mut soon = create(|_| {});
    soon.scheduled_at = later(10);
    assert_eq!(ctx.reason("Space", "CreateSpace", &host, &soon).await, "INVALID_SCHEDULE");
    let mut far = create(|_| {});
    far.scheduled_at = later(40 * 86400);
    assert_eq!(ctx.reason("Space", "CreateSpace", &host, &far).await, "INVALID_SCHEDULE");
    // Only friends can be invited.
    let mut inviting = create(|_| {});
    inviting.invitee_ids = vec![stranger.id.to_string()];
    assert_eq!(ctx.reason("Space", "CreateSpace", &host, &inviting).await, "NOT_FRIENDS");
    inviting.invitee_ids = vec!["not-an-id".into()];
    assert_eq!(ctx.reason("Space", "CreateSpace", &host, &inviting).await, "INVALID_ID");
    assert_eq!(ctx.anonymous_reason("Space", "CreateSpace", &create(|_| {})).await, "SIGN_IN_REQUIRED");

    // The audience defaults to friends: the cautious choice.
    let (space, _) = ctx.open_room(&host, "ルーム", SpaceAudience::SPACE_AUDIENCE_UNSPECIFIED).await;
    assert_eq!(space.audience, SpaceAudience::SPACE_AUDIENCE_FRIENDS);
    // One live room per host.
    assert_eq!(ctx.reason("Space", "CreateSpace", &host, &create(|_| {})).await, "ALREADY_HOSTING");

    // Upcoming rooms are limited per host.
    for _ in 0..2 {
        ctx.schedule(&host, "あとで", SpaceAudience::SPACE_AUDIENCE_FRIENDS).await;
    }
    let mut third = create(|_| {});
    third.scheduled_at = later(3600);
    assert_eq!(ctx.reason("Space", "CreateSpace", &host, &third).await, "TOO_MANY_SPACES");
}

#[tokio::test]
async fn a_scheduled_room_is_announced_and_started() {
    let ctx = start().await;
    let (host, friend) = ctx.friends("taro", "hanako").await;
    let other_friend = ctx.app.user("saburo").await;
    ctx.befriend(&host, &other_friend).await;
    let stranger = ctx.app.user("jiro").await;

    let space = ctx.schedule(&host, "テスト前夜の作戦会議", SpaceAudience::SPACE_AUDIENCE_FRIENDS).await;
    assert_eq!(space.state, SpaceState::SPACE_STATE_SCHEDULED);
    assert!(space.scheduled_at.is_set() && space.started_at.is_unset());
    assert_eq!((space.listener_count, space.stage.len()), (0, 0));
    assert_eq!(space.viewer.as_option().unwrap().role, SpaceRole::SPACE_ROLE_HOST);
    assert!(ctx.search_changes(&space.id).await.is_empty(), "a friends room is not searchable");

    // Friends see it coming; strangers do not know it exists.
    assert_eq!(ctx.get(&friend, &space.id).await.title, "テスト前夜の作戦会議");
    assert_eq!(ctx.reason("Space", "GetSpace", &stranger, &id_request(&space.id)).await, "SPACE_NOT_FOUND");
    assert_eq!(ctx.anonymous_reason("Space", "GetSpace", &id_request(&space.id)).await, "SPACE_NOT_FOUND");
    let upcoming = |user: &TestUser| {
        let user = user.clone();
        let ctx = &ctx;
        let space_id = space.id.clone();
        async move {
            let listed: ListUpcomingSpacesResponse =
                ctx.call("Space", "ListUpcomingSpaces", &user, &ListUpcomingSpacesRequest::default()).await.unwrap();
            listed.spaces.iter().any(|listed| listed.id == space_id)
        }
    };
    assert!(upcoming(&friend).await && upcoming(&host).await);
    assert!(!upcoming(&stranger).await);

    // Nobody can enter before it starts.
    assert_eq!(ctx.reason("Space", "JoinSpace", &friend, &id_request(&space.id)).await, "SPACE_NOT_LIVE");

    // "Remind me."
    let remind = |remind: bool| SetReminderRequest { space_id: space.id.clone(), remind, ..Default::default() };
    let reminded: SetReminderResponse = ctx.call("Space", "SetReminder", &friend, &remind(true)).await.unwrap();
    let reminded = reminded.space.as_option().unwrap();
    assert_eq!(reminded.reminder_count, 1);
    assert!(reminded.viewer.as_option().unwrap().reminder_set);
    let _: SetReminderResponse = ctx.call("Space", "SetReminder", &friend, &remind(true)).await.unwrap();
    assert_eq!(ctx.get(&host, &space.id).await.reminder_count, 1, "asking twice counts once");
    assert_eq!(ctx.reason("Space", "SetReminder", &stranger, &remind(true)).await, "SPACE_NOT_FOUND");
    let mine: ListUpcomingSpacesResponse = ctx
        .call(
            "Space",
            "ListUpcomingSpaces",
            &friend,
            &ListUpcomingSpacesRequest { reminders_only: true, ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(mine.spaces.iter().map(|space| space.id.as_str()).collect::<Vec<_>>(), [space.id.as_str()]);

    // Only the host starts it.
    let start_request = StartSpaceRequest { space_id: space.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Space", "StartSpace", &friend, &start_request).await, "NOT_HOST");
    assert_eq!(ctx.reason("Space", "StartSpace", &stranger, &start_request).await, "SPACE_NOT_FOUND");
    let mut room_events = ctx.listen_room(&space.id).await;
    let started: StartSpaceResponse = ctx.call("Space", "StartSpace", &host, &start_request).await.unwrap();
    let live = started.space.as_option().unwrap();
    assert_eq!(live.state, SpaceState::SPACE_STATE_LIVE);
    assert_eq!((live.listener_count, live.stage.len()), (1, 1));
    assert!(live.started_at.is_set());
    assert_eq!(ctx.claims(started.credentials.as_option().unwrap()).metadata, r#"{"role":"host"}"#);
    assert_eq!(ctx.reason("Space", "StartSpace", &host, &start_request).await, "SPACE_NOT_SCHEDULED");
    assert_eq!(ctx.reason("Space", "SetReminder", &other_friend, &remind(true)).await, "SPACE_NOT_SCHEDULED");
    // Whoever is looking at the room sees it go live.
    let pushed = ctx.events(&mut room_events).await;
    assert!(pushed.iter().any(|event| matches!(
        event,
        spaces_event::Event::SpaceUpdated(updated) if updated.space.state == SpaceState::SPACE_STATE_LIVE
    )));

    // The news goes out: a reminder to who asked for one, "started" to the other friends.
    let event = started_event(&ctx, &host, &space.id).await;
    assert_eq!(event.data["scheduled"], true);
    ctx.spaces.handle_event(&event).await.unwrap();
    let to_friend = ctx.notifications(&friend).await;
    assert_eq!(to_friend.len(), 1);
    assert_eq!(to_friend[0].kind, "spaces.reminder");
    assert_eq!(to_friend[0].actor_id, Some(host.id));
    assert_eq!(to_friend[0].path, format!("/spaces/{}", space.id));
    let to_other = ctx.notifications(&other_friend).await;
    assert_eq!(to_other.len(), 1);
    assert_eq!(to_other[0].kind, "spaces.started");
    assert_eq!(to_other[0].title.ja, "taroがルームを始めたよ!");
    assert!(ctx.notifications(&stranger).await.is_empty());
    assert!(ctx.notifications(&host).await.is_empty());
    // The event arriving twice tells nobody twice.
    ctx.spaces.handle_event(&event).await.unwrap();
    assert_eq!(ctx.notifications(&friend).await.len(), 1);
    assert_eq!(ctx.notifications(&other_friend).await.len(), 1);
}

#[tokio::test]
async fn a_public_room_reminds_only_who_asked() {
    let ctx = start().await;
    let (host, friend) = ctx.friends("taro", "hanako").await;
    let fan = ctx.app.user("fan").await;
    let space = ctx.schedule(&host, "公開ラジオ", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    // Upcoming public rooms can be found, by anyone.
    assert!(ctx.search_changes(&space.id).await.pop().unwrap().doc.is_some());
    let listed: ListUpcomingSpacesResponse =
        ctx.anonymous("Space", "ListUpcomingSpaces", &ListUpcomingSpacesRequest::default()).await.unwrap();
    assert!(listed.spaces.iter().any(|listed| listed.id == space.id));
    assert_eq!(
        ctx.anonymous_reason(
            "Space",
            "ListUpcomingSpaces",
            &ListUpcomingSpacesRequest { reminders_only: true, ..Default::default() }
        )
        .await,
        "SIGN_IN_REQUIRED"
    );

    let remind = SetReminderRequest { space_id: space.id.clone(), remind: true, ..Default::default() };
    let _: SetReminderResponse = ctx.call("Space", "SetReminder", &fan, &remind).await.unwrap();
    let _: StartSpaceResponse = ctx
        .call("Space", "StartSpace", &host, &StartSpaceRequest { space_id: space.id.clone(), ..Default::default() })
        .await
        .unwrap();
    ctx.spaces.handle_event(&started_event(&ctx, &host, &space.id).await).await.unwrap();
    assert_eq!(ctx.notifications(&fan).await.iter().map(|n| n.kind.as_str()).collect::<Vec<_>>(), ["spaces.reminder"]);
    assert!(ctx.notifications(&friend).await.is_empty(), "friends are not told about public rooms unless they asked");
}

#[tokio::test]
async fn a_room_is_updated_by_its_host() {
    let ctx = start().await;
    let (host, friend) = ctx.friends("taro", "hanako").await;
    let space = ctx.schedule(&host, "まだ決めてない", SpaceAudience::SPACE_AUDIENCE_FRIENDS).await;
    let update = |change: fn(&mut UpdateSpaceRequest)| {
        let mut request = UpdateSpaceRequest { space_id: space.id.clone(), ..Default::default() };
        change(&mut request);
        request
    };

    let mut room_events = ctx.listen_room(&space.id).await;
    let mut request = update(|r| {
        r.title = Some("文化祭の打ち合わせ".into());
        r.tags = vec!["文化祭".into()];
        r.set_tags = true;
        r.language = Some("ja".into());
        r.audience = Some(SpaceAudience::SPACE_AUDIENCE_PUBLIC.into());
    });
    let new_time = chrono::Utc::now() + chrono::Duration::hours(5);
    request.scheduled_at = tokuly_core::time::ts(new_time);
    let updated: UpdateSpaceResponse = ctx.call("Space", "UpdateSpace", &host, &request).await.unwrap();
    let updated = updated.space.as_option().unwrap();
    assert_eq!((updated.title.as_str(), updated.language.as_str()), ("文化祭の打ち合わせ", "ja"));
    assert_eq!(updated.tags, ["文化祭"]);
    assert_eq!(updated.audience, SpaceAudience::SPACE_AUDIENCE_PUBLIC);
    assert_eq!(updated.scheduled_at.seconds, new_time.timestamp());
    assert!(
        ctx.search_changes(&space.id).await.pop().unwrap().doc.is_some(),
        "it became public, so it became searchable"
    );
    assert!(matches!(ctx.events(&mut room_events).await.as_slice(), [spaces_event::Event::SpaceUpdated(_)]));

    // Leaving a field out leaves it alone.
    let renamed: UpdateSpaceResponse =
        ctx.call("Space", "UpdateSpace", &host, &update(|r| r.title = Some("打ち合わせ".into()))).await.unwrap();
    assert_eq!(renamed.space.tags, ["文化祭"]);
    // Back to friends only: gone from search.
    let _: UpdateSpaceResponse = ctx
        .call(
            "Space",
            "UpdateSpace",
            &host,
            &update(|r| r.audience = Some(SpaceAudience::SPACE_AUDIENCE_FRIENDS.into())),
        )
        .await
        .unwrap();
    assert!(ctx.search_changes(&space.id).await.pop().unwrap().doc.is_none());

    assert_eq!(
        ctx.reason("Space", "UpdateSpace", &friend, &update(|r| r.title = Some("のっとり".into()))).await,
        "NOT_HOST"
    );
    assert_eq!(
        ctx.reason("Space", "UpdateSpace", &host, &update(|r| r.title = Some(String::new()))).await,
        "INVALID_TITLE"
    );

    // Once it runs, the audience and the schedule are settled; the title is not.
    let _: StartSpaceResponse = ctx
        .call("Space", "StartSpace", &host, &StartSpaceRequest { space_id: space.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(
        ctx.reason(
            "Space",
            "UpdateSpace",
            &host,
            &update(|r| r.audience = Some(SpaceAudience::SPACE_AUDIENCE_PUBLIC.into()))
        )
        .await,
        "SPACE_NOT_SCHEDULED"
    );
    let live: UpdateSpaceResponse =
        ctx.call("Space", "UpdateSpace", &host, &update(|r| r.title = Some("本番".into()))).await.unwrap();
    assert_eq!(live.space.title, "本番");
    let _: EndSpaceResponse = ctx
        .call("Space", "EndSpace", &host, &EndSpaceRequest { space_id: space.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(
        ctx.reason("Space", "UpdateSpace", &host, &update(|r| r.title = Some("あとから".into()))).await,
        "SPACE_ENDED"
    );
}

#[tokio::test]
async fn a_room_is_cancelled_ended_and_deleted() {
    let ctx = start().await;
    let (host, friend) = ctx.friends("taro", "hanako").await;
    let cancel = |space_id: &str| CancelSpaceRequest { space_id: space_id.to_owned(), ..Default::default() };
    let end = |space_id: &str| EndSpaceRequest { space_id: space_id.to_owned(), ..Default::default() };
    let delete = |space_id: &str| DeleteSpaceRequest { space_id: space_id.to_owned(), ..Default::default() };

    // An upcoming room is called off.
    let upcoming = ctx.schedule(&host, "やっぱやめた", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    assert_eq!(ctx.reason("Space", "EndSpace", &host, &end(&upcoming.id)).await, "SPACE_NOT_LIVE");
    assert_eq!(ctx.reason("Space", "CancelSpace", &friend, &cancel(&upcoming.id)).await, "NOT_HOST");
    assert_eq!(ctx.reason("Space", "DeleteSpace", &host, &delete(&upcoming.id)).await, "SPACE_NOT_ENDED");
    let cancelled: CancelSpaceResponse = ctx.call("Space", "CancelSpace", &host, &cancel(&upcoming.id)).await.unwrap();
    assert_eq!(cancelled.space.state, SpaceState::SPACE_STATE_CANCELLED);
    assert_eq!(cancelled.space.end_reason, SpaceEndReason::SPACE_END_REASON_HOST_ENDED);
    assert!(ctx.search_changes(&upcoming.id).await.pop().unwrap().doc.is_none(), "gone from search");
    assert_eq!(ctx.reason("Space", "CancelSpace", &host, &cancel(&upcoming.id)).await, "SPACE_NOT_SCHEDULED");

    // A live room is ended for everyone.
    let (live, credentials) = ctx.open_room(&host, "おしゃべり", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    ctx.join(&friend, &live.id).await;
    assert_eq!(ctx.reason("Space", "CancelSpace", &host, &cancel(&live.id)).await, "SPACE_NOT_SCHEDULED");
    assert_eq!(ctx.reason("Space", "EndSpace", &friend, &end(&live.id)).await, "NOT_HOST");
    assert_eq!(ctx.reason("Space", "DeleteSpace", &host, &delete(&live.id)).await, "SPACE_NOT_ENDED");
    let mut room_events = ctx.listen_room(&live.id).await;
    let mut friend_events = ctx.listen(&friend).await;
    let ended: EndSpaceResponse = ctx.call("Space", "EndSpace", &host, &end(&live.id)).await.unwrap();
    let ended = ended.space.as_option().unwrap();
    assert_eq!(ended.state, SpaceState::SPACE_STATE_ENDED);
    assert_eq!(ended.end_reason, SpaceEndReason::SPACE_END_REASON_HOST_ENDED);
    assert!(ended.ended_at.is_set());
    assert_eq!((ended.listener_count, ended.peak_listeners, ended.total_listeners), (0, 2, 2));
    assert!(ended.stage.is_empty() && !ended.viewer.inside);
    // Ending twice changes nothing.
    let again: EndSpaceResponse = ctx.call("Space", "EndSpace", &host, &end(&live.id)).await.unwrap();
    assert_eq!(again.space.ended_at, ended.ended_at);

    // The room hears it, the listener is told personally, the audio is cut.
    assert!(ctx.events(&mut room_events).await.iter().any(|event| matches!(
        event,
        spaces_event::Event::SpaceUpdated(updated) if updated.space.state == SpaceState::SPACE_STATE_ENDED
    )));
    assert!(ctx.events(&mut friend_events).await.iter().any(|event| matches!(
        event,
        spaces_event::Event::ParticipantLeft(left) if left.reason == SpaceLeaveReason::SPACE_LEAVE_REASON_ENDED
    )));
    assert!(
        ctx.livekit().list_rooms(std::slice::from_ref(&credentials.room)).await.unwrap().is_empty(),
        "the LiveKit room is closed"
    );
    assert_eq!(ctx.reason("Space", "JoinSpace", &friend, &id_request(&live.id)).await, "SPACE_NOT_LIVE");

    // The other domains hear how it went.
    let event = ctx.app.events_by("spaces.room.ended", host.id).await.pop().unwrap();
    assert_eq!(event.data["space_id"], live.id);
    assert_eq!((event.data["peak_listeners"].as_i64(), event.data["total_listeners"].as_i64()), (Some(2), Some(2)));
    assert_eq!(event.data["reason"].as_i64(), Some(1));
    assert!(event.data["minutes"].as_i64().unwrap() <= 1, "a started minute counts as one");
    assert!(ctx.search_changes(&live.id).await.pop().unwrap().doc.is_none());

    // Past rooms stay in the host's history until deleted.
    let mine: ListMySpacesResponse =
        ctx.call("Space", "ListMySpaces", &host, &ListMySpacesRequest::default()).await.unwrap();
    assert_eq!(
        mine.spaces.iter().map(|space| space.id.as_str()).collect::<Vec<_>>(),
        [live.id.as_str(), upcoming.id.as_str()]
    );
    assert_eq!(ctx.reason("Space", "DeleteSpace", &friend, &delete(&live.id)).await, "NOT_HOST");
    let _: DeleteSpaceResponse = ctx.call("Space", "DeleteSpace", &host, &delete(&live.id)).await.unwrap();
    let _: DeleteSpaceResponse = ctx.call("Space", "DeleteSpace", &host, &delete(&upcoming.id)).await.unwrap();
    assert_eq!(ctx.reason("Space", "GetSpace", &host, &id_request(&live.id)).await, "SPACE_NOT_FOUND");
    let mine: ListMySpacesResponse =
        ctx.call("Space", "ListMySpaces", &host, &ListMySpacesRequest::default()).await.unwrap();
    assert!(mine.spaces.is_empty());
}

#[tokio::test]
async fn the_audience_decides_who_gets_in() {
    let ctx = start().await;
    let (host, friend) = ctx.friends("taro", "hanako").await;
    let friend_of_friend = ctx.app.user("saburo").await;
    ctx.befriend(&friend, &friend_of_friend).await;
    let stranger = ctx.app.user("jiro").await;

    // A friends room: friends of the host, and whoever a moderator invites.
    let (friends_room, _) = ctx.open_room(&host, "内輪", SpaceAudience::SPACE_AUDIENCE_FRIENDS).await;
    assert_eq!(ctx.reason("Space", "GetSpace", &stranger, &id_request(&friends_room.id)).await, "SPACE_NOT_FOUND");
    assert_eq!(ctx.reason("Space", "JoinSpace", &stranger, &id_request(&friends_room.id)).await, "SPACE_NOT_FOUND");
    assert_eq!(
        ctx.reason("Space", "JoinSpace", &friend_of_friend, &id_request(&friends_room.id)).await,
        "SPACE_NOT_FOUND"
    );
    assert_eq!(ctx.anonymous_reason("Space", "GetSpace", &id_request(&friends_room.id)).await, "SPACE_NOT_FOUND");
    ctx.join(&friend, &friends_room.id).await;
    // A listener cannot open the room to their own friends; a co-host can.
    let invite = InviteToSpaceRequest {
        space_id: friends_room.id.clone(),
        user_ids: vec![friend_of_friend.id.to_string()],
        ..Default::default()
    };
    assert_eq!(ctx.reason("Space", "InviteToSpace", &friend, &invite).await, "NOT_MODERATOR");
    ctx.make_cohost(&host, &friend, &friends_room.id).await;
    let invited: InviteToSpaceResponse = ctx.call("Space", "InviteToSpace", &friend, &invite).await.unwrap();
    assert_eq!(invited.invited_count, 1);
    assert!(ctx.get(&friend_of_friend, &friends_room.id).await.viewer.invited);
    ctx.join(&friend_of_friend, &friends_room.id).await;
    let _: EndSpaceResponse = ctx
        .call("Space", "EndSpace", &host, &EndSpaceRequest { space_id: friends_room.id.clone(), ..Default::default() })
        .await
        .unwrap();

    // An invited-only room: not even friends, unless invited.
    let request = CreateSpaceRequest {
        title: "ひみつ会議".into(),
        audience: SpaceAudience::SPACE_AUDIENCE_INVITED.into(),
        invitee_ids: vec![friend.id.to_string()],
        ..Default::default()
    };
    let other_friend = ctx.app.user("shiro").await;
    ctx.befriend(&host, &other_friend).await;
    let mut invitee_events = ctx.listen(&friend).await;
    let created: CreateSpaceResponse = ctx.call("Space", "CreateSpace", &host, &request).await.unwrap();
    let secret = created.space.as_option().unwrap();
    assert_eq!(ctx.reason("Space", "JoinSpace", &other_friend, &id_request(&secret.id)).await, "SPACE_NOT_FOUND");
    ctx.join(&friend, &secret.id).await;
    // The invitation arrived as a notification and over realtime.
    let notification = ctx.notifications(&friend).await.pop().unwrap();
    assert_eq!((notification.kind.as_str(), notification.actor_id), ("spaces.invited", Some(host.id)));
    assert_eq!(notification.data["space_id"], secret.id);
    assert!(ctx.events(&mut invitee_events).await.iter().any(|event| matches!(
        event,
        spaces_event::Event::SpaceInvitation(invitation) if invitation.space.id == secret.id && invitation.invited_by.id == host.id.to_string()
    )));
}

#[tokio::test]
async fn a_block_closes_the_door_both_ways() {
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let listener = ctx.app.user("jiro").await;
    let room = ctx.public_room(&host, "だれでもどうぞ").await;
    let credentials = ctx.join(&listener, &room.id).await.credentials.as_option().unwrap().clone();
    let _connection = ctx.connect(&credentials).await;

    // The host blocks someone who is inside: out they go, at once.
    let mut listener_events = ctx.listen(&listener).await;
    ctx.block(&host, &listener).await;
    let blocked = ctx.app.events_by("social.user.blocked", host.id).await.pop().unwrap();
    ctx.spaces.handle_event(&blocked).await.unwrap();
    assert_eq!(ctx.get(&host, &room.id).await.listener_count, 1);
    assert!(ctx.events(&mut listener_events).await.iter().any(|event| matches!(
        event,
        spaces_event::Event::ParticipantLeft(left) if left.reason == SpaceLeaveReason::SPACE_LEAVE_REASON_REMOVED
    )));
    ctx.wait_until_connected(&credentials.room, &listener.id.to_string(), false).await;

    // From then on the host's rooms do not exist for them — public or not.
    assert_eq!(ctx.reason("Space", "GetSpace", &listener, &id_request(&room.id)).await, "SPACE_NOT_FOUND");
    assert_eq!(ctx.reason("Space", "JoinSpace", &listener, &id_request(&room.id)).await, "SPACE_NOT_FOUND");
    let public: ListLiveSpacesResponse = ctx
        .call(
            "Space",
            "ListLiveSpaces",
            &listener,
            &ListLiveSpacesRequest { scope: LiveScope::LIVE_SCOPE_PUBLIC.into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(!public.spaces.iter().any(|space| space.id == room.id));
    assert!(!ctx.app.core.topic_guards.may_subscribe(Some(listener.id), &format!("space:{}", room.id)).await.unwrap());

    // And the other way round: the blocker cannot enter the blocked person's room.
    let their_room = ctx.public_room(&listener, "こっちのルーム").await;
    assert_eq!(ctx.reason("Space", "JoinSpace", &host, &id_request(&their_room.id)).await, "SPACE_NOT_FOUND");
}
