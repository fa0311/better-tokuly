//! `MomentAdminService`: an admin sends a region's prompt of today now.
//!
//! A binary of its own, so a database of its own: these tests prompt the
//! real bands, which the `moments` binary never does (its accounts live in
//! regions of their own). Each test takes a band of its own; everyone made
//! here lives in Tokyo (`utc+7..+11`), the testkit's time zone.
#[path = "../moments/common.rs"]
mod common;

use chrono::{NaiveDate, Utc};
use tokuly_moments::{KIND_PROMPT, Sent, rules};
use tokuly_proto::moments::*;
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

use crate::common::*;

const SERVICE: &str = "tokuly.moments.v1.MomentAdminService";

impl Ctx {
    async fn send_now(&self, user: Option<&TestUser>, region: &str) -> Result<SendPromptNowResponse, RpcError> {
        let request = SendPromptNowRequest { region: region.into(), ..Default::default() };
        self.app.call(&format!("{SERVICE}/SendPromptNow"), user, &request).await
    }

    async fn regions(&self, user: &TestUser) -> Vec<PromptRegion> {
        let response: ListPromptRegionsResponse = self
            .app
            .call(&format!("{SERVICE}/ListPromptRegions"), Some(user), &ListPromptRegionsRequest::default())
            .await
            .expect("regions");
        response.regions
    }

    /// The band's day of today, drawn the way the planner draws it.
    async fn today_of_band(&self, region: &str) -> (Uuid, NaiveDate) {
        self.moments.refresh_regions().await.unwrap();
        self.moments.draw_days(Utc::now()).await.unwrap();
        let band = rules::BANDS.iter().find(|band| band.name == region).unwrap();
        let date = rules::band_date(band, Utc::now());
        let id = sqlx::query_scalar("SELECT id FROM moment_days WHERE region = $1 AND date = $2")
            .bind(region)
            .bind(date)
            .fetch_one(&self.app.core.db)
            .await
            .unwrap();
        (id, date)
    }
}

#[tokio::test]
async fn only_admins_send_a_prompt_now() {
    let ctx = start().await;
    let user = ctx.app.user("ふつう").await;
    let moderator = ctx.app.user_with_roles("もでれ", &["moderator"]).await;
    for (caller, code) in
        [(None, "unauthenticated"), (Some(&user), "permission_denied"), (Some(&moderator), "permission_denied")]
    {
        let err = ctx.send_now(caller, "utc-11..-7").await.unwrap_err();
        assert_eq!(err.code, code);
        let err = ctx
            .app
            .call::<_, ListPromptRegionsResponse>(
                &format!("{SERVICE}/ListPromptRegions"),
                caller,
                &ListPromptRegionsRequest::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code);
    }
    let admin = ctx.app.user_with_roles("かんり", &["admin"]).await;
    for region in ["", "test/nowhere", "Asia/Tokyo"] {
        assert_eq!(ctx.send_now(Some(&admin), region).await.unwrap_err().reason, "REGION_NOT_FOUND");
    }
}

#[tokio::test]
async fn an_admin_sends_todays_prompt_now_once() {
    let ctx = start().await;
    let region = "utc+7..+11";
    let admin = ctx.app.user_with_roles("かんり", &["admin"]).await;
    let (aoi, ren) = (ctx.app.user("あおい").await, ctx.app.user("れん").await);
    let (day, date) = ctx.today_of_band(region).await;
    // Far ahead, so the planner would not send it for hours.
    sqlx::query("UPDATE moment_days SET prompt_at = now() + interval '3 hours' WHERE id = $1")
        .bind(day)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();

    // Pressed twice at once, by two tabs: one sends it, the other is told it went out.
    let runs = futures::future::join_all((0..4).map(|_| ctx.send_now(Some(&admin), region))).await;
    let (sent, refused): (Vec<_>, Vec<_>) = runs.into_iter().partition(Result::is_ok);
    assert_eq!(sent.len(), 1, "{refused:?}");
    assert!(refused.iter().all(|run| run.as_ref().unwrap_err().reason == "PROMPT_ALREADY_SENT"));
    let shown = sent.into_iter().next().unwrap().unwrap().region.into_option().unwrap();
    assert_eq!((shown.region.as_str(), shown.date.as_str()), (region, date.to_string().as_str()));
    assert!(shown.sent_at.as_option().is_some());
    assert_eq!(shown.sent_by, admin.id.to_string());
    assert_eq!(ctx.send_now(Some(&admin), region).await.unwrap_err().reason, "PROMPT_ALREADY_SENT");

    // The day started as the planner's job starts it: once, with the admin as its actor …
    let started: Vec<_> = ctx
        .app
        .events("moments.prompt.sent")
        .await
        .into_iter()
        .filter(|event| event.data["day_id"] == day.to_string())
        .collect();
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].actor_id, Some(admin.id));
    // … it is on its way to the region already, and the job tells everyone once.
    assert_eq!(ctx.today(&aoi).await.day.as_option().unwrap().date, date.to_string());
    assert!(matches!(ctx.moments.send_prompt(day).await.unwrap(), Sent::Told(_)));
    assert_eq!(ctx.moments.send_prompt(day).await.unwrap(), Sent::Told(0));
    for user in [&aoi, &ren] {
        assert_eq!(ctx.notifications(user, KIND_PROMPT).await.len(), 1, "{}", user.handle);
    }
    // The planner has nothing left to do for it.
    assert!(!ctx.moments.due_days().await.unwrap().contains(&day));

    let listed = ctx.regions(&admin).await;
    let names: Vec<&str> = listed.iter().map(|row| row.region.as_str()).collect();
    assert_eq!(names, rules::BANDS.iter().map(|band| band.name).collect::<Vec<_>>());
    let row = listed.iter().find(|row| row.region == region).unwrap();
    assert_eq!((row.date.as_str(), row.sent_by.as_str()), (date.to_string().as_str(), admin.id.to_string().as_str()));
    assert!(row.sent_at.as_option().is_some());
}

#[tokio::test]
async fn a_prompt_the_planner_sent_is_not_sent_again() {
    let ctx = start().await;
    let region = "utc-6..-2";
    let admin = ctx.app.user_with_roles("かんり", &["admin"]).await;
    let (day, _) = ctx.today_of_band(region).await;
    sqlx::query("UPDATE moment_days SET prompt_at = now() - interval '1 second' WHERE id = $1")
        .bind(day)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    // Due and not yet out: the job is on its way, so it is not the admin's to send.
    assert_eq!(ctx.send_now(Some(&admin), region).await.unwrap_err().reason, "PROMPT_ALREADY_SENT");
    assert!(matches!(ctx.moments.send_prompt(day).await.unwrap(), Sent::Told(_)));
    assert_eq!(ctx.send_now(Some(&admin), region).await.unwrap_err().reason, "PROMPT_ALREADY_SENT");

    let row = ctx.regions(&admin).await.into_iter().find(|row| row.region == region).unwrap();
    assert!(row.sent_at.as_option().is_some());
    assert_eq!(row.sent_by, "", "the planner sent it");
}

#[tokio::test]
async fn a_skipped_day_can_still_go_out() {
    let ctx = start().await;
    let region = "utc+12..+14";
    let admin = ctx.app.user_with_roles("かんり", &["admin"]).await;
    let (day, _) = ctx.today_of_band(region).await;
    // Workers were down past its time: the job skipped it.
    sqlx::query("UPDATE moment_days SET prompt_at = now() - interval '2 hours' WHERE id = $1")
        .bind(day)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(ctx.moments.send_prompt(day).await.unwrap(), Sent::Skipped);
    assert!(
        ctx.regions(&admin).await.into_iter().find(|row| row.region == region).unwrap().sent_at.as_option().is_none()
    );

    let sent = ctx.send_now(Some(&admin), region).await.unwrap().region.into_option().unwrap();
    assert!(sent.sent_at.as_option().is_some());
    assert!(matches!(ctx.moments.send_prompt(day).await.unwrap(), Sent::Told(_)));
    assert_eq!(ctx.send_now(Some(&admin), region).await.unwrap_err().reason, "PROMPT_ALREADY_SENT");
}
