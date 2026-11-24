//! Keeping what was reported.
//!
//! A report quotes its subject, but the uploads in it (the photo in a chat
//! message, the file in a post) belong to the domain that keeps the content,
//! and media collects an upload nothing holds any more. Without a hold of
//! its own, a sender who unsends a reported message takes the photo with it
//! two days later, and the case has nothing left to judge.
//!
//! So every case holds the uploads of its subject from the moment a report
//! files into it, under the holder `admin:case:<case id>`, and lets go
//! `TOKULY_MODERATION_EVIDENCE_DAYS` after the case is resolved (a reopened
//! case keeps them). This domain may not depend on media or on the content
//! domains, so the server hands in the lookup ([`ReportEvidence`]); without
//! one nothing is held. A takedown still deletes the uploads: media drops
//! the holds of an upload it deletes on a domain's authority.
use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgConnection;
use tokuly_core::prelude::*;
use tokuly_proto::common::SubjectType;

use crate::Admin;

/// Cases whose evidence is let go per pass of the maintenance timer.
const RELEASE_BATCH: i64 = 200;

/// Holds and releases the uploads of reported content.
#[async_trait]
pub trait ReportEvidence: Send + Sync + 'static {
    /// Holds every upload the subject has now under `holder`, so that it
    /// outlives the subject being edited, unsent or deleted by its owner.
    /// Runs on the transaction that files the report; idempotent. Subject
    /// types the implementation does not know hold nothing.
    async fn hold(&self, conn: &mut PgConnection, subject_type: SubjectType, subject_id: Uuid, holder: &str) -> AppResult<()>;

    /// Lets go of everything `holder` holds.
    async fn release(&self, conn: &mut PgConnection, holder: &str) -> AppResult<()>;
}

/// The holder string of a case's evidence.
pub(crate) fn case_holder(case_id: Uuid) -> String {
    format!("admin:case:{case_id}")
}

impl Admin {
    /// Hands in the lookup that holds the uploads of reported content (see
    /// [`ReportEvidence`]). Call once at startup; a second call is ignored.
    pub fn set_report_evidence(&self, evidence: Arc<dyn ReportEvidence>) {
        let _ = self.evidence.set(evidence);
    }

    /// Holds the subject's uploads for `case_id`, on the transaction filing
    /// a report into it.
    pub(crate) async fn hold_evidence(&self, conn: &mut PgConnection, case_id: Uuid, subject_type: i32, subject_id: Uuid) -> AppResult<()> {
        let (Some(evidence), Some(subject_type)) = (self.evidence.get(), buffa::EnumValue::<SubjectType>::from(subject_type).as_known()) else {
            return Ok(());
        };
        evidence.hold(&mut *conn, subject_type, subject_id, &case_holder(case_id)).await?;
        sqlx::query("UPDATE moderation_cases SET evidence_held = true WHERE id = $1 AND NOT evidence_held").bind(case_id).execute(&mut *conn).await?;
        Ok(())
    }

    /// Lets go of the evidence of cases resolved more than
    /// `TOKULY_MODERATION_EVIDENCE_DAYS` ago. Returns how many cases let go.
    /// Safe to run from several workers at once.
    pub async fn release_due_evidence(&self) -> AppResult<u64> {
        let Some(evidence) = self.evidence.get() else {
            return Ok(0);
        };
        let due: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM moderation_cases
             WHERE evidence_held AND status = 'resolved' AND resolved_at < now() - make_interval(days => $1)
             ORDER BY resolved_at LIMIT $2",
        )
        .bind(self.cfg.moderation_evidence_days)
        .bind(RELEASE_BATCH)
        .fetch_all(&self.core.db)
        .await?;
        let mut released = 0;
        for case_id in due {
            let mut tx = self.core.db.begin().await?;
            // Re-checked on the row: a reopened case, or another worker, wins.
            let still_due: bool = sqlx::query_scalar(
                "UPDATE moderation_cases SET evidence_held = false
                 WHERE id = $1 AND evidence_held AND status = 'resolved' AND resolved_at < now() - make_interval(days => $2)
                 RETURNING true",
            )
            .bind(case_id)
            .bind(self.cfg.moderation_evidence_days)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or(false);
            if !still_due {
                continue;
            }
            evidence.release(&mut tx, &case_holder(case_id)).await?;
            tx.commit().await?;
            released += 1;
        }
        Ok(released)
    }
}
