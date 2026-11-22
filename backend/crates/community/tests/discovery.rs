//! `CommunityDiscoveryService`: the directory, trending, friends' and
//! school communities, popular tags.
//!
//! The tests of this file share a database, so each one marks its
//! communities with a tag (or a school) of its own and asks for those.
mod common;

use common::*;
use tokuly_proto::community::*;
use tokuly_testkit::TestUser;

const SVC: &str = "CommunityDiscoveryService";

fn unique_tag() -> String {
    format!("t{}", tokuly_core::ids::random_base62(10).to_ascii_lowercase())
}

async fn directory(ctx: &Ctx, viewer: Option<&TestUser>, request: &ListDirectoryRequest) -> ListDirectoryResponse {
    ctx.call(SVC, "ListDirectory", viewer, request).await.unwrap()
}

fn names(response: &ListDirectoryResponse) -> Vec<&str> {
    response.communities.iter().map(|community| community.name.as_str()).collect()
}

async fn show_on_profile(ctx: &Ctx, user: &TestUser, community: &Community) {
    let request = UpdateMembershipRequest {
        community_id: community.id.clone(),
        show_on_profile: Some(true),
        ..Default::default()
    };
    let _: UpdateMembershipResponse =
        ctx.call("CommunityService", "UpdateMembership", Some(user), &request).await.unwrap();
}

#[tokio::test]
async fn the_directory_filters_and_sorts_listed_communities() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let tag = unique_tag();
    let make = |name: &str, visibility: CommunityVisibility, category: CommunityCategory, language: &str| {
        CreateCommunityRequest {
            name: name.into(),
            // Handles are searched too: these say nothing the test asks for.
            handle: unique_handle("listed"),
            category: category.into(),
            language: language.into(),
            tags: vec![tag.clone()],
            ..ctx.new_community(visibility)
        }
    };
    use CommunityCategory::*;
    use CommunityVisibility::*;
    let big = ctx
        .try_create(&owner, &make("Big Apex", COMMUNITY_VISIBILITY_PUBLIC, COMMUNITY_CATEGORY_GAME, "ja"))
        .await
        .unwrap();
    let small = ctx
        .try_create(&owner, &make("Small Apex", COMMUNITY_VISIBILITY_RESTRICTED, COMMUNITY_CATEGORY_GAME, "ja"))
        .await
        .unwrap();
    let study = ctx
        .try_create(&owner, &make("Study Hall", COMMUNITY_VISIBILITY_PUBLIC, COMMUNITY_CATEGORY_STUDY, "en"))
        .await
        .unwrap();
    let secret = ctx
        .try_create(&owner, &make("Secret Apex", COMMUNITY_VISIBILITY_HIDDEN, COMMUNITY_CATEGORY_GAME, "ja"))
        .await
        .unwrap();
    let member = ctx.member(&owner, &big, "Member").await;
    ctx.member(&owner, &big, "Another").await;
    ctx.member(&owner, &secret, "Insider").await;
    let tagged = ListDirectoryRequest { tag: format!("#{}", tag.to_uppercase()), ..Default::default() };

    // Most members first by default; the hidden one is never listed, the restricted one is.
    let listed = directory(&ctx, None, &tagged).await;
    assert_eq!(names(&listed), vec!["Big Apex", "Study Hall", "Small Apex"]);
    assert_eq!(listed.communities[0].member_count, 3);
    assert!(listed.communities[0].viewer.as_option().is_none());
    // Each entry says what the caller is to it.
    let as_member = directory(&ctx, Some(&member), &tagged).await;
    assert_eq!(as_member.communities[0].viewer.status, MembershipStatus::MEMBERSHIP_STATUS_MEMBER);
    assert_eq!(as_member.communities[1].viewer.status, MembershipStatus::MEMBERSHIP_STATUS_NONE);

    let games = ListDirectoryRequest { category: COMMUNITY_CATEGORY_GAME.into(), ..tagged.clone() };
    assert_eq!(names(&directory(&ctx, None, &games).await), vec!["Big Apex", "Small Apex"]);
    let english = ListDirectoryRequest { language: "en".into(), ..tagged.clone() };
    assert_eq!(names(&directory(&ctx, None, &english).await), vec!["Study Hall"]);
    let apex = ListDirectoryRequest { query: "apex".into(), ..tagged.clone() };
    assert_eq!(names(&directory(&ctx, None, &apex).await), vec!["Big Apex", "Small Apex"]);
    let by_handle = ListDirectoryRequest { query: study.handle.clone(), ..Default::default() };
    assert_eq!(names(&directory(&ctx, None, &by_handle).await), vec!["Study Hall"]);
    // Wildcards in the search text are text.
    let wildcard = ListDirectoryRequest { query: "%".into(), ..tagged.clone() };
    assert!(directory(&ctx, None, &wildcard).await.communities.is_empty());

    let newest = ListDirectoryRequest { sort: DirectorySort::DIRECTORY_SORT_NEW.into(), ..tagged.clone() };
    assert_eq!(names(&directory(&ctx, None, &newest).await), vec!["Study Hall", "Small Apex", "Big Apex"]);
    ctx.thread(&owner, &ctx.general(&owner, &small).await, "動きあり", "x").await;
    let active = ListDirectoryRequest { sort: DirectorySort::DIRECTORY_SORT_ACTIVE.into(), ..tagged.clone() };
    assert_eq!(names(&directory(&ctx, None, &active).await), vec!["Small Apex", "Study Hall", "Big Apex"]);

    // Every order pages through everything once.
    for sort in
        [DirectorySort::DIRECTORY_SORT_MEMBERS, DirectorySort::DIRECTORY_SORT_NEW, DirectorySort::DIRECTORY_SORT_ACTIVE]
    {
        let request = ListDirectoryRequest { sort: sort.into(), ..tagged.clone() };
        let first = directory(&ctx, None, &ListDirectoryRequest { page: page(2, ""), ..request.clone() }).await;
        assert!(first.page.has_more);
        let second =
            directory(&ctx, None, &ListDirectoryRequest { page: page(2, &first.page.next_cursor), ..request.clone() })
                .await;
        assert!(!second.page.has_more);
        let mut seen: Vec<&str> = names(&first);
        seen.extend(names(&second));
        seen.sort();
        assert_eq!(seen, vec!["Big Apex", "Small Apex", "Study Hall"], "{sort:?}");
    }

    assert_eq!(
        ctx.reason(SVC, "ListDirectory", None, &ListDirectoryRequest { query: "あ".repeat(65), ..Default::default() })
            .await,
        "QUERY_TOO_LONG"
    );
    assert_eq!(
        ctx.reason(SVC, "ListDirectory", None, &ListDirectoryRequest { tag: "a b".into(), ..Default::default() }).await,
        "TAG_INVALID"
    );
    assert_eq!(ctx.reason(SVC, "ListDirectory", None, &serde_json::json!({ "sort": 42 })).await, "SORT_INVALID");
    let _ = secret;
}

#[tokio::test]
async fn trending_follows_recent_growth_and_activity() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let quiet = ctx.public(&owner).await;
    let lively = ctx.public(&owner).await;
    let secret = ctx.hidden(&owner).await;
    for community in [&lively, &secret] {
        for name in ["A", "B", "C"] {
            ctx.member(&owner, community, name).await;
        }
        let general = ctx.general(&owner, community).await;
        let thread = ctx.thread(&owner, &general, "にぎやか", "x").await;
        ctx.reply(&owner, &thread, "わいわい").await;
    }
    assert!(ctx.community.refresh_trend_scores().await.unwrap() >= 2);

    let trending = |request: ListTrendingRequest| {
        let ctx = &ctx;
        async move {
            let response: ListTrendingResponse = ctx.call(SVC, "ListTrending", None, &request).await.unwrap();
            response.communities.into_iter().map(|community| community.id).collect::<Vec<_>>()
        }
    };
    let all = trending(ListTrendingRequest { limit: 50, ..Default::default() }).await;
    let position = |community: &Community| all.iter().position(|id| *id == community.id);
    // A community that just started trends a little (its first member counts);
    // one that people join and post in trends more. Hidden ones never show.
    assert!(position(&lively).unwrap() < position(&quiet).unwrap());
    assert!(position(&secret).is_none());
    let other_category = ListTrendingRequest {
        category: CommunityCategory::COMMUNITY_CATEGORY_LOCAL.into(),
        limit: 50,
        ..Default::default()
    };
    assert!(!trending(other_category).await.contains(&lively.id));

    // What happened more than a week ago no longer counts.
    let id: uuid::Uuid = lively.id.parse().unwrap();
    for statement in [
        "UPDATE community_members SET joined_at = now() - interval '30 days' WHERE community_id = $1",
        "UPDATE community_threads SET created_at = now() - interval '30 days' WHERE community_id = $1",
        "UPDATE community_replies SET created_at = now() - interval '30 days' WHERE community_id = $1",
    ] {
        sqlx::query(statement).bind(id).execute(&ctx.app.core.db).await.unwrap();
    }
    ctx.community.refresh_trend_scores().await.unwrap();
    assert!(!trending(ListTrendingRequest { limit: 50, ..Default::default() }).await.contains(&lively.id));
}

#[tokio::test]
async fn friends_communities_show_where_friends_chose_to_be_seen() {
    let ctx = start().await;
    let (me, owner) = (ctx.app.user("Me").await, ctx.app.user("Owner").await);
    let (friend, other_friend, stranger) =
        (ctx.app.user("Friend").await, ctx.app.user("OtherFriend").await, ctx.app.user("Stranger").await);
    ctx.befriend(&me, &friend).await;
    ctx.befriend(&me, &other_friend).await;

    let shared = ctx.public(&owner).await;
    let one = ctx.public(&owner).await;
    let unshown = ctx.public(&owner).await;
    let restricted = ctx.restricted(&owner).await;
    let mine = ctx.public(&owner).await;
    let strangers = ctx.public(&stranger).await;
    for community in [&shared, &one, &unshown, &mine] {
        ctx.join(&friend, community).await;
    }
    ctx.add(&owner, &restricted, &friend).await;
    ctx.join(&other_friend, &shared).await;
    ctx.join(&me, &mine).await;
    for community in [&shared, &one, &restricted, &mine] {
        show_on_profile(&ctx, &friend, community).await;
    }
    show_on_profile(&ctx, &other_friend, &shared).await;
    show_on_profile(&ctx, &stranger, &strangers).await;

    let response: ListFriendsCommunitiesResponse =
        ctx.call(SVC, "ListFriendsCommunities", Some(&me), &ListFriendsCommunitiesRequest::default()).await.unwrap();
    // Most friends first. Not listed: a membership that is not shown, a
    // community that is not public, one I am in myself, one only strangers are in.
    let ids: Vec<&str> = response.communities.iter().map(|entry| entry.community.id.as_str()).collect();
    assert_eq!(ids, vec![shared.id.as_str(), one.id.as_str()]);
    assert_eq!(response.communities[0].friend_count, 2);
    let mut preview: Vec<String> = response.communities[0].friends.iter().map(|user| user.id.clone()).collect();
    preview.sort();
    let mut expected = vec![friend.id.to_string(), other_friend.id.to_string()];
    expected.sort();
    assert_eq!(preview, expected);
    assert_eq!(response.communities[1].friends[0].id, friend.id.to_string());

    let first: ListFriendsCommunitiesResponse = ctx
        .call(
            SVC,
            "ListFriendsCommunities",
            Some(&me),
            &ListFriendsCommunitiesRequest { page: page(1, ""), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(first.page.has_more);
    let second: ListFriendsCommunitiesResponse = ctx
        .call(
            SVC,
            "ListFriendsCommunities",
            Some(&me),
            &ListFriendsCommunitiesRequest { page: page(1, &first.page.next_cursor), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(second.communities[0].community.id, one.id);
    assert!(!second.page.has_more);

    // Someone without friends sees nothing; guests are asked to sign in.
    let lonely: ListFriendsCommunitiesResponse = ctx
        .call(SVC, "ListFriendsCommunities", Some(&stranger), &ListFriendsCommunitiesRequest::default())
        .await
        .unwrap();
    assert!(lonely.communities.is_empty());
    assert_eq!(
        ctx.reason(SVC, "ListFriendsCommunities", None, &ListFriendsCommunitiesRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
    let _ = unshown;
}

#[tokio::test]
async fn school_communities_are_found_by_the_callers_school() {
    let ctx = start().await;
    let school = format!("徳川高校{}", tokuly_core::ids::random_base62(6));
    let (teacher, student, elsewhere) =
        (ctx.app.user("Teacher").await, ctx.app.user("Student").await, ctx.app.user("Elsewhere").await);
    for user in [&teacher, &student] {
        sqlx::query("UPDATE users SET school_name = $2 WHERE id = $1")
            .bind(user.id)
            .bind(&school)
            .execute(&ctx.app.core.db)
            .await
            .unwrap();
    }
    let make = |name: &str, category: CommunityCategory, school_name: &str| CreateCommunityRequest {
        name: name.into(),
        category: category.into(),
        school_name: school_name.into(),
        ..ctx.new_community(CommunityVisibility::COMMUNITY_VISIBILITY_RESTRICTED)
    };
    use CommunityCategory::*;
    // The class takes the teacher's school; the club names it; the game community has none.
    let class = ctx.try_create(&teacher, &make("3年B組", COMMUNITY_CATEGORY_SCHOOL, "")).await.unwrap();
    let club = ctx.try_create(&elsewhere, &make("吹奏楽部", COMMUNITY_CATEGORY_CLUB, &school)).await.unwrap();
    ctx.try_create(&teacher, &make("ゲーム仲間", COMMUNITY_CATEGORY_GAME, &school)).await.unwrap();
    ctx.try_create(&elsewhere, &make("よその部活", COMMUNITY_CATEGORY_CLUB, "よその高校")).await.unwrap();
    ctx.member(&elsewhere, &club, "Flute").await;

    let response: ListSchoolCommunitiesResponse =
        ctx.call(SVC, "ListSchoolCommunities", Some(&student), &ListSchoolCommunitiesRequest::default()).await.unwrap();
    assert_eq!(response.school_name, school);
    assert_eq!(
        response.communities.iter().map(|community| community.id.as_str()).collect::<Vec<_>>(),
        vec![club.id.as_str(), class.id.as_str()]
    );
    assert_eq!(response.communities[1].school_name, school);

    // No school on the profile, nothing to list.
    let none: ListSchoolCommunitiesResponse = ctx
        .call(SVC, "ListSchoolCommunities", Some(&elsewhere), &ListSchoolCommunitiesRequest::default())
        .await
        .unwrap();
    assert!(none.communities.is_empty() && none.school_name.is_empty());
    assert_eq!(
        ctx.reason(SVC, "ListSchoolCommunities", None, &ListSchoolCommunitiesRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
}

#[tokio::test]
async fn popular_tags_count_listed_communities() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let (common_tag, rare_tag, secret_tag) = (unique_tag(), unique_tag(), unique_tag());
    let make = |tags: Vec<&String>, visibility: CommunityVisibility| CreateCommunityRequest {
        category: CommunityCategory::COMMUNITY_CATEGORY_TECH.into(),
        tags: tags.into_iter().cloned().collect(),
        ..ctx.new_community(visibility)
    };
    use CommunityVisibility::*;
    ctx.try_create(&owner, &make(vec![&common_tag, &rare_tag], COMMUNITY_VISIBILITY_PUBLIC)).await.unwrap();
    ctx.try_create(&owner, &make(vec![&common_tag], COMMUNITY_VISIBILITY_RESTRICTED)).await.unwrap();
    ctx.try_create(&owner, &make(vec![&secret_tag], COMMUNITY_VISIBILITY_HIDDEN)).await.unwrap();
    // The list is cached for a few minutes; start from a clean slate.
    let tech = CommunityCategory::COMMUNITY_CATEGORY_TECH as i32;
    for key in [format!("community:tags:{tech}"), "community:tags:0".to_owned()] {
        ctx.app.core.cache.del(&key).await.unwrap();
    }

    let request = ListPopularTagsRequest {
        category: CommunityCategory::COMMUNITY_CATEGORY_TECH.into(),
        limit: 100,
        ..Default::default()
    };
    let response: ListPopularTagsResponse = ctx.call(SVC, "ListPopularTags", None, &request).await.unwrap();
    let count = |tag: &String| response.tags.iter().find(|entry| entry.tag == *tag).map(|entry| entry.community_count);
    assert_eq!((count(&common_tag), count(&rare_tag), count(&secret_tag)), (Some(2), Some(1), None));
    let position = |tag: &String| response.tags.iter().position(|entry| entry.tag == *tag).unwrap();
    assert!(position(&common_tag) < position(&rare_tag));

    let one: ListPopularTagsResponse =
        ctx.call(SVC, "ListPopularTags", None, &ListPopularTagsRequest { limit: 1, ..request.clone() }).await.unwrap();
    assert_eq!(one.tags.len(), 1);
    let elsewhere = ListPopularTagsRequest {
        category: CommunityCategory::COMMUNITY_CATEGORY_LOCAL.into(),
        limit: 100,
        ..Default::default()
    };
    let response: ListPopularTagsResponse = ctx.call(SVC, "ListPopularTags", None, &elsewhere).await.unwrap();
    assert!(!response.tags.iter().any(|entry| entry.tag == common_tag));
}
