//! Inside a room: entering and leaving, raised hands, the stage, co-hosts,
//! handing the room over, muting, removing and banning, emoji bursts, the
//! pin and invitations.
mod common;

use common::*;
use tokuly_proto::spaces::{spaces_event::Event as Push, *};
use tokuly_spaces::SpacesConfig;
use tokuly_testkit::TestUser;

fn id_request(space_id: &str) -> JoinSpaceRequest {
    JoinSpaceRequest { space_id: space_id.to_owned(), ..Default::default() }
}

fn hand(space_id: &str, raised: bool) -> SetHandRaisedRequest {
    SetHandRaisedRequest { space_id: space_id.to_owned(), raised, ..Default::default() }
}

fn to_stage(space_id: &str, user: &TestUser) -> InviteToStageRequest {
    InviteToStageRequest { space_id: space_id.to_owned(), user_id: user.id.to_string(), ..Default::default() }
}

fn to_audience(space_id: &str, user: Option<&TestUser>) -> MoveToAudienceRequest {
    MoveToAudienceRequest {
        space_id: space_id.to_owned(),
        user_id: user.map(|user| user.id.to_string()).unwrap_or_default(),
        ..Default::default()
    }
}

fn cohost(space_id: &str, user: &TestUser, cohost: bool) -> SetCohostRequest {
    SetCohostRequest { space_id: space_id.to_owned(), user_id: user.id.to_string(), cohost, ..Default::default() }
}

fn remove(space_id: &str, user: &TestUser, ban: bool) -> RemoveParticipantRequest {
    RemoveParticipantRequest { space_id: space_id.to_owned(), user_id: user.id.to_string(), ban, ..Default::default() }
}

async fn participants(
    ctx: &Ctx,
    viewer: &TestUser,
    space_id: &str,
    filter: ParticipantFilter,
) -> Vec<SpaceParticipant> {
    let request =
        ListParticipantsRequest { space_id: space_id.to_owned(), filter: filter.into(), ..Default::default() };
    let response: ListParticipantsResponse = ctx.call("Space", "ListParticipants", viewer, &request).await.unwrap();
    response.participants
}

fn names(participants: &[SpaceParticipant]) -> Vec<&str> {
    participants.iter().map(|participant| participant.user.display_name.as_str()).collect()
}

#[tokio::test]
async fn listeners_enter_hear_and_leave() {
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let listener = ctx.app.user("hanako").await;
    let room = ctx.public_room(&host, "深夜ラジオ").await;

    let mut room_events = ctx.listen_room(&room.id).await;
    let joined = ctx.join(&listener, &room.id).await;
    let me = joined.me.as_option().unwrap();
    assert_eq!(me.role, SpaceRole::SPACE_ROLE_LISTENER);
    assert_eq!(me.user.id, listener.id.to_string());
    assert_eq!((joined.space.listener_count, joined.space.speaker_count, joined.space.peak_listeners), (2, 1, 2));
    let viewer = joined.space.viewer.as_option().unwrap();
    assert!(viewer.inside);
    assert_eq!(viewer.role, SpaceRole::SPACE_ROLE_LISTENER);

    // A listener's token lets them hear the room, and nothing else.
    let credentials = joined.credentials.as_option().unwrap();
    assert!(!credentials.can_publish);
    let claims = ctx.claims(credentials);
    assert_eq!((claims.sub.as_str(), claims.name.as_str()), (listener.id.to_string().as_str(), "hanako"));
    assert_eq!(claims.metadata, r#"{"role":"listener"}"#);
    assert_eq!(
        (claims.video.can_subscribe, claims.video.can_publish, claims.video.can_publish_data),
        (Some(true), Some(false), Some(false))
    );
    assert!(claims.video.room_join && !claims.video.room_admin);

    // The room hears a number, not a name: listeners come and go quietly.
    let pushed = ctx.events(&mut room_events).await;
    assert!(
        matches!(pushed.as_slice(), [Push::Counts(counts)] if counts.listener_count == 2 && counts.speaker_count == 1)
    );

    // Other domains hear of the first time someone enters a room, once.
    let event = ctx.app.events_by("spaces.room.joined", listener.id).await.pop().unwrap();
    assert_eq!(
        (event.data["space_id"].as_str(), event.data["host_id"].as_str()),
        (Some(room.id.as_str()), Some(host.id.to_string().as_str()))
    );
    assert!(ctx.app.events_by("spaces.room.joined", host.id).await.is_empty(), "the host started it instead");

    // Asking again (a reconnect, another device) gives fresh credentials and counts nobody twice.
    let again = ctx.join(&listener, &room.id).await;
    assert_eq!(again.space.listener_count, 2);
    assert_eq!(ctx.claims(again.credentials.as_option().unwrap()).metadata, r#"{"role":"listener"}"#);

    ctx.leave(&listener, &room.id).await;
    let after = ctx.get(&listener, &room.id).await;
    assert_eq!((after.listener_count, after.peak_listeners, after.total_listeners), (1, 2, 2));
    assert!(!after.viewer.inside);
    ctx.leave(&listener, &room.id).await; // leaving twice is fine
    ctx.join(&listener, &room.id).await;
    assert_eq!(ctx.app.events_by("spaces.room.joined", listener.id).await.len(), 1, "coming back is not a first time");

    assert_eq!(ctx.anonymous_reason("Space", "JoinSpace", &id_request(&room.id)).await, "SIGN_IN_REQUIRED");
    let unknown = uuid::Uuid::now_v7().to_string();
    assert_eq!(ctx.reason("Space", "JoinSpace", &listener, &id_request(&unknown)).await, "SPACE_NOT_FOUND");
    assert_eq!(ctx.reason("Space", "LeaveSpace", &listener, &id_request(&unknown)).await, "SPACE_NOT_FOUND");
    assert_eq!(ctx.reason("Space", "JoinSpace", &listener, &id_request("nonsense")).await, "INVALID_ID");
}

#[tokio::test]
async fn a_person_is_in_one_room_at_a_time() {
    let ctx = start().await;
    let (first_host, second_host) = (ctx.app.user("taro").await, ctx.app.user("jiro").await);
    let wanderer = ctx.app.user("hanako").await;
    let first = ctx.public_room(&first_host, "一つ目").await;
    let second = ctx.public_room(&second_host, "二つ目").await;

    ctx.join(&wanderer, &first.id).await;
    ctx.join(&wanderer, &second.id).await;
    assert_eq!(ctx.get(&wanderer, &first.id).await.listener_count, 1, "entering the second room left the first");
    assert_eq!(ctx.get(&wanderer, &second.id).await.listener_count, 2);

    // A hangout counts as a room too.
    ctx.befriend(&wanderer, &first_host).await;
    let hangout = ctx.hangout(&wanderer, "いつメン", &[&first_host]).await;
    ctx.join_hangout(&wanderer, &hangout.id).await;
    assert_eq!(ctx.get(&wanderer, &second.id).await.listener_count, 1);
    ctx.join(&wanderer, &first.id).await;
    assert_eq!(ctx.get_hangout(&wanderer, &hangout.id).await.inside_count, 0);
}

#[tokio::test]
async fn a_full_room_takes_nobody_else() {
    let ctx =
        start_with(SpacesConfig { max_listeners: 2, max_speakers: 2, max_cohosts: 1, ..Default::default() }).await;
    let host = ctx.app.user("taro").await;
    let (first, second) = (ctx.app.user("hanako").await, ctx.app.user("jiro").await);
    let room = ctx.public_room(&host, "せまい部屋").await;
    ctx.join(&first, &room.id).await;
    assert_eq!(ctx.reason("Space", "JoinSpace", &second, &id_request(&room.id)).await, "SPACE_FULL");
    // The host always gets back into their own room.
    ctx.leave(&host, &room.id).await;
    ctx.join(&second, &room.id).await;
    let back = ctx.join(&host, &room.id).await;
    assert_eq!(back.me.role, SpaceRole::SPACE_ROLE_HOST);
    assert_eq!(back.space.listener_count, 3);
}

#[tokio::test]
async fn the_list_shows_stage_then_hands_then_audience() {
    let ctx = start().await;
    let host = ctx.app.user("host").await;
    let room = ctx.public_room(&host, "満員御礼").await;
    let speaker = ctx.app.user("speaker").await;
    let (quiet, eager, keen) = (ctx.app.user("quiet").await, ctx.app.user("eager").await, ctx.app.user("keen").await);
    for user in [&speaker, &quiet, &eager, &keen] {
        ctx.join(user, &room.id).await;
    }
    ctx.bring_on_stage(&host, &speaker, &room.id).await;
    for user in [&keen, &eager] {
        let _: SetHandRaisedResponse = ctx.call("Space", "SetHandRaised", user, &hand(&room.id, true)).await.unwrap();
    }

    let everyone = participants(&ctx, &quiet, &room.id, ParticipantFilter::PARTICIPANT_FILTER_UNSPECIFIED).await;
    assert_eq!(names(&everyone), ["host", "speaker", "keen", "eager", "quiet"], "stage, hands as raised, the rest");
    assert_eq!(everyone[0].role, SpaceRole::SPACE_ROLE_HOST);
    assert!(everyone[2].hand_raised_at.is_set() && everyone[4].hand_raised_at.is_unset());
    assert_eq!(
        names(&participants(&ctx, &quiet, &room.id, ParticipantFilter::PARTICIPANT_FILTER_STAGE).await),
        ["host", "speaker"]
    );
    assert_eq!(
        names(&participants(&ctx, &quiet, &room.id, ParticipantFilter::PARTICIPANT_FILTER_HANDS).await),
        ["keen", "eager"]
    );
    assert_eq!(
        names(&participants(&ctx, &quiet, &room.id, ParticipantFilter::PARTICIPANT_FILTER_AUDIENCE).await),
        ["keen", "eager", "quiet"]
    );

    // Pages.
    let page = |cursor: &str| ListParticipantsRequest {
        space_id: room.id.clone(),
        page: tokuly_proto::common::PageRequest { limit: 2, cursor: cursor.to_owned(), ..Default::default() }.into(),
        ..Default::default()
    };
    let first: ListParticipantsResponse = ctx.call("Space", "ListParticipants", &quiet, &page("")).await.unwrap();
    assert_eq!(names(&first.participants), ["host", "speaker"]);
    assert!(first.page.has_more);
    let second: ListParticipantsResponse =
        ctx.call("Space", "ListParticipants", &quiet, &page(&first.page.next_cursor)).await.unwrap();
    assert_eq!(names(&second.participants), ["keen", "eager"]);
    let third: ListParticipantsResponse =
        ctx.call("Space", "ListParticipants", &quiet, &page(&second.page.next_cursor)).await.unwrap();
    assert_eq!(names(&third.participants), ["quiet"]);
    assert!(!third.page.has_more);

    // The room's own summary agrees.
    let space = ctx.get(&quiet, &room.id).await;
    assert_eq!((space.listener_count, space.speaker_count, space.hand_count), (5, 2, 2));
    assert_eq!(space.stage.len(), 2);
    let stranger_list = ListParticipantsRequest { space_id: uuid::Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.reason("Space", "ListParticipants", &quiet, &stranger_list).await, "SPACE_NOT_FOUND");
}

#[tokio::test]
async fn a_raised_hand_is_answered_and_the_connection_follows() {
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let listener = ctx.app.user("hanako").await;
    let outsider = ctx.app.user("jiro").await;
    let room = ctx.public_room(&host, "質問コーナー").await;
    let joined = ctx.join(&listener, &room.id).await;
    let credentials = joined.credentials.as_option().unwrap();
    let _connection = ctx.connect(credentials).await;
    let identity = listener.id.to_string();
    let connected = ctx.connected(&credentials.room, &identity).await.unwrap();
    let as_listener = connected.permission.as_ref().unwrap();
    assert!(!as_listener.can_publish, "LiveKit took the listener's token at its word");
    assert!(as_listener.hidden, "LiveKit does not tell the other clients who is listening");

    assert_eq!(ctx.reason("Space", "SetHandRaised", &outsider, &hand(&room.id, true)).await, "NOT_IN_SPACE");
    assert_eq!(ctx.reason("Space", "SetHandRaised", &host, &hand(&room.id, true)).await, "ALREADY_ON_STAGE");

    let mut room_events = ctx.listen_room(&room.id).await;
    let raised: SetHandRaisedResponse =
        ctx.call("Space", "SetHandRaised", &listener, &hand(&room.id, true)).await.unwrap();
    assert!(raised.me.hand_raised_at.is_set());
    let pushed = ctx.events(&mut room_events).await;
    assert!(pushed.iter().any(|event| matches!(
        event,
        Push::ParticipantUpdated(updated) if updated.participant.user.id == identity && updated.participant.hand_raised_at.is_set()
    )));
    assert!(pushed.iter().any(|event| matches!(event, Push::Counts(counts) if counts.hand_count == 1)));
    // Lowering and raising again.
    let lowered: SetHandRaisedResponse =
        ctx.call("Space", "SetHandRaised", &listener, &hand(&room.id, false)).await.unwrap();
    assert!(lowered.me.hand_raised_at.is_unset());
    assert_eq!(ctx.get(&host, &room.id).await.hand_count, 0);
    let _: SetHandRaisedResponse = ctx.call("Space", "SetHandRaised", &listener, &hand(&room.id, true)).await.unwrap();

    // Only moderators answer hands.
    assert_eq!(ctx.reason("Space", "InviteToStage", &listener, &to_stage(&room.id, &listener)).await, "NOT_MODERATOR");
    assert_eq!(
        ctx.reason("Space", "InviteToStage", &host, &to_stage(&room.id, &outsider)).await,
        "TARGET_NOT_IN_SPACE"
    );
    assert_eq!(ctx.reason("Space", "InviteToStage", &host, &to_stage(&room.id, &host)).await, "ALREADY_ON_STAGE");

    let mut room_events = ctx.listen_room(&room.id).await;
    let promoted: InviteToStageResponse =
        ctx.call("Space", "InviteToStage", &host, &to_stage(&room.id, &listener)).await.unwrap();
    assert!(promoted.on_stage);
    assert_eq!(promoted.participant.role, SpaceRole::SPACE_ROLE_SPEAKER);
    assert!(promoted.participant.hand_raised_at.is_unset(), "the hand is answered");
    let pushed = ctx.events(&mut room_events).await;
    assert!(pushed.iter().any(|event| matches!(
        event,
        Push::ParticipantUpdated(updated) if updated.participant.role == SpaceRole::SPACE_ROLE_SPEAKER
    )));
    assert!(
        pushed
            .iter()
            .any(|event| matches!(event, Push::Counts(counts) if counts.speaker_count == 2 && counts.hand_count == 0))
    );

    // The role change reached the open connection: they may talk now, without reconnecting.
    let connected = ctx.connected(&credentials.room, &identity).await.unwrap();
    let permission = connected.permission.unwrap();
    assert!(permission.can_publish && permission.can_subscribe && !permission.hidden, "on stage, for all to see");
    assert!(!permission.can_publish_data, "nothing uses the data channel");
    assert_eq!(connected.metadata, r#"{"role":"speaker"}"#);
    assert_eq!(ctx.reason("Space", "InviteToStage", &host, &to_stage(&room.id, &listener)).await, "ALREADY_ON_STAGE");

    // Stepping down takes the microphone away again.
    let stepped: MoveToAudienceResponse =
        ctx.call("Space", "MoveToAudience", &listener, &to_audience(&room.id, None)).await.unwrap();
    assert_eq!(stepped.participant.role, SpaceRole::SPACE_ROLE_LISTENER);
    let connected = ctx.connected(&credentials.room, &identity).await.unwrap();
    let back_in_the_audience = connected.permission.unwrap();
    assert!(!back_in_the_audience.can_publish && back_in_the_audience.hidden);
    assert_eq!(connected.metadata, r#"{"role":"listener"}"#);
    assert_eq!(ctx.get(&host, &room.id).await.speaker_count, 1);
}

#[tokio::test]
async fn an_invitation_on_stage_is_accepted_or_declined() {
    let ctx = start_with(SpacesConfig { max_speakers: 3, ..Default::default() }).await;
    let host = ctx.app.user("taro").await;
    let (shy, bold, late) = (ctx.app.user("shy").await, ctx.app.user("bold").await, ctx.app.user("late").await);
    let room = ctx.public_room(&host, "ゲストを呼ぶ回").await;
    for user in [&shy, &bold, &late] {
        ctx.join(user, &room.id).await;
    }
    let respond =
        |accept: bool| RespondToStageInviteRequest { space_id: room.id.clone(), accept, ..Default::default() };

    assert_eq!(ctx.reason("Space", "RespondToStageInvite", &shy, &respond(true)).await, "NO_STAGE_INVITATION");

    // No hand up: an invitation they have to answer.
    let mut shy_events = ctx.listen(&shy).await;
    let invited: InviteToStageResponse =
        ctx.call("Space", "InviteToStage", &host, &to_stage(&room.id, &shy)).await.unwrap();
    assert!(!invited.on_stage);
    assert_eq!(invited.participant.role, SpaceRole::SPACE_ROLE_LISTENER);
    assert!(invited.participant.stage_invited);
    assert!(ctx.events(&mut shy_events).await.iter().any(|event| matches!(
        event,
        Push::StageInvitation(invitation) if invitation.invited && invitation.invited_by.id == host.id.to_string()
    )));
    assert!(ctx.get(&shy, &room.id).await.viewer.stage_invited);
    // The invitation is between the moderators and the invited person.
    let seen_by_other = participants(&ctx, &bold, &room.id, ParticipantFilter::PARTICIPANT_FILTER_AUDIENCE).await;
    assert!(seen_by_other.iter().all(|participant| !participant.stage_invited));
    let seen_by_host = participants(&ctx, &host, &room.id, ParticipantFilter::PARTICIPANT_FILTER_AUDIENCE).await;
    assert!(seen_by_host.iter().any(|participant| participant.stage_invited));

    let declined: RespondToStageInviteResponse =
        ctx.call("Space", "RespondToStageInvite", &shy, &respond(false)).await.unwrap();
    assert_eq!(declined.me.role, SpaceRole::SPACE_ROLE_LISTENER);
    assert!(!ctx.get(&shy, &room.id).await.viewer.stage_invited);
    assert_eq!(ctx.reason("Space", "RespondToStageInvite", &shy, &respond(true)).await, "NO_STAGE_INVITATION");

    // Accepting makes a speaker, with credentials to match.
    let _: InviteToStageResponse = ctx.call("Space", "InviteToStage", &host, &to_stage(&room.id, &bold)).await.unwrap();
    let accepted: RespondToStageInviteResponse =
        ctx.call("Space", "RespondToStageInvite", &bold, &respond(true)).await.unwrap();
    assert_eq!(accepted.me.role, SpaceRole::SPACE_ROLE_SPEAKER);
    let credentials = accepted.credentials.as_option().unwrap();
    assert!(credentials.can_publish);
    assert_eq!(ctx.claims(credentials).video.can_publish, Some(true));

    // A moderator can take an invitation back.
    let _: InviteToStageResponse = ctx.call("Space", "InviteToStage", &host, &to_stage(&room.id, &shy)).await.unwrap();
    let mut shy_events = ctx.listen(&shy).await;
    let _: MoveToAudienceResponse =
        ctx.call("Space", "MoveToAudience", &host, &to_audience(&room.id, Some(&shy))).await.unwrap();
    assert!(
        ctx.events(&mut shy_events)
            .await
            .iter()
            .any(|event| matches!(event, Push::StageInvitation(invitation) if !invitation.invited))
    );
    assert_eq!(ctx.reason("Space", "RespondToStageInvite", &shy, &respond(true)).await, "NO_STAGE_INVITATION");

    // The stage has three seats here: host, bold, and one more.
    ctx.bring_on_stage(&host, &shy, &room.id).await;
    let _: SetHandRaisedResponse = ctx.call("Space", "SetHandRaised", &late, &hand(&room.id, true)).await.unwrap();
    assert_eq!(ctx.reason("Space", "InviteToStage", &host, &to_stage(&room.id, &late)).await, "STAGE_FULL");
    // A seat frees up when someone is moved back.
    assert_eq!(ctx.reason("Space", "MoveToAudience", &bold, &to_audience(&room.id, Some(&shy))).await, "NOT_MODERATOR");
    let moved: MoveToAudienceResponse =
        ctx.call("Space", "MoveToAudience", &host, &to_audience(&room.id, Some(&shy))).await.unwrap();
    assert_eq!(moved.participant.role, SpaceRole::SPACE_ROLE_LISTENER);
    let promoted: InviteToStageResponse =
        ctx.call("Space", "InviteToStage", &host, &to_stage(&room.id, &late)).await.unwrap();
    assert!(promoted.on_stage);

    assert_eq!(
        ctx.reason("Space", "MoveToAudience", &host, &to_audience(&room.id, None)).await,
        "HOST_CANNOT_STEP_DOWN"
    );
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(
        ctx.reason("Space", "MoveToAudience", &host, &to_audience(&room.id, Some(&outsider))).await,
        "TARGET_NOT_IN_SPACE"
    );
    assert_eq!(ctx.reason("Space", "MoveToAudience", &outsider, &to_audience(&room.id, None)).await, "NOT_IN_SPACE");
}

#[tokio::test]
async fn cohosts_moderate_below_themselves() {
    let ctx = start_with(SpacesConfig { max_cohosts: 1, ..Default::default() }).await;
    let host = ctx.app.user("taro").await;
    let (deputy, speaker, listener) =
        (ctx.app.user("deputy").await, ctx.app.user("speaker").await, ctx.app.user("listener").await);
    let room = ctx.public_room(&host, "番組").await;
    for user in [&deputy, &speaker, &listener] {
        ctx.join(user, &room.id).await;
    }
    ctx.bring_on_stage(&host, &speaker, &room.id).await;

    // Only the host names co-hosts.
    assert_eq!(ctx.reason("Space", "SetCohost", &speaker, &cohost(&room.id, &listener, true)).await, "NOT_HOST");
    assert_eq!(ctx.reason("Space", "SetCohost", &host, &cohost(&room.id, &host, true)).await, "INVALID_USER");
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(
        ctx.reason("Space", "SetCohost", &host, &cohost(&room.id, &outsider, true)).await,
        "TARGET_NOT_IN_SPACE"
    );
    let named: SetCohostResponse =
        ctx.call("Space", "SetCohost", &host, &cohost(&room.id, &deputy, true)).await.unwrap();
    assert_eq!(named.participant.role, SpaceRole::SPACE_ROLE_COHOST);
    assert_eq!(ctx.reason("Space", "SetCohost", &host, &cohost(&room.id, &speaker, true)).await, "TOO_MANY_COHOSTS");
    assert_eq!(ctx.get(&deputy, &room.id).await.speaker_count, 3, "a co-host is on stage");

    // A co-host runs the stage for speakers and listeners…
    let _: SetHandRaisedResponse = ctx.call("Space", "SetHandRaised", &listener, &hand(&room.id, true)).await.unwrap();
    let promoted: InviteToStageResponse =
        ctx.call("Space", "InviteToStage", &deputy, &to_stage(&room.id, &listener)).await.unwrap();
    assert!(promoted.on_stage);
    let moved: MoveToAudienceResponse =
        ctx.call("Space", "MoveToAudience", &deputy, &to_audience(&room.id, Some(&speaker))).await.unwrap();
    assert_eq!(moved.participant.role, SpaceRole::SPACE_ROLE_LISTENER);
    // …but not for the host, and cannot do what is the host's alone.
    assert_eq!(
        ctx.reason("Space", "MoveToAudience", &deputy, &to_audience(&room.id, Some(&host))).await,
        "CANNOT_MODERATE_USER"
    );
    assert_eq!(
        ctx.reason("Space", "RemoveParticipant", &deputy, &remove(&room.id, &host, false)).await,
        "CANNOT_MODERATE_USER"
    );
    assert_eq!(ctx.reason("Space", "SetCohost", &deputy, &cohost(&room.id, &speaker, true)).await, "NOT_HOST");
    let end = EndSpaceRequest { space_id: room.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Space", "EndSpace", &deputy, &end).await, "NOT_HOST");

    // A co-host who leaves comes back as one.
    ctx.leave(&deputy, &room.id).await;
    assert_eq!(ctx.get(&deputy, &room.id).await.viewer.role, SpaceRole::SPACE_ROLE_COHOST);
    let back = ctx.join(&deputy, &room.id).await;
    assert_eq!(back.me.role, SpaceRole::SPACE_ROLE_COHOST);
    assert!(back.credentials.can_publish);
    // A speaker who leaves comes back as a listener.
    ctx.leave(&listener, &room.id).await;
    assert_eq!(ctx.join(&listener, &room.id).await.me.role, SpaceRole::SPACE_ROLE_LISTENER);

    // The role can be taken away again; the person stays on stage as a speaker.
    let demoted: SetCohostResponse =
        ctx.call("Space", "SetCohost", &host, &cohost(&room.id, &deputy, false)).await.unwrap();
    assert_eq!(demoted.participant.role, SpaceRole::SPACE_ROLE_SPEAKER);
    assert_eq!(ctx.reason("Space", "InviteToStage", &deputy, &to_stage(&room.id, &speaker)).await, "NOT_MODERATOR");
}

#[tokio::test]
async fn the_host_hands_the_room_over() {
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let (heir, busy) = (ctx.app.user("heir").await, ctx.app.user("busy").await);
    let room = ctx.public_room(&host, "引き継ぎ").await;
    let heir_credentials = ctx.join(&heir, &room.id).await.credentials.as_option().unwrap().clone();
    let _connection = ctx.connect(&heir_credentials).await;
    let transfer = |user: &TestUser| TransferHostRequest {
        space_id: room.id.clone(),
        user_id: user.id.to_string(),
        ..Default::default()
    };

    assert_eq!(ctx.reason("Space", "TransferHost", &heir, &transfer(&heir)).await, "NOT_HOST");
    assert_eq!(ctx.reason("Space", "TransferHost", &host, &transfer(&host)).await, "INVALID_USER");
    assert_eq!(ctx.reason("Space", "TransferHost", &host, &transfer(&busy)).await, "TARGET_NOT_IN_SPACE");

    let mut room_events = ctx.listen_room(&room.id).await;
    let handed: TransferHostResponse = ctx.call("Space", "TransferHost", &host, &transfer(&heir)).await.unwrap();
    let space = handed.space.as_option().unwrap();
    assert_eq!(space.host.id, heir.id.to_string());
    assert_eq!(space.viewer.role, SpaceRole::SPACE_ROLE_COHOST, "the former host stays as a co-host");
    assert_eq!(space.speaker_count, 2);
    assert_eq!(space.stage[0].user.id, heir.id.to_string());
    assert!(ctx.events(&mut room_events).await.iter().any(|event| matches!(
        event,
        Push::SpaceUpdated(updated) if updated.space.host.id == heir.id.to_string()
    )));
    // The new host's connection may talk at once.
    let connected = ctx.connected(&heir_credentials.room, &heir.id.to_string()).await.unwrap();
    assert!(connected.permission.unwrap().can_publish);
    assert_eq!(connected.metadata, r#"{"role":"host"}"#);

    // The room is the new host's to end; the old host is free to start another.
    let end = EndSpaceRequest { space_id: room.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Space", "EndSpace", &host, &end).await, "NOT_HOST");
    let mine: ListMySpacesResponse =
        ctx.call("Space", "ListMySpaces", &heir, &ListMySpacesRequest::default()).await.unwrap();
    assert_eq!(mine.spaces.len(), 1);
    let _: EndSpaceResponse = ctx.call("Space", "EndSpace", &heir, &end).await.unwrap();
}

#[tokio::test]
async fn speakers_are_muted_and_people_removed_and_banned() {
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let (speaker, heckler) = (ctx.app.user("speaker").await, ctx.app.user("heckler").await);
    let room = ctx.public_room(&host, "まじめな話").await;
    ctx.join(&speaker, &room.id).await;
    ctx.bring_on_stage(&host, &speaker, &room.id).await;
    let heckler_credentials = ctx.join(&heckler, &room.id).await.credentials.as_option().unwrap().clone();
    let _connection = ctx.connect(&heckler_credentials).await;
    let mute = |user: &TestUser| MuteSpeakerRequest {
        space_id: room.id.clone(),
        user_id: user.id.to_string(),
        ..Default::default()
    };

    // Muting is for the stage, by moderators.
    assert_eq!(ctx.reason("Space", "MuteSpeaker", &speaker, &mute(&speaker)).await, "NOT_MODERATOR");
    assert_eq!(ctx.reason("Space", "MuteSpeaker", &host, &mute(&heckler)).await, "NOT_ON_STAGE");
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(ctx.reason("Space", "MuteSpeaker", &host, &mute(&outsider)).await, "TARGET_NOT_IN_SPACE");
    let mut room_events = ctx.listen_room(&room.id).await;
    let muted: MuteSpeakerResponse = ctx.call("Space", "MuteSpeaker", &host, &mute(&speaker)).await.unwrap();
    assert_eq!(muted.muted_tracks, 0, "the speaker has no microphone open in this test");
    assert!(ctx.events(&mut room_events).await.iter().any(|event| matches!(
        event,
        Push::SpeakerMuted(muted) if muted.user_id == speaker.id.to_string() && muted.muted_by_id == host.id.to_string()
    )));

    // Removing: out of the room and off LiveKit, but free to come back.
    assert_eq!(
        ctx.reason("Space", "RemoveParticipant", &speaker, &remove(&room.id, &heckler, false)).await,
        "NOT_MODERATOR"
    );
    assert_eq!(ctx.reason("Space", "RemoveParticipant", &host, &remove(&room.id, &host, false)).await, "INVALID_USER");
    assert_eq!(
        ctx.reason("Space", "RemoveParticipant", &host, &remove(&room.id, &outsider, false)).await,
        "TARGET_NOT_IN_SPACE"
    );
    let mut heckler_events = ctx.listen(&heckler).await;
    let _: RemoveParticipantResponse =
        ctx.call("Space", "RemoveParticipant", &host, &remove(&room.id, &heckler, false)).await.unwrap();
    assert!(ctx.events(&mut heckler_events).await.iter().any(|event| matches!(
        event,
        Push::ParticipantLeft(left) if left.reason == SpaceLeaveReason::SPACE_LEAVE_REASON_REMOVED && left.space_id == room.id
    )));
    ctx.wait_until_connected(&heckler_credentials.room, &heckler.id.to_string(), false).await;
    assert_eq!(ctx.get(&host, &room.id).await.listener_count, 2);
    ctx.join(&heckler, &room.id).await;

    // Banning: out for good.
    let _: RemoveParticipantResponse =
        ctx.call("Space", "RemoveParticipant", &host, &remove(&room.id, &heckler, true)).await.unwrap();
    // …and from then on the room is not there for them: not to enter, not to
    // look at, not to follow.
    assert_eq!(ctx.reason("Space", "JoinSpace", &heckler, &id_request(&room.id)).await, "SPACE_NOT_FOUND");
    assert_eq!(ctx.reason("Space", "GetSpace", &heckler, &id_request(&room.id)).await, "SPACE_NOT_FOUND");
    let list = ListParticipantsRequest { space_id: room.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Space", "ListParticipants", &heckler, &list).await, "SPACE_NOT_FOUND");
    assert!(!ctx.app.core.topic_guards.may_subscribe(Some(heckler.id), &format!("space:{}", room.id)).await.unwrap());
    // Someone who is not inside can be banned ahead of time.
    let _: RemoveParticipantResponse =
        ctx.call("Space", "RemoveParticipant", &host, &remove(&room.id, &outsider, true)).await.unwrap();
    assert_eq!(ctx.reason("Space", "JoinSpace", &outsider, &id_request(&room.id)).await, "SPACE_NOT_FOUND");

    let bans = ListBansRequest { space_id: room.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Space", "ListBans", &speaker, &bans).await, "NOT_MODERATOR");
    let listed: ListBansResponse = ctx.call("Space", "ListBans", &host, &bans).await.unwrap();
    assert_eq!(
        listed.bans.iter().map(|ban| ban.user.display_name.as_str()).collect::<Vec<_>>(),
        ["outsider", "heckler"]
    );
    assert_eq!(listed.bans[0].banned_by_id, host.id.to_string());

    let unban = UnbanUserRequest { space_id: room.id.clone(), user_id: heckler.id.to_string(), ..Default::default() };
    assert_eq!(ctx.reason("Space", "UnbanUser", &speaker, &unban).await, "NOT_MODERATOR");
    let _: UnbanUserResponse = ctx.call("Space", "UnbanUser", &host, &unban).await.unwrap();
    ctx.join(&heckler, &room.id).await;
}

#[tokio::test]
async fn emoji_bursts_reach_the_room_and_are_limited() {
    let ctx = start_with(SpacesConfig { reactions_per_10s: 3, ..Default::default() }).await;
    let host = ctx.app.user("taro").await;
    let listener = ctx.app.user("hanako").await;
    let outsider = ctx.app.user("jiro").await;
    let room = ctx.public_room(&host, "盛り上がってる").await;
    ctx.join(&listener, &room.id).await;
    let react =
        |emoji: &str| SendReactionRequest { space_id: room.id.clone(), emoji: emoji.to_owned(), ..Default::default() };

    let mut room_events = ctx.listen_room(&room.id).await;
    let _: SendReactionResponse = ctx.call("Space", "SendReaction", &listener, &react("🔥")).await.unwrap();
    let _: SendReactionResponse = ctx.call("Space", "SendReaction", &listener, &react(":tokuly_wave:")).await.unwrap();
    let pushed = ctx.events(&mut room_events).await;
    let bursts: Vec<(&str, &str)> = pushed
        .iter()
        .filter_map(|event| match event {
            Push::Reaction(reaction) => Some((reaction.user_id.as_str(), reaction.emoji.as_str())),
            _ => None,
        })
        .collect();
    let listener_id = listener.id.to_string();
    assert_eq!(bursts, [(listener_id.as_str(), "🔥"), (listener_id.as_str(), ":tokuly_wave:")]);

    assert_eq!(ctx.reason("Space", "SendReaction", &listener, &react("すごい")).await, "INVALID_EMOJI");
    assert_eq!(ctx.reason("Space", "SendReaction", &listener, &react("🔥🔥🔥")).await, "INVALID_EMOJI");
    assert_eq!(ctx.reason("Space", "SendReaction", &outsider, &react("🔥")).await, "NOT_IN_SPACE");
    // Three per ten seconds here: the third goes through, the fourth does not.
    let _: SendReactionResponse = ctx.call("Space", "SendReaction", &listener, &react("👏")).await.unwrap();
    assert_eq!(ctx.reason("Space", "SendReaction", &listener, &react("👏")).await, "RATE_LIMITED");
    // The limit is each person's own.
    let _: SendReactionResponse = ctx.call("Space", "SendReaction", &host, &react("❤️")).await.unwrap();
}

#[tokio::test]
async fn a_note_and_a_link_are_pinned() {
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let listener = ctx.app.user("hanako").await;
    let room = ctx.public_room(&host, "今日のテーマ").await;
    ctx.join(&listener, &room.id).await;
    let pin = |text: &str, url: &str| SetPinRequest {
        space_id: room.id.clone(),
        text: text.to_owned(),
        url: url.to_owned(),
        ..Default::default()
    };

    let mut room_events = ctx.listen_room(&room.id).await;
    let pinned: SetPinResponse =
        ctx.call("Space", "SetPin", &host, &pin("アンケートはこちら", "https://example.com/form")).await.unwrap();
    let shown = pinned.space.pin.as_option().unwrap();
    assert_eq!((shown.text.as_str(), shown.url.as_str()), ("アンケートはこちら", "https://example.com/form"));
    assert_eq!(shown.pinned_by_id, host.id.to_string());
    assert!(shown.pinned_at.is_set());
    assert!(ctx.events(&mut room_events).await.iter().any(|event| matches!(
        event,
        Push::SpaceUpdated(updated) if updated.space.pin.text == "アンケートはこちら"
    )));
    assert_eq!(ctx.get(&listener, &room.id).await.pin.url, "https://example.com/form");

    assert_eq!(ctx.reason("Space", "SetPin", &listener, &pin("のっとり", "")).await, "NOT_MODERATOR");
    assert_eq!(ctx.reason("Space", "SetPin", &host, &pin("", "javascript:alert(1)")).await, "INVALID_PIN");
    assert_eq!(ctx.reason("Space", "SetPin", &host, &pin(&"あ".repeat(201), "")).await, "INVALID_PIN");

    let unpinned: SetPinResponse = ctx.call("Space", "SetPin", &host, &pin("", "")).await.unwrap();
    assert!(unpinned.space.pin.is_unset());
    let _: EndSpaceResponse = ctx
        .call("Space", "EndSpace", &host, &EndSpaceRequest { space_id: room.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(ctx.reason("Space", "SetPin", &host, &pin("おわり", "")).await, "SPACE_ENDED");
}

#[tokio::test]
async fn friends_are_invited_to_rooms() {
    let ctx = start().await;
    let (host, friend) = ctx.friends("taro", "hanako").await;
    let listener = ctx.app.user("jiro").await;
    let listeners_friend = ctx.app.user("saburo").await;
    ctx.befriend(&listener, &listeners_friend).await;
    let stranger = ctx.app.user("shiro").await;
    let room = ctx.public_room(&host, "みんなおいで").await;
    ctx.join(&listener, &room.id).await;
    let invite = |users: &[&TestUser]| InviteToSpaceRequest {
        space_id: room.id.clone(),
        user_ids: users.iter().map(|user| user.id.to_string()).collect(),
        ..Default::default()
    };

    // People invite their own friends, nobody else.
    assert_eq!(ctx.reason("Space", "InviteToSpace", &host, &invite(&[&stranger])).await, "NOT_FRIENDS");
    let mut friend_events = ctx.listen(&friend).await;
    let invited: InviteToSpaceResponse = ctx.call("Space", "InviteToSpace", &host, &invite(&[&friend])).await.unwrap();
    assert_eq!(invited.invited_count, 1);
    let notification = ctx.notifications(&friend).await.pop().unwrap();
    assert_eq!(notification.kind, "spaces.invited");
    assert_eq!(notification.title.ja, "taroがルームに招待したよ!");
    assert!(ctx.events(&mut friend_events).await.iter().any(|event| matches!(event, Push::SpaceInvitation(_))));
    // Inviting again tells nobody twice.
    let again: InviteToSpaceResponse = ctx.call("Space", "InviteToSpace", &host, &invite(&[&friend])).await.unwrap();
    assert_eq!(again.invited_count, 0);
    assert_eq!(ctx.notifications(&friend).await.len(), 1);

    // A public room is everybody's to share — if they are inside.
    let shared: InviteToSpaceResponse =
        ctx.call("Space", "InviteToSpace", &listener, &invite(&[&listeners_friend])).await.unwrap();
    assert_eq!(shared.invited_count, 1);
    assert_eq!(ctx.reason("Space", "InviteToSpace", &stranger, &invite(&[])).await, "NOT_MODERATOR");
    let too_many = InviteToSpaceRequest {
        space_id: room.id.clone(),
        user_ids: (0..51).map(|_| uuid::Uuid::now_v7().to_string()).collect(),
        ..Default::default()
    };
    assert_eq!(ctx.reason("Space", "InviteToSpace", &host, &too_many).await, "TOO_MANY_INVITES");

    // Moderators see and manage the invitations.
    let list = ListSpaceInvitesRequest { space_id: room.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Space", "ListSpaceInvites", &listener, &list).await, "NOT_MODERATOR");
    let listed: ListSpaceInvitesResponse = ctx.call("Space", "ListSpaceInvites", &host, &list).await.unwrap();
    assert_eq!(
        listed.invites.iter().map(|invite| invite.user.display_name.as_str()).collect::<Vec<_>>(),
        ["saburo", "hanako"]
    );
    assert_eq!(listed.invites[0].invited_by_id, listener.id.to_string());
    let revoke = RevokeSpaceInviteRequest {
        space_id: room.id.clone(),
        user_id: listeners_friend.id.to_string(),
        ..Default::default()
    };
    assert_eq!(ctx.reason("Space", "RevokeSpaceInvite", &listener, &revoke).await, "NOT_MODERATOR");
    let _: RevokeSpaceInviteResponse = ctx.call("Space", "RevokeSpaceInvite", &host, &revoke).await.unwrap();
    let listed: ListSpaceInvitesResponse = ctx.call("Space", "ListSpaceInvites", &host, &list).await.unwrap();
    assert_eq!(listed.invites.len(), 1);
    assert!(!ctx.get(&listeners_friend, &room.id).await.viewer.invited);
}

#[tokio::test]
async fn people_who_are_gone_can_still_be_found_and_reported() {
    let ctx = start().await;
    let (host, invited) = ctx.friends("taro", "invited").await;
    let (loud, quiet, witness) =
        (ctx.app.user("loud").await, ctx.app.user("quiet").await, ctx.app.user("witness").await);
    let room = ctx.public_room(&host, "あれた回").await;
    for user in [&loud, &quiet, &witness] {
        ctx.join(user, &room.id).await;
    }
    // Someone gets on stage, says their piece and leaves; someone else is banned from the audience.
    ctx.bring_on_stage(&host, &loud, &room.id).await;
    ctx.leave(&loud, &room.id).await;
    let _: RemoveParticipantResponse =
        ctx.call("Space", "RemoveParticipant", &host, &remove(&room.id, &quiet, true)).await.unwrap();

    // Whoever was on stage can be found by everyone, with what they were.
    let past = participants(&ctx, &witness, &room.id, ParticipantFilter::PARTICIPANT_FILTER_PAST).await;
    assert_eq!(names(&past), ["loud"], "a listener who left is the moderators' business");
    assert_eq!(past[0].role, SpaceRole::SPACE_ROLE_SPEAKER, "the highest role they held");
    assert!(past[0].left_at.is_set());
    assert_eq!(past[0].leave_reason, SpaceLeaveReason::SPACE_LEAVE_REASON_LEFT);
    // Moderators see everyone who is gone, last to go first.
    let past = participants(&ctx, &host, &room.id, ParticipantFilter::PARTICIPANT_FILTER_PAST).await;
    assert_eq!(names(&past), ["quiet", "loud"]);
    assert_eq!(past[0].leave_reason, SpaceLeaveReason::SPACE_LEAVE_REASON_BANNED);

    // Still so after the room ended.
    let _: EndSpaceResponse = ctx
        .call("Space", "EndSpace", &host, &EndSpaceRequest { space_id: room.id.clone(), ..Default::default() })
        .await
        .unwrap();
    let past = participants(&ctx, &witness, &room.id, ParticipantFilter::PARTICIPANT_FILTER_PAST).await;
    assert_eq!(names(&past).len(), 2, "the host and the speaker: {:?}", names(&past));
    assert!(past.iter().any(|p| p.user.display_name == "taro" && p.role == SpaceRole::SPACE_ROLE_HOST));

    // Being thrown out, banned or uninvited does not take away the right to report the room.
    let subjects = &ctx.app.core.subjects;
    let space = tokuly_proto::common::SubjectType::SUBJECT_TYPE_SPACE;
    let request = CreateSpaceRequest {
        title: "招待制".into(),
        audience: SpaceAudience::SPACE_AUDIENCE_INVITED.into(),
        invitee_ids: vec![invited.id.to_string()],
        ..Default::default()
    };
    let created: CreateSpaceResponse = ctx.call("Space", "CreateSpace", &host, &request).await.unwrap();
    let secret = created.space.id.clone();
    ctx.join(&invited, &secret).await;
    let _: RemoveParticipantResponse =
        ctx.call("Space", "RemoveParticipant", &host, &remove(&secret, &invited, true)).await.unwrap();
    assert_eq!(ctx.reason("Space", "GetSpace", &invited, &id_request(&secret)).await, "SPACE_NOT_FOUND");
    let subject = subjects.resolve(space, &secret, Some(invited.id)).await.expect("still reportable");
    assert_eq!(subject.owner_id, host.id);
    assert!(subjects.resolve(space, &secret, Some(witness.id)).await.is_err(), "never part of it");
    // …nor does the host deleting the room afterwards.
    let _: EndSpaceResponse = ctx
        .call("Space", "EndSpace", &host, &EndSpaceRequest { space_id: secret.clone(), ..Default::default() })
        .await
        .unwrap();
    let _: DeleteSpaceResponse = ctx
        .call("Space", "DeleteSpace", &host, &DeleteSpaceRequest { space_id: secret.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(ctx.reason("Space", "GetSpace", &host, &id_request(&secret)).await, "SPACE_NOT_FOUND");
    assert!(subjects.resolve(space, &secret, Some(invited.id)).await.is_ok());
    assert!(subjects.resolve(space, &secret, Some(witness.id)).await.is_err());
}

#[tokio::test]
async fn what_reaches_other_people_is_rate_limited() {
    let ctx = start().await;
    let (host, friend) = ctx.friends("taro", "hanako").await;
    let listener = ctx.app.user("jiro").await;
    let room = ctx.public_room(&host, "しつこい").await;
    ctx.join(&listener, &room.id).await;

    // Waving a hand up and down: ten times a minute.
    for n in 0..10 {
        let _: SetHandRaisedResponse =
            ctx.call("Space", "SetHandRaised", &listener, &hand(&room.id, n % 2 == 0)).await.unwrap();
    }
    assert_eq!(ctx.reason("Space", "SetHandRaised", &listener, &hand(&room.id, true)).await, "RATE_LIMITED");

    // Calling the same listener on stage again and again: four times a minute.
    let mut listener_events = ctx.listen(&listener).await;
    for _ in 0..4 {
        let _: InviteToStageResponse =
            ctx.call("Space", "InviteToStage", &host, &to_stage(&room.id, &listener)).await.unwrap();
        let _: MoveToAudienceResponse =
            ctx.call("Space", "MoveToAudience", &host, &to_audience(&room.id, Some(&listener))).await.unwrap();
    }
    assert_eq!(ctx.reason("Space", "InviteToStage", &host, &to_stage(&room.id, &listener)).await, "RATE_LIMITED");
    let rung = ctx
        .events(&mut listener_events)
        .await
        .iter()
        .filter(|e| matches!(e, Push::StageInvitation(i) if i.invited))
        .count();
    assert_eq!(rung, 4);

    // Inviting, taking it back and inviting again rings the friend's phone three times a day, not more.
    let invite =
        InviteToSpaceRequest { space_id: room.id.clone(), user_ids: vec![friend.id.to_string()], ..Default::default() };
    let revoke =
        RevokeSpaceInviteRequest { space_id: room.id.clone(), user_id: friend.id.to_string(), ..Default::default() };
    for _ in 0..5 {
        let invited: InviteToSpaceResponse = ctx.call("Space", "InviteToSpace", &host, &invite).await.unwrap();
        assert_eq!(invited.invited_count, 1, "the invitation itself always counts");
        let _: RevokeSpaceInviteResponse = ctx.call("Space", "RevokeSpaceInvite", &host, &revoke).await.unwrap();
    }
    assert_eq!(ctx.notifications(&friend).await.iter().filter(|n| n.kind == "spaces.invited").count(), 3);

    // Pinning: twenty changes a minute.
    let pin =
        |n: usize| SetPinRequest { space_id: room.id.clone(), text: format!("お知らせ{n}"), ..Default::default() };
    for n in 0..20 {
        let _: SetPinResponse = ctx.call("Space", "SetPin", &host, &pin(n)).await.unwrap();
    }
    assert_eq!(ctx.reason("Space", "SetPin", &host, &pin(21)).await, "RATE_LIMITED");
}

#[tokio::test]
async fn friends_are_told_about_one_room_not_about_every_room() {
    let ctx = start().await;
    let (host, friend) = ctx.friends("taro", "hanako").await;
    for title in ["一つ目", "二つ目"] {
        let (room, _) = ctx.open_room(&host, title, SpaceAudience::SPACE_AUDIENCE_FRIENDS).await;
        let started = ctx.app.events_by("spaces.room.started", host.id).await.pop().unwrap();
        ctx.spaces.handle_event(&started).await.unwrap();
        let _: EndSpaceResponse = ctx
            .call("Space", "EndSpace", &host, &EndSpaceRequest { space_id: room.id.clone(), ..Default::default() })
            .await
            .unwrap();
    }
    let told = ctx.notifications(&friend).await;
    assert_eq!(
        told.iter().filter(|n| n.kind == "spaces.started").count(),
        1,
        "the second room within half an hour starts quietly"
    );
}

#[tokio::test]
async fn who_is_listening_is_not_for_everyone() {
    let ctx = start().await;
    let host = ctx.app.user("host").await;
    let (open, shy, watcher) = (ctx.app.user("open").await, ctx.app.user("shy").await, ctx.app.user("watcher").await);
    let room = ctx.public_room(&host, "だれがいる?").await;
    for user in [&open, &shy] {
        ctx.join(user, &room.id).await;
    }
    ctx.set_privacy(&shy, |privacy| privacy.share_online_status = false).await;
    let list = |filter: ParticipantFilter| ListParticipantsRequest {
        space_id: room.id.clone(),
        filter: filter.into(),
        ..Default::default()
    };

    // From outside, a room shows its stage and nothing of its audience: a
    // list of listeners would say where each of them is right now.
    let seen = participants(&ctx, &watcher, &room.id, ParticipantFilter::PARTICIPANT_FILTER_UNSPECIFIED).await;
    assert_eq!(names(&seen), ["host"]);
    assert_eq!(
        names(&participants(&ctx, &watcher, &room.id, ParticipantFilter::PARTICIPANT_FILTER_STAGE).await),
        ["host"]
    );
    for filter in [
        ParticipantFilter::PARTICIPANT_FILTER_AUDIENCE,
        ParticipantFilter::PARTICIPANT_FILTER_HANDS,
        ParticipantFilter::PARTICIPANT_FILTER_PAST,
    ] {
        assert_eq!(
            ctx.reason("Space", "ListParticipants", &watcher, &list(filter)).await,
            "NOT_IN_SPACE",
            "{filter:?}"
        );
    }

    // From inside, the audience shows — except who keeps their presence to
    // themselves, who is only counted.
    ctx.join(&watcher, &room.id).await;
    let audience: ListParticipantsResponse = ctx
        .call("Space", "ListParticipants", &watcher, &list(ParticipantFilter::PARTICIPANT_FILTER_AUDIENCE))
        .await
        .unwrap();
    assert_eq!(names(&audience.participants), ["open", "watcher"]);
    assert_eq!(audience.hidden_count, 1);
    // They see themselves, of course.
    let own: ListParticipantsResponse = ctx
        .call("Space", "ListParticipants", &shy, &list(ParticipantFilter::PARTICIPANT_FILTER_AUDIENCE))
        .await
        .unwrap();
    assert_eq!(names(&own.participants), ["open", "shy", "watcher"]);
    assert_eq!(own.hidden_count, 0);
    // Moderators see everyone: they answer for the room.
    let all: ListParticipantsResponse = ctx
        .call("Space", "ListParticipants", &host, &list(ParticipantFilter::PARTICIPANT_FILTER_AUDIENCE))
        .await
        .unwrap();
    assert_eq!(names(&all.participants), ["open", "shy", "watcher"]);
    assert_eq!(all.hidden_count, 0);
    // On stage there is nothing left to hide.
    ctx.bring_on_stage(&host, &shy, &room.id).await;
    let everyone = participants(&ctx, &watcher, &room.id, ParticipantFilter::PARTICIPANT_FILTER_UNSPECIFIED).await;
    assert_eq!(names(&everyone), ["host", "shy", "open", "watcher"]);

    // People on either side of a block do not show up in each other's lists,
    // in someone else's room either.
    ctx.block(&open, &watcher).await;
    for (viewer, other) in [(&watcher, "open"), (&open, "watcher")] {
        let seen = participants(&ctx, viewer, &room.id, ParticipantFilter::PARTICIPANT_FILTER_UNSPECIFIED).await;
        assert!(!names(&seen).contains(&other), "{:?}", names(&seen));
        assert_eq!(seen.len(), 3);
    }
    assert_eq!(participants(&ctx, &host, &room.id, ParticipantFilter::PARTICIPANT_FILTER_UNSPECIFIED).await.len(), 4);

    // Someone who left can still look up who was on stage, not who is listening now.
    ctx.leave(&watcher, &room.id).await;
    ctx.leave(&shy, &room.id).await;
    let past = participants(&ctx, &watcher, &room.id, ParticipantFilter::PARTICIPANT_FILTER_PAST).await;
    assert_eq!(names(&past), ["shy"]);
    assert_eq!(
        ctx.reason("Space", "ListParticipants", &watcher, &list(ParticipantFilter::PARTICIPANT_FILTER_AUDIENCE)).await,
        "NOT_IN_SPACE"
    );
}
