//! The background workers as the worker process runs them: started with
//! `spawn_workers`, driven by the clock and fed through the event stream.
//! (What they do is tested in detail in `tests/background.rs`; this checks
//! the wiring.)
mod common;

use std::time::Duration;

use chrono::Utc;
use common::*;
use tokuly_core::events;
use tokuly_plans::PlansConfig;
use tokuly_proto::{common::SubjectType, plans::*};

#[tokio::test]
async fn workers_send_what_is_due_and_consume_events() {
    let ctx = start_with(PlansConfig { worker_interval_seconds: 1, ..PlansConfig::default() }).await;
    let core = &ctx.app.core;
    ctx.plans.spawn_workers();
    // The worker process also runs the relay that moves outbox rows to the event stream.
    tokio::spawn(events::run_outbox_relay(core.db.clone(), core.bus.clone(), core.shutdown.clone()));

    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);

    // The clock: a status whose window is over is removed by the next pass.
    let status = SetFreeStatusRequest {
        window: FreeWindow::FREE_WINDOW_NOW.into(),
        audience: FreeAudience::FREE_AUDIENCE_FRIENDS.into(),
        ..Default::default()
    };
    let _: SetFreeStatusResponse = ctx.call(FREE, "SetFreeStatus", mika, &status).await.unwrap();
    ctx.sql("UPDATE free_statuses SET starts_at = now() - interval '4 hours', ends_at = now() - interval '1 minute' WHERE user_id = $1", mika.id).await;
    let mut left: i64 = 1;
    for _ in 0..120 {
        left = sqlx::query_scalar("SELECT count(*) FROM free_statuses WHERE user_id = $1")
            .bind(mika.id)
            .fetch_one(&core.db)
            .await
            .unwrap();
        if left == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(left, 0, "the worker removed the expired status");

    // The event stream: a moderator's takedown reaches the consumer.
    let plan = ctx.create_plan(mika, plan_at("よくない予定", Utc::now() + chrono::Duration::days(2), &[yui])).await;
    let removed = serde_json::json!({ "subject_type": SubjectType::SUBJECT_TYPE_PLAN as i32, "subject_id": plan.id, "reason": "spam", "moderator_id": yui.id });
    core.emit("admin.content.removed", Some(yui.id), &removed).await.unwrap();
    let mut gone = false;
    for _ in 0..120 {
        gone = ctx
            .call::<_, GetPlanResponse>(
                PLANS,
                "GetPlan",
                mika,
                &GetPlanRequest { plan_id: plan.id.clone(), ..Default::default() },
            )
            .await
            .is_err();
        if gone {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(gone, "the event consumer took the plan down");

    core.shutdown.cancel();
}
