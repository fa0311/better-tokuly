//! Plans as a subject (`SUBJECT_TYPE_PLAN`), so that comments, reactions and
//! reports of the engagement domain can attach to a plan.
//!
//! A plan resolves for the people in it: to everyone else it does not exist,
//! exactly as in this domain's own RPCs. One exception keeps reporting
//! possible: someone who *was* in the plan — removed, uninvited when a
//! friendship ended or a block was set, or gone by their own choice — still
//! resolves it, as they last saw it and with interactions off. Otherwise an
//! abusive invitation could be made unreportable by withdrawing it, and
//! blocking its sender would do the same.
use async_trait::async_trait;
use tokuly_core::{
    prelude::*,
    subject::{SubjectInfo, SubjectResolver},
};

use crate::{Plans, plan::plan_path};

#[async_trait]
impl SubjectResolver for Plans {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        let (Ok(plan_id), Some(viewer)) = (Uuid::parse_str(id), viewer) else { return Ok(None) };
        match self.plan_access(plan_id, viewer).await {
            Ok(access) => Ok(Some(SubjectInfo {
                id: plan_id,
                owner_id: access.plan.host_id,
                title: access.plan.name(),
                path: plan_path(plan_id),
                image_url: None,
                // A cancelled plan stays readable, but the conversation is over.
                interactions_enabled: access.plan.is_active(),
            })),
            Err(e) if e.is_not_found() => {
                // Not in the plan (any more). The title is the one they saw
                // when they left: later changes are none of their business.
                let past: Option<(Uuid, String)> = sqlx::query_as(
                    "SELECT p.host_id, f.title FROM plan_past_participants f JOIN plans p ON p.id = f.plan_id
                     WHERE f.plan_id = $1 AND f.user_id = $2 AND p.removed_at IS NULL",
                )
                .bind(plan_id)
                .bind(viewer)
                .fetch_optional(&self.core.db)
                .await?;
                Ok(past.map(|(host_id, title)| SubjectInfo {
                    id: plan_id,
                    owner_id: host_id,
                    title,
                    path: plan_path(plan_id),
                    image_url: None,
                    interactions_enabled: false,
                }))
            }
            Err(e) => Err(e),
        }
    }

    async fn counter_changed(&self, id: Uuid, counter: &str, delta: i64) -> AppResult<()> {
        if counter == "comment_count" {
            sqlx::query("UPDATE plans SET comment_count = GREATEST(comment_count + $2, 0) WHERE id = $1")
                .bind(id)
                .bind(delta as i32)
                .execute(&self.core.db)
                .await?;
        }
        Ok(())
    }
}
