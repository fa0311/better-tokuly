//! Reactions to what happens in other domains: moderation takes a room
//! down, an account is suspended or erased, one person blocks another.
//! No relay runs in tests, so the handler is called with the event.
mod common;

use common::*;
use tokuly_core::events::Event;
use tokuly_proto::{
    common::SubjectType,
    spaces::{spaces_event::Event as Push, *},
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

fn event(topic: &str, data: serde_json::Value) -> Event {
    Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id: None, occurred_at: chrono::Utc::now(), data }
}

fn removed(subject_type: SubjectType, subject_id: &str) -> Event {
    event(
        "admin.content.removed",
        serde_json::json!({
            "subject_type": subject_type as i32,
            "subject_id": subject_id,
            "reason": "規約違反",
            "moderator_id": Uuid::now_v7(),
        }),
    )
}

fn account(topic: &str, user: &TestUser) -> Event {
    event(topic, serde_json::json!({ "user_id": user.id }))
}

fn space_request(space_id: &str) -> GetSpaceRequest {
    GetSpaceRequest { space_id: space_id.to_owned(), ..Default::default() }
}

fn hangout_request(hangout_id: &str) -> GetHangoutRequest {
    GetHangoutRequest { hangout_id: hangout_id.to_owned(), ..Default::default() }
}

#[tokio::test]
async fn moderation_takes_rooms_and_hangouts_down() {
    let ctx = start().await;
    let (host, friend) = ctx.friends("taro", "hanako").await;
    let (live, credentials) = ctx.open_room(&host, "よくない話", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    ctx.join(&friend, &live.id).await;
    let upcoming = ctx.schedule(&host, "よくない予定", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    let hangout = ctx.hangout(&host, "よくない場", &[&friend]).await;

    // Someone else's kind of content: not ours to touch.
    ctx.spaces.handle_event(&removed(SubjectType::SUBJECT_TYPE_NOTE, &live.id)).await.unwrap();
    assert_eq!(ctx.get(&friend, &live.id).await.state, SpaceState::SPACE_STATE_LIVE);

    let mut friend_events = ctx.listen(&friend).await;
    ctx.spaces.handle_event(&removed(SubjectType::SUBJECT_TYPE_SPACE, &live.id)).await.unwrap();
    let ended = ctx.get(&friend, &live.id).await;
    assert_eq!(ended.state, SpaceState::SPACE_STATE_ENDED);
    assert_eq!(ended.end_reason, SpaceEndReason::SPACE_END_REASON_MODERATION);
    assert!(ctx.events(&mut friend_events).await.iter().any(|event| matches!(event, Push::ParticipantLeft(_))));
    assert!(ctx.livekit().list_rooms(std::slice::from_ref(&credentials.room)).await.unwrap().is_empty());
    assert_eq!(ctx.app.events_by("spaces.room.ended", host.id).await.pop().unwrap().data["reason"].as_i64(), Some(4));

    ctx.spaces.handle_event(&removed(SubjectType::SUBJECT_TYPE_SPACE, &upcoming.id)).await.unwrap();
    let cancelled = ctx.get(&host, &upcoming.id).await;
    assert_eq!(cancelled.state, SpaceState::SPACE_STATE_CANCELLED);
    assert_eq!(cancelled.end_reason, SpaceEndReason::SPACE_END_REASON_MODERATION);

    let mut friend_events = ctx.listen(&friend).await;
    ctx.spaces.handle_event(&removed(SubjectType::SUBJECT_TYPE_SPACE, &hangout.id)).await.unwrap();
    assert_eq!(ctx.reason("Hangout", "GetHangout", &host, &hangout_request(&hangout.id)).await, "HANGOUT_NOT_FOUND");
    assert!(ctx.events(&mut friend_events).await.iter().any(|event| matches!(event, Push::HangoutRemoved(_))));

    // The event arriving again, or naming something unknown, is done.
    ctx.spaces.handle_event(&removed(SubjectType::SUBJECT_TYPE_SPACE, &live.id)).await.unwrap();
    ctx.spaces.handle_event(&removed(SubjectType::SUBJECT_TYPE_SPACE, &hangout.id)).await.unwrap();
    ctx.spaces.handle_event(&removed(SubjectType::SUBJECT_TYPE_SPACE, &Uuid::now_v7().to_string())).await.unwrap();
    // Topics the domain does not know are ignored.
    ctx.spaces.handle_event(&event("something.else.happened", serde_json::json!({}))).await.unwrap();
}

/// Suspends an account the way the admin domain does, and delivers the news.
async fn suspend(ctx: &Ctx, user: &TestUser) {
    ctx.app.identity.set_banned(user.id, Some("テスト")).await.expect("suspend");
    ctx.spaces.handle_event(&account("identity.user.banned", user)).await.unwrap();
}

async fn stored_end_reason(ctx: &Ctx, space_id: &str) -> i16 {
    sqlx::query_scalar("SELECT end_reason FROM spaces WHERE id = $1::uuid")
        .bind(space_id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_suspended_account_is_out_of_every_room() {
    let ctx = start().await;
    let (suspended, friend) = ctx.friends("taro", "hanako").await;
    let (own_room, _) = ctx.open_room(&suspended, "じぶんのルーム", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    ctx.join(&friend, &own_room.id).await;
    let upcoming = ctx.schedule(&suspended, "あしたの予定", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;

    // News of a suspension that is not (or no longer) in force changes nothing.
    ctx.spaces.handle_event(&account("identity.user.banned", &suspended)).await.unwrap();
    assert_eq!(ctx.get(&friend, &own_room.id).await.state, SpaceState::SPACE_STATE_LIVE);

    let mut room_events = ctx.listen_room(&own_room.id).await;
    suspend(&ctx, &suspended).await;
    let ended = ctx.get(&friend, &own_room.id).await;
    assert_eq!(ended.state, SpaceState::SPACE_STATE_ENDED);
    let cancelled = ctx.get(&friend, &upcoming.id).await;
    assert_eq!(cancelled.state, SpaceState::SPACE_STATE_CANCELLED);
    // Why the room ended is on record, but that an account was closed is
    // not for the listeners to be told: to them the host is simply gone.
    for room in [&own_room, &upcoming] {
        assert_eq!(stored_end_reason(&ctx, &room.id).await, 5);
    }
    assert_eq!(ended.end_reason, SpaceEndReason::SPACE_END_REASON_HOST_LEFT);
    assert_eq!(cancelled.end_reason, SpaceEndReason::SPACE_END_REASON_HOST_LEFT);
    let pushed = ctx.events(&mut room_events).await;
    assert!(pushed.iter().any(|event| matches!(
        event,
        Push::SpaceUpdated(updated)
            if updated.space.state == SpaceState::SPACE_STATE_ENDED
                && updated.space.end_reason == SpaceEndReason::SPACE_END_REASON_HOST_LEFT
    )));

    // As a guest somewhere else.
    let other_host = ctx.app.user("jiro").await;
    let guest = ctx.app.user("guest").await;
    let their_room = ctx.public_room(&other_host, "よそのルーム").await;
    ctx.join(&guest, &their_room.id).await;
    suspend(&ctx, &guest).await;
    assert_eq!(ctx.get(&other_host, &their_room.id).await.listener_count, 1);

    // In a hangout's voice room: out of it; the membership waits for them.
    let voice = ctx.app.user("voice").await;
    ctx.befriend(&friend, &voice).await;
    let hangout = ctx.hangout(&friend, "いつメン", &[&voice]).await;
    ctx.join_hangout(&voice, &hangout.id).await;
    suspend(&ctx, &voice).await;
    let now = ctx.get_hangout(&friend, &hangout.id).await;
    assert_eq!((now.inside_count, now.member_count), (0, 2));
}

#[tokio::test]
async fn an_erased_account_leaves_nothing_behind() {
    let ctx = start().await;
    let (erased, friend) = ctx.friends("taro", "hanako").await;
    let other = ctx.app.user("jiro").await;
    ctx.befriend(&erased, &other).await;
    ctx.befriend(&friend, &other).await;

    // What the account has: a live room, a past room, a place in someone's room,
    // an invitation it sent, a reminder, a hangout it owns, one it belongs to.
    let past = ctx.public_room(&erased, "むかしのルーム").await;
    let _: EndSpaceResponse = ctx
        .call("Space", "EndSpace", &erased, &EndSpaceRequest { space_id: past.id.clone(), ..Default::default() })
        .await
        .unwrap();
    let live = ctx.public_room(&erased, "いまのルーム").await;
    ctx.join(&friend, &live.id).await;
    let friends_upcoming = ctx.schedule(&friend, "ともだちの予定", SpaceAudience::SPACE_AUDIENCE_FRIENDS).await;
    let remind = SetReminderRequest { space_id: friends_upcoming.id.clone(), remind: true, ..Default::default() };
    let _: SetReminderResponse = ctx.call("Space", "SetReminder", &erased, &remind).await.unwrap();
    let invite =
        InviteToSpaceRequest { space_id: live.id.clone(), user_ids: vec![other.id.to_string()], ..Default::default() };
    let _: InviteToSpaceResponse = ctx.call("Space", "InviteToSpace", &erased, &invite).await.unwrap();
    let owned = ctx.hangout(&erased, "じぶんの場", &[&friend, &other]).await;
    let lonely = ctx.hangout(&erased, "ひとりの場", &[]).await;
    let belonging = ctx.hangout(&friend, "ともだちの場", &[&erased]).await;

    let mut friend_events = ctx.listen(&friend).await;
    ctx.spaces.handle_event(&account("identity.user.deleted", &erased)).await.unwrap();

    // Its rooms are gone, the live one after ending for everyone inside.
    assert!(ctx.events(&mut friend_events).await.iter().any(|event| matches!(
        event,
        Push::ParticipantLeft(left) if left.space_id == live.id && left.reason == SpaceLeaveReason::SPACE_LEAVE_REASON_ENDED
    )));
    for room in [&past, &live] {
        assert_eq!(ctx.reason("Space", "GetSpace", &friend, &space_request(&room.id)).await, "SPACE_NOT_FOUND");
    }
    assert_eq!(ctx.app.events_by("spaces.room.ended", erased.id).await.pop().unwrap().data["reason"].as_i64(), Some(5));
    // Its reminder no longer counts.
    assert_eq!(ctx.get(&friend, &friends_upcoming.id).await.reminder_count, 0);

    // The hangout it owned passes to who has belonged longest; the one nobody else was in closes.
    let inherited = ctx.get_hangout(&friend, &owned.id).await;
    assert_eq!(inherited.owner.id, friend.id.to_string());
    assert_eq!(inherited.member_count, 2);
    assert!(inherited.members.iter().all(|member| member.user.id != erased.id.to_string()));
    assert_eq!(ctx.reason("Hangout", "GetHangout", &friend, &hangout_request(&lonely.id)).await, "HANGOUT_NOT_FOUND");
    assert_eq!(ctx.get_hangout(&friend, &belonging.id).await.member_count, 1);

    // Nothing of the account is left in the domain's tables.
    let left: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM spaces WHERE host_id = $1)
              + (SELECT count(*) FROM space_participants WHERE user_id = $1)
              + (SELECT count(*) FROM space_invites WHERE user_id = $1 OR invited_by = $1)
              + (SELECT count(*) FROM space_reminders WHERE user_id = $1)
              + (SELECT count(*) FROM space_bans WHERE user_id = $1 OR banned_by = $1)
              + (SELECT count(*) FROM hangouts WHERE owner_id = $1)
              + (SELECT count(*) FROM hangout_members WHERE user_id = $1 OR invited_by = $1)
              + (SELECT count(*) FROM hangout_presence WHERE user_id = $1)",
    )
    .bind(erased.id)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(left, 0);
    // The event arriving twice is harmless.
    ctx.spaces.handle_event(&account("identity.user.deleted", &erased)).await.unwrap();
}

#[tokio::test]
async fn a_block_ends_what_the_two_shared() {
    let ctx = start().await;
    let (blocker, blocked) = ctx.friends("taro", "hanako").await;
    let third = ctx.app.user("jiro").await;
    ctx.befriend(&third, &blocker).await;
    ctx.befriend(&third, &blocked).await;

    // The blocked person waits for the blocker's upcoming room, invited and with a reminder.
    let request = CreateSpaceRequest {
        title: "もうすぐ".into(),
        scheduled_at: tokuly_core::time::ts(chrono::Utc::now() + chrono::Duration::hours(1)),
        invitee_ids: vec![blocked.id.to_string()],
        ..Default::default()
    };
    let created: CreateSpaceResponse = ctx.call("Space", "CreateSpace", &blocker, &request).await.unwrap();
    let upcoming = created.space.id.clone();
    let remind = SetReminderRequest { space_id: upcoming.clone(), remind: true, ..Default::default() };
    let _: SetReminderResponse = ctx.call("Space", "SetReminder", &blocked, &remind).await.unwrap();
    // They are in the blocker's hangout, and invited by the blocker to a third person's.
    let blockers_hangout = ctx.hangout(&blocker, "ブロックする人の場", &[&blocked, &third]).await;
    let thirds_hangout = ctx.hangout(&third, "三人目の場", &[&blocker]).await;
    let invite = InviteToHangoutRequest {
        hangout_id: thirds_hangout.id.clone(),
        user_ids: vec![blocked.id.to_string()],
        ..Default::default()
    };
    let _: InviteToHangoutResponse = ctx.call("Hangout", "InviteToHangout", &blocker, &invite).await.unwrap();
    // And both belong to another hangout of the third person.
    let shared = ctx.hangout(&third, "みんなの場", &[&blocker, &blocked]).await;

    let mut blocked_events = ctx.listen(&blocked).await;
    ctx.block(&blocker, &blocked).await;
    let event = ctx.app.events_by("social.user.blocked", blocker.id).await.pop().unwrap();
    ctx.spaces.handle_event(&event).await.unwrap();

    // The blocker's upcoming room is gone for them, and so is their invitation.
    assert_eq!(ctx.reason("Space", "GetSpace", &blocked, &space_request(&upcoming)).await, "SPACE_NOT_FOUND");
    let invites: ListSpaceInvitesResponse = ctx
        .call(
            "Space",
            "ListSpaceInvites",
            &blocker,
            &ListSpaceInvitesRequest { space_id: upcoming.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(invites.invites.is_empty());
    // The reminder is left alone — a count that moved with a block would
    // answer "did they set one?" — and simply never fires.
    assert_eq!(ctx.get(&blocker, &upcoming).await.reminder_count, 1);
    let start = StartSpaceRequest { space_id: upcoming.clone(), ..Default::default() };
    let _: StartSpaceResponse = ctx.call("Space", "StartSpace", &blocker, &start).await.unwrap();
    let started = ctx.app.events_by("spaces.room.started", blocker.id).await.pop().unwrap();
    ctx.spaces.handle_event(&started).await.unwrap();
    assert!(
        ctx.notifications(&blocked).await.iter().all(|n| n.kind != "spaces.reminder" && n.kind != "spaces.started")
    );

    // Out of the blocker's hangout, and the blocker's invitation elsewhere is withdrawn.
    assert_eq!(
        ctx.reason("Hangout", "GetHangout", &blocked, &hangout_request(&blockers_hangout.id)).await,
        "HANGOUT_NOT_FOUND"
    );
    assert_eq!(ctx.get_hangout(&blocker, &blockers_hangout.id).await.member_count, 2);
    assert_eq!(
        ctx.reason("Hangout", "GetHangout", &blocked, &hangout_request(&thirds_hangout.id)).await,
        "HANGOUT_NOT_FOUND"
    );
    let removals =
        ctx.events(&mut blocked_events).await.iter().filter(|event| matches!(event, Push::HangoutRemoved(_))).count();
    assert_eq!(removals, 2);
    // A third person's hangout keeps them both.
    assert_eq!(ctx.get_hangout(&third, &shared.id).await.member_count, 3);

    // Neither can invite the other any more: they are not friends.
    let again = InviteToHangoutRequest {
        hangout_id: blockers_hangout.id.clone(),
        user_ids: vec![blocked.id.to_string()],
        ..Default::default()
    };
    assert_eq!(ctx.reason("Hangout", "InviteToHangout", &blocker, &again).await, "NOT_FRIENDS");
    // A third person cannot bring the blocked one into the blocker's
    // hangout either — and is not told why: an error here would let them
    // ask "are these two in a block?" about any two of their friends.
    let quietly: InviteToHangoutResponse = ctx.call("Hangout", "InviteToHangout", &third, &again).await.unwrap();
    assert_eq!(quietly.invited_count, 0);
    assert_eq!(
        ctx.reason("Hangout", "GetHangout", &blocked, &hangout_request(&blockers_hangout.id)).await,
        "HANGOUT_NOT_FOUND"
    );
    // The same for the blocker's room.
    let to_room = InviteToSpaceRequest {
        space_id: upcoming.clone(),
        user_ids: vec![blocked.id.to_string()],
        ..Default::default()
    };
    ctx.join(&third, &upcoming).await;
    ctx.make_cohost(&blocker, &third, &upcoming).await;
    let quietly: InviteToSpaceResponse = ctx.call("Space", "InviteToSpace", &third, &to_room).await.unwrap();
    assert_eq!(quietly.invited_count, 0);

    // The event arriving twice is harmless.
    ctx.spaces.handle_event(&event).await.unwrap();
}

#[tokio::test]
async fn a_guest_who_blocks_the_host_just_walks_away() {
    let ctx = start().await;
    let (host, guest) = ctx.friends("taro", "hanako").await;
    // The guest is inside the host's room on an invitation, with a seat in
    // the host's hangout.
    let request = CreateSpaceRequest {
        title: "ホストのルーム".into(),
        invitee_ids: vec![guest.id.to_string()],
        ..Default::default()
    };
    let created: CreateSpaceResponse = ctx.call("Space", "CreateSpace", &host, &request).await.unwrap();
    let room = created.space.id.clone();
    ctx.join(&guest, &room).await;
    let hangout = ctx.hangout(&host, "ホストの場", &[&guest]).await;

    let mut guest_events = ctx.listen(&guest).await;
    let mut host_events = ctx.listen(&host).await;
    ctx.block(&guest, &host).await;
    let event = ctx.app.events_by("social.user.blocked", guest.id).await.pop().unwrap();
    ctx.spaces.handle_event(&event).await.unwrap();

    // To the host it looks like the guest left the room and quit the
    // hangout. Blocks are silent: nothing else moves.
    assert_eq!(ctx.get(&host, &room).await.listener_count, 1);
    let reason: i16 =
        sqlx::query_scalar("SELECT leave_reason FROM space_participants WHERE space_id = $1::uuid AND user_id = $2")
            .bind(&room)
            .bind(guest.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(reason, 1, "left, not removed");
    assert!(
        !ctx.events(&mut guest_events).await.iter().any(|event| matches!(event, Push::ParticipantLeft(_))),
        "nobody removed them"
    );
    let invites: ListSpaceInvitesResponse = ctx
        .call(
            "Space",
            "ListSpaceInvites",
            &host,
            &ListSpaceInvitesRequest { space_id: room.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(invites.invites.len(), 1, "the invitation the host sent is still on the host's list");
    assert_eq!(ctx.get_hangout(&host, &hangout.id).await.member_count, 1);
    assert!(ctx.events(&mut host_events).await.iter().any(|event| matches!(
        event,
        Push::HangoutUpdated(updated) if updated.hangout.member_count == 1
    )));
    // For the guest, the host's room is gone all the same.
    assert_eq!(ctx.reason("Space", "JoinSpace", &guest, &space_request(&room)).await, "SPACE_NOT_FOUND");
}

#[tokio::test]
async fn late_news_of_a_block_that_was_lifted_changes_nothing() {
    let ctx = start().await;
    let host = ctx.app.user("taro").await;
    let listener = ctx.app.user("hanako").await;
    let room = ctx.public_room(&host, "仲直り").await;
    ctx.block(&host, &listener).await;
    let event = ctx.app.events_by("social.user.blocked", host.id).await.pop().unwrap();
    let unblock = tokuly_proto::social::UnblockUserRequest { user_id: listener.id.to_string(), ..Default::default() };
    let _: serde_json::Value =
        ctx.app.call("tokuly.social.v1.FriendService/UnblockUser", Some(&host), &unblock).await.expect("unblock");
    ctx.join(&listener, &room.id).await;

    // The event about the block arrives only now.
    ctx.spaces.handle_event(&event).await.unwrap();
    assert_eq!(ctx.get(&host, &room.id).await.listener_count, 2, "nobody is thrown out over a block that is over");
}

#[tokio::test]
async fn a_blocked_cohost_loses_the_role_kept_for_later() {
    let ctx = start().await;
    let (host, deputy) = ctx.friends("taro", "hanako").await;
    let room = ctx.public_room(&host, "番組").await;
    ctx.join(&deputy, &room.id).await;
    ctx.make_cohost(&host, &deputy, &room.id).await;
    ctx.leave(&deputy, &room.id).await;
    assert_eq!(ctx.get(&deputy, &room.id).await.viewer.role, SpaceRole::SPACE_ROLE_COHOST);

    ctx.block(&host, &deputy).await;
    let event = ctx.app.events_by("social.user.blocked", host.id).await.pop().unwrap();
    ctx.spaces.handle_event(&event).await.unwrap();
    let role: i16 =
        sqlx::query_scalar("SELECT role FROM space_participants WHERE space_id = $1::uuid AND user_id = $2")
            .bind(&room.id)
            .bind(deputy.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(role, 1, "an ordinary listener, should the block ever be lifted");
}
