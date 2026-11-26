//! A reported message keeps its uploads for the moderation queue: unsending
//! it does not take the evidence along (`tokuly_chat::hold_message_media`,
//! used by the server's `ReportEvidence` bridge for the admin domain).
mod common;

use std::sync::Arc;

use async_trait::async_trait;
use common::*;
use sqlx::PgConnection;
use tokuly_admin::{Admin, AdminConfig, ReportEvidence};
use tokuly_chat::Chat;
use tokuly_core::{AppResult, events::Event};
use tokuly_proto::{chat::*, common::SubjectType};
use tokuly_social::{Social, SocialConfig};
use tokuly_stamp::{Stamp, StampConfig};
use tokuly_testkit::TestApp;
use uuid::Uuid;

/// What the server hands the admin domain (`bridges.rs`, `EvidenceOfReports`).
struct Evidence;

#[async_trait]
impl ReportEvidence for Evidence {
    async fn hold(
        &self,
        conn: &mut PgConnection,
        subject_type: SubjectType,
        subject_id: Uuid,
        holder: &str,
    ) -> AppResult<()> {
        if subject_type == SubjectType::SUBJECT_TYPE_CHAT_MESSAGE {
            tokuly_chat::hold_message_media(conn, subject_id, holder).await?;
        }
        Ok(())
    }

    async fn release(&self, conn: &mut PgConnection, holder: &str) -> AppResult<()> {
        tokuly_media::assets::detach_holder(conn, holder).await?;
        Ok(())
    }
}

async fn start_with_admin() -> (Ctx, Admin) {
    let mut domains = None;
    let migrations: Vec<&str> = MIGRATIONS.iter().copied().chain(["1600"]).collect();
    let app = TestApp::start(&migrations, |core, identity, router| {
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social domain");
        let stamp =
            Stamp::new(core.clone(), identity.clone(), StampConfig { seed_official: false, ..Default::default() })
                .expect("stamp domain");
        let chat = Chat::new(core.clone(), identity.clone(), test_config()).expect("chat domain");
        let admin = Admin::new(core.clone(), identity.clone(), AdminConfig::default()).expect("admin domain");
        let router = admin.register(chat.register(stamp.register(social.register(identity.register(router)))));
        domains = Some((chat, admin));
        router
    })
    .await;
    let (chat, admin) = domains.expect("register ran");
    admin.set_report_evidence(Arc::new(Evidence));
    (Ctx { app, chat }, admin)
}

#[tokio::test]
async fn unsending_a_reported_message_keeps_its_photo_for_the_case() {
    let (ctx, admin) = start_with_admin().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation = ctx.direct(&taro, &hanako).await;
    let photo = ctx.photo(&taro).await;
    let request = SendMessageRequest {
        conversation_id: conversation.id.clone(),
        client_id: client_id(),
        attachment_asset_ids: vec![photo.clone()],
        ..Default::default()
    };
    let message = ctx.send(&taro, &request).await.unwrap().message.into_option().unwrap();
    let photo_id: Uuid = photo.parse().unwrap();

    // Hanako reports it; the queue files the report.
    let report = Event {
        id: Uuid::now_v7(),
        topic: "engagement.report.created".into(),
        actor_id: Some(hanako.id),
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({
            "report_id": Uuid::now_v7(),
            "reporter_id": hanako.id,
            "subject_type": SubjectType::SUBJECT_TYPE_CHAT_MESSAGE as i32,
            "subject_id": message.id,
            "subject_owner_id": taro.id,
            "category": "HARASSMENT",
        }),
    };
    admin.handle_event(report).await.unwrap();
    let case_id: Uuid = sqlx::query_scalar("SELECT id FROM moderation_cases WHERE subject_id = $1::uuid")
        .bind(&message.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();

    // Taro unsends it: the message lets go of the photo, the case does not.
    let _: DeleteMessageResponse = ctx
        .call(
            "Message",
            "DeleteMessage",
            &taro,
            &DeleteMessageRequest { message_id: message.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let holders: Vec<String> = sqlx::query_scalar("SELECT holder FROM media_asset_holders WHERE asset_id = $1")
        .bind(photo_id)
        .fetch_all(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(holders, vec![format!("admin:case:{case_id}")]);
    assert!(tokuly_media::assets::is_attached(&ctx.app.core, photo_id).await.unwrap(), "the janitor leaves it alone");

    // Long after the case is resolved, it lets go too.
    sqlx::query(
        "UPDATE moderation_cases SET status = 'resolved', resolved_at = now() - interval '91 days' WHERE id = $1",
    )
    .bind(case_id)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    admin.release_due_evidence().await.unwrap();
    assert!(!tokuly_media::assets::is_attached(&ctx.app.core, photo_id).await.unwrap());
}
