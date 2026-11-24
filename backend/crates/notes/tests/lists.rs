//! Lists: creating and changing them, members, following, leaving, the
//! list timeline, blocks, locked accounts, limits under concurrency, and
//! what an erased account leaves behind.
mod common;

use common::*;
use serde::{Serialize, de::DeserializeOwned};
use tokuly_proto::{
    common::{UserSummary, Visibility},
    notes::*,
};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

async fn call<Req: Serialize, Resp: DeserializeOwned>(
    ctx: &Ctx,
    method: &str,
    user: Option<&TestUser>,
    request: &Req,
) -> Result<Resp, RpcError> {
    ctx.app.call(&format!("tokuly.notes.v1.NoteListService/{method}"), user, request).await
}

async fn reason<Req: Serialize>(ctx: &Ctx, method: &str, user: Option<&TestUser>, request: &Req) -> String {
    match call::<_, serde_json::Value>(ctx, method, user, request).await {
        Ok(response) => panic!("NoteListService/{method} should fail, got {response}"),
        Err(error) => error.reason,
    }
}

async fn create(ctx: &Ctx, owner: &TestUser, name: &str, public: bool) -> NoteList {
    let response: CreateListResponse = call(
        ctx,
        "CreateList",
        Some(owner),
        &CreateListRequest { name: name.to_owned(), public, ..Default::default() },
    )
    .await
    .expect("create list");
    response.list.as_option().cloned().expect("the list")
}

fn add_request(list: &NoteList, users: &[&TestUser]) -> AddListMembersRequest {
    AddListMembersRequest {
        list_id: list.id.clone(),
        user_ids: users.iter().map(|user| user.id.to_string()).collect(),
        ..Default::default()
    }
}

async fn add(ctx: &Ctx, owner: &TestUser, list: &NoteList, users: &[&TestUser]) -> NoteList {
    let response: AddListMembersResponse =
        call(ctx, "AddListMembers", Some(owner), &add_request(list, users)).await.expect("add members");
    response.list.as_option().cloned().expect("the list")
}

async fn get(ctx: &Ctx, viewer: Option<&TestUser>, list: &NoteList) -> Result<NoteList, RpcError> {
    let response: GetListResponse =
        call(ctx, "GetList", viewer, &GetListRequest { list_id: list.id.clone(), ..Default::default() }).await?;
    Ok(response.list.as_option().cloned().expect("the list"))
}

async fn members(ctx: &Ctx, viewer: Option<&TestUser>, list: &NoteList) -> Vec<String> {
    let response: ListListMembersResponse = call(
        ctx,
        "ListListMembers",
        viewer,
        &ListListMembersRequest { list_id: list.id.clone(), page: page(100, ""), ..Default::default() },
    )
    .await
    .expect("list members");
    response.members.iter().map(|member: &UserSummary| member.id.clone()).collect()
}

async fn timeline(ctx: &Ctx, viewer: Option<&TestUser>, list: &NoteList) -> Vec<String> {
    let response: GetListTimelineResponse = call(
        ctx,
        "GetListTimeline",
        viewer,
        &GetListTimelineRequest { list_id: list.id.clone(), page: page(50, ""), ..Default::default() },
    )
    .await
    .expect("list timeline");
    texts(&response.entries).iter().map(|text| text.to_string()).collect()
}

async fn lists_of(ctx: &Ctx, viewer: Option<&TestUser>, user: &TestUser) -> Vec<String> {
    let response: ListListsResponse =
        call(ctx, "ListLists", viewer, &ListListsRequest { user_id: user.id.to_string(), ..Default::default() })
            .await
            .expect("list lists");
    response.lists.iter().map(|list| list.name.clone()).collect()
}

fn event(topic: &str, data: serde_json::Value) -> tokuly_core::events::Event {
    tokuly_core::events::Event {
        id: Uuid::now_v7(),
        topic: topic.to_owned(),
        actor_id: None,
        occurred_at: chrono::Utc::now(),
        data,
    }
}

async fn notes_export(ctx: &Ctx, user_id: Uuid) -> serde_json::Value {
    let exporter = ctx.app.core.exporters.all().into_iter().find(|exporter| exporter.name() == "notes").unwrap();
    exporter.export(user_id).await.unwrap()
}

fn ids(users: &[&TestUser]) -> Vec<String> {
    users.iter().map(|user| user.id.to_string()).collect()
}

#[tokio::test]
async fn lists_are_created_changed_and_deleted_by_their_owner() {
    let ctx = start().await;
    let (owner, other) = (ctx.member("owner").await, ctx.member("other").await);
    let list = create(&ctx, &owner, "  クラスの友達 ", false).await;
    assert_eq!(list.name, "クラスの友達");
    assert!(!list.public);
    assert_eq!(list.owner.id, owner.id.to_string());
    assert_eq!((list.member_count, list.follower_count), (0, 0));

    let updated: UpdateListResponse = call(
        &ctx,
        "UpdateList",
        Some(&owner),
        &UpdateListRequest {
            list_id: list.id.clone(),
            name: "部活".into(),
            description: "サッカー部のみんな".into(),
            public: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let updated = updated.list.as_option().unwrap();
    assert_eq!(
        (updated.name.as_str(), updated.description.as_str(), updated.public),
        ("部活", "サッカー部のみんな", true)
    );

    // Somebody else's list cannot be changed or deleted, and looks missing.
    let change = UpdateListRequest { list_id: list.id.clone(), name: "mine".into(), ..Default::default() };
    assert_eq!(reason(&ctx, "UpdateList", Some(&other), &change).await, "LIST_NOT_FOUND");
    let delete = DeleteListRequest { list_id: list.id.clone(), ..Default::default() };
    assert_eq!(reason(&ctx, "DeleteList", Some(&other), &delete).await, "LIST_NOT_FOUND");
    assert_eq!(
        reason(&ctx, "CreateList", None, &CreateListRequest { name: "x".into(), ..Default::default() }).await,
        "SIGN_IN_REQUIRED"
    );

    let _: DeleteListResponse = call(&ctx, "DeleteList", Some(&owner), &delete).await.unwrap();
    assert_eq!(get(&ctx, Some(&owner), &list).await.unwrap_err().reason, "LIST_NOT_FOUND");
    assert_eq!(reason(&ctx, "DeleteList", Some(&owner), &delete).await, "LIST_NOT_FOUND");
}

#[tokio::test]
async fn names_and_descriptions_are_validated() {
    let ctx = start().await;
    let owner = ctx.member("owner").await;
    let bad = |name: &str, description: &str| CreateListRequest {
        name: name.to_owned(),
        description: description.to_owned(),
        ..Default::default()
    };
    assert_eq!(reason(&ctx, "CreateList", Some(&owner), &bad("", "")).await, "INVALID_NAME");
    assert_eq!(reason(&ctx, "CreateList", Some(&owner), &bad(&"a".repeat(51), "")).await, "INVALID_NAME");
    assert_eq!(reason(&ctx, "CreateList", Some(&owner), &bad("ok", &"a".repeat(201))).await, "DESCRIPTION_TOO_LONG");
    let list = create(&ctx, &owner, "ok", false).await;
    let change = UpdateListRequest { list_id: list.id.clone(), name: " ".into(), ..Default::default() };
    assert_eq!(reason(&ctx, "UpdateList", Some(&owner), &change).await, "INVALID_NAME");
    let change = UpdateListRequest {
        list_id: list.id.clone(),
        name: "ok".into(),
        description: "a".repeat(201),
        ..Default::default()
    };
    assert_eq!(reason(&ctx, "UpdateList", Some(&owner), &change).await, "DESCRIPTION_TOO_LONG");
}

#[tokio::test]
async fn private_lists_are_only_their_owners() {
    let ctx = start().await;
    let (owner, member, stranger) =
        (ctx.member("owner").await, ctx.member("member").await, ctx.member("stranger").await);
    let secret = create(&ctx, &owner, "secret", false).await;
    let open = create(&ctx, &owner, "open", true).await;
    add(&ctx, &owner, &secret, &[&member]).await;

    for viewer in [Some(&member), Some(&stranger), None] {
        assert_eq!(get(&ctx, viewer, &secret).await.unwrap_err().reason, "LIST_NOT_FOUND");
        let request = ListListMembersRequest { list_id: secret.id.clone(), ..Default::default() };
        assert_eq!(reason(&ctx, "ListListMembers", viewer, &request).await, "LIST_NOT_FOUND");
        let request = GetListTimelineRequest { list_id: secret.id.clone(), ..Default::default() };
        assert_eq!(reason(&ctx, "GetListTimeline", viewer, &request).await, "LIST_NOT_FOUND");
        assert_eq!(lists_of(&ctx, viewer, &owner).await, vec!["open"]);
    }
    assert_eq!(lists_of(&ctx, Some(&owner), &owner).await, vec!["secret", "open"]);
    // Empty = the caller; anonymous callers must name someone.
    let mine: ListListsResponse = call(&ctx, "ListLists", Some(&owner), &ListListsRequest::default()).await.unwrap();
    assert_eq!(mine.lists.len(), 2);
    assert_eq!(reason(&ctx, "ListLists", None, &ListListsRequest::default()).await, "SIGN_IN_REQUIRED");
    let nobody = ListListsRequest { user_id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(reason(&ctx, "ListLists", Some(&owner), &nobody).await, "USER_NOT_FOUND");

    // A private list never tells anyone; following it is impossible.
    assert!(ctx.notifications_for(member.id, "notes.list_added").await.is_empty());
    let follow = FollowListRequest { list_id: secret.id.clone(), ..Default::default() };
    assert_eq!(reason(&ctx, "FollowList", Some(&stranger), &follow).await, "LIST_NOT_FOUND");
    let leave = LeaveListRequest { list_id: secret.id.clone(), ..Default::default() };
    assert_eq!(reason(&ctx, "LeaveList", Some(&member), &leave).await, "LIST_NOT_FOUND");
    let mine: ListListsContainingMeResponse =
        call(&ctx, "ListListsContainingMe", Some(&member), &ListListsContainingMeRequest::default()).await.unwrap();
    assert!(mine.lists.is_empty());
    assert!(get(&ctx, None, &open).await.is_ok(), "a public list is readable signed out");
}

#[tokio::test]
async fn members_are_added_removed_and_told_about_public_lists() {
    let ctx = start().await;
    let (owner, a, b) = (ctx.member("owner").await, ctx.member("a").await, ctx.member("b").await);
    let list = create(&ctx, &owner, "推し", true).await;
    let after = add(&ctx, &owner, &list, &[&a, &b, &a]).await;
    assert_eq!(after.member_count, 2);
    assert_eq!(members(&ctx, None, &list).await, ids(&[&b, &a]), "most recently added first");

    let told = ctx.notifications_for(a.id, "notes.list_added").await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["path"], format!("/notes/lists/{}", list.id));
    assert_eq!(told[0].data["collapse_key"], format!("notes.list_added:{}", list.id));
    // Taken off and put back: not told again.
    let remove = RemoveListMembersRequest { list_id: list.id.clone(), user_ids: ids(&[&a]), ..Default::default() };
    let removed: RemoveListMembersResponse = call(&ctx, "RemoveListMembers", Some(&owner), &remove).await.unwrap();
    assert_eq!(removed.list.member_count, 1);
    add(&ctx, &owner, &list, &[&a]).await;
    assert_eq!(ctx.notifications_for(a.id, "notes.list_added").await.len(), 1);

    // Only the owner changes members.
    assert_eq!(reason(&ctx, "AddListMembers", Some(&a), &add_request(&list, &[&b])).await, "LIST_NOT_FOUND");
    assert_eq!(reason(&ctx, "RemoveListMembers", Some(&a), &remove).await, "LIST_NOT_FOUND");
    // Unknown people, and too many at once.
    let unknown = AddListMembersRequest {
        list_id: list.id.clone(),
        user_ids: vec![Uuid::now_v7().to_string()],
        ..Default::default()
    };
    assert_eq!(reason(&ctx, "AddListMembers", Some(&owner), &unknown).await, "USER_NOT_FOUND");
    let crowd = AddListMembersRequest {
        list_id: list.id.clone(),
        user_ids: (0..51).map(|_| Uuid::now_v7().to_string()).collect(),
        ..Default::default()
    };
    assert_eq!(reason(&ctx, "AddListMembers", Some(&owner), &crowd).await, "TOO_MANY_MEMBERS");
}

#[tokio::test]
async fn members_leave_public_lists_and_see_the_lists_they_are_on() {
    let ctx = start().await;
    let (owner, me) = (ctx.member("owner").await, ctx.member("me").await);
    let list = create(&ctx, &owner, "みんな", true).await;
    add(&ctx, &owner, &list, &[&me]).await;
    let mine: ListListsContainingMeResponse =
        call(&ctx, "ListListsContainingMe", Some(&me), &ListListsContainingMeRequest::default()).await.unwrap();
    assert_eq!(mine.lists.len(), 1);
    assert!(mine.lists[0].is_member);

    let leave = LeaveListRequest { list_id: list.id.clone(), ..Default::default() };
    let _: LeaveListResponse = call(&ctx, "LeaveList", Some(&me), &leave).await.unwrap();
    assert!(members(&ctx, Some(&owner), &list).await.is_empty());
    let mine: ListListsContainingMeResponse =
        call(&ctx, "ListListsContainingMe", Some(&me), &ListListsContainingMeRequest::default()).await.unwrap();
    assert!(mine.lists.is_empty());
    assert_eq!(
        reason(
            &ctx,
            "LeaveList",
            Some(&me),
            &LeaveListRequest { list_id: Uuid::now_v7().to_string(), ..Default::default() }
        )
        .await,
        "LIST_NOT_FOUND"
    );
}

#[tokio::test]
async fn public_lists_are_followed_and_unfollowed() {
    let ctx = start().await;
    let (owner, fan) = (ctx.member("owner").await, ctx.member("fan").await);
    let first = create(&ctx, &owner, "first", true).await;
    let second = create(&ctx, &owner, "second", true).await;
    let follow = |list: &NoteList| FollowListRequest { list_id: list.id.clone(), ..Default::default() };
    let followed: FollowListResponse = call(&ctx, "FollowList", Some(&fan), &follow(&first)).await.unwrap();
    assert!(followed.list.following);
    assert_eq!(followed.list.follower_count, 1);
    // Following again changes nothing.
    let again: FollowListResponse = call(&ctx, "FollowList", Some(&fan), &follow(&first)).await.unwrap();
    assert_eq!(again.list.follower_count, 1);
    let _: FollowListResponse = call(&ctx, "FollowList", Some(&fan), &follow(&second)).await.unwrap();
    let mine: ListFollowedListsResponse = call(
        &ctx,
        "ListFollowedLists",
        Some(&fan),
        &ListFollowedListsRequest { page: page(1, ""), ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!(mine.lists.iter().map(|list| list.name.as_str()).collect::<Vec<_>>(), vec!["second"]);
    let next: ListFollowedListsResponse = call(
        &ctx,
        "ListFollowedLists",
        Some(&fan),
        &ListFollowedListsRequest { page: page(1, &mine.page.next_cursor), ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!(next.lists.iter().map(|list| list.name.as_str()).collect::<Vec<_>>(), vec!["first"]);
    assert!(!next.page.has_more);

    assert_eq!(reason(&ctx, "FollowList", Some(&owner), &follow(&first)).await, "CANNOT_FOLLOW_OWN_LIST");
    let unfollow = UnfollowListRequest { list_id: first.id.clone(), ..Default::default() };
    let _: UnfollowListResponse = call(&ctx, "UnfollowList", Some(&fan), &unfollow).await.unwrap();
    let _: UnfollowListResponse = call(&ctx, "UnfollowList", Some(&fan), &unfollow).await.unwrap();
    assert_eq!(get(&ctx, Some(&fan), &first).await.unwrap().follower_count, 0);

    // A list made private loses its followers.
    let hide =
        UpdateListRequest { list_id: second.id.clone(), name: "second".into(), public: false, ..Default::default() };
    let _: UpdateListResponse = call(&ctx, "UpdateList", Some(&owner), &hide).await.unwrap();
    let mine: ListFollowedListsResponse =
        call(&ctx, "ListFollowedLists", Some(&fan), &ListFollowedListsRequest::default()).await.unwrap();
    assert!(mine.lists.is_empty());
    assert_eq!(get(&ctx, Some(&owner), &second).await.unwrap().follower_count, 0);
}

#[tokio::test]
async fn too_many_followed_lists_are_refused() {
    let ctx = start().await;
    let (owner, fan) = (ctx.member("owner").await, ctx.member("fan").await);
    let list = create(&ctx, &owner, "one more", true).await;
    // 200 follows already, made directly (lists of a throwaway owner).
    sqlx::query(
        "WITH made AS (INSERT INTO note_lists (owner_id, name, public) SELECT $1, 'filler ' || g, true FROM generate_series(1, 200) g RETURNING id)
         INSERT INTO note_list_follows (list_id, user_id) SELECT id, $2 FROM made",
    )
    .bind(ctx.app.user("filler").await.id)
    .bind(fan.id)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    let follow = FollowListRequest { list_id: list.id.clone(), ..Default::default() };
    assert_eq!(reason(&ctx, "FollowList", Some(&fan), &follow).await, "TOO_MANY_FOLLOWED_LISTS");
}

#[tokio::test]
async fn nobody_is_added_across_a_block_and_a_block_unties_lists() {
    let ctx = start().await;
    let (owner, blocked, blocker, fan) =
        (ctx.member("owner").await, ctx.member("blocked").await, ctx.member("blocker").await, ctx.member("fan").await);
    ctx.block(&owner, &blocked).await;
    ctx.block(&blocker, &owner).await;
    let list = create(&ctx, &owner, "list", false).await;
    for user in [&blocked, &blocker] {
        assert_eq!(reason(&ctx, "AddListMembers", Some(&owner), &add_request(&list, &[user])).await, "USER_NOT_FOUND");
    }

    // An existing tie is cut both ways when the block happens.
    let owners_list = create(&ctx, &owner, "owner's", true).await;
    let fans_list = create(&ctx, &fan, "fan's", true).await;
    add(&ctx, &owner, &owners_list, &[&fan]).await;
    add(&ctx, &fan, &fans_list, &[&owner]).await;
    let _: FollowListResponse = call(
        &ctx,
        "FollowList",
        Some(&fan),
        &FollowListRequest { list_id: owners_list.id.clone(), ..Default::default() },
    )
    .await
    .unwrap();
    ctx.block(&fan, &owner).await;
    let event = ctx.app.events_by("social.user.blocked", fan.id).await.pop().expect("a block event");
    ctx.notes.handle_event(event.clone()).await.unwrap();
    ctx.notes.handle_event(event).await.unwrap();
    assert!(members(&ctx, Some(&owner), &owners_list).await.is_empty());
    assert!(members(&ctx, Some(&fan), &fans_list).await.is_empty());
    assert_eq!(get(&ctx, Some(&owner), &owners_list).await.unwrap().follower_count, 0);
    // And neither sees the other's list any more.
    assert_eq!(get(&ctx, Some(&fan), &owners_list).await.unwrap_err().reason, "LIST_NOT_FOUND");
    assert_eq!(get(&ctx, Some(&owner), &fans_list).await.unwrap_err().reason, "LIST_NOT_FOUND");
}

#[tokio::test]
async fn locked_accounts_go_on_public_lists_only_of_approved_followers() {
    let ctx = start().await;
    let (owner, approved, locked) =
        (ctx.member("owner").await, ctx.member("approved").await, ctx.member("locked").await);
    ctx.follow(&owner, &approved).await;
    ctx.set_locked(&approved, true).await;
    ctx.set_locked(&locked, true).await;
    let public = create(&ctx, &owner, "public", true).await;
    let private = create(&ctx, &owner, "private", false).await;
    assert_eq!(
        reason(&ctx, "AddListMembers", Some(&owner), &add_request(&public, &[&locked])).await,
        "MEMBER_NOT_ALLOWED"
    );
    add(&ctx, &owner, &public, &[&approved]).await;
    add(&ctx, &owner, &private, &[&locked, &approved]).await;

    // Making the private list public drops whoever would not be allowed.
    let open =
        UpdateListRequest { list_id: private.id.clone(), name: "private".into(), public: true, ..Default::default() };
    let _: UpdateListResponse = call(&ctx, "UpdateList", Some(&owner), &open).await.unwrap();
    assert_eq!(members(&ctx, Some(&owner), &private).await, ids(&[&approved]));
    assert!(ctx.notifications_for(locked.id, "notes.list_added").await.is_empty());
}

#[tokio::test]
async fn the_timeline_shows_members_as_the_viewer_may_see_them() {
    let ctx = start().await;
    let (owner, a, b, outsider, stranger) = (
        ctx.member("owner").await,
        ctx.member("a").await,
        ctx.member("b").await,
        ctx.member("outsider").await,
        ctx.member("stranger").await,
    );
    ctx.follow(&owner, &a).await;
    let list = create(&ctx, &owner, "tl", true).await;
    add(&ctx, &owner, &list, &[&a, &b]).await;
    let first = ctx.post(&a, "a: hello").await;
    ctx.post_visible_to(&a, "a: followers only", Visibility::VISIBILITY_FOLLOWERS).await;
    ctx.reply(&b, &first, "b answers a").await;
    let outside = ctx.post(&outsider, "outsider").await;
    ctx.reply(&a, &outside, "a answers an outsider").await;
    ctx.renote(&b, &outside).await;

    assert_eq!(
        timeline(&ctx, Some(&owner), &list).await,
        vec!["outsider", "b answers a", "a: followers only", "a: hello"]
    );
    // A stranger, signed in or not, never sees the followers-only note.
    for viewer in [Some(&stranger), None] {
        assert_eq!(timeline(&ctx, viewer, &list).await, vec!["outsider", "b answers a", "a: hello"]);
    }
    // A member the viewer blocked is left out of the viewer's view, and
    // out of the member count they are shown.
    ctx.block(&stranger, &b).await;
    assert_eq!(timeline(&ctx, Some(&stranger), &list).await, vec!["a: hello"]);
    assert_eq!(get(&ctx, Some(&stranger), &list).await.unwrap().member_count, 1);
    assert_eq!(members(&ctx, Some(&stranger), &list).await, ids(&[&a]));
    assert_eq!(get(&ctx, None, &list).await.unwrap().member_count, 2);

    // The owner muted `a` as a followee: gone from the owner's view, and
    // with them the replies to them.
    let mute = tokuly_proto::social::UpdateFollowPrefsRequest {
        user_id: a.id.to_string(),
        muted: Some(true),
        ..Default::default()
    };
    let _: tokuly_proto::social::UpdateFollowPrefsResponse =
        ctx.app.call("tokuly.social.v1.FollowService/UpdateFollowPrefs", Some(&owner), &mute).await.unwrap();
    assert_eq!(timeline(&ctx, Some(&owner), &list).await, vec!["outsider"]);
}

#[tokio::test]
async fn too_many_lists_are_refused_even_when_asked_at_once() {
    let ctx = start().await;
    let owner = ctx.member("owner").await;
    sqlx::query("INSERT INTO note_lists (owner_id, name) SELECT $1, 'filler ' || g FROM generate_series(1, 47) g")
        .bind(owner.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let request = CreateListRequest { name: "race".into(), ..Default::default() };
    let attempts = futures::future::join_all(
        (0..6).map(|_| call::<_, CreateListResponse>(&ctx, "CreateList", Some(&owner), &request)),
    )
    .await;
    let made = attempts.iter().filter(|attempt| attempt.is_ok()).count();
    assert_eq!(made, 3);
    assert!(attempts.iter().filter_map(|attempt| attempt.as_ref().err()).all(|e| e.reason == "TOO_MANY_LISTS"));
}

#[tokio::test]
async fn the_member_limit_holds_under_concurrent_adds() {
    let ctx = start().await;
    let owner = ctx.member("owner").await;
    let list = create(&ctx, &owner, "big", false).await;
    let tag = unique("fill");
    // 490 members already.
    sqlx::query(
        "WITH made AS (
             INSERT INTO users (handle, display_name, email, email_verified_at, birthdate)
             SELECT $2 || g, 'filler', $2 || g || '@test.tokuly.localhost', now(), '2000-01-01' FROM generate_series(1, 490) g
             RETURNING id)
         INSERT INTO note_list_members (list_id, user_id) SELECT $1, id FROM made",
    )
    .bind(Uuid::parse_str(&list.id).unwrap())
    .bind(&tag)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    let mut batches = Vec::new();
    for batch in 0..5 {
        let mut users = Vec::new();
        for index in 0..4 {
            users.push(ctx.app.user(&format!("new{batch}x{index}")).await);
        }
        batches.push(users);
    }
    // As many calls at once as the test pool has connections.
    let (ctx, owner) = (&ctx, &owner);
    let attempts = futures::future::join_all(batches.iter().map(|users| {
        let users: Vec<&TestUser> = users.iter().collect();
        let request = add_request(&list, &users);
        async move { call::<_, AddListMembersResponse>(ctx, "AddListMembers", Some(owner), &request).await }
    }))
    .await;
    assert_eq!(attempts.iter().filter(|attempt| attempt.is_ok()).count(), 2, "{attempts:?}");
    assert!(attempts.iter().filter_map(|attempt| attempt.as_ref().err()).all(|e| e.reason == "TOO_MANY_MEMBERS"));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM note_list_members WHERE list_id = $1")
        .bind(Uuid::parse_str(&list.id).unwrap())
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(count, 498);
}

#[tokio::test]
async fn an_erased_account_leaves_no_lists_behind() {
    let ctx = start().await;
    let (gone, owner) = (ctx.member("gone").await, ctx.member("owner").await);
    let theirs = create(&ctx, &gone, "theirs", true).await;
    let ours = create(&ctx, &owner, "ours", true).await;
    add(&ctx, &owner, &ours, &[&gone]).await;
    let _: FollowListResponse =
        call(&ctx, "FollowList", Some(&gone), &FollowListRequest { list_id: ours.id.clone(), ..Default::default() })
            .await
            .unwrap();
    let export = notes_export(&ctx, gone.id).await;
    assert_eq!(export["lists"][0]["name"], "theirs");
    assert_eq!(export["list_follows"][0]["list_id"], ours.id);

    let event = event("identity.user.deleted", serde_json::json!({ "user_id": gone.id }));
    ctx.notes.handle_event(event.clone()).await.unwrap();
    ctx.notes.handle_event(event).await.unwrap();
    assert_eq!(get(&ctx, Some(&owner), &theirs).await.unwrap_err().reason, "LIST_NOT_FOUND");
    let ours_now = get(&ctx, Some(&owner), &ours).await.unwrap();
    assert_eq!((ours_now.member_count, ours_now.follower_count), (0, 0));
}

#[tokio::test]
async fn bad_ids_are_rejected() {
    let ctx = start().await;
    let owner = ctx.member("owner").await;
    let request = GetListRequest { list_id: "nope".into(), ..Default::default() };
    assert_eq!(reason(&ctx, "GetList", Some(&owner), &request).await, "INVALID_ID");
    let request = GetListRequest { list_id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(reason(&ctx, "GetList", None, &request).await, "LIST_NOT_FOUND");
}

#[tokio::test]
async fn follows_count_once_and_stop_at_the_limit_under_concurrency() {
    let ctx = start().await;
    let (owner, fan) = (ctx.member("owner").await, ctx.member("fan").await);
    let list = create(&ctx, &owner, "popular", true).await;
    let follow = FollowListRequest { list_id: list.id.clone(), ..Default::default() };
    let attempts = futures::future::join_all(
        (0..5).map(|_| call::<_, FollowListResponse>(&ctx, "FollowList", Some(&fan), &follow)),
    )
    .await;
    assert!(attempts.iter().all(Result::is_ok), "{attempts:?}");
    assert_eq!(get(&ctx, Some(&owner), &list).await.unwrap().follower_count, 1);

    // 198 more follows (199 in all), then five new lists at once: one fits.
    sqlx::query(
        "WITH made AS (INSERT INTO note_lists (owner_id, name, public) SELECT $1, 'filler ' || g, true FROM generate_series(1, 198) g RETURNING id)
         INSERT INTO note_list_follows (list_id, user_id) SELECT id, $2 FROM made",
    )
    .bind(ctx.app.user("filler").await.id)
    .bind(fan.id)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    let mut more = Vec::new();
    for index in 0..5 {
        more.push(create(&ctx, &owner, &format!("more {index}"), true).await);
    }
    let attempts = futures::future::join_all(more.iter().map(|list| {
        let request = FollowListRequest { list_id: list.id.clone(), ..Default::default() };
        let (ctx, fan) = (&ctx, &fan);
        async move { call::<_, FollowListResponse>(ctx, "FollowList", Some(fan), &request).await }
    }))
    .await;
    assert_eq!(attempts.iter().filter(|attempt| attempt.is_ok()).count(), 1, "{attempts:?}");
    assert!(
        attempts.iter().filter_map(|attempt| attempt.as_ref().err()).all(|e| e.reason == "TOO_MANY_FOLLOWED_LISTS")
    );
}
