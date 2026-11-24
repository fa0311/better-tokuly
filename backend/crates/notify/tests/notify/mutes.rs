//! `PreferencesService`: per-person mutes.
use tokuly_proto::{common::PageRequest, notify::*};
use tokuly_testkit::TestUser;
use uuid::Uuid;

use crate::common::*;

pub const MUTE: &str = "tokuly.notify.v1.PreferencesService/MuteUser";
pub const UNMUTE: &str = "tokuly.notify.v1.PreferencesService/UnmuteUser";
pub const LIST_MUTED: &str = "tokuly.notify.v1.PreferencesService/ListMutedUsers";

async fn mute(ctx: &Ctx, user: &TestUser, target: &str) -> Result<MuteUserResponse, tokuly_testkit::RpcError> {
    ctx.app.call(MUTE, Some(user), &MuteUserRequest { user_id: target.to_owned(), ..Default::default() }).await
}

async fn unmute(ctx: &Ctx, user: &TestUser, target: &str) -> Result<UnmuteUserResponse, tokuly_testkit::RpcError> {
    ctx.app.call(UNMUTE, Some(user), &UnmuteUserRequest { user_id: target.to_owned(), ..Default::default() }).await
}

async fn muted(ctx: &Ctx, user: &TestUser, page: PageRequest) -> ListMutedUsersResponse {
    ctx.app
        .call(LIST_MUTED, Some(user), &ListMutedUsersRequest { page: page.into(), ..Default::default() })
        .await
        .unwrap()
}

#[tokio::test]
async fn muting_is_idempotent_and_announced() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);

    mute(&ctx, &alice, &bob.id.to_string()).await.unwrap();
    mute(&ctx, &alice, &bob.id.to_string()).await.unwrap();
    let list = muted(&ctx, &alice, PageRequest::default()).await;
    assert_eq!(list.users.len(), 1);
    let entry = &list.users[0];
    assert_eq!(entry.user.as_option().unwrap().id, bob.id.to_string());
    assert_eq!(entry.user.as_option().unwrap().handle, bob.handle);
    assert!(entry.muted_at.as_option().is_some());
    assert!(muted(&ctx, &bob, PageRequest::default()).await.users.is_empty(), "mutes are one-directional and private");

    unmute(&ctx, &alice, &bob.id.to_string()).await.unwrap();
    unmute(&ctx, &alice, &bob.id.to_string()).await.unwrap();
    assert!(muted(&ctx, &alice, PageRequest::default()).await.users.is_empty());

    // One event per actual change, so the friends list can follow.
    let events = ctx.app.events_by("notify.mute.changed", alice.id).await;
    let changes: Vec<(String, bool)> = events
        .iter()
        .map(|e| (e.data["muted_user_id"].as_str().unwrap().to_owned(), e.data["muted"].as_bool().unwrap()))
        .collect();
    assert_eq!(changes, [(bob.id.to_string(), true), (bob.id.to_string(), false)]);
    assert_eq!(events[0].data["user_id"], alice.id.to_string());
}

#[tokio::test]
async fn mute_targets_are_validated() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;

    let err = mute(&ctx, &alice, &alice.id.to_string()).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("invalid_argument", "CANNOT_MUTE_SELF"));
    let err = mute(&ctx, &alice, &Uuid::now_v7().to_string()).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("not_found", "USER_NOT_FOUND"));
    let deleted = ctx.app.user("deleted").await;
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(deleted.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(mute(&ctx, &alice, &deleted.id.to_string()).await.unwrap_err().reason, "USER_NOT_FOUND");
    assert_eq!(mute(&ctx, &alice, "not-an-id").await.unwrap_err().reason, "INVALID_ID");
    assert_eq!(unmute(&ctx, &alice, "not-an-id").await.unwrap_err().reason, "INVALID_ID");
    // Unmuting someone who was never muted (or does not exist) is a no-op.
    unmute(&ctx, &alice, &Uuid::now_v7().to_string()).await.unwrap();
    assert!(ctx.app.events_by("notify.mute.changed", alice.id).await.is_empty());
}

#[tokio::test]
async fn muted_users_are_listed_most_recent_first_in_pages() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let mut targets = Vec::new();
    for index in 0..5 {
        let target = ctx.app.user(&format!("target{index}")).await;
        mute(&ctx, &alice, &target.id.to_string()).await.unwrap();
        targets.push(target.id.to_string());
    }
    targets.reverse(); // most recently muted first

    let ids = |response: &ListMutedUsersResponse| {
        response.users.iter().map(|m| m.user.as_option().unwrap().id.clone()).collect::<Vec<_>>()
    };
    let first = muted(&ctx, &alice, PageRequest { limit: 2, ..Default::default() }).await;
    assert_eq!(ids(&first), targets[..2]);
    let info = first.page.as_option().unwrap();
    assert!(info.has_more);
    let second =
        muted(&ctx, &alice, PageRequest { limit: 2, cursor: info.next_cursor.clone(), ..Default::default() }).await;
    assert_eq!(ids(&second), targets[2..4]);
    let third = muted(
        &ctx,
        &alice,
        PageRequest { limit: 2, cursor: second.page.as_option().unwrap().next_cursor.clone(), ..Default::default() },
    )
    .await;
    assert_eq!(ids(&third), targets[4..]);
    assert!(!third.page.as_option().unwrap().has_more);

    let err = ctx
        .app
        .call::<_, ListMutedUsersResponse>(
            LIST_MUTED,
            Some(&alice),
            &ListMutedUsersRequest {
                page: PageRequest { cursor: "garbage!".into(), ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.reason, "INVALID_CURSOR");
}
