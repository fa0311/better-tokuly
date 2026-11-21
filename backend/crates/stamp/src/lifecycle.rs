//! The pack lifecycle: every status change with its side effects, in one place.
//!
//! ```text
//!            submit                approve
//!   DRAFT ───────────► IN_REVIEW ───────────► PUBLISHED
//!     ▲  ◄─ withdraw ──┘   │ reject               │
//!     │                    ▼                      │ take down
//!     │                 REJECTED                  ▼
//!     └──────────── unpublish ◄── PUBLISHED    REMOVED
//! ```
//!
//! REJECTED and REMOVED packs are submitted again like drafts. Submitting
//! skips IN_REVIEW when no moderator is needed (see
//! [`access::submission_needs_moderator`](crate::access::submission_needs_moderator)).
//!
//! Each function expects the pack row to be locked by the caller's
//! transaction ([`packs::lock`]) and checks nothing but what it needs to be
//! correct; who may trigger a transition is the caller's business.
use sqlx::{Postgres, Transaction};
use tokuly_core::{
    events,
    outbound::{self, ActivityAudience, ActivityEntry, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{common::SubjectType, stamp::ReviewDecision};

use crate::{
    events::{PACK_PUBLISHED, PACK_REJECTED, PACK_REMOVED, PACK_SUBMITTED, PACK_UNPUBLISHED, PackDecision, PackEvent},
    model::{PackRow, status},
    packs,
};

/// How a pack came to be published.
#[derive(Debug, Clone, Copy)]
pub enum Approval {
    /// No review was needed: an official pack, a trusted creator, or nothing
    /// new since the last approval.
    Automatic,
    /// A moderator approved it.
    Moderator(Uuid),
}

fn pack_event(pack: &PackRow) -> PackEvent {
    PackEvent { pack_id: pack.id, creator_id: pack.creator_id, official: pack.official }
}

async fn record_decision(
    tx: &mut Transaction<'_, Postgres>,
    pack_id: Uuid,
    moderator_id: Option<Uuid>,
    decision: ReviewDecision,
    note: &str,
) -> AppResult<()> {
    sqlx::query("INSERT INTO stamp_pack_reviews (pack_id, moderator_id, decision, note) VALUES ($1, $2, $3, $4)")
        .bind(pack_id)
        .bind(moderator_id)
        .bind(decision as i32)
        .bind(note)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// A notification to the pack's creator about a moderation outcome. It
/// names no actor: creators hear from "Tokuly", not from a specific moderator.
fn creator_notice(
    pack: &PackRow,
    creator_id: Uuid,
    kind: &str,
    title: Localized,
    body: Localized,
    path: String,
) -> NotificationRequest {
    let notice = NotificationRequest::new(creator_id, kind, title, body, path)
        .subject(SubjectType::SUBJECT_TYPE_STAMP_PACK, pack.id);
    match pack.cover_url() {
        Some(url) => notice.image(url),
        None => notice,
    }
}

/// Puts the pack in the review queue.
pub async fn send_to_review(tx: &mut Transaction<'_, Postgres>, pack: &PackRow, actor_id: Uuid) -> AppResult<()> {
    sqlx::query("UPDATE stamp_packs SET status = $2, submitted_at = now() WHERE id = $1")
        .bind(pack.id)
        .bind(status::IN_REVIEW)
        .execute(&mut **tx)
        .await?;
    events::emit(&mut **tx, PACK_SUBMITTED, Some(actor_id), &pack_event(pack)).await
}

/// Publishes the pack: every stamp it has counts as reviewed from now on,
/// the pack appears in the store and in search, and the creator's friends
/// hear about a first publication. Returns the published row.
///
/// `actor_id` is who triggered it: the creator, the moderator, or nobody
/// when the platform seeds its own packs.
pub async fn publish(
    tx: &mut Transaction<'_, Postgres>,
    pack: &PackRow,
    approval: Approval,
    actor_id: Option<Uuid>,
) -> AppResult<PackRow> {
    let first_publication = pack.published_at.is_none();
    let moderator_id = match approval {
        Approval::Moderator(id) => Some(id),
        Approval::Automatic => None,
    };
    sqlx::query("UPDATE stamps SET approved_at = now() WHERE pack_id = $1 AND approved_at IS NULL")
        .bind(pack.id)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "UPDATE stamp_packs SET status = $2, needs_review = false, review_note = '', published_at = now(),
                submitted_at = COALESCE(submitted_at, now()),
                reviewed_at = CASE WHEN $3 THEN now() ELSE reviewed_at END
         WHERE id = $1",
    )
    .bind(pack.id)
    .bind(status::PUBLISHED)
    .bind(moderator_id.is_some())
    .execute(&mut **tx)
    .await?;
    if moderator_id.is_some() {
        record_decision(tx, pack.id, moderator_id, ReviewDecision::REVIEW_DECISION_APPROVED, "").await?;
    }
    let published = packs::refresh_derived(tx, pack.id).await?;
    packs::index(tx, &published).await?;
    events::emit(&mut **tx, PACK_PUBLISHED, actor_id, &pack_event(&published)).await?;

    let Some(creator_id) = published.creator_id else { return Ok(published) };
    if first_publication {
        let summary = Localized::new(
            format!("スタンプ「{}」を公開したよ!", published.name),
            format!("Published the stamp pack “{}”", published.name),
        );
        let mut entry = ActivityEntry::new(creator_id, "stamp.pack.published", ActivityAudience::Public, summary)
            .subject(SubjectType::SUBJECT_TYPE_STAMP_PACK, published.id)
            .path(published.path());
        if let Some(url) = published.cover_url() {
            entry = entry.image(url);
        }
        outbound::activity(&mut **tx, &entry).await?;
    }
    if moderator_id.is_some() {
        let notice = creator_notice(
            &published,
            creator_id,
            "stamp.pack.approved",
            Localized::new("スタンプが公開されたよ!", "Your stamp pack is live!"),
            Localized::new(
                format!("「{}」が審査を通過して、ストアに並んだよ!", published.name),
                format!("“{}” passed review and is now in the store.", published.name),
            ),
            published.path(),
        );
        outbound::notify(&mut **tx, &notice).await?;
    }
    Ok(published)
}

/// Declines a pack in review and tells the creator why.
pub async fn reject(
    tx: &mut Transaction<'_, Postgres>,
    pack: &PackRow,
    moderator_id: Uuid,
    reason: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE stamp_packs SET status = $2, needs_review = true, review_note = $3, reviewed_at = now() WHERE id = $1",
    )
    .bind(pack.id)
    .bind(status::REJECTED)
    .bind(reason)
    .execute(&mut **tx)
    .await?;
    record_decision(tx, pack.id, Some(moderator_id), ReviewDecision::REVIEW_DECISION_REJECTED, reason).await?;
    let decision = PackDecision {
        pack_id: pack.id,
        creator_id: pack.creator_id,
        moderator_id: Some(moderator_id),
        reason: reason.to_owned(),
    };
    events::emit(&mut **tx, PACK_REJECTED, Some(moderator_id), &decision).await?;
    if let Some(creator_id) = pack.creator_id {
        let notice = creator_notice(
            pack,
            creator_id,
            "stamp.pack.rejected",
            Localized::new("スタンプの審査結果のお知らせ", "About your stamp pack review"),
            Localized::new(
                format!("「{}」は公開できなかったよ。理由: {reason}", pack.name),
                format!("“{}” was not approved. Reason: {reason}", pack.name),
            ),
            pack.studio_path(),
        );
        outbound::notify(&mut **tx, &notice).await?;
    }
    Ok(())
}

/// Takes the pack out of the store (if published) or out of the review
/// queue (if in review) and back to a draft. People who hold it keep it.
pub async fn unpublish(tx: &mut Transaction<'_, Postgres>, pack: &PackRow, actor_id: Option<Uuid>) -> AppResult<()> {
    sqlx::query("UPDATE stamp_packs SET status = $2, featured_at = NULL WHERE id = $1")
        .bind(pack.id)
        .bind(status::DRAFT)
        .execute(&mut **tx)
        .await?;
    if pack.is_published() {
        packs::unindex(tx, pack.id).await?;
    }
    events::emit(&mut **tx, PACK_UNPUBLISHED, actor_id, &pack_event(pack)).await
}

/// Takes a pack down for breaking the rules. Its stamps stop resolving and
/// stop being sendable for everyone; the creator is told why and may fix
/// and resubmit, which always goes through review. Does nothing when the
/// pack is already taken down.
pub async fn take_down(
    tx: &mut Transaction<'_, Postgres>,
    pack: &PackRow,
    moderator_id: Option<Uuid>,
    reason: &str,
) -> AppResult<()> {
    if pack.status == status::REMOVED {
        return Ok(());
    }
    sqlx::query("UPDATE stamp_packs SET status = $2, needs_review = true, review_note = $3, reviewed_at = now(), featured_at = NULL WHERE id = $1")
        .bind(pack.id)
        .bind(status::REMOVED)
        .bind(reason)
        .execute(&mut **tx)
        .await?;
    record_decision(tx, pack.id, moderator_id, ReviewDecision::REVIEW_DECISION_REMOVED, reason).await?;
    packs::unindex(tx, pack.id).await?;
    let decision =
        PackDecision { pack_id: pack.id, creator_id: pack.creator_id, moderator_id, reason: reason.to_owned() };
    events::emit(&mut **tx, PACK_REMOVED, moderator_id, &decision).await?;
    if let Some(creator_id) = pack.creator_id {
        let notice = creator_notice(
            pack,
            creator_id,
            "stamp.pack.removed",
            Localized::new("スタンプが非公開になったよ", "Your stamp pack was taken down"),
            Localized::new(
                format!("「{}」はルール違反のため非公開になったよ。理由: {reason}", pack.name),
                format!("“{}” was taken down for breaking the rules. Reason: {reason}", pack.name),
            ),
            pack.studio_path(),
        );
        outbound::notify(&mut **tx, &notice).await?;
    }
    Ok(())
}
