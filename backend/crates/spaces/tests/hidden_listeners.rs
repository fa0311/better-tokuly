//! The rule behind hidden listeners, as a property: **someone who is not a
//! moderator learns nothing that tells which hidden listener did
//! something.**
//!
//! A listener who does not share their online status is counted, not
//! named. Two worlds are run side by side. Both have a host, an observer
//! inside the room (who also follows its topic, as anyone may for a public
//! room) and two hidden listeners, Shy and Quiet. In one world Shy raises
//! her hand, sends emoji, lowers the hand, has it dismissed by the host and
//! finally leaves with her hand up; in the other Quiet does exactly the
//! same and Shy just sits there. After every step, everything the observer
//! can see — every frame on the room's topic, every push to their devices,
//! every list of participants and the room itself — must be the same in
//! both worlds, IDs aside (each world has its own people, and they are
//! named by role before comparing: Shy stays "shy" in both worlds, so a
//! frame naming her in one world and not in the other is a difference).
//!
//! The moderators and the person themselves still get what they need: the
//! host is told whose hand is up, and Shy sees her own hand and burst.
mod common;

use common::*;
use serde_json::Value;
use tokuly_proto::spaces::{spaces_event::Event as Push, *};
use tokuly_testkit::{FrameListener, TestUser};

struct World {
    room: Space,
    host: TestUser,
    observer: TestUser,
    shy: TestUser,
    quiet: TestUser,
    topic: FrameListener,
    observer_devices: FrameListener,
}

impl World {
    /// Shy and Quiet hide from the start.
    async fn new(ctx: &Ctx) -> World {
        World::with(ctx, true).await
    }

    async fn with(ctx: &Ctx, hidden_from_start: bool) -> World {
        let host = ctx.app.user("host").await;
        let observer = ctx.app.user("observer").await;
        let shy = ctx.app.user("shy").await;
        let quiet = ctx.app.user("quiet").await;
        if hidden_from_start {
            for hidden in [&shy, &quiet] {
                ctx.set_privacy(hidden, |privacy| privacy.share_online_status = false).await;
            }
        }
        let room = ctx.public_room(&host, "ひみつのラジオ").await;
        for user in [&observer, &shy, &quiet] {
            ctx.join(user, &room.id).await;
        }
        let topic = ctx.listen_room(&room.id).await;
        let observer_devices = ctx.listen(&observer).await;
        World { room, host, observer, shy, quiet, topic, observer_devices }
    }

    /// Replaces what differs between worlds by nature (IDs, handles, times)
    /// with fixed labels: the same person gets the same label in both.
    fn normalize(&self, value: &mut Value) {
        let names: Vec<(String, &str)> = [
            (self.room.id.clone(), "<room>"),
            (self.host.id.to_string(), "<host>"),
            (self.observer.id.to_string(), "<observer>"),
            (self.shy.id.to_string(), "<shy>"),
            (self.quiet.id.to_string(), "<quiet>"),
        ]
        .into_iter()
        .filter(|(id, _)| !id.is_empty())
        .collect();
        fn walk(value: &mut Value, names: &[(String, &str)]) {
            match value {
                Value::String(text) => {
                    for (id, label) in names {
                        if text.contains(id.as_str()) {
                            *text = text.replace(id.as_str(), label);
                        }
                    }
                    // Times differ between worlds; whether one is there does not.
                    if chrono::DateTime::parse_from_rfc3339(text).is_ok() {
                        *text = "<time>".into();
                    }
                }
                Value::Array(items) => items.iter_mut().for_each(|item| walk(item, names)),
                Value::Object(fields) => {
                    for (key, field) in fields.iter_mut() {
                        // A handle is made unique per test user; the display name is the label.
                        if key == "handle" {
                            *field = Value::String("<handle>".into());
                        } else {
                            walk(field, names);
                        }
                    }
                }
                _ => {}
            }
        }
        walk(value, &names);
    }

    /// Everything the observer can see right now.
    /// Also returns the topic's events as they are, for checks of their own.
    async fn observe(&mut self, ctx: &Ctx) -> (Value, Vec<Push>) {
        let events: Vec<SpacesEvent> =
            self.topic.drain(SOON).await.iter().filter_map(|frame| frame.event::<SpacesEvent>()).collect();
        let topic: Vec<Value> = events.iter().map(|event| serde_json::to_value(event).unwrap()).collect();
        let devices: Vec<Value> = self
            .observer_devices
            .drain(SOON)
            .await
            .iter()
            .filter_map(|frame| frame.event::<SpacesEvent>())
            .map(|event| serde_json::to_value(&event).unwrap())
            .collect();
        let mut lists = Vec::new();
        for filter in [
            ParticipantFilter::PARTICIPANT_FILTER_UNSPECIFIED,
            ParticipantFilter::PARTICIPANT_FILTER_STAGE,
            ParticipantFilter::PARTICIPANT_FILTER_HANDS,
            ParticipantFilter::PARTICIPANT_FILTER_AUDIENCE,
            ParticipantFilter::PARTICIPANT_FILTER_PAST,
        ] {
            let request =
                ListParticipantsRequest { space_id: self.room.id.clone(), filter: filter.into(), ..Default::default() };
            let response: ListParticipantsResponse =
                ctx.call("Space", "ListParticipants", &self.observer, &request).await.unwrap();
            lists.push(serde_json::to_value(&response).unwrap());
        }
        let room = serde_json::to_value(ctx.get(&self.observer, &self.room.id).await).unwrap();
        let mut seen = serde_json::json!({ "topic": topic, "devices": devices, "lists": lists, "room": room });
        self.normalize(&mut seen);
        (seen, events.into_iter().filter_map(|event| event.event).collect())
    }
}

fn hand(space_id: &str, raised: bool) -> SetHandRaisedRequest {
    SetHandRaisedRequest { space_id: space_id.to_owned(), raised, ..Default::default() }
}

fn react(space_id: &str, emoji: &str, nonce: &str) -> SendReactionRequest {
    SendReactionRequest {
        space_id: space_id.to_owned(),
        emoji: emoji.to_owned(),
        nonce: nonce.to_owned(),
        ..Default::default()
    }
}

/// The steps the acting listener goes through, one at a time.
async fn step(ctx: &Ctx, world: &World, actor: &TestUser, number: usize) {
    let room = &world.room.id;
    match number {
        0 => {
            let _: SetHandRaisedResponse = ctx.call("Space", "SetHandRaised", actor, &hand(room, true)).await.unwrap();
        }
        1 => {
            let _: SendReactionResponse =
                ctx.call("Space", "SendReaction", actor, &react(room, "🔥", "burst-1")).await.unwrap();
            let _: SendReactionResponse =
                ctx.call("Space", "SendReaction", actor, &react(room, "👏", "")).await.unwrap();
        }
        2 => {
            let _: SetHandRaisedResponse = ctx.call("Space", "SetHandRaised", actor, &hand(room, false)).await.unwrap();
        }
        3 => {
            let _: SetHandRaisedResponse = ctx.call("Space", "SetHandRaised", actor, &hand(room, true)).await.unwrap();
            // The host dismisses the hand.
            let dismiss =
                MoveToAudienceRequest { space_id: room.clone(), user_id: actor.id.to_string(), ..Default::default() };
            let _: MoveToAudienceResponse = ctx.call("Space", "MoveToAudience", &world.host, &dismiss).await.unwrap();
        }
        4 => {
            let _: SetHandRaisedResponse = ctx.call("Space", "SetHandRaised", actor, &hand(room, true)).await.unwrap();
            ctx.leave(actor, room).await;
        }
        _ => unreachable!(),
    }
}

const STEPS: usize = 5;

#[tokio::test]
async fn whoever_is_not_a_moderator_cannot_tell_which_hidden_listener_acted() {
    let ctx = start().await;
    let mut shy_acts = World::new(&ctx).await;
    let mut quiet_acts = World::new(&ctx).await;
    // What is there before anyone does anything is the same, too.
    assert_eq!(shy_acts.observe(&ctx).await.0, quiet_acts.observe(&ctx).await.0);

    // The host and Shy herself, in the world where she acts.
    let mut host_devices = ctx.listen(&shy_acts.host).await;
    let mut shy_devices = ctx.listen(&shy_acts.shy).await;
    let mut host_heard = Vec::new();
    let mut shy_heard = Vec::new();

    for number in 0..STEPS {
        step(&ctx, &shy_acts, &shy_acts.shy, number).await;
        step(&ctx, &quiet_acts, &quiet_acts.quiet, number).await;
        let ((a, pushed), (b, _)) = (shy_acts.observe(&ctx).await, quiet_acts.observe(&ctx).await);
        assert_eq!(a, b, "step {number}: the observer can tell who acted");
        // Something did happen: the observer sees the anonymous part.
        let hands: Vec<i32> = pushed
            .iter()
            .filter_map(|event| match event {
                Push::Counts(counts) => Some(counts.hand_count),
                _ => None,
            })
            .collect();
        let bursts = pushed.iter().filter(|event| matches!(event, Push::Reaction(_))).count();
        match number {
            0 => assert_eq!(hands, [1]),
            1 => assert_eq!((hands.len(), bursts), (0, 2)),
            2 => assert_eq!(hands, [0]),
            3 => assert_eq!(hands, [1, 0]),
            _ => assert_eq!(hands.last(), Some(&0)),
        }
        assert!(
            pushed.iter().all(|event| matches!(event, Push::Counts(_) | Push::Reaction(_))),
            "step {number}: the topic hears nothing but counts and bursts: {pushed:?}"
        );
        host_heard.extend(ctx.events(&mut host_devices).await);
        shy_heard.extend(ctx.events(&mut shy_devices).await);
        if number == 0 {
            // The host can see whose hand is up in the queue.
            let request = ListParticipantsRequest {
                space_id: shy_acts.room.id.clone(),
                filter: ParticipantFilter::PARTICIPANT_FILTER_HANDS.into(),
                ..Default::default()
            };
            let hands: ListParticipantsResponse =
                ctx.call("Space", "ListParticipants", &shy_acts.host, &request).await.unwrap();
            assert_eq!(hands.participants.len(), 1);
            assert_eq!(hands.participants[0].user.id, shy_acts.shy.id.to_string());
            assert!(hands.participants[0].hand_raised_at.is_set() && hands.participants[0].joined_at.is_set());
        }
    }

    // The host was told of every hand by name, and of the departure with a hand up.
    let shy_id = shy_acts.shy.id.to_string();
    let hands: Vec<bool> = host_heard
        .iter()
        .filter_map(|event| match event {
            Push::ParticipantUpdated(updated) if updated.participant.user.id == shy_id => {
                Some(updated.participant.hand_raised_at.is_set())
            }
            _ => None,
        })
        .collect();
    assert_eq!(hands, [true, false, true, false, true], "raise, lower, raise, dismissed, raise");
    assert!(
        host_heard.iter().any(|event| matches!(event, Push::ParticipantLeft(left) if left.user_id == shy_id)),
        "the host's hand queue loses her"
    );
    // Shy sees her own hand go up and down.
    let own: Vec<bool> = shy_heard
        .iter()
        .filter_map(|event| match event {
            Push::ParticipantUpdated(updated) if updated.participant.user.id == shy_id => {
                Some(updated.participant.hand_raised_at.is_set())
            }
            _ => None,
        })
        .collect();
    assert_eq!(own, [true, false, true, false, true]);
}

#[tokio::test]
async fn bursts_carry_no_sender_but_the_nonce() {
    let ctx = start().await;
    let host = ctx.app.user("host").await;
    let listener = ctx.app.user("listener").await;
    let room = ctx.public_room(&host, "ばくはつ").await;
    ctx.join(&listener, &room.id).await;
    let mut topic = ctx.listen_room(&room.id).await;
    let _: SendReactionResponse =
        ctx.call("Space", "SendReaction", &listener, &react(&room.id, "🎉", "a1-B_2")).await.unwrap();
    let bursts: Vec<(String, String)> = ctx
        .events(&mut topic)
        .await
        .into_iter()
        .filter_map(|event| match event {
            Push::Reaction(reaction) => Some((reaction.emoji, reaction.nonce)),
            _ => None,
        })
        .collect();
    assert_eq!(bursts, [("🎉".to_owned(), "a1-B_2".to_owned())]);
    for nonce in ["has space", "日本語", &"x".repeat(65)] {
        assert_eq!(
            ctx.reason("Space", "SendReaction", &listener, &react(&room.id, "🎉", nonce)).await,
            "INVALID_NONCE",
            "{nonce}"
        );
    }
}

#[tokio::test]
async fn nothing_from_the_hidden_time_comes_out_later() {
    let ctx = start().await;
    let host = ctx.app.user("host").await;
    let observer = ctx.app.user("observer").await;
    let shy = ctx.app.user("shy").await;
    ctx.set_privacy(&shy, |privacy| privacy.share_online_status = false).await;
    let room = ctx.public_room(&host, "あとから").await;
    for user in [&observer, &shy] {
        ctx.join(user, &room.id).await;
    }
    let raise = SetHandRaisedRequest { space_id: room.id.clone(), raised: true, ..Default::default() };
    let _: SetHandRaisedResponse = ctx.call("Space", "SetHandRaised", &shy, &raise).await.unwrap();

    // She stops hiding: from now on she shows, but not the hand she raised
    // while hidden, nor since when she has been inside.
    ctx.set_privacy(&shy, |privacy| privacy.share_online_status = true).await;
    let list = |filter: ParticipantFilter| ListParticipantsRequest {
        space_id: room.id.clone(),
        filter: filter.into(),
        ..Default::default()
    };
    let hands: ListParticipantsResponse = ctx
        .call("Space", "ListParticipants", &observer, &list(ParticipantFilter::PARTICIPANT_FILTER_HANDS))
        .await
        .unwrap();
    assert!(hands.participants.is_empty());
    let audience: ListParticipantsResponse = ctx
        .call("Space", "ListParticipants", &observer, &list(ParticipantFilter::PARTICIPANT_FILTER_AUDIENCE))
        .await
        .unwrap();
    let her = audience.participants.iter().find(|p| p.user.id == shy.id.to_string()).expect("she shows now");
    assert!(her.hand_raised_at.is_unset() && her.joined_at.is_unset(), "{her:?}");
    // She herself still sees all of it.
    let own: ListParticipantsResponse = ctx
        .call("Space", "ListParticipants", &shy, &list(ParticipantFilter::PARTICIPANT_FILTER_AUDIENCE))
        .await
        .unwrap();
    let me = own.participants.iter().find(|p| p.user.id == shy.id.to_string()).unwrap();
    assert!(me.hand_raised_at.is_set() && me.joined_at.is_set());

    // Lowering that hand is not announced to the room either.
    let mut topic = ctx.listen_room(&room.id).await;
    let lower = SetHandRaisedRequest { space_id: room.id.clone(), raised: false, ..Default::default() };
    let _: SetHandRaisedResponse = ctx.call("Space", "SetHandRaised", &shy, &lower).await.unwrap();
    let pushed = ctx.events(&mut topic).await;
    assert!(pushed.iter().all(|event| matches!(event, Push::Counts(_))), "{pushed:?}");

    // A hand raised now, visible, is announced as always.
    let _: SetHandRaisedResponse = ctx.call("Space", "SetHandRaised", &shy, &raise).await.unwrap();
    let pushed = ctx.events(&mut topic).await;
    assert!(
        pushed
            .iter()
            .any(|event| matches!(event, Push::ParticipantUpdated(u) if u.participant.user.id == shy.id.to_string()))
    );

    // On stage she shows from when she came up, not from when she entered.
    ctx.set_privacy(&shy, |privacy| privacy.share_online_status = false).await;
    let before_stage = chrono::Utc::now();
    let invite = InviteToStageRequest { space_id: room.id.clone(), user_id: shy.id.to_string(), ..Default::default() };
    let _: InviteToStageResponse = ctx.call("Space", "InviteToStage", &host, &invite).await.unwrap();
    let stage = ctx.get(&observer, &room.id).await.stage;
    let on_stage = stage.iter().find(|p| p.user.id == shy.id.to_string()).expect("on stage for all to see");
    let since = on_stage.joined_at.as_option().expect("the time she came on stage");
    assert!(since.seconds >= before_stage.timestamp() - 1, "joined_at is the time she came on stage");

    // Stepping down makes her hidden again; when she then leaves, the past
    // list does not change for others: it lists departures from the stage.
    let down = MoveToAudienceRequest { space_id: room.id.clone(), ..Default::default() };
    let _: MoveToAudienceResponse = ctx.call("Space", "MoveToAudience", &shy, &down).await.unwrap();
    let past = || async {
        let past: ListParticipantsResponse = ctx
            .call("Space", "ListParticipants", &observer, &list(ParticipantFilter::PARTICIPANT_FILTER_PAST))
            .await
            .unwrap();
        past.participants
    };
    assert!(past().await.is_empty());
    ctx.leave(&shy, &room.id).await;
    assert!(past().await.is_empty(), "her departure from the audience is not for others to see");
    let for_host: ListParticipantsResponse =
        ctx.call("Space", "ListParticipants", &host, &list(ParticipantFilter::PARTICIPANT_FILTER_PAST)).await.unwrap();
    let gone = for_host.participants.iter().find(|p| p.user.id == shy.id.to_string()).unwrap();
    assert!(gone.left_at.is_set(), "moderators see when");

    // Leaving from the stage is in plain view, and stays listed, with its
    // time, also while she is back inside quietly.
    ctx.join(&shy, &room.id).await;
    ctx.bring_on_stage(&host, &shy, &room.id).await;
    ctx.leave(&shy, &room.id).await;
    let listed = past().await;
    assert_eq!(listed.len(), 1);
    assert!(listed[0].left_at.is_set() && listed[0].joined_at.is_unset());
    assert_eq!(listed[0].role, SpaceRole::SPACE_ROLE_SPEAKER);
    ctx.join(&shy, &room.id).await;
    assert_eq!(past().await, listed, "coming back quietly changes nothing");
}

/// Which of the two listeners acts in a world, and who stays put.
fn cast(world: &World, shy_acts: bool) -> (&TestUser, &TestUser) {
    if shy_acts { (&world.shy, &world.quiet) } else { (&world.quiet, &world.shy) }
}

/// What a hand that went up in view can end with.
#[derive(Debug, Clone, Copy)]
enum HandEnd {
    Lowered,
    Dismissed,
    Left,
}

#[tokio::test]
async fn a_hand_raised_in_view_names_nobody_once_its_owner_hides() {
    let ctx = start().await;
    for end in [HandEnd::Lowered, HandEnd::Dismissed, HandEnd::Left] {
        let mut worlds = Vec::new();
        for shy_acts in [true, false] {
            // Both raise a hand while visible (the room sees that), then both hide.
            let mut world = World::with(&ctx, false).await;
            for user in [&world.shy, &world.quiet] {
                let _: SetHandRaisedResponse =
                    ctx.call("Space", "SetHandRaised", user, &hand(&world.room.id, true)).await.unwrap();
            }
            for user in [&world.shy, &world.quiet] {
                ctx.set_privacy(user, |privacy| privacy.share_online_status = false).await;
            }
            let before = world.observe(&ctx).await.0;
            worlds.push((world, shy_acts, before));
        }
        assert_eq!(worlds[0].2, worlds[1].2, "{end:?}: the worlds start out alike");

        let mut seen = Vec::new();
        for (world, shy_acts, _) in &mut worlds {
            let (actor, _) = cast(world, *shy_acts);
            let room = world.room.id.clone();
            match end {
                HandEnd::Lowered => {
                    let _: SetHandRaisedResponse =
                        ctx.call("Space", "SetHandRaised", actor, &hand(&room, false)).await.unwrap();
                }
                HandEnd::Dismissed => {
                    let dismiss =
                        MoveToAudienceRequest { space_id: room, user_id: actor.id.to_string(), ..Default::default() };
                    let _: MoveToAudienceResponse =
                        ctx.call("Space", "MoveToAudience", &world.host, &dismiss).await.unwrap();
                }
                HandEnd::Left => ctx.leave(actor, &room).await,
            }
        }
        for (world, _, _) in &mut worlds {
            seen.push(world.observe(&ctx).await);
        }
        assert_eq!(seen[0].0, seen[1].0, "{end:?}: the observer can tell who acted");
        assert!(
            seen[0].1.iter().all(|event| matches!(event, Push::Counts(_))),
            "{end:?}: the topic hears the count and nothing else: {:?}",
            seen[0].1
        );
    }
}

#[tokio::test]
async fn a_former_speaker_who_hides_again_leaves_unnoticed() {
    let ctx = start().await;
    let mut worlds = Vec::new();
    for shy_acts in [true, false] {
        let mut world = World::new(&ctx).await;
        // Shy came in first; Quiet comes on stage first.
        ctx.bring_on_stage(&world.host, &world.quiet, &world.room.id).await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        ctx.bring_on_stage(&world.host, &world.shy, &world.room.id).await;
        let stage: Vec<String> = ctx
            .get(&world.observer, &world.room.id)
            .await
            .stage
            .iter()
            .filter(|p| p.role == SpaceRole::SPACE_ROLE_SPEAKER)
            .map(|p| p.user.display_name.clone())
            .collect();
        assert_eq!(stage, ["quiet", "shy"], "the stage in the order people came on it, not in the order they arrived");
        for user in [&world.shy, &world.quiet] {
            let down = MoveToAudienceRequest { space_id: world.room.id.clone(), ..Default::default() };
            let _: MoveToAudienceResponse = ctx.call("Space", "MoveToAudience", user, &down).await.unwrap();
        }
        let before = world.observe(&ctx).await.0;
        worlds.push((world, shy_acts, before));
    }
    assert_eq!(worlds[0].2, worlds[1].2, "the worlds start out alike");

    // One of them leaves, the other stays: the observer cannot tell which.
    for (world, shy_acts, _) in &worlds {
        let (actor, _) = cast(world, *shy_acts);
        ctx.leave(actor, &world.room.id).await;
    }
    let mut seen = Vec::new();
    for (world, _, _) in &mut worlds {
        seen.push(world.observe(&ctx).await);
    }
    assert_eq!(seen[0].0, seen[1].0, "the observer can tell who left");
    // And it is not even visible that someone did, apart from the count.
    assert_eq!(worlds[0].2["lists"][4], seen[0].0["lists"][4], "the past list did not move");
    assert!(seen[0].1.iter().all(|event| matches!(event, Push::Counts(_))), "{:?}", seen[0].1);
}
