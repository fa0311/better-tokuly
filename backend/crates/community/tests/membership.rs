//! `CommunityService`: joining and leaving, join requests, invites, the
//! member directory, roles and a member's own settings.
mod common;

use common::*;
use tokuly_community::CommunityConfig;
use tokuly_proto::community::{community_event::Event as Pushed, *};
use tokuly_testkit::TestUser;

const SVC: &str = "CommunityService";

fn of(community: &Community) -> String {
    community.id.clone()
}

async fn requests(ctx: &Ctx, staff: &TestUser, community: &Community) -> Vec<JoinRequest> {
    let request = ListJoinRequestsRequest { community_id: of(community), ..Default::default() };
    let response: ListJoinRequestsResponse = ctx.call(SVC, "ListJoinRequests", Some(staff), &request).await.unwrap();
    response.requests
}

async fn respond(ctx: &Ctx, staff: &TestUser, request_id: &str, approve: bool) -> RespondJoinRequestResponse {
    let request = RespondJoinRequestRequest { request_id: request_id.to_owned(), approve, ..Default::default() };
    ctx.call(SVC, "RespondJoinRequest", Some(staff), &request).await.unwrap()
}

async fn members(ctx: &Ctx, viewer: &TestUser, request: &ListMembersRequest) -> ListMembersResponse {
    ctx.call(SVC, "ListMembers", Some(viewer), request).await.unwrap()
}

#[tokio::test]
async fn anyone_joins_and_leaves_a_public_community() {
    let ctx = start().await;
    let (owner, user) = (ctx.app.user("Owner").await, ctx.app.user("User").await);
    let community = ctx.public(&owner).await;
    let mut frames = ctx.app.listen_user(user.id).await;
    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;

    let joined = ctx.try_join(&user, &community.id, "").await.unwrap();
    assert_eq!(joined.status, MembershipStatus::MEMBERSHIP_STATUS_MEMBER);
    assert_eq!(joined.community.member_count, 2);
    assert_eq!(joined.community.viewer.role, MemberRole::MEMBER_ROLE_MEMBER);
    assert_eq!(joined.community.viewer.permissions, vec![Permission::PERMISSION_POST]);
    assert!(pushes(&mut frames).await.iter().any(
        |push| matches!(push, Pushed::MembershipChanged(change) if change.status == MembershipStatus::MEMBERSHIP_STATUS_MEMBER)
    ));
    assert!(
        pushes(&mut topic)
            .await
            .iter()
            .any(|push| matches!(push, Pushed::CommunityChanged(change) if change.member_count == 2))
    );
    assert_eq!(ctx.app.events_by("community.member.joined", user.id).await.len(), 1);

    // Joining again changes nothing.
    assert_eq!(ctx.try_join(&user, &community.handle, "").await.unwrap().community.member_count, 2);
    assert_eq!(ctx.app.events_by("community.member.joined", user.id).await.len(), 1);

    let leave = LeaveCommunityRequest { community_id: of(&community), ..Default::default() };
    let _: LeaveCommunityResponse = ctx.call(SVC, "LeaveCommunity", Some(&user), &leave).await.unwrap();
    let after = ctx.get(Some(&user), &community.id).await.unwrap();
    assert_eq!(after.member_count, 1);
    assert_eq!(after.viewer.status, MembershipStatus::MEMBERSHIP_STATUS_NONE);
    assert_eq!(ctx.app.events_by("community.member.left", user.id).await[0].data["cause"], "left");

    assert_eq!(ctx.reason(SVC, "LeaveCommunity", Some(&user), &leave).await, "NOT_A_MEMBER");
    assert_eq!(ctx.reason(SVC, "LeaveCommunity", Some(&owner), &leave).await, "OWNER_CANNOT_LEAVE");
    assert_eq!(
        ctx.reason(
            SVC,
            "JoinCommunity",
            None,
            &JoinCommunityRequest { community_id: of(&community), ..Default::default() }
        )
        .await,
        "SIGN_IN_REQUIRED"
    );
}

#[tokio::test]
async fn a_restricted_community_is_joined_by_request() {
    let ctx = start().await;
    let (owner, user) = (ctx.app.user("Owner").await, ctx.app.user("User").await);
    let request = CreateCommunityRequest {
        join_questions: vec!["何年生?".into(), "なぜ入りたい?".into()],
        ..ctx.new_community(CommunityVisibility::COMMUNITY_VISIBILITY_RESTRICTED)
    };
    let community = ctx.try_create(&owner, &request).await.unwrap();
    let moderator = ctx.moderator(&owner, &community).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let join = |answers: &[&str]| JoinCommunityRequest {
        community_id: of(&community),
        answers: answers.iter().map(|answer| answer.to_string()).collect(),
        ..Default::default()
    };

    // The questions are part of the front page, and they must be answered.
    assert_eq!(ctx.get(Some(&user), &community.id).await.unwrap().join_questions.len(), 2);
    assert_eq!(ctx.reason(SVC, "JoinCommunity", Some(&user), &join(&[])).await, "ANSWERS_REQUIRED");
    assert_eq!(ctx.reason(SVC, "JoinCommunity", Some(&user), &join(&["2年", " "])).await, "ANSWERS_REQUIRED");
    assert_eq!(
        ctx.reason(SVC, "JoinCommunity", Some(&user), &join(&["2年", &"あ".repeat(501)])).await,
        "ANSWER_TOO_LONG"
    );

    let mut staff_frames = ctx.app.listen_user(moderator.id).await;
    let filed: JoinCommunityResponse =
        ctx.call(SVC, "JoinCommunity", Some(&user), &join(&["2年", "楽しそう"])).await.unwrap();
    assert_eq!(filed.status, MembershipStatus::MEMBERSHIP_STATUS_REQUESTED);
    assert_eq!(filed.community.viewer.status, MembershipStatus::MEMBERSHIP_STATUS_REQUESTED);
    assert_eq!(filed.community.member_count, 3, "a request is not a membership");
    // The staff hears about it, members do not.
    assert_eq!(ctx.notifications(owner.id, "community.join_request").await.len(), 1);
    assert_eq!(ctx.notifications(moderator.id, "community.join_request").await.len(), 1);
    assert!(ctx.notifications(member.id, "community.join_request").await.is_empty());
    assert!(
        pushes(&mut staff_frames)
            .await
            .iter()
            .any(|push| matches!(push, Pushed::StaffQueueChanged(queue) if queue.pending_request_count == 1))
    );

    let list = ListJoinRequestsRequest { community_id: of(&community), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "ListJoinRequests", Some(&member), &list).await, "PERMISSION_DENIED");
    assert_eq!(ctx.reason(SVC, "ListJoinRequests", Some(&user), &list).await, "PERMISSION_DENIED");
    let pending = requests(&ctx, &moderator, &community).await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].user.id, user.id.to_string());
    assert_eq!(pending[0].answers[1].question, "なぜ入りたい?");
    assert_eq!(pending[0].answers[1].answer, "楽しそう");

    // Only the staff decides.
    let decide = RespondJoinRequestRequest { request_id: pending[0].id.clone(), approve: true, ..Default::default() };
    assert_eq!(ctx.reason(SVC, "RespondJoinRequest", Some(&member), &decide).await, "PERMISSION_DENIED");
    let approved = respond(&ctx, &moderator, &pending[0].id, true).await;
    assert_eq!(approved.member.user.id, user.id.to_string());
    assert_eq!(approved.member.role, MemberRole::MEMBER_ROLE_MEMBER);
    assert_eq!(ctx.viewer(&user, &community.id).await.status, MembershipStatus::MEMBERSHIP_STATUS_MEMBER);
    assert_eq!(ctx.notifications(user.id, "community.join_approved").await.len(), 1);
    assert!(requests(&ctx, &owner, &community).await.is_empty());
    assert_eq!(ctx.reason(SVC, "RespondJoinRequest", Some(&moderator), &decide).await, "REQUEST_NOT_FOUND");
    let log = ctx.moderation_log(&owner, &community).await;
    assert!(log.iter().any(|entry| entry.action == ModerationAction::MODERATION_ACTION_JOIN_REQUEST_APPROVED
        && entry.target_user.id == user.id.to_string()));
}

#[tokio::test]
async fn declining_is_silent_and_requests_can_be_withdrawn() {
    let ctx = start().await;
    let (owner, declined, withdrawn) =
        (ctx.app.user("Owner").await, ctx.app.user("Declined").await, ctx.app.user("Withdrawn").await);
    let community = ctx.restricted(&owner).await;
    ctx.try_join(&declined, &community.id, "").await.unwrap();
    ctx.try_join(&withdrawn, &community.id, "").await.unwrap();
    let pending = requests(&ctx, &owner, &community).await;
    assert_eq!(pending.len(), 2);
    let of_declined = pending.iter().find(|request| request.user.id == declined.id.to_string()).unwrap();

    let response = respond(&ctx, &owner, &of_declined.id, false).await;
    assert!(response.member.as_option().is_none());
    // The requester is not told, keeps seeing "requested", and asking again
    // does not put a new request in front of the staff.
    assert!(ctx.notification_kinds(declined.id).await.is_empty());
    assert_eq!(ctx.viewer(&declined, &community.id).await.status, MembershipStatus::MEMBERSHIP_STATUS_REQUESTED);
    assert_eq!(
        ctx.try_join(&declined, &community.id, "").await.unwrap().status,
        MembershipStatus::MEMBERSHIP_STATUS_REQUESTED
    );
    assert_eq!(requests(&ctx, &owner, &community).await.len(), 1);

    // After the cooldown the request can be made again.
    sqlx::query("UPDATE community_join_requests SET declined_at = now() - interval '8 days' WHERE user_id = $1")
        .bind(declined.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.try_join(&declined, &community.id, "").await.unwrap();
    assert_eq!(requests(&ctx, &owner, &community).await.len(), 2);

    let cancel = CancelJoinRequestRequest { community_id: of(&community), ..Default::default() };
    let _: CancelJoinRequestResponse = ctx.call(SVC, "CancelJoinRequest", Some(&withdrawn), &cancel).await.unwrap();
    assert_eq!(ctx.viewer(&withdrawn, &community.id).await.status, MembershipStatus::MEMBERSHIP_STATUS_NONE);
    assert_eq!(ctx.reason(SVC, "CancelJoinRequest", Some(&withdrawn), &cancel).await, "REQUEST_NOT_FOUND");
    let unknown =
        RespondJoinRequestRequest { request_id: uuid::Uuid::now_v7().to_string(), approve: true, ..Default::default() };
    assert_eq!(ctx.reason(SVC, "RespondJoinRequest", Some(&owner), &unknown).await, "REQUEST_NOT_FOUND");
}

#[tokio::test]
async fn invites_lead_into_restricted_and_hidden_communities() {
    let ctx = start().await;
    let (owner, guest, late) = (ctx.app.user("Owner").await, ctx.app.user("Guest").await, ctx.app.user("Late").await);
    let hidden = ctx.hidden(&owner).await;
    let create = CreateInviteRequest {
        community_id: of(&hidden),
        max_uses: 1,
        expires_in_seconds: 3600,
        note: "文化祭のポスター用".into(),
        ..Default::default()
    };
    let response: CreateInviteResponse = ctx.call(SVC, "CreateInvite", Some(&owner), &create).await.unwrap();
    let invite = response.invite.as_option().unwrap().clone();
    assert_eq!(invite.code.len(), 10);
    assert!(invite.url.ends_with(&format!("/c/join/{}", invite.code)));
    assert_eq!((invite.max_uses, invite.use_count), (1, 0));
    assert!(invite.expires_at.as_option().is_some());

    // The invite shows the hidden community's front page; without it there is nothing.
    assert_eq!(ctx.get(Some(&guest), &hidden.id).await.unwrap_err().reason, "COMMUNITY_NOT_FOUND");
    assert_eq!(ctx.try_join(&guest, &hidden.id, "").await.unwrap_err().reason, "COMMUNITY_NOT_FOUND");
    let preview = PreviewInviteRequest { invite_code: invite.code.clone(), ..Default::default() };
    let shown: PreviewInviteResponse = ctx.call(SVC, "PreviewInvite", Some(&guest), &preview).await.unwrap();
    assert_eq!(shown.community.id, hidden.id);
    assert_eq!(shown.inviter.id, owner.id.to_string());
    let anonymous: PreviewInviteResponse = ctx.call(SVC, "PreviewInvite", None, &preview).await.unwrap();
    assert_eq!(anonymous.community.name, hidden.name);

    let joined = ctx.try_join(&guest, "", &invite.code).await.unwrap();
    assert_eq!(joined.status, MembershipStatus::MEMBERSHIP_STATUS_MEMBER);
    assert_eq!(ctx.get(Some(&guest), &hidden.id).await.unwrap().member_count, 2);

    // Used up: the next one is turned away, and the preview is gone too.
    assert_eq!(ctx.try_join(&late, "", &invite.code).await.unwrap_err().reason, "INVITE_INVALID");
    assert_eq!(ctx.reason(SVC, "PreviewInvite", Some(&late), &preview).await, "INVITE_INVALID");
    for garbage in ["", "short", "0123456789", "has space!"] {
        let request = PreviewInviteRequest { invite_code: garbage.into(), ..Default::default() };
        assert_eq!(ctx.reason(SVC, "PreviewInvite", Some(&late), &request).await, "INVITE_INVALID", "{garbage:?}");
    }

    // Expired and revoked invites stop working.
    let expiring = ctx.invite(&owner, &hidden).await;
    sqlx::query("UPDATE community_invites SET expires_at = now() - interval '1 minute' WHERE code = $1")
        .bind(&expiring.code)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(ctx.try_join(&late, "", &expiring.code).await.unwrap_err().reason, "INVITE_INVALID");
    let revoked = ctx.invite(&owner, &hidden).await;
    let revoke = RevokeInviteRequest { invite_id: revoked.id.clone(), ..Default::default() };
    let _: RevokeInviteResponse = ctx.call(SVC, "RevokeInvite", Some(&owner), &revoke).await.unwrap();
    assert_eq!(ctx.try_join(&late, "", &revoked.code).await.unwrap_err().reason, "INVITE_INVALID");
    assert_eq!(ctx.reason(SVC, "RevokeInvite", Some(&owner), &revoke).await, "INVITE_NOT_FOUND");

    // An invite is for its own community only.
    let other = ctx.public(&owner).await;
    let usable = ctx.invite(&owner, &hidden).await;
    assert_eq!(ctx.try_join(&late, &other.id, &usable.code).await.unwrap_err().reason, "INVITE_INVALID");
    assert_eq!(
        ctx.try_join(&late, &hidden.id, &usable.code).await.unwrap().status,
        MembershipStatus::MEMBERSHIP_STATUS_MEMBER
    );
}

#[tokio::test]
async fn a_restricted_community_lets_invited_people_straight_in() {
    let ctx = start().await;
    let (owner, guest) = (ctx.app.user("Owner").await, ctx.app.user("Guest").await);
    let request = CreateCommunityRequest {
        join_questions: vec!["なぜ?".into()],
        ..ctx.new_community(CommunityVisibility::COMMUNITY_VISIBILITY_RESTRICTED)
    };
    let community = ctx.try_create(&owner, &request).await.unwrap();
    let invite = ctx.invite(&owner, &community).await;
    // No approval and no questions for someone holding an invite.
    assert_eq!(
        ctx.try_join(&guest, "", &invite.code).await.unwrap().status,
        MembershipStatus::MEMBERSHIP_STATUS_MEMBER
    );
}

#[tokio::test]
async fn invites_follow_the_permission_table() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.restricted(&owner).await;
    let (member, other) =
        (ctx.member(&owner, &community, "Member").await, ctx.member(&owner, &community, "Other").await);
    let moderator = ctx.moderator(&owner, &community).await;
    let create = CreateInviteRequest { community_id: of(&community), ..Default::default() };
    let list = ListInvitesRequest { community_id: of(&community), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CreateInvite", Some(&member), &create).await, "PERMISSION_DENIED");
    assert_eq!(ctx.reason(SVC, "ListInvites", Some(&member), &list).await, "PERMISSION_DENIED");

    // The community may open invites to its members.
    let open = UpdateCommunityRequest {
        community_id: of(&community),
        settings: CommunitySettings { members_can_invite: true, ..Default::default() }.into(),
        ..Default::default()
    };
    let _: UpdateCommunityResponse = ctx.call(SVC, "UpdateCommunity", Some(&owner), &open).await.unwrap();
    assert!(
        ctx.viewer(&member, &community.id).await.permissions.contains(&Permission::PERMISSION_CREATE_INVITE.into())
    );
    let mine = ctx.invite(&member, &community).await;
    let theirs = ctx.invite(&other, &community).await;

    // Members see their own invites, the staff all of them.
    let own: ListInvitesResponse = ctx.call(SVC, "ListInvites", Some(&member), &list).await.unwrap();
    assert_eq!(own.invites.iter().map(|invite| invite.id.clone()).collect::<Vec<_>>(), vec![mine.id.clone()]);
    let all: ListInvitesResponse = ctx.call(SVC, "ListInvites", Some(&moderator), &list).await.unwrap();
    assert!(all.invites.len() >= 3, "two member invites plus the ones that brought the members in");

    // A member revokes their own; someone else's takes the staff.
    let revoke = |invite: &Invite| RevokeInviteRequest { invite_id: invite.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "RevokeInvite", Some(&member), &revoke(&theirs)).await, "PERMISSION_DENIED");
    let _: RevokeInviteResponse = ctx.call(SVC, "RevokeInvite", Some(&member), &revoke(&mine)).await.unwrap();
    let _: RevokeInviteResponse = ctx.call(SVC, "RevokeInvite", Some(&moderator), &revoke(&theirs)).await.unwrap();
    let log = ctx.moderation_log(&owner, &community).await;
    assert!(log.iter().any(|entry| entry.action == ModerationAction::MODERATION_ACTION_INVITE_REVOKED
        && entry.target_user.id == other.id.to_string()));
    let outsider = ctx.app.user("Outsider").await;
    let left = ctx.invite(&owner, &community).await;
    assert_eq!(ctx.reason(SVC, "RevokeInvite", Some(&outsider), &revoke(&left)).await, "INVITE_NOT_FOUND");

    for (reason, request) in [
        ("EXPIRY_INVALID", CreateInviteRequest { expires_in_seconds: 31 * 24 * 3600, ..create.clone() }),
        ("EXPIRY_INVALID", CreateInviteRequest { expires_in_seconds: -1, ..create.clone() }),
        ("MAX_USES_INVALID", CreateInviteRequest { max_uses: 10_001, ..create.clone() }),
        ("NOTE_TOO_LONG", CreateInviteRequest { note: "あ".repeat(101), ..create.clone() }),
    ] {
        assert_eq!(ctx.reason(SVC, "CreateInvite", Some(&owner), &request).await, reason);
    }
}

#[tokio::test]
async fn the_member_cap_and_the_join_limit_hold() {
    let ctx = start_with(CommunityConfig { max_members: 2, max_joined: 2, ..test_config() }).await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    assert_eq!(community.max_members, 2);
    let (second, third) = (ctx.app.user("Second").await, ctx.app.user("Third").await);
    ctx.join(&second, &community).await;
    assert_eq!(ctx.try_join(&third, &community.id, "").await.unwrap_err().reason, "COMMUNITY_FULL");

    // Two communities is all one account may be in here.
    let elsewhere = ctx.app.user("Elsewhere").await;
    let (a, b) = (ctx.public(&elsewhere).await, ctx.public(&elsewhere).await);
    let _ = a;
    let joiner = ctx.app.user("Joiner").await;
    let roomy = ctx.app.user("Roomy").await;
    let (c, d) = (ctx.public(&roomy).await, ctx.public(&roomy).await);
    ctx.join(&joiner, &c).await;
    ctx.join(&joiner, &d).await;
    assert_eq!(ctx.try_join(&joiner, &b.id, "").await.unwrap_err().reason, "JOIN_LIMIT_REACHED");
}

#[tokio::test]
async fn the_directory_lists_staff_first_and_searches() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.restricted(&owner).await;
    let early = ctx.member(&owner, &community, "Hanako").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let late = ctx.member(&owner, &community, "Taro").await;
    let outsider = ctx.app.user("Outsider").await;
    let all = ListMembersRequest { community_id: of(&community), ..Default::default() };

    let listed = members(&ctx, &early, &all).await;
    let ids: Vec<&str> = listed.members.iter().map(|member| member.user.id.as_str()).collect();
    assert_eq!(ids, vec![owner.id.to_string(), moderator.id.to_string(), early.id.to_string(), late.id.to_string()]);
    assert_eq!(listed.members[0].role, MemberRole::MEMBER_ROLE_OWNER);

    // Pages follow the same order.
    let first = members(&ctx, &early, &ListMembersRequest { page: page(3, ""), ..all.clone() }).await;
    assert!(first.page.has_more);
    let second =
        members(&ctx, &early, &ListMembersRequest { page: page(3, &first.page.next_cursor), ..all.clone() }).await;
    assert_eq!(second.members.len(), 1);
    assert_eq!(second.members[0].user.id, late.id.to_string());

    let search = members(&ctx, &early, &ListMembersRequest { query: "hana".into(), ..all.clone() }).await;
    assert_eq!(search.members.len(), 1);
    assert_eq!(search.members[0].user.display_name, "Hanako");
    let by_handle = members(&ctx, &early, &ListMembersRequest { query: late.handle.clone(), ..all.clone() }).await;
    assert_eq!(by_handle.members.len(), 1);
    let staff =
        members(&ctx, &early, &ListMembersRequest { role: MemberRole::MEMBER_ROLE_MODERATOR.into(), ..all.clone() })
            .await;
    assert_eq!(staff.members.len(), 1);

    // The directory is for members; the count on the front page is all outsiders get.
    assert_eq!(ctx.reason(SVC, "ListMembers", Some(&outsider), &all).await, "NOT_A_MEMBER");
    assert_eq!(ctx.reason(SVC, "ListMembers", None, &all).await, "SIGN_IN_REQUIRED");
    assert_eq!(
        ctx.reason(SVC, "ListMembers", Some(&early), &ListMembersRequest { query: "あ".repeat(65), ..all.clone() })
            .await,
        "QUERY_TOO_LONG"
    );

    let one = |user: &TestUser| GetMemberRequest {
        community_id: of(&community),
        user_id: user.id.to_string(),
        ..Default::default()
    };
    let got: GetMemberResponse = ctx.call(SVC, "GetMember", Some(&early), &one(&moderator)).await.unwrap();
    assert_eq!(got.member.role, MemberRole::MEMBER_ROLE_MODERATOR);
    assert_eq!(ctx.reason(SVC, "GetMember", Some(&early), &one(&outsider)).await, "MEMBER_NOT_FOUND");
    assert_eq!(ctx.reason(SVC, "GetMember", Some(&outsider), &one(&early)).await, "NOT_A_MEMBER");

    // People in a block do not see each other in the directory either.
    ctx.block(&late, &early).await;
    let listed = members(&ctx, &early, &all).await;
    assert!(!listed.members.iter().any(|member| member.user.id == late.id.to_string()));
    assert_eq!(ctx.reason(SVC, "GetMember", Some(&early), &one(&late)).await, "MEMBER_NOT_FOUND");
}

#[tokio::test]
async fn roles_are_handed_out_downwards() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let admin = ctx.staff(&owner, &community, "Admin", MemberRole::MEMBER_ROLE_ADMIN).await;
    let other_admin = ctx.staff(&owner, &community, "OtherAdmin", MemberRole::MEMBER_ROLE_ADMIN).await;
    let moderator = ctx.moderator(&owner, &community).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let set = |user: &TestUser, role: MemberRole| SetMemberRoleRequest {
        community_id: of(&community),
        user_id: user.id.to_string(),
        role: role.into(),
        ..Default::default()
    };
    use MemberRole::*;

    // Moderators and members appoint nobody.
    assert_eq!(
        ctx.reason(SVC, "SetMemberRole", Some(&moderator), &set(&member, MEMBER_ROLE_MODERATOR)).await,
        "PERMISSION_DENIED"
    );
    assert_eq!(
        ctx.reason(SVC, "SetMemberRole", Some(&member), &set(&moderator, MEMBER_ROLE_MEMBER)).await,
        "PERMISSION_DENIED"
    );
    // Admins appoint and dismiss moderators, not admins.
    assert_eq!(
        ctx.reason(SVC, "SetMemberRole", Some(&admin), &set(&member, MEMBER_ROLE_ADMIN)).await,
        "PERMISSION_DENIED"
    );
    assert_eq!(
        ctx.reason(SVC, "SetMemberRole", Some(&admin), &set(&other_admin, MEMBER_ROLE_MEMBER)).await,
        "PERMISSION_DENIED"
    );
    assert_eq!(
        ctx.reason(SVC, "SetMemberRole", Some(&admin), &set(&owner, MEMBER_ROLE_MEMBER)).await,
        "PERMISSION_DENIED"
    );
    assert_eq!(
        ctx.reason(SVC, "SetMemberRole", Some(&admin), &set(&admin, MEMBER_ROLE_MEMBER)).await,
        "CANNOT_TARGET_SELF"
    );
    assert_eq!(ctx.reason(SVC, "SetMemberRole", Some(&owner), &set(&member, MEMBER_ROLE_OWNER)).await, "ROLE_INVALID");
    assert_eq!(
        ctx.reason(SVC, "SetMemberRole", Some(&owner), &set(&member, MEMBER_ROLE_UNSPECIFIED)).await,
        "ROLE_INVALID"
    );
    let outsider = ctx.app.user("Outsider").await;
    assert_eq!(
        ctx.reason(SVC, "SetMemberRole", Some(&owner), &set(&outsider, MEMBER_ROLE_MODERATOR)).await,
        "MEMBER_NOT_FOUND"
    );

    let mut frames = ctx.app.listen_user(member.id).await;
    let promoted: SetMemberRoleResponse =
        ctx.call(SVC, "SetMemberRole", Some(&admin), &set(&member, MEMBER_ROLE_MODERATOR)).await.unwrap();
    assert_eq!(promoted.member.role, MEMBER_ROLE_MODERATOR);
    assert!(
        ctx.viewer(&member, &community.id).await.permissions.contains(&Permission::PERMISSION_MODERATE_CONTENT.into())
    );
    assert!(
        pushes(&mut frames)
            .await
            .iter()
            .any(|push| matches!(push, Pushed::MembershipChanged(change) if change.role == MEMBER_ROLE_MODERATOR))
    );
    let told = ctx.notifications(member.id, "community.membership").await;
    assert_eq!(told.last().unwrap()["data"]["change"], "role");
    let log = ctx.moderation_log(&owner, &community).await;
    let entry = log
        .iter()
        .find(|entry| {
            entry.action == ModerationAction::MODERATION_ACTION_ROLE_CHANGED
                && entry.target_user.id == member.id.to_string()
        })
        .unwrap();
    assert_eq!((entry.details["from"].as_str(), entry.details["to"].as_str()), ("member", "moderator"));

    // The owner dismisses an admin.
    let demoted: SetMemberRoleResponse =
        ctx.call(SVC, "SetMemberRole", Some(&owner), &set(&other_admin, MEMBER_ROLE_MEMBER)).await.unwrap();
    assert_eq!(demoted.member.role, MEMBER_ROLE_MEMBER);
}

#[tokio::test]
async fn members_set_their_own_notification_level() {
    let ctx = start().await;
    let (owner, outsider) = (ctx.app.user("Owner").await, ctx.app.user("Outsider").await);
    let community = ctx.public(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let update = UpdateMembershipRequest {
        community_id: of(&community),
        notification_level: Some(NotificationLevel::NOTIFICATION_LEVEL_MENTIONS.into()),
        show_on_profile: Some(true),
        ..Default::default()
    };
    let response: UpdateMembershipResponse = ctx.call(SVC, "UpdateMembership", Some(&member), &update).await.unwrap();
    assert_eq!(response.viewer.notification_level, NotificationLevel::NOTIFICATION_LEVEL_MENTIONS);
    assert!(response.viewer.show_on_profile);
    // A field that is not sent stays as it is.
    let only_level = UpdateMembershipRequest {
        community_id: of(&community),
        notification_level: Some(NotificationLevel::NOTIFICATION_LEVEL_NONE.into()),
        ..Default::default()
    };
    let response: UpdateMembershipResponse =
        ctx.call(SVC, "UpdateMembership", Some(&member), &only_level).await.unwrap();
    assert!(response.viewer.show_on_profile);
    assert_eq!(ctx.viewer(&member, &community.id).await.notification_level, NotificationLevel::NOTIFICATION_LEVEL_NONE);

    let invalid = UpdateMembershipRequest {
        community_id: of(&community),
        notification_level: Some(NotificationLevel::NOTIFICATION_LEVEL_UNSPECIFIED.into()),
        ..Default::default()
    };
    assert_eq!(ctx.reason(SVC, "UpdateMembership", Some(&member), &invalid).await, "LEVEL_INVALID");
    assert_eq!(ctx.reason(SVC, "UpdateMembership", Some(&outsider), &update).await, "NOT_A_MEMBER");
}

#[tokio::test]
async fn other_domains_can_ask_who_is_a_member() {
    let ctx = start().await;
    let (owner, outsider) = (ctx.app.user("Owner").await, ctx.app.user("Outsider").await);
    let community = ctx.public(&owner).await;
    let id: uuid::Uuid = community.id.parse().unwrap();
    assert_eq!(ctx.community.role_of(id, owner.id).await.unwrap(), Some(tokuly_community::Role::Owner));
    assert!(ctx.community.is_member(id, owner.id).await.unwrap());
    assert!(!ctx.community.is_member(id, outsider.id).await.unwrap());
}

#[tokio::test]
async fn an_invite_does_not_outlive_its_creators_standing() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.hidden(&owner).await;
    let open = |on: bool| UpdateCommunityRequest {
        community_id: community.id.clone(),
        settings: CommunitySettings { members_can_invite: on, ..Default::default() }.into(),
        ..Default::default()
    };
    let _: UpdateCommunityResponse = ctx.call(SVC, "UpdateCommunity", Some(&owner), &open(true)).await.unwrap();
    let kicked = ctx.member(&owner, &community, "Kicked").await;
    let leaver = ctx.member(&owner, &community, "Leaver").await;
    let demoted = ctx.moderator(&owner, &community).await;
    let staying = ctx.member(&owner, &community, "Staying").await;
    // Each of them keeps a way back in for later.
    let (of_kicked, of_leaver) = (ctx.invite(&kicked, &community).await, ctx.invite(&leaver, &community).await);
    let (of_demoted, of_staying) = (ctx.invite(&demoted, &community).await, ctx.invite(&staying, &community).await);
    let join = |user: &TestUser, invite: &Invite| {
        let (ctx, user, code) = (&ctx, user.clone(), invite.code.clone());
        async move { ctx.try_join(&user, "", &code).await.map(|response| response.status) }
    };

    // Removed: the invite they made does not bring them (or anyone) back.
    let kick = RemoveMemberRequest {
        community_id: community.id.clone(),
        user_id: kicked.id.to_string(),
        ..Default::default()
    };
    let _: RemoveMemberResponse =
        ctx.call("CommunityModerationService", "RemoveMember", Some(&owner), &kick).await.unwrap();
    assert_eq!(join(&kicked, &of_kicked).await.unwrap_err().reason, "INVITE_INVALID");
    assert_eq!(join(&ctx.app.user("Friend").await, &of_kicked).await.unwrap_err().reason, "INVITE_INVALID");
    assert!(!ctx.community.is_member(community.id.parse().unwrap(), kicked.id).await.unwrap());
    // Left: the same.
    let leave = LeaveCommunityRequest { community_id: community.id.clone(), ..Default::default() };
    let _: LeaveCommunityResponse = ctx.call(SVC, "LeaveCommunity", Some(&leaver), &leave).await.unwrap();
    assert_eq!(join(&leaver, &of_leaver).await.unwrap_err().reason, "INVITE_INVALID");
    let preview = PreviewInviteRequest { invite_code: of_leaver.code.clone(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "PreviewInvite", None, &preview).await, "INVITE_INVALID");

    // Members lose the right to invite: their invites stop working, the
    // staff's keep working — and come back with the setting.
    let _: UpdateCommunityResponse = ctx.call(SVC, "UpdateCommunity", Some(&owner), &open(false)).await.unwrap();
    let guest = ctx.app.user("Guest").await;
    assert_eq!(join(&guest, &of_staying).await.unwrap_err().reason, "INVITE_INVALID");
    // Demoted below the right to invite: revoked for good.
    ctx.set_role(&owner, &community, &demoted, MemberRole::MEMBER_ROLE_MEMBER).await;
    assert_eq!(join(&guest, &of_demoted).await.unwrap_err().reason, "INVITE_INVALID");
    let _: UpdateCommunityResponse = ctx.call(SVC, "UpdateCommunity", Some(&owner), &open(true)).await.unwrap();
    assert_eq!(join(&guest, &of_demoted).await.unwrap_err().reason, "INVITE_INVALID");
    assert_eq!(join(&guest, &of_staying).await.unwrap(), MembershipStatus::MEMBERSHIP_STATUS_MEMBER);
    // The owner's invite still lets a removed member back in: that is the owner's call.
    let again = ctx.invite(&owner, &community).await;
    assert_eq!(join(&kicked, &again).await.unwrap(), MembershipStatus::MEMBERSHIP_STATUS_MEMBER);
}
