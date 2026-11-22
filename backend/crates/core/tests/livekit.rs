//! The room service against a real LiveKit server (`TOKULY_LIVEKIT_*`, as
//! exported by `tools/dev/infra.sh env`).
//!
//! Nobody connects to the rooms made here; what a connected participant
//! looks like is covered where a domain uses it (`tokuly-spaces`).
use tokuly_core::livekit::{DataOptions, LiveKit, ParticipantUpdate, Permission, REJECTED, RoomOptions, UNAVAILABLE};

fn setting(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} is not set; load the dev infrastructure settings: eval \"$(tools/dev/infra.sh env)\"")
    })
}

fn livekit() -> LiveKit {
    LiveKit::new(
        &setting("TOKULY_LIVEKIT_URL"),
        &setting("TOKULY_LIVEKIT_PUBLIC_URL"),
        &setting("TOKULY_LIVEKIT_API_KEY"),
        &setting("TOKULY_LIVEKIT_API_SECRET"),
    )
    .expect("LiveKit settings are valid")
}

/// A room name no other test (or test run) uses.
fn room_name() -> String {
    format!("tokuly-coretest-{}", uuid::Uuid::now_v7().simple())
}

#[tokio::test]
async fn a_room_is_created_listed_and_deleted() {
    let livekit = livekit();
    let name = room_name();

    let options = RoomOptions::named(&name)
        .empty_timeout(120)
        .departure_timeout(7)
        .max_participants(12)
        .metadata("{\"kind\":\"test\"}");
    let created = livekit.create_room(&options).await.expect("create room");
    assert_eq!(created.name, name);
    assert!(created.sid.starts_with("RM_"), "{created:?}");
    assert_eq!((created.empty_timeout, created.departure_timeout, created.max_participants), (120, 7, 12));
    assert_eq!(created.metadata, "{\"kind\":\"test\"}");
    assert_eq!(created.num_participants, 0);
    assert!(created.creation_time > 1_700_000_000, "{created:?}");

    // Creating it again returns the room that exists, settings untouched.
    let again = livekit.create_room(&RoomOptions::named(&name).max_participants(99)).await.expect("create again");
    assert_eq!((again.sid.as_str(), again.max_participants), (created.sid.as_str(), 12));

    let listed = livekit.list_rooms(std::slice::from_ref(&name)).await.expect("list rooms");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].sid, created.sid);
    let everything = livekit.list_rooms(&[]).await.expect("list every room");
    assert!(everything.iter().any(|room| room.name == name));

    assert!(livekit.list_participants(&name).await.expect("list participants").is_empty(), "nobody connected");

    assert!(livekit.delete_room(&name).await.expect("delete room"));
    assert!(livekit.list_rooms(std::slice::from_ref(&name)).await.unwrap().is_empty());
    assert!(!livekit.delete_room(&name).await.expect("deleting twice is fine"), "it was already gone");
    assert!(livekit.list_participants(&name).await.unwrap().is_empty(), "nobody is in a room that is gone");
}

#[tokio::test]
async fn someone_who_is_not_connected_is_not_an_error() {
    let livekit = livekit();
    let name = room_name();
    livekit.create_room(&RoomOptions::named(&name).empty_timeout(60)).await.expect("create room");

    assert!(livekit.get_participant(&name, "nobody").await.expect("get participant").is_none());
    assert!(!livekit.remove_participant(&name, "nobody").await.expect("remove participant"));
    let update = ParticipantUpdate {
        permission: Some(Permission { can_subscribe: true, ..Default::default() }),
        metadata: Some("{}".into()),
        name: None,
    };
    assert!(livekit.update_participant(&name, "nobody", &update).await.expect("update participant").is_none());
    // Nothing answers for a participant who is not there.
    let error = livekit.mute_track(&name, "nobody", "TR_none", true).await.unwrap_err();
    assert_eq!(error.reason, UNAVAILABLE, "{error}");
    // A message for an empty room is accepted and reaches nobody.
    livekit
        .send_data(&name, b"hello", &DataOptions { topic: "test".into(), ..Default::default() })
        .await
        .expect("send data");

    assert!(livekit.delete_room(&name).await.unwrap());
    let error = livekit.send_data(&name, b"hello", &DataOptions::default()).await.unwrap_err();
    assert_eq!(error.reason, UNAVAILABLE, "nothing answers for a room that is gone: {error}");
}

#[tokio::test]
async fn another_key_pair_is_turned_away() {
    let impostor = LiveKit::new(
        &setting("TOKULY_LIVEKIT_URL"),
        &setting("TOKULY_LIVEKIT_PUBLIC_URL"),
        &setting("TOKULY_LIVEKIT_API_KEY"),
        "not-the-secret-livekit-was-started-with",
    )
    .unwrap();
    let error = impostor.create_room(&RoomOptions::named(&room_name())).await.unwrap_err();
    assert_eq!(error.reason, REJECTED, "{error}");
    assert_eq!(error.code, connectrpc::ErrorCode::Unavailable);
    let error = impostor.list_participants("any").await.unwrap_err();
    assert_eq!(error.reason, REJECTED, "{error}");
}
