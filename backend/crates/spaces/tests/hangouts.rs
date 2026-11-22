//! Hangouts (たまり場): the group — invitations, members, the owner — and
//! the voice room that is open whenever somebody is inside.
mod common;

use common::*;
use tokuly_proto::spaces::{spaces_event::Event as Push, *};
use tokuly_spaces::{SpacesConfig, Venue};
use tokuly_testkit::TestUser;
use uuid::Uuid;

fn id_request(hangout_id: &str) -> GetHangoutRequest {
    GetHangoutRequest { hangout_id: hangout_id.to_owned(), ..Default::default() }
}

fn invite(hangout_id: &str, users: &[&TestUser]) -> InviteToHangoutRequest {
    InviteToHangoutRequest {
        hangout_id: hangout_id.to_owned(),
        user_ids: users.iter().map(|user| user.id.to_string()).collect(),
        ..Default::default()
    }
}

fn respond(hangout_id: &str, accept: bool) -> RespondToHangoutInviteRequest {
    RespondToHangoutInviteRequest { hangout_id: hangout_id.to_owned(), accept, ..Default::default() }
}

fn remove(hangout_id: &str, user: &TestUser) -> RemoveHangoutMemberRequest {
    RemoveHangoutMemberRequest { hangout_id: hangout_id.to_owned(), user_id: user.id.to_string(), ..Default::default() }
}

fn member_names(hangout: &Hangout) -> Vec<&str> {
    hangout.members.iter().map(|member| member.user.display_name.as_str()).collect()
}

async fn listed(ctx: &Ctx, user: &TestUser) -> ListHangoutsResponse {
    ctx.call("Hangout", "ListHangouts", user, &ListHangoutsRequest::default()).await.expect("list hangouts")
}

#[tokio::test]
async fn a_hangout_is_created_and_friends_are_invited() {
    let ctx = start().await;
    let owner = ctx.app.user("taro").await;
    let (first, second) = (ctx.app.user("hanako").await, ctx.app.user("jiro").await);
    ctx.befriend(&owner, &first).await;
    ctx.befriend(&owner, &second).await;
    let stranger = ctx.app.user("shiro").await;
    let create = |change: fn(&mut CreateHangoutRequest)| {
        let mut request = CreateHangoutRequest { name: "いつメン".into(), ..Default::default() };
        change(&mut request);
        request
    };

    assert_eq!(ctx.reason("Hangout", "CreateHangout", &owner, &create(|r| r.name = "  ".into())).await, "INVALID_NAME");
    assert_eq!(
        ctx.reason("Hangout", "CreateHangout", &owner, &create(|r| r.name = "あ".repeat(41))).await,
        "INVALID_NAME"
    );
    assert_eq!(
        ctx.reason("Hangout", "CreateHangout", &owner, &create(|r| r.emoji = "abc".into())).await,
        "INVALID_EMOJI"
    );
    let mut with_stranger = create(|_| {});
    with_stranger.invitee_ids = vec![stranger.id.to_string()];
    assert_eq!(ctx.reason("Hangout", "CreateHangout", &owner, &with_stranger).await, "NOT_FRIENDS");
    assert_eq!(ctx.anonymous_reason("Hangout", "CreateHangout", &create(|_| {})).await, "SIGN_IN_REQUIRED");

    let mut first_events = ctx.listen(&first).await;
    let mut request = create(|r| r.members_can_invite = true);
    request.invitee_ids = vec![first.id.to_string(), second.id.to_string()];
    let created: CreateHangoutResponse = ctx.call("Hangout", "CreateHangout", &owner, &request).await.unwrap();
    let hangout = created.hangout.as_option().unwrap();
    assert_eq!((hangout.name.as_str(), hangout.emoji.as_str()), ("いつメン", "🛋️"));
    assert_eq!(hangout.owner.id, owner.id.to_string());
    assert_eq!((hangout.member_count, hangout.inside_count), (1, 0), "invitations are not members yet");
    assert_eq!(member_names(hangout)[0], "taro");
    assert_eq!(hangout.members.len(), 3);
    assert!(hangout.members[0].owner && hangout.members[0].state == HangoutMemberState::HANGOUT_MEMBER_STATE_MEMBER);
    assert!(hangout.members[1..].iter().all(|member| {
        member.state == HangoutMemberState::HANGOUT_MEMBER_STATE_INVITED && member.invited_by_id == owner.id.to_string()
    }));
    let viewer = hangout.viewer.as_option().unwrap();
    assert!(viewer.owner && viewer.can_invite && !viewer.inside && !viewer.muted);
    assert_eq!(viewer.state, HangoutMemberState::HANGOUT_MEMBER_STATE_MEMBER);

    // The invitation: a notification, a realtime event, an entry under "invitations".
    let notification = ctx.notifications(&first).await.pop().unwrap();
    assert_eq!((notification.kind.as_str(), notification.actor_id), ("spaces.invited", Some(owner.id)));
    assert_eq!(notification.title.ja, "taroがたまり場に招待したよ!");
    assert_eq!(notification.data["hangout_id"], hangout.id);
    assert_eq!(notification.path, format!("/hangouts/{}", hangout.id));
    assert!(ctx.events(&mut first_events).await.iter().any(|event| matches!(
        event,
        Push::HangoutInvitation(invitation) if invitation.hangout.id == hangout.id && invitation.invited_by.id == owner.id.to_string()
    )));
    let waiting = listed(&ctx, &first).await;
    assert!(waiting.hangouts.is_empty());
    assert_eq!(waiting.invitations.len(), 1);
    // Someone who is only invited sees who they would be joining, not who else was asked.
    assert_eq!(member_names(&waiting.invitations[0]), ["taro"]);
    assert_eq!(waiting.invitations[0].viewer.state, HangoutMemberState::HANGOUT_MEMBER_STATE_INVITED);
    assert!(!waiting.invitations[0].viewer.can_invite);

    // Not a member yet: no voice, no invitations of their own.
    let join = JoinHangoutRequest { hangout_id: hangout.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Hangout", "JoinHangout", &first, &join).await, "NOT_A_MEMBER");
    assert_eq!(
        ctx.reason("Hangout", "InviteToHangout", &first, &invite(&hangout.id, &[])).await,
        "INVITES_NOT_ALLOWED"
    );
    // Strangers do not know it exists.
    assert_eq!(ctx.reason("Hangout", "GetHangout", &stranger, &id_request(&hangout.id)).await, "HANGOUT_NOT_FOUND");
    assert_eq!(
        ctx.reason("Hangout", "RespondToHangoutInvite", &stranger, &respond(&hangout.id, true)).await,
        "HANGOUT_NOT_FOUND"
    );
    assert_eq!(ctx.reason("Hangout", "JoinHangout", &stranger, &join).await, "HANGOUT_NOT_FOUND");

    // Accepting makes a member; the others see it happen.
    let mut owner_events = ctx.listen(&owner).await;
    let accepted: RespondToHangoutInviteResponse =
        ctx.call("Hangout", "RespondToHangoutInvite", &first, &respond(&hangout.id, true)).await.unwrap();
    let joined = accepted.hangout.as_option().unwrap();
    assert_eq!(joined.member_count, 2);
    assert_eq!(joined.viewer.state, HangoutMemberState::HANGOUT_MEMBER_STATE_MEMBER);
    assert!(joined.viewer.can_invite, "members may invite here");
    assert!(ctx.events(&mut owner_events).await.iter().any(|event| matches!(
        event,
        Push::HangoutUpdated(updated) if updated.hangout.member_count == 2
    )));
    assert_eq!(
        ctx.reason("Hangout", "RespondToHangoutInvite", &first, &respond(&hangout.id, true)).await,
        "NO_INVITATION"
    );

    // Declining leaves no trace.
    let declined: RespondToHangoutInviteResponse =
        ctx.call("Hangout", "RespondToHangoutInvite", &second, &respond(&hangout.id, false)).await.unwrap();
    assert!(declined.hangout.is_unset());
    assert_eq!(ctx.reason("Hangout", "GetHangout", &second, &id_request(&hangout.id)).await, "HANGOUT_NOT_FOUND");
    let now = ctx.get_hangout(&owner, &hangout.id).await;
    assert_eq!(member_names(&now), ["taro", "hanako"]);
    let mine = listed(&ctx, &first).await;
    assert_eq!((mine.hangouts.len(), mine.invitations.len()), (1, 0));
}

#[tokio::test]
async fn the_group_is_managed_by_its_owner_and_members() {
    let ctx = start().await;
    let owner = ctx.app.user("taro").await;
    let (member, other) = (ctx.app.user("hanako").await, ctx.app.user("jiro").await);
    let members_friend = ctx.app.user("saburo").await;
    ctx.befriend(&owner, &member).await;
    ctx.befriend(&owner, &other).await;
    ctx.befriend(&member, &members_friend).await;
    let stranger = ctx.app.user("shiro").await;

    // Created without letting members invite.
    let request = CreateHangoutRequest {
        name: "部室".into(),
        emoji: "🎸".into(),
        invitee_ids: vec![member.id.to_string(), other.id.to_string()],
        ..Default::default()
    };
    let created: CreateHangoutResponse = ctx.call("Hangout", "CreateHangout", &owner, &request).await.unwrap();
    let hangout_id = created.hangout.id.clone();
    for user in [&member, &other] {
        let _: RespondToHangoutInviteResponse =
            ctx.call("Hangout", "RespondToHangoutInvite", user, &respond(&hangout_id, true)).await.unwrap();
    }
    assert_eq!(
        ctx.reason("Hangout", "InviteToHangout", &member, &invite(&hangout_id, &[&members_friend])).await,
        "INVITES_NOT_ALLOWED"
    );

    // The owner changes the hangout.
    let update = |change: fn(&mut UpdateHangoutRequest)| {
        let mut request = UpdateHangoutRequest { hangout_id: hangout_id.clone(), ..Default::default() };
        change(&mut request);
        request
    };
    assert_eq!(
        ctx.reason("Hangout", "UpdateHangout", &member, &update(|r| r.name = Some("のっとり".into()))).await,
        "NOT_OWNER"
    );
    assert_eq!(
        ctx.reason("Hangout", "UpdateHangout", &owner, &update(|r| r.name = Some(String::new()))).await,
        "INVALID_NAME"
    );
    assert_eq!(
        ctx.reason("Hangout", "UpdateHangout", &owner, &update(|r| r.emoji = Some("🎸🎸".into()))).await,
        "INVALID_EMOJI"
    );
    assert_eq!(
        ctx.reason("Hangout", "UpdateHangout", &stranger, &update(|r| r.name = Some("x".into()))).await,
        "HANGOUT_NOT_FOUND"
    );
    let mut member_events = ctx.listen(&member).await;
    let updated: UpdateHangoutResponse = ctx
        .call(
            "Hangout",
            "UpdateHangout",
            &owner,
            &update(|r| {
                r.name = Some("軽音部".into());
                r.members_can_invite = Some(true);
            }),
        )
        .await
        .unwrap();
    assert_eq!((updated.hangout.name.as_str(), updated.hangout.emoji.as_str()), ("軽音部", "🎸"));
    assert!(updated.hangout.members_can_invite);
    assert!(ctx.events(&mut member_events).await.iter().any(|event| matches!(
        event,
        Push::HangoutUpdated(updated) if updated.hangout.name == "軽音部"
    )));

    // Now a member invites their own friend — and only their own friends.
    assert_eq!(
        ctx.reason("Hangout", "InviteToHangout", &member, &invite(&hangout_id, &[&stranger])).await,
        "NOT_FRIENDS"
    );
    let invited: InviteToHangoutResponse =
        ctx.call("Hangout", "InviteToHangout", &member, &invite(&hangout_id, &[&members_friend])).await.unwrap();
    assert_eq!(invited.invited_count, 1);
    assert_eq!(invited.hangout.members.len(), 4);
    let again: InviteToHangoutResponse =
        ctx.call("Hangout", "InviteToHangout", &member, &invite(&hangout_id, &[&members_friend])).await.unwrap();
    assert_eq!(again.invited_count, 0);
    assert_eq!(ctx.notifications(&members_friend).await.len(), 1);

    // Taking back an invitation is for whoever sent it, and for the owner.
    assert_eq!(
        ctx.reason("Hangout", "RemoveHangoutMember", &other, &remove(&hangout_id, &members_friend)).await,
        "NOT_OWNER"
    );
    let mut invitee_events = ctx.listen(&members_friend).await;
    let _: RemoveHangoutMemberResponse =
        ctx.call("Hangout", "RemoveHangoutMember", &member, &remove(&hangout_id, &members_friend)).await.unwrap();
    assert!(ctx.events(&mut invitee_events).await.iter().any(|event| matches!(
        event,
        Push::HangoutRemoved(removed) if removed.hangout_id == hangout_id
    )));
    assert_eq!(
        ctx.reason("Hangout", "GetHangout", &members_friend, &id_request(&hangout_id)).await,
        "HANGOUT_NOT_FOUND"
    );

    // Removing members is the owner's.
    assert_eq!(ctx.reason("Hangout", "RemoveHangoutMember", &member, &remove(&hangout_id, &other)).await, "NOT_OWNER");
    assert_eq!(
        ctx.reason("Hangout", "RemoveHangoutMember", &owner, &remove(&hangout_id, &owner)).await,
        "INVALID_USER"
    );
    assert_eq!(
        ctx.reason("Hangout", "RemoveHangoutMember", &owner, &remove(&hangout_id, &stranger)).await,
        "TARGET_NOT_A_MEMBER"
    );
    assert_eq!(
        ctx.reason("Hangout", "RemoveHangoutMember", &stranger, &remove(&hangout_id, &other)).await,
        "HANGOUT_NOT_FOUND"
    );
    let removed: RemoveHangoutMemberResponse =
        ctx.call("Hangout", "RemoveHangoutMember", &owner, &remove(&hangout_id, &other)).await.unwrap();
    assert_eq!(removed.hangout.member_count, 2);
    assert_eq!(ctx.reason("Hangout", "GetHangout", &other, &id_request(&hangout_id)).await, "HANGOUT_NOT_FOUND");
    // Being removed (or uninvited) does not take away the right to report the hangout.
    let subjects = &ctx.app.core.subjects;
    let subject_type = tokuly_proto::common::SubjectType::SUBJECT_TYPE_SPACE;
    for former in [&other, &members_friend] {
        let subject = subjects.resolve(subject_type, &hangout_id, Some(former.id)).await.expect("still reportable");
        assert_eq!(subject.owner_id, owner.id);
    }
    assert!(subjects.resolve(subject_type, &hangout_id, Some(stranger.id)).await.is_err());

    // Each member decides for themselves whether the hangout may notify them.
    let mute = SetHangoutMutedRequest { hangout_id: hangout_id.clone(), muted: true, ..Default::default() };
    let _: SetHangoutMutedResponse = ctx.call("Hangout", "SetHangoutMuted", &member, &mute).await.unwrap();
    assert!(ctx.get_hangout(&member, &hangout_id).await.viewer.muted);
    assert!(!ctx.get_hangout(&owner, &hangout_id).await.viewer.muted);
    assert_eq!(ctx.reason("Hangout", "SetHangoutMuted", &stranger, &mute).await, "HANGOUT_NOT_FOUND");

    // The owner cannot just walk away: the hangout is handed over first.
    let quit = QuitHangoutRequest { hangout_id: hangout_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Hangout", "QuitHangout", &owner, &quit).await, "OWNER_CANNOT_QUIT");
    let transfer = |user: &TestUser| TransferHangoutRequest {
        hangout_id: hangout_id.clone(),
        user_id: user.id.to_string(),
        ..Default::default()
    };
    assert_eq!(ctx.reason("Hangout", "TransferHangout", &member, &transfer(&member)).await, "NOT_OWNER");
    assert_eq!(ctx.reason("Hangout", "TransferHangout", &owner, &transfer(&owner)).await, "INVALID_USER");
    assert_eq!(ctx.reason("Hangout", "TransferHangout", &owner, &transfer(&stranger)).await, "TARGET_NOT_A_MEMBER");
    let handed: TransferHangoutResponse =
        ctx.call("Hangout", "TransferHangout", &owner, &transfer(&member)).await.unwrap();
    assert_eq!(handed.hangout.owner.id, member.id.to_string());
    assert!(!handed.hangout.viewer.owner);
    let _: QuitHangoutResponse = ctx.call("Hangout", "QuitHangout", &owner, &quit).await.unwrap();
    assert_eq!(ctx.reason("Hangout", "GetHangout", &owner, &id_request(&hangout_id)).await, "HANGOUT_NOT_FOUND");
    assert_eq!(ctx.reason("Hangout", "QuitHangout", &owner, &quit).await, "HANGOUT_NOT_FOUND");

    // Deleting is the (new) owner's.
    ctx.befriend(&member, &other).await;
    let _: InviteToHangoutResponse =
        ctx.call("Hangout", "InviteToHangout", &member, &invite(&hangout_id, &[&other])).await.unwrap();
    let _: RespondToHangoutInviteResponse =
        ctx.call("Hangout", "RespondToHangoutInvite", &other, &respond(&hangout_id, true)).await.unwrap();
    let delete = DeleteHangoutRequest { hangout_id: hangout_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Hangout", "DeleteHangout", &other, &delete).await, "NOT_OWNER");
    let mut other_events = ctx.listen(&other).await;
    let _: DeleteHangoutResponse = ctx.call("Hangout", "DeleteHangout", &member, &delete).await.unwrap();
    assert!(ctx.events(&mut other_events).await.iter().any(|event| matches!(event, Push::HangoutRemoved(_))));
    assert_eq!(ctx.reason("Hangout", "GetHangout", &member, &id_request(&hangout_id)).await, "HANGOUT_NOT_FOUND");
    assert!(listed(&ctx, &other).await.hangouts.is_empty());
    // Closed, but not beyond a report by someone who was in it.
    assert!(subjects.resolve(subject_type, &hangout_id, Some(other.id)).await.is_ok());
}

#[tokio::test]
async fn hangouts_have_limits() {
    let ctx = start_with(SpacesConfig { max_hangouts: 1, max_hangout_members: 3, ..Default::default() }).await;
    let owner = ctx.app.user("taro").await;
    let friends = [ctx.app.user("a").await, ctx.app.user("b").await, ctx.app.user("c").await];
    for friend in &friends {
        ctx.befriend(&owner, friend).await;
    }
    let hangout = ctx.hangout(&owner, "少人数", &[&friends[0]]).await;
    let second = CreateHangoutRequest { name: "二つ目".into(), ..Default::default() };
    assert_eq!(ctx.reason("Hangout", "CreateHangout", &owner, &second).await, "TOO_MANY_HANGOUTS");

    // Three people at most, open invitations included.
    assert_eq!(
        ctx.reason("Hangout", "InviteToHangout", &owner, &invite(&hangout.id, &[&friends[1], &friends[2]])).await,
        "HANGOUT_FULL"
    );
    let invited: InviteToHangoutResponse =
        ctx.call("Hangout", "InviteToHangout", &owner, &invite(&hangout.id, &[&friends[1]])).await.unwrap();
    assert_eq!(invited.invited_count, 1);
    assert_eq!(
        ctx.reason("Hangout", "InviteToHangout", &owner, &invite(&hangout.id, &[&friends[2]])).await,
        "HANGOUT_FULL"
    );
    // The hangout cannot be handed to someone who owns as many as they may.
    let _: CreateHangoutResponse = ctx.call("Hangout", "CreateHangout", &friends[0], &second).await.unwrap();
    let transfer = TransferHangoutRequest {
        hangout_id: hangout.id.clone(),
        user_id: friends[0].id.to_string(),
        ..Default::default()
    };
    assert_eq!(ctx.reason("Hangout", "TransferHangout", &owner, &transfer).await, "TOO_MANY_HANGOUTS");
}

#[tokio::test]
async fn the_voice_room_shows_who_is_inside() {
    let ctx = start().await;
    let owner = ctx.app.user("taro").await;
    let (muted, eager) = (ctx.app.user("hanako").await, ctx.app.user("jiro").await);
    ctx.befriend(&owner, &muted).await;
    ctx.befriend(&owner, &eager).await;
    let hangout = ctx.hangout(&owner, "いつメン", &[&muted, &eager]).await;
    let mute = SetHangoutMutedRequest { hangout_id: hangout.id.clone(), muted: true, ..Default::default() };
    let _: SetHangoutMutedResponse = ctx.call("Hangout", "SetHangoutMuted", &muted, &mute).await.unwrap();

    // The first one in opens the hangout.
    let mut eager_events = ctx.listen(&eager).await;
    let joined = ctx.join_hangout(&owner, &hangout.id).await;
    assert_eq!(joined.hangout.inside_count, 1);
    assert!(joined.hangout.viewer.inside);
    assert!(joined.hangout.last_active_at.is_set());
    // Everyone inside may talk.
    let credentials = joined.credentials.as_option().unwrap();
    assert!(credentials.can_publish);
    assert_eq!(credentials.room, format!("tokuly-hangout-{}", Uuid::parse_str(&hangout.id).unwrap().simple()));
    let claims = ctx.claims(credentials);
    assert_eq!(claims.sub, owner.id.to_string());
    assert_eq!((claims.video.can_publish, claims.video.can_subscribe), (Some(true), Some(true)));
    assert_eq!(claims.video.can_publish_sources, ["microphone"]);
    assert!(!claims.video.room_admin);

    // The members see it at once…
    assert!(ctx.events(&mut eager_events).await.iter().any(|event| matches!(
        event,
        Push::HangoutPresence(presence)
            if presence.hangout_id == hangout.id && presence.inside && presence.inside_count == 1 && presence.user.id == owner.id.to_string()
    )));
    // …and friends among them are told, unless they silenced the hangout.
    let told = ctx.notifications(&eager).await;
    assert_eq!(told.iter().map(|n| n.kind.as_str()).collect::<Vec<_>>(), ["spaces.invited", "spaces.hangout_joined"]);
    let notification = told.last().unwrap();
    assert_eq!(notification.title.ja, "taroがたまり場にいるよ!");
    assert_eq!(notification.actor_id, Some(owner.id), "so that muting the friend silences it too");
    assert!(!notification.urgent, "quiet hours apply");
    assert_eq!(notification.collapse_key.as_deref(), Some(format!("spaces:hangout:{}", hangout.id).as_str()));
    assert_eq!(ctx.notifications(&muted).await.iter().filter(|n| n.kind == "spaces.hangout_joined").count(), 0);
    let event = ctx.app.events_by("spaces.hangout.joined", owner.id).await.pop().unwrap();
    assert_eq!((event.data["first"].as_bool(), event.data["inside_count"].as_i64()), (Some(true), Some(1)));

    // The second one in is no news for the notifications, but shows on the list.
    let _ = ctx.join_hangout(&eager, &hangout.id).await;
    assert_eq!(ctx.notifications(&owner).await.iter().filter(|n| n.kind == "spaces.hangout_joined").count(), 0);
    let seen = listed(&ctx, &muted).await;
    assert_eq!(seen.hangouts[0].inside_count, 2);
    let inside: Vec<&str> = seen.hangouts[0]
        .members
        .iter()
        .filter(|member| member.inside)
        .map(|member| member.user.display_name.as_str())
        .collect();
    assert_eq!(inside, ["taro", "jiro"], "who came first leads");
    assert!(seen.hangouts[0].members[2].inside_since.is_unset());
    // Coming in again (a reconnect) counts nobody twice.
    assert_eq!(ctx.join_hangout(&eager, &hangout.id).await.hangout.inside_count, 2);

    // Friends see each other in there, for friend lists and the map.
    let friends: ListFriendsInRoomsResponse =
        ctx.call("Space", "ListFriendsInRooms", &muted, &ListFriendsInRoomsRequest::default()).await.unwrap();
    assert_eq!(friends.friends.len(), 1, "hanako and jiro are not friends");
    assert_eq!(friends.friends[0].user.id, owner.id.to_string());
    assert_eq!(
        (friends.friends[0].hangout_id.as_str(), friends.friends[0].title.as_str()),
        (hangout.id.as_str(), "いつメン")
    );
    assert!(friends.friends[0].space_id.is_empty());

    // Leaving: the membership stays, the presence goes.
    let leave = LeaveHangoutRequest { hangout_id: hangout.id.clone(), ..Default::default() };
    let mut muted_events = ctx.listen(&muted).await;
    let _: LeaveHangoutResponse = ctx.call("Hangout", "LeaveHangout", &owner, &leave).await.unwrap();
    assert!(ctx.events(&mut muted_events).await.iter().any(|event| matches!(
        event,
        Push::HangoutPresence(presence) if !presence.inside && presence.inside_count == 1 && presence.user.id == owner.id.to_string()
    )));
    let _: LeaveHangoutResponse = ctx.call("Hangout", "LeaveHangout", &owner, &leave).await.unwrap();
    let _: LeaveHangoutResponse = ctx.call("Hangout", "LeaveHangout", &eager, &leave).await.unwrap();
    let empty = ctx.get_hangout(&owner, &hangout.id).await;
    assert_eq!((empty.inside_count, empty.member_count), (0, 3));
    let stranger = ctx.app.user("shiro").await;
    assert_eq!(ctx.reason("Hangout", "LeaveHangout", &stranger, &leave).await, "HANGOUT_NOT_FOUND");

    // Opening it again a moment later does not ring everyone's phone again.
    let _ = ctx.join_hangout(&owner, &hangout.id).await;
    assert_eq!(ctx.notifications(&eager).await.iter().filter(|n| n.kind == "spaces.hangout_joined").count(), 1);
    assert_eq!(ctx.app.events_by("spaces.hangout.joined", owner.id).await.len(), 2);
}

#[tokio::test]
async fn a_member_who_is_removed_is_disconnected() {
    let ctx = start().await;
    let (owner, member) = ctx.friends("taro", "hanako").await;
    let hangout = ctx.hangout(&owner, "部室", &[&member]).await;
    let credentials = ctx.join_hangout(&member, &hangout.id).await.credentials.as_option().unwrap().clone();
    let _connection = ctx.connect(&credentials).await;
    let connected = ctx.connected(&credentials.room, &member.id.to_string()).await.unwrap();
    assert!(connected.permission.unwrap().can_publish);

    let mut owner_events = ctx.listen(&owner).await;
    let removed: RemoveHangoutMemberResponse =
        ctx.call("Hangout", "RemoveHangoutMember", &owner, &remove(&hangout.id, &member)).await.unwrap();
    assert_eq!((removed.hangout.member_count, removed.hangout.inside_count), (1, 0));
    ctx.wait_until_connected(&credentials.room, &member.id.to_string(), false).await;
    assert!(ctx.events(&mut owner_events).await.iter().any(|event| matches!(
        event,
        Push::HangoutPresence(presence) if !presence.inside && presence.inside_count == 0
    )));
}

#[tokio::test]
async fn a_ghost_in_a_hangout_is_taken_out() {
    let cfg = SpacesConfig { connect_grace_seconds: 0, reconnect_grace_seconds: 0, ..Default::default() };
    let ctx = start_with(cfg).await;
    let (owner, member) = ctx.friends("taro", "hanako").await;
    let hangout = ctx.hangout(&owner, "いつメン", &[&member]).await;
    let venue = Venue::Hangout(Uuid::parse_str(&hangout.id).unwrap());
    let owner_credentials = ctx.join_hangout(&owner, &hangout.id).await.credentials.as_option().unwrap().clone();
    let _connection = ctx.connect(&owner_credentials).await;
    // Entered and never connected.
    let _ = ctx.join_hangout(&member, &hangout.id).await;
    assert_eq!(ctx.get_hangout(&owner, &hangout.id).await.inside_count, 2);

    let mut owner_events = ctx.listen(&owner).await;
    let changed = ctx.spaces.reconcile_venue(venue).await.unwrap();
    assert_eq!((changed.ghosts, changed.strays), (1, 0));
    let after = ctx.get_hangout(&owner, &hangout.id).await;
    assert_eq!(after.inside_count, 1);
    assert!(after.members.iter().any(|m| m.user.id == owner.id.to_string() && m.inside));
    assert!(after.members.iter().any(|m| m.user.id == member.id.to_string() && !m.inside));
    assert!(ctx.events(&mut owner_events).await.iter().any(|event| matches!(
        event,
        Push::HangoutPresence(presence) if !presence.inside && presence.user.id == member.id.to_string()
    )));
}
