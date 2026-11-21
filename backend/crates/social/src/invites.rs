//! Invite links / QR codes and the "intros" they produce.
//!
//! Every user has one current invite code. Someone who signs up with it is
//! recorded as an intro of the inviter (when `identity.user.created` arrives),
//! and a friend request goes from the newcomer to the inviter.
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokuly_core::{events, ids, prelude::*};
use tokuly_identity::users;
use tokuly_proto::{
    common::{PageInfo, UserSummary},
    identity::Relationship,
    social::{FriendSource, Invite},
};

use crate::{
    Social,
    events::{INTRO_CREATED, IntroCreated},
    graph,
    requests::RequestVia,
    suggestions::SuggestionReason,
};

const CODE_LENGTH: usize = 10;
const MAX_INTROS_PAGE: i64 = 100;

/// Keeps the user's code if they have one. (The no-op update makes
/// `RETURNING` yield the existing row.)
const CREATE_IF_MISSING: &str = "INSERT INTO friend_invites (user_id, code) VALUES ($1, $2)
     ON CONFLICT (user_id) DO UPDATE SET user_id = EXCLUDED.user_id
     RETURNING code";
const REPLACE: &str = "INSERT INTO friend_invites (user_id, code) VALUES ($1, $2)
     ON CONFLICT (user_id) DO UPDATE SET code = EXCLUDED.code, created_at = now()
     RETURNING code";

/// The code in what a user typed or pasted: the bare code, or the invite
/// link it came in (`https://.../invite/<code>`).
fn code_in(input: &str) -> &str {
    input.trim().trim_end_matches('/').rsplit('/').next().unwrap_or_default()
}

/// Whether `code` could be an invite code at all (saves a query for junk).
fn plausible_code(code: &str) -> bool {
    (4..=32).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// How many people joined through `user_id`'s invites.
pub(crate) async fn intro_count(core: &Core, user_id: Uuid) -> AppResult<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM friend_intros WHERE inviter_id = $1").bind(user_id).fetch_one(&core.db).await?)
}

impl Social {
    fn invite(&self, code: String, intro_count: i64) -> Invite {
        Invite { url: self.core.web_url(&format!("invite/{code}")), code, intro_count: intro_count as i32, ..Default::default() }
    }

    /// The user's invite, created on first use.
    pub(crate) async fn my_invite(&self, user_id: Uuid) -> AppResult<Invite> {
        let code = self.store_invite_code(user_id, CREATE_IF_MISSING).await?;
        Ok(self.invite(code, intro_count(&self.core, user_id).await?))
    }

    /// Replaces the user's invite code; the old link stops working. Intros
    /// already counted stay.
    pub(crate) async fn reset_invite(&self, user_id: Uuid) -> AppResult<Invite> {
        self.core.cache.rate_limit(&format!("invite-reset:{user_id}"), 10, Duration::from_secs(24 * 3600)).await?;
        let code = self.store_invite_code(user_id, REPLACE).await?;
        Ok(self.invite(code, intro_count(&self.core, user_id).await?))
    }

    /// Runs an upsert of `(user_id, fresh code)` and returns the stored code,
    /// retrying in the (astronomically unlikely) case the fresh code is taken.
    async fn store_invite_code(&self, user_id: Uuid, upsert: &'static str) -> AppResult<String> {
        let mut attempts = 0;
        loop {
            let stored: Result<String, sqlx::Error> =
                sqlx::query_scalar(upsert).bind(user_id).bind(ids::random_base62(CODE_LENGTH)).fetch_one(&self.core.db).await;
            match stored.map_err(AppError::from) {
                Ok(code) => return Ok(code),
                Err(e) if e.is_already_exists() && attempts < 3 => attempts += 1,
                Err(e) => return Err(e),
            }
        }
    }

    /// The owner of an invite code (or invite link), if it is current.
    async fn inviter_of(&self, code_or_link: &str) -> AppResult<Option<Uuid>> {
        let code = code_in(code_or_link);
        if !plausible_code(code) {
            return Ok(None);
        }
        Ok(sqlx::query_scalar("SELECT user_id FROM friend_invites WHERE code = $1").bind(code).fetch_optional(&self.core.db).await?)
    }

    /// Who an invite code belongs to, and what `viewer` (if signed in) is to them.
    pub(crate) async fn resolve_invite(&self, viewer: Option<Uuid>, code: &str, client_ip: &str) -> AppResult<(UserSummary, Relationship)> {
        let core = &self.core;
        // Codes are short; make guessing them slow.
        let limiter = viewer.map(|id| id.to_string()).unwrap_or_else(|| client_ip.to_owned());
        core.cache.rate_limit(&format!("invite-resolve:{limiter}"), 60, Duration::from_secs(60)).await?;

        // An inviter who is gone, suspended or in a block with the viewer
        // looks the same as a code that never existed.
        const GONE: &str = "this invite does not exist anymore";
        let inviter = self.inviter_of(code).await?.or_not_found("INVITE_NOT_FOUND", GONE)?;
        ensure!(!users::filter_active(core, &[inviter]).await?.is_empty(), not_found, "INVITE_NOT_FOUND", "{GONE}");
        let relationship = match viewer {
            Some(viewer) => {
                ensure!(!graph::is_blocked_either(core, viewer, inviter).await?, not_found, "INVITE_NOT_FOUND", "{GONE}");
                graph::relationship(core, viewer, inviter).await?
            }
            None => Relationship::RELATIONSHIP_UNSPECIFIED,
        };
        Ok((tokuly_identity::summary(core, inviter).await?, relationship))
    }

    /// One page of the people who joined through `owner`'s invites, newest
    /// first, with the total. Follows the owner's friend list privacy.
    pub(crate) async fn list_intros(&self, viewer: Uuid, owner: Uuid, page: &Page) -> AppResult<(Vec<UserSummary>, PageInfo, i32)> {
        self.ensure_can_see_friends_of(viewer, owner).await?;
        let core = &self.core;
        let hidden = if viewer == owner { Vec::new() } else { graph::blocked_ids_either(core, viewer).await? };
        let cursor: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
        let (after_time, after_id) = cursor.unzip();
        let mut rows: Vec<(Uuid, DateTime<Utc>)> = sqlx::query_as(
            "SELECT i.invitee_id, i.created_at FROM friend_intros i JOIN users u ON u.id = i.invitee_id
             WHERE i.inviter_id = $1 AND u.deleted_at IS NULL AND NOT (i.invitee_id = ANY($2))
               AND ($3::timestamptz IS NULL OR (i.created_at, i.invitee_id) < ($3, $4))
             ORDER BY i.created_at DESC, i.invitee_id DESC
             LIMIT $5",
        )
        .bind(owner)
        .bind(&hidden)
        .bind(after_time)
        .bind(after_id)
        .bind(page.fetch())
        .fetch_all(&core.db)
        .await?;
        let info = page.finish(&mut rows, |(id, at)| (*at, *id));
        let ids: Vec<Uuid> = rows.iter().map(|(id, _)| *id).collect();
        let total = intro_count(core, owner).await?;
        Ok((tokuly_identity::summaries_ordered(core, &ids).await?, info, total as i32))
    }

    pub(crate) fn intros_page(page: Option<&tokuly_proto::common::PageRequestView<'_>>) -> Page {
        Page::from_view(page, MAX_INTROS_PAGE)
    }

    /// Handles a sign-up that carried an invite code: records the intro,
    /// suggests the two to each other and sends the newcomer's friend request
    /// to the inviter. Safe to run again for the same sign-up.
    pub(crate) async fn record_intro(&self, user_id: Uuid, invite_code: &str) -> AppResult<()> {
        let core = &self.core;
        let Some(inviter) = self.inviter_of(invite_code).await? else { return Ok(()) };
        if inviter == user_id {
            return Ok(());
        }
        let already: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM friend_intros WHERE invitee_id = $1)").bind(user_id).fetch_one(&core.db).await?;
        if already {
            return Ok(());
        }

        // The request goes first and is idempotent, so a redelivered event
        // that failed half-way picks up where it stopped. The inviter handed
        // out the link, so their "who may send me requests" audience does
        // not apply; reasons such as a full friend list are not this event's
        // problem and must not make it retry forever.
        if let Err(e) = self.request_friendship(user_id, inviter, FriendSource::FRIEND_SOURCE_INVITE_LINK, "", RequestVia::Encounter).await {
            if e.code == connectrpc::ErrorCode::Internal || e.code == connectrpc::ErrorCode::Unavailable {
                return Err(e);
            }
            tracing::debug!(%user_id, %inviter, reason = %e.reason, "no friend request for an invite sign-up");
        }

        let mut tx = core.db.begin().await?;
        let recorded = sqlx::query("INSERT INTO friend_intros (invitee_id, inviter_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(user_id)
            .bind(inviter)
            .execute(&mut *tx)
            .await?;
        if recorded.rows_affected() > 0 {
            sqlx::query("INSERT INTO friend_suggestions (user_id, suggested_id, reason) VALUES ($1, $2, $3), ($2, $1, $4) ON CONFLICT DO NOTHING")
                .bind(user_id)
                .bind(inviter)
                .bind(SuggestionReason::InvitedBy.as_str())
                .bind(SuggestionReason::Invited.as_str())
                .execute(&mut *tx)
                .await?;
            events::emit(&mut *tx, INTRO_CREATED, Some(user_id), &IntroCreated { inviter_id: inviter, user_id }).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_found_in_pasted_links() {
        assert_eq!(code_in(" aB3dE6gH9k "), "aB3dE6gH9k");
        assert_eq!(code_in("https://tokuly.example/invite/aB3dE6gH9k"), "aB3dE6gH9k");
        assert_eq!(code_in("https://tokuly.example/invite/aB3dE6gH9k/"), "aB3dE6gH9k");
        assert_eq!(code_in(""), "");
    }

    #[test]
    fn junk_is_not_looked_up_as_an_invite_code() {
        assert!(plausible_code("aB3dE6gH9k"));
        assert!(!plausible_code(""));
        assert!(!plausible_code("abc"));
        assert!(!plausible_code("has space1"));
        assert!(!plausible_code("'; DROP TABLE"));
        assert!(!plausible_code(&"a".repeat(33)));
    }
}
