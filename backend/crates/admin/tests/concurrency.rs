//! Bursts from one staff account: nothing that holds a transaction asks the
//! pool for a second connection, and racing changes end in a state that
//! makes sense.
mod common;

use std::time::{Duration, Instant};

use common::*;
use futures::future::join_all;
use tokuly_proto::admin::*;

/// Twice the test pool (5 connections).
const BURST: usize = 10;

#[tokio::test]
async fn a_burst_of_suspensions_from_one_moderator_does_not_starve_the_pool() {
    let (app, admin) = start().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let mut targets = Vec::new();
    for i in 0..BURST {
        targets.push(app.user(&format!("target{i}")).await);
    }
    let started = Instant::now();

    // Direct suspensions, each locking the account while identity locks it too.
    let (app, moderator) = (&app, &moderator);
    let suspended = join_all(targets.iter().map(|target| async move {
        let request = SuspendUserRequest {
            user_id: target.id.to_string(),
            reason: "スパム".into(),
            suspension_seconds: 86_400,
            ..Default::default()
        };
        app.call::<_, SuspendUserResponse>(&format!("{USERS}/SuspendUser"), Some(moderator), &request).await
    }))
    .await;
    for result in &suspended {
        assert!(result.is_ok(), "{result:?}");
    }

    // Lifting them again, while the expiry worker runs alongside.
    sqlx::query("UPDATE user_suspensions SET expires_at = now() - interval '1 second' WHERE user_id = ANY($1) AND ended_at IS NULL")
        .bind(targets.iter().take(BURST / 2).map(|t| t.id).collect::<Vec<_>>())
        .execute(&app.core.db)
        .await
        .unwrap();
    let lifts = targets.iter().skip(BURST / 2).map(|target| async move {
        let request = UnsuspendUserRequest { user_id: target.id.to_string(), ..Default::default() };
        app.call::<_, UnsuspendUserResponse>(&format!("{USERS}/UnsuspendUser"), Some(moderator), &request).await
    });
    let (lifted, expired) =
        tokio::join!(join_all(lifts), async { join_all((0..3).map(|_| admin.expire_due_suspensions())).await });
    for result in &lifted {
        assert!(result.is_ok(), "{result:?}");
    }
    assert_eq!(
        expired.into_iter().map(|r| r.unwrap()).sum::<u64>(),
        (BURST / 2) as u64,
        "every due suspension is lifted exactly once"
    );

    // Cases resolved with a ban: the case row is locked while the owner is.
    let reporter = app.user("rep").await;
    let mut cases = Vec::new();
    for owner in &targets {
        cases.push(open_case(app, &admin, moderator, &reporter, owner).await);
    }
    let bans = join_all(cases.iter().map(|case| {
        resolve(app, moderator, resolution(case, ModerationActionKind::MODERATION_ACTION_KIND_BAN, "スパム"))
    }))
    .await;
    for result in &bans {
        assert!(result.is_ok(), "{result:?}");
    }
    for target in &targets {
        assert!(is_banned(app, target.id).await);
    }
    // The pool's acquire timeout is 10 s: a starved request would take at least that long.
    assert!(started.elapsed() < Duration::from_secs(8), "took {:?}", started.elapsed());
}

#[tokio::test]
async fn two_admins_demoting_each_other_at_once_leave_one_of_them_admin() {
    let (app, _admin) = start().await;
    for round in 0..5 {
        let a = app.user_with_roles("a", &["admin"]).await;
        let b = app.user_with_roles("b", &["admin"]).await;
        let demote = |by: &tokuly_testkit::TestUser, whom: &tokuly_testkit::TestUser| {
            let (app, by, request) =
                (&app, by.clone(), SetUserRolesRequest { user_id: whom.id.to_string(), ..Default::default() });
            async move { app.call::<_, SetUserRolesResponse>(&format!("{USERS}/SetUserRoles"), Some(&by), &request).await }
        };
        let (a_demotes_b, b_demotes_a) = tokio::join!(demote(&a, &b), demote(&b, &a));
        assert!(!(a_demotes_b.is_ok() && b_demotes_a.is_ok()), "round {round}: both admins lost the role");
        let admins: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE id = ANY($1) AND 'admin' = ANY(roles)")
            .bind(vec![a.id, b.id])
            .fetch_one(&app.core.db)
            .await
            .unwrap();
        assert_eq!(admins, 1, "round {round}");
    }
}
