//! `CommunityService`: creating, reading, editing, closing and handing over
//! communities.
mod common;

use common::*;
use tokuly_community::CommunityConfig;
use tokuly_proto::{
    community::{community_event::Event as Pushed, *},
    media::AssetPurpose,
};

const SVC: &str = "CommunityService";

#[tokio::test]
async fn creating_a_community_makes_the_caller_its_owner() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let mut request = ctx.new_community(CommunityVisibility::COMMUNITY_VISIBILITY_PUBLIC);
    request.rules =
        vec![Rule { title: " なかよく ".into(), body: "けんかしない".into(), ..Default::default() }];
    request.tags = vec!["#APEX".into(), "apex".into(), "ランク".into()];
    let community = ctx.try_create(&owner, &request).await.unwrap();

    assert_eq!(community.name, "Apex部");
    assert_eq!(community.handle, request.handle);
    assert_eq!(community.tags, vec!["apex", "ランク"]);
    assert_eq!(community.rules[0].title, "なかよく");
    assert_eq!(community.category, CommunityCategory::COMMUNITY_CATEGORY_GAME);
    assert_eq!(community.member_count, 1);
    assert_eq!(community.owner.id, owner.id.to_string());
    // A game community carries no school.
    assert!(community.school_name.is_empty());

    let viewer = community.viewer.as_option().unwrap();
    assert_eq!(viewer.status, MembershipStatus::MEMBERSHIP_STATUS_MEMBER);
    assert_eq!(viewer.role, MemberRole::MEMBER_ROLE_OWNER);
    assert_eq!(viewer.permissions.len(), 14);
    assert_eq!(viewer.notification_level, NotificationLevel::NOTIFICATION_LEVEL_ALL);
    assert!(!viewer.show_on_profile, "memberships show on profiles only when chosen");

    // It starts with one channel to talk in.
    let channels = ctx.channels(Some(&owner), &community).await;
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].name, "雑談");
    assert!(channels[0].can_post);

    // The rest of the system hears about it.
    assert_eq!(ctx.community_events("community.created", &community.id).await.len(), 1);
    assert_eq!(ctx.community_events("community.member.joined", &community.id).await.len(), 1);
    let docs = ctx.search_changes("community", &community.id).await;
    assert_eq!(docs.last().unwrap()["doc"]["title"], "Apex部");

    let mine: ListMyCommunitiesResponse =
        ctx.call(SVC, "ListMyCommunities", Some(&owner), &ListMyCommunitiesRequest::default()).await.unwrap();
    assert_eq!(mine.communities.len(), 1);
    assert_eq!((mine.owned_count, mine.max_owned), (1, 50));
}

#[tokio::test]
async fn creation_validates_what_it_is_given() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let base = || ctx.new_community(CommunityVisibility::COMMUNITY_VISIBILITY_PUBLIC);
    let taken = ctx.public(&owner).await.handle;

    let cases: Vec<(&str, CreateCommunityRequest)> = vec![
        ("NAME_INVALID", CreateCommunityRequest { name: "  ".into(), ..base() }),
        ("NAME_INVALID", CreateCommunityRequest { name: "あ".repeat(51), ..base() }),
        ("HANDLE_INVALID", CreateCommunityRequest { handle: "ab".into(), ..base() }),
        ("HANDLE_RESERVED", CreateCommunityRequest { handle: "admin".into(), ..base() }),
        ("HANDLE_TAKEN", CreateCommunityRequest { handle: taken.to_uppercase(), ..base() }),
        ("DESCRIPTION_TOO_LONG", CreateCommunityRequest { description: "あ".repeat(1001), ..base() }),
        ("CATEGORY_INVALID", CreateCommunityRequest { category: Default::default(), ..base() }),
        ("VISIBILITY_INVALID", CreateCommunityRequest { visibility: Default::default(), ..base() }),
        ("LANGUAGE_INVALID", CreateCommunityRequest { language: "日本語".into(), ..base() }),
        ("TAG_INVALID", CreateCommunityRequest { tags: vec!["a b".into()], ..base() }),
        ("TOO_MANY_TAGS", CreateCommunityRequest { tags: (0..11).map(|i| format!("t{i}")).collect(), ..base() }),
        ("RULE_INVALID", CreateCommunityRequest { rules: vec![Rule::default()], ..base() }),
        (
            "TOO_MANY_RULES",
            CreateCommunityRequest { rules: vec![Rule { title: "r".into(), ..Default::default() }; 21], ..base() },
        ),
        ("QUESTION_INVALID", CreateCommunityRequest { join_questions: vec![String::new()], ..base() }),
        ("TOO_MANY_QUESTIONS", CreateCommunityRequest { join_questions: vec!["q".into(); 6], ..base() }),
    ];
    for (reason, request) in cases {
        assert_eq!(ctx.reason(SVC, "CreateCommunity", Some(&owner), &request).await, reason);
    }
    assert_eq!(ctx.reason(SVC, "CreateCommunity", None, &base()).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn one_account_owns_only_so_many_communities() {
    let ctx = start_with(CommunityConfig { max_owned: 2, ..test_config() }).await;
    let owner = ctx.app.user("Owner").await;
    ctx.public(&owner).await;
    let second = ctx.public(&owner).await;
    let request = ctx.new_community(CommunityVisibility::COMMUNITY_VISIBILITY_PUBLIC);
    assert_eq!(ctx.reason(SVC, "CreateCommunity", Some(&owner), &request).await, "COMMUNITY_LIMIT_REACHED");

    // Closing one makes room again.
    let _: DeleteCommunityResponse =
        ctx.call(SVC, "DeleteCommunity", Some(&owner), &by_id_request(&second)).await.unwrap();
    ctx.try_create(&owner, &request).await.unwrap();
}

fn by_id_request(community: &Community) -> DeleteCommunityRequest {
    DeleteCommunityRequest { community_id: community.id.clone(), ..Default::default() }
}

#[tokio::test]
async fn school_and_club_communities_take_the_creators_school() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    sqlx::query("UPDATE users SET school_name = '徳川高校' WHERE id = $1")
        .bind(owner.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let request = CreateCommunityRequest {
        category: CommunityCategory::COMMUNITY_CATEGORY_CLUB.into(),
        ..ctx.new_community(CommunityVisibility::COMMUNITY_VISIBILITY_RESTRICTED)
    };
    assert_eq!(ctx.try_create(&owner, &request).await.unwrap().school_name, "徳川高校");

    let named = CreateCommunityRequest {
        category: CommunityCategory::COMMUNITY_CATEGORY_SCHOOL.into(),
        school_name: " 葵中学 ".into(),
        ..ctx.new_community(CommunityVisibility::COMMUNITY_VISIBILITY_RESTRICTED)
    };
    assert_eq!(ctx.try_create(&owner, &named).await.unwrap().school_name, "葵中学");
}

#[tokio::test]
async fn icons_and_banners_are_held_while_they_are_shown() {
    let ctx = start().await;
    let (owner, other) = (ctx.app.user("Owner").await, ctx.app.user("Other").await);
    let icon = ctx.image(&owner, AssetPurpose::ASSET_PURPOSE_COMMUNITY_ICON).await;
    let banner = ctx.image(&owner, AssetPurpose::ASSET_PURPOSE_COMMUNITY_BANNER).await;
    let request = CreateCommunityRequest {
        icon_asset_id: icon.clone(),
        banner_asset_id: banner.clone(),
        ..ctx.new_community(CommunityVisibility::COMMUNITY_VISIBILITY_PUBLIC)
    };
    let community = ctx.try_create(&owner, &request).await.unwrap();
    assert_eq!(community.icon.asset_id, icon);
    assert_eq!(community.banner.asset_id, banner);
    assert!(ctx.is_held(&icon, &format!("community:icon:{}", community.id)).await);
    assert!(ctx.is_held(&banner, &format!("community:banner:{}", community.id)).await);

    // A new icon replaces the old one; an empty ID removes the banner.
    // (An upload made as an avatar, what icons were before they had a
    // purpose of their own, is still taken.)
    let new_icon = ctx.image(&owner, AssetPurpose::ASSET_PURPOSE_AVATAR).await;
    let update = UpdateCommunityRequest {
        community_id: community.id.clone(),
        icon_asset_id: Some(new_icon.clone()),
        banner_asset_id: Some(String::new()),
        ..Default::default()
    };
    let updated: UpdateCommunityResponse = ctx.call(SVC, "UpdateCommunity", Some(&owner), &update).await.unwrap();
    assert_eq!(updated.community.icon.asset_id, new_icon);
    assert!(updated.community.banner.as_option().is_none());
    assert!(!ctx.is_held(&icon, &format!("community:icon:{}", community.id)).await);
    assert!(ctx.is_held(&new_icon, &format!("community:icon:{}", community.id)).await);
    assert!(!ctx.is_held(&banner, &format!("community:banner:{}", community.id)).await);

    // Uploads must be the caller's own, and made for this.
    let foreign = ctx.image(&other, AssetPurpose::ASSET_PURPOSE_AVATAR).await;
    let wrong = ctx.image(&owner, AssetPurpose::ASSET_PURPOSE_STAMP).await;
    let with_icon = |asset: &str| CreateCommunityRequest {
        icon_asset_id: asset.to_owned(),
        ..ctx.new_community(CommunityVisibility::COMMUNITY_VISIBILITY_PUBLIC)
    };
    assert_eq!(ctx.reason(SVC, "CreateCommunity", Some(&owner), &with_icon(&foreign)).await, "ASSET_NOT_FOUND");
    assert_eq!(ctx.reason(SVC, "CreateCommunity", Some(&owner), &with_icon(&wrong)).await, "ASSET_WRONG_PURPOSE");
}

#[tokio::test]
async fn visibility_decides_who_sees_what() {
    let ctx = start().await;
    let (owner, outsider) = (ctx.app.user("Owner").await, ctx.app.user("Outsider").await);
    let public = ctx.public(&owner).await;
    let restricted = ctx.restricted(&owner).await;
    let hidden = ctx.hidden(&owner).await;
    let list = |community: &Community| ListChannelsRequest { community_id: community.id.clone(), ..Default::default() };

    // Public: anyone, signed in or not, reads everything. By handle too.
    assert!(ctx.get(None, &public.id).await.unwrap().viewer.as_option().is_none());
    assert_eq!(ctx.get(None, &public.handle).await.unwrap().id, public.id);
    assert_eq!(ctx.channels(None, &public).await.len(), 1);

    // Restricted: the front page is open, the content is for members.
    let front = ctx.get(Some(&outsider), &restricted.id).await.unwrap();
    assert_eq!(front.viewer.status, MembershipStatus::MEMBERSHIP_STATUS_NONE);
    assert!(front.viewer.permissions.is_empty());
    assert_eq!(ctx.get(None, &restricted.id).await.unwrap().member_count, 1);
    assert_eq!(
        ctx.reason("CommunityBoardService", "ListChannels", Some(&outsider), &list(&restricted)).await,
        "NOT_A_MEMBER"
    );
    assert_eq!(ctx.reason("CommunityBoardService", "ListChannels", None, &list(&restricted)).await, "NOT_A_MEMBER");

    // Hidden: it does not exist for outsiders.
    assert_eq!(ctx.get(Some(&outsider), &hidden.id).await.unwrap_err().reason, "COMMUNITY_NOT_FOUND");
    assert_eq!(ctx.get(None, &hidden.handle).await.unwrap_err().reason, "COMMUNITY_NOT_FOUND");
    assert_eq!(
        ctx.reason("CommunityBoardService", "ListChannels", Some(&outsider), &list(&hidden)).await,
        "COMMUNITY_NOT_FOUND"
    );
    assert_eq!(ctx.get(Some(&owner), &hidden.id).await.unwrap().id, hidden.id);

    assert_eq!(ctx.get(None, "no-such-community").await.unwrap_err().reason, "COMMUNITY_NOT_FOUND");
    assert_eq!(ctx.get(None, "not a handle!").await.unwrap_err().reason, "COMMUNITY_NOT_FOUND");
}

#[tokio::test]
async fn site_moderators_read_hidden_communities() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let moderator = ctx.app.user_with_roles("SiteMod", &["moderator"]).await;
    let hidden = ctx.hidden(&owner).await;
    let seen = ctx.get(Some(&moderator), &hidden.id).await.unwrap();
    assert_eq!(seen.viewer.status, MembershipStatus::MEMBERSHIP_STATUS_NONE);
    assert!(seen.viewer.permissions.is_empty(), "the site role is not a community role");
    assert_eq!(ctx.channels(Some(&moderator), &hidden).await.len(), 1);
}

#[tokio::test]
async fn editing_needs_an_admin() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let admin = ctx.staff(&owner, &community, "Admin", MemberRole::MEMBER_ROLE_ADMIN).await;
    let outsider = ctx.app.user("Outsider").await;
    let rename = UpdateCommunityRequest {
        community_id: community.id.clone(),
        name: Some("新しい名前".into()),
        ..Default::default()
    };
    for user in [&member, &moderator, &outsider] {
        assert_eq!(ctx.reason(SVC, "UpdateCommunity", Some(user), &rename).await, "PERMISSION_DENIED");
    }
    assert_eq!(ctx.reason(SVC, "UpdateCommunity", None, &rename).await, "SIGN_IN_REQUIRED");

    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    let update = UpdateCommunityRequest {
        community_id: community.id.clone(),
        name: Some("新しい名前".into()),
        description: Some("説明".into()),
        tags: StringList { values: vec!["Ranked".into()], ..Default::default() }.into(),
        rules: RuleList { rules: vec![Rule { title: "ルール".into(), ..Default::default() }], ..Default::default() }
            .into(),
        join_questions: StringList { values: vec!["どこで知った?".into()], ..Default::default() }.into(),
        visibility: Some(CommunityVisibility::COMMUNITY_VISIBILITY_RESTRICTED.into()),
        settings: CommunitySettings { members_can_invite: true, ..Default::default() }.into(),
        ..Default::default()
    };
    let updated: UpdateCommunityResponse = ctx.call(SVC, "UpdateCommunity", Some(&admin), &update).await.unwrap();
    let updated = updated.community.as_option().unwrap();
    assert_eq!(updated.name, "新しい名前");
    assert_eq!(updated.tags, vec!["ranked"]);
    assert_eq!(updated.rules.len(), 1);
    assert_eq!(updated.join_questions, vec!["どこで知った?"]);
    assert_eq!(updated.visibility, CommunityVisibility::COMMUNITY_VISIBILITY_RESTRICTED);
    assert!(updated.settings.members_can_invite);
    // What was not sent stays.
    assert_eq!(updated.handle, community.handle);
    assert_eq!(updated.language, "ja");

    assert!(pushes(&mut topic).await.iter().any(|push| matches!(push, Pushed::CommunityChanged(_))));
    let log = ctx.moderation_log(&owner, &community).await;
    let entry = log.iter().find(|entry| entry.action == ModerationAction::MODERATION_ACTION_COMMUNITY_UPDATED).unwrap();
    assert_eq!(entry.actor.id, admin.id.to_string());
    assert!(entry.details["changed"].contains("visibility"));

    // A handle someone else has is not there to take; validation applies to edits too.
    let other = ctx.public(&owner).await;
    let steal = UpdateCommunityRequest {
        community_id: community.id.clone(),
        handle: Some(other.handle.clone()),
        ..Default::default()
    };
    assert_eq!(ctx.reason(SVC, "UpdateCommunity", Some(&admin), &steal).await, "HANDLE_TAKEN");
    let bad =
        UpdateCommunityRequest { community_id: community.id.clone(), name: Some(String::new()), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UpdateCommunity", Some(&admin), &bad).await, "NAME_INVALID");
}

#[tokio::test]
async fn handles_are_checked_before_they_are_taken() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let check = |handle: &str| CheckHandleRequest { handle: handle.to_owned(), ..Default::default() };
    let free: CheckHandleResponse = ctx.call(SVC, "CheckHandle", None, &check(&unique_handle("Fresh"))).await.unwrap();
    assert!(free.available && free.reason.is_empty());
    assert!(free.normalized.starts_with("fresh-"));
    for (handle, reason) in
        [("ab", "HANDLE_INVALID"), ("admin", "HANDLE_RESERVED"), (community.handle.as_str(), "HANDLE_TAKEN")]
    {
        let response: CheckHandleResponse = ctx.call(SVC, "CheckHandle", Some(&owner), &check(handle)).await.unwrap();
        assert!(!response.available);
        assert_eq!(response.reason, reason);
    }
}

#[tokio::test]
async fn ownership_passes_to_a_member() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let heir = ctx.member(&owner, &community, "Heir").await;
    let admin = ctx.staff(&owner, &community, "Admin", MemberRole::MEMBER_ROLE_ADMIN).await;
    let outsider = ctx.app.user("Outsider").await;
    let to = |user: &tokuly_testkit::TestUser| TransferOwnershipRequest {
        community_id: community.id.clone(),
        user_id: user.id.to_string(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(SVC, "TransferOwnership", Some(&admin), &to(&heir)).await, "PERMISSION_DENIED");
    assert_eq!(ctx.reason(SVC, "TransferOwnership", Some(&owner), &to(&owner)).await, "CANNOT_TARGET_SELF");
    assert_eq!(ctx.reason(SVC, "TransferOwnership", Some(&owner), &to(&outsider)).await, "MEMBER_NOT_FOUND");

    let mut heir_frames = ctx.app.listen_user(heir.id).await;
    let response: TransferOwnershipResponse =
        ctx.call(SVC, "TransferOwnership", Some(&owner), &to(&heir)).await.unwrap();
    assert_eq!(response.community.owner.id, heir.id.to_string());
    // The previous owner stays, as an admin.
    assert_eq!(response.community.viewer.role, MemberRole::MEMBER_ROLE_ADMIN);
    assert_eq!(ctx.viewer(&heir, &community.id).await.role, MemberRole::MEMBER_ROLE_OWNER);
    assert!(
        pushes(&mut heir_frames).await.iter().any(
            |push| matches!(push, Pushed::MembershipChanged(change) if change.role == MemberRole::MEMBER_ROLE_OWNER)
        )
    );
    let told = ctx.notifications(heir.id, "community.membership").await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0]["data"]["change"], "ownership");
    let log = ctx.moderation_log(&heir, &community).await;
    assert!(log.iter().any(|entry| entry.action == ModerationAction::MODERATION_ACTION_OWNERSHIP_TRANSFERRED
        && entry.target_user.id == heir.id.to_string()));

    // The old owner can no longer hand it on.
    assert_eq!(ctx.reason(SVC, "TransferOwnership", Some(&owner), &to(&admin)).await, "PERMISSION_DENIED");
}

#[tokio::test]
async fn a_new_owner_must_have_room_for_another_community() {
    let ctx = start_with(CommunityConfig { max_owned: 1, ..test_config() }).await;
    let (owner, busy) = (ctx.app.user("Owner").await, ctx.app.user("Busy").await);
    let community = ctx.public(&owner).await;
    ctx.public(&busy).await;
    ctx.add(&owner, &community, &busy).await;
    let request = TransferOwnershipRequest {
        community_id: community.id.clone(),
        user_id: busy.id.to_string(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(SVC, "TransferOwnership", Some(&owner), &request).await, "COMMUNITY_LIMIT_REACHED");
}

#[tokio::test]
async fn deleting_closes_the_community_for_everyone() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let admin = ctx.staff(&owner, &community, "Admin", MemberRole::MEMBER_ROLE_ADMIN).await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&owner, &general, "さよなら", "また会おう").await;
    let delete = by_id_request(&community);
    assert_eq!(ctx.reason(SVC, "DeleteCommunity", Some(&admin), &delete).await, "PERMISSION_DENIED");

    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    let _: DeleteCommunityResponse = ctx.call(SVC, "DeleteCommunity", Some(&owner), &delete).await.unwrap();
    assert!(
        pushes(&mut topic).await.iter().any(|push| matches!(push, Pushed::CommunityChanged(change) if change.deleted))
    );

    assert_eq!(ctx.get(Some(&owner), &community.id).await.unwrap_err().reason, "COMMUNITY_NOT_FOUND");
    assert_eq!(ctx.get_thread(Some(&owner), &thread.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    assert_eq!(ctx.reason(SVC, "DeleteCommunity", Some(&owner), &delete).await, "COMMUNITY_NOT_FOUND");
    let mine: ListMyCommunitiesResponse =
        ctx.call(SVC, "ListMyCommunities", Some(&admin), &ListMyCommunitiesRequest::default()).await.unwrap();
    assert!(mine.communities.is_empty());

    // The handle is free again, and search forgets the community and its threads.
    let check: CheckHandleResponse = ctx
        .call(SVC, "CheckHandle", None, &CheckHandleRequest { handle: community.handle.clone(), ..Default::default() })
        .await
        .unwrap();
    assert!(check.available);
    assert!(ctx.search_changes("community", &community.id).await.last().unwrap()["doc"].is_null());
    assert_eq!(ctx.community_events("community.deleted", &community.id).await.len(), 1);
    ctx.community
        .run_sweep(tokuly_community::SweepJob { community_id: community.id.parse().unwrap(), channel_id: None })
        .await
        .unwrap();
    assert!(ctx.search_changes("thread", &thread.id).await.last().unwrap()["doc"].is_null());
}

#[tokio::test]
async fn my_communities_come_most_recently_active_first() {
    let ctx = start().await;
    let (owner, user) = (ctx.app.user("Owner").await, ctx.app.user("User").await);
    let quiet = ctx.public(&owner).await;
    let busy = ctx.public(&owner).await;
    let waiting = ctx.restricted(&owner).await;
    ctx.join(&user, &quiet).await;
    ctx.join(&user, &busy).await;
    assert_eq!(
        ctx.try_join(&user, &waiting.id, "").await.unwrap().status,
        MembershipStatus::MEMBERSHIP_STATUS_REQUESTED
    );
    let general = ctx.general(&owner, &busy).await;
    ctx.thread(&owner, &general, "にぎやか", "わいわい").await;

    let first: ListMyCommunitiesResponse = ctx
        .call(
            SVC,
            "ListMyCommunities",
            Some(&user),
            &ListMyCommunitiesRequest { page: page(1, ""), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(first.communities.len(), 1);
    assert_eq!(first.communities[0].id, busy.id);
    assert!(first.communities[0].viewer.has_unread, "the owner's new thread is unread");
    assert!(first.page.has_more);
    // What the caller asked to join comes with the first page.
    assert_eq!(first.requested.len(), 1);
    assert_eq!(first.requested[0].id, waiting.id);
    assert_eq!(first.requested[0].viewer.status, MembershipStatus::MEMBERSHIP_STATUS_REQUESTED);

    let second: ListMyCommunitiesResponse = ctx
        .call(
            SVC,
            "ListMyCommunities",
            Some(&user),
            &ListMyCommunitiesRequest { page: page(1, &first.page.next_cursor), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(second.communities[0].id, quiet.id);
    assert!(!second.communities[0].viewer.has_unread);
    assert!(second.requested.is_empty() && !second.page.has_more);
    assert_eq!(
        ctx.reason(SVC, "ListMyCommunities", None, &ListMyCommunitiesRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
}

#[tokio::test]
async fn profiles_show_only_the_memberships_a_member_chose_to_show() {
    let ctx = start().await;
    let (owner, user, viewer) = (ctx.app.user("Owner").await, ctx.app.user("User").await, ctx.app.user("Viewer").await);
    let shown = ctx.public(&owner).await;
    let unshown = ctx.public(&owner).await;
    let restricted = ctx.restricted(&owner).await;
    ctx.join(&user, &shown).await;
    ctx.join(&user, &unshown).await;
    ctx.add(&owner, &restricted, &user).await;
    let of_user = ListUserCommunitiesRequest { user_id: user.id.to_string(), ..Default::default() };
    let list = |viewer| {
        let request = of_user.clone();
        let ctx = &ctx;
        async move {
            let response: ListUserCommunitiesResponse =
                ctx.call(SVC, "ListUserCommunities", viewer, &request).await.unwrap();
            response.communities.into_iter().map(|community| community.id).collect::<Vec<_>>()
        }
    };
    assert!(list(Some(&viewer)).await.is_empty(), "nothing shows until the member says so");

    for community in [&shown, &restricted] {
        let request = UpdateMembershipRequest {
            community_id: community.id.clone(),
            show_on_profile: Some(true),
            ..Default::default()
        };
        let _: UpdateMembershipResponse = ctx.call(SVC, "UpdateMembership", Some(&user), &request).await.unwrap();
    }
    // The restricted one never shows, whatever the member chose.
    assert_eq!(list(Some(&viewer)).await, vec![shown.id.clone()]);
    assert_eq!(list(None).await, vec![shown.id.clone()]);

    // The profile card says the same.
    let profile: tokuly_proto::identity::GetProfileResponse = ctx
        .app
        .call(
            "tokuly.identity.v1.ProfileService/GetProfile",
            Some(&viewer),
            &serde_json::json!({ "userId": user.id.to_string() }),
        )
        .await
        .unwrap();
    let card = profile.profile.cards.iter().find(|card| card.kind == "community.memberships").expect("the card");
    assert_eq!(card.text.ja, shown.name);
    assert_eq!(card.data["count"], "1");
    assert_eq!(card.data["handles"], shown.handle);

    // A block hides the list; an account that does not exist has none.
    ctx.block(&user, &viewer).await;
    assert_eq!(ctx.reason(SVC, "ListUserCommunities", Some(&viewer), &of_user).await, "USER_NOT_FOUND");
    let nobody = ListUserCommunitiesRequest { user_id: uuid::Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "ListUserCommunities", Some(&viewer), &nobody).await, "USER_NOT_FOUND");
}

#[tokio::test]
async fn the_front_page_tells_the_staff_what_waits_for_them() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.restricted(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let applicant = ctx.app.user("Applicant").await;
    ctx.try_join(&applicant, &community.id, "").await.unwrap();
    assert_eq!(ctx.viewer(&owner, &community.id).await.pending_request_count, 1);
    // Members are not told about the queue.
    assert_eq!(ctx.viewer(&member, &community.id).await.pending_request_count, 0);
}

#[tokio::test]
async fn a_community_cannot_take_a_reserved_look_alike_or_a_held_handle() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let check = |handle: &str| CheckHandleRequest { handle: handle.to_owned(), ..Default::default() };
    let lookalike: CheckHandleResponse =
        ctx.call(SVC, "CheckHandle", Some(&owner), &check("t0kuly-0fficial")).await.unwrap();
    assert_eq!((lookalike.available, lookalike.reason.as_str()), (false, "HANDLE_RESERVED"));

    // A person gives a handle up: identity holds it for them.
    let carol = ctx.app.user("Carol").await;
    let given_up = carol.handle.clone();
    let _: tokuly_proto::identity::UpdateProfileResponse = ctx
        .app
        .call(
            "tokuly.identity.v1.AccountService/UpdateProfile",
            Some(&carol),
            &tokuly_proto::identity::UpdateProfileRequest {
                handle: Some(unique_handle("carol")),
                ..Default::default()
            },
        )
        .await
        .expect("carol renames");
    let held: CheckHandleResponse = ctx.call(SVC, "CheckHandle", Some(&owner), &check(&given_up)).await.unwrap();
    assert_eq!((held.available, held.reason.as_str()), (false, "HANDLE_TAKEN"));

    let mut request = ctx.new_community(CommunityVisibility::COMMUNITY_VISIBILITY_PUBLIC);
    request.handle = given_up.clone();
    assert_eq!(ctx.try_create(&owner, &request).await.unwrap_err().reason, "HANDLE_TAKEN");

    let community = ctx.public(&owner).await;
    let rename =
        UpdateCommunityRequest { community_id: community.id.clone(), handle: Some(given_up), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UpdateCommunity", Some(&owner), &rename).await, "HANDLE_TAKEN");
}
