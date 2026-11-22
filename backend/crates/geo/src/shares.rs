//! Live shares: time-limited public links to a user's live location.
//!
//! The token in the link is the capability: whoever has it sees the precise
//! pin until the share expires or is revoked, with or without an account.
//! Positions are published on the realtime topic `share:<token>` (public, no
//! guard — knowing the token is the permission) and served by the
//! `GetLiveShare` / `WatchLiveShare` RPCs.
use std::time::Duration;

use buffa::Message as _;
use chrono::{DateTime, Utc};
use connectrpc::{ConnectError, ServiceStream};
use futures::StreamExt as _;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokuly_core::{ids, prelude::*, realtime};
use tokuly_proto::{
    geo::{FriendLocation, GetLiveShareResponse, LiveShare, WatchLiveShareResponse, geo_event::Event},
    realtime::{ServerFrame, server_frame::Body},
};

use crate::{
    Geo,
    fanout::frame,
    visibility::{self, Subject},
};

const MIN_SECONDS: i32 = 5 * 60;
const MAX_SECONDS: i32 = 24 * 3600;
const MAX_LABEL_CHARS: usize = 60;
const MAX_ACTIVE: i64 = 10;
/// 32 base62 characters: about 190 bits.
const TOKEN_CHARS: usize = 32;

/// Realtime topic of a share.
pub(crate) fn topic(token: &str) -> String {
    format!("share:{token}")
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ShareRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token: String,
    pub label: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub view_count: i32,
}

const SHARE_COLUMNS: &str = "id, user_id, token, label, created_at, expires_at, view_count";

fn share_not_found() -> AppError {
    AppError::not_found("SHARE_NOT_FOUND", "this share link does not exist or has ended")
}

/// Tokens are base62; anything else cannot be one (and must not reach a topic name).
fn looks_like_token(token: &str) -> bool {
    token.len() == TOKEN_CHARS && token.bytes().all(|b| b.is_ascii_alphanumeric())
}

impl Geo {
    fn share_message(&self, row: &ShareRow) -> LiveShare {
        LiveShare {
            id: row.id.to_string(),
            label: row.label.clone(),
            url: self.core.web_url(&format!("/share/{}", row.token)),
            created_at: ts(row.created_at),
            expires_at: ts(row.expires_at),
            view_count: row.view_count,
            ..Default::default()
        }
    }

    pub(crate) async fn create_live_share(
        &self,
        owner: Uuid,
        duration_seconds: i32,
        label: &str,
    ) -> AppResult<LiveShare> {
        ensure!(
            (MIN_SECONDS..=MAX_SECONDS).contains(&duration_seconds),
            invalid,
            "INVALID_DURATION",
            "a live share lasts between 5 minutes and 24 hours"
        );
        let label = label.trim();
        ensure!(
            label.chars().count() <= MAX_LABEL_CHARS && !label.chars().any(char::is_control),
            invalid,
            "LABEL_TOO_LONG",
            "a label has at most {MAX_LABEL_CHARS} characters"
        );
        self.core.cache.rate_limit(&format!("geo-share:{owner}"), 20, Duration::from_secs(3600)).await?;
        let active: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM geo_live_shares WHERE user_id = $1 AND ended_at IS NULL AND expires_at > now()",
        )
        .bind(owner)
        .fetch_one(&self.core.db)
        .await?;
        ensure!(active < MAX_ACTIVE, precondition, "TOO_MANY_SHARES", "at most {MAX_ACTIVE} live shares at a time");
        let row: ShareRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO geo_live_shares (user_id, token, label, expires_at) VALUES ($1, $2, $3, now() + make_interval(secs => $4))
             RETURNING {SHARE_COLUMNS}"
        )))
        .bind(owner)
        .bind(ids::random_base62(TOKEN_CHARS))
        .bind(label)
        .bind(duration_seconds as f64)
        .fetch_one(&self.core.db)
        .await?;
        self.invalidate_ctx(&[owner]).await;
        Ok(self.share_message(&row))
    }

    pub(crate) async fn list_live_shares(&self, owner: Uuid) -> AppResult<Vec<LiveShare>> {
        let rows: Vec<ShareRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {SHARE_COLUMNS} FROM geo_live_shares WHERE user_id = $1 AND ended_at IS NULL AND expires_at > now() ORDER BY created_at DESC"
        )))
        .bind(owner)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows.iter().map(|row| self.share_message(row)).collect())
    }

    pub(crate) async fn revoke_live_share(&self, owner: Uuid, share_id: &str) -> AppResult<()> {
        let share_id = parse_id(share_id)?;
        let token: Option<String> = sqlx::query_scalar("UPDATE geo_live_shares SET ended_at = now() WHERE id = $1 AND user_id = $2 AND ended_at IS NULL RETURNING token")
            .bind(share_id)
            .bind(owner)
            .fetch_optional(&self.core.db)
            .await?;
        match token {
            Some(token) => {
                self.invalidate_ctx(&[owner]).await;
                self.announce_share_end(owner, &token).await;
                Ok(())
            }
            None => {
                // Revoking twice is fine; revoking someone else's share is not a thing.
                let exists: bool =
                    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM geo_live_shares WHERE id = $1 AND user_id = $2)")
                        .bind(share_id)
                        .bind(owner)
                        .fetch_one(&self.core.db)
                        .await?;
                if exists { Ok(()) } else { Err(share_not_found()) }
            }
        }
    }

    /// Tells whoever follows a share that it is over.
    async fn announce_share_end(&self, owner: Uuid, token: &str) {
        self.core.rt.notify_topic(&topic(token), frame(Event::LocationRemovedUserId(owner.to_string()))).await;
    }

    /// Ends shares whose time is up. Returns how many ended.
    pub async fn expire_live_shares(&self) -> AppResult<u64> {
        let ended: Vec<(Uuid, String)> =
            sqlx::query_as("UPDATE geo_live_shares SET ended_at = now() WHERE ended_at IS NULL AND expires_at <= now() RETURNING user_id, token")
                .fetch_all(&self.core.db)
                .await?;
        for (owner, token) in &ended {
            self.invalidate_ctx(&[*owner]).await;
            self.announce_share_end(*owner, token).await;
        }
        Ok(ended.len() as u64)
    }

    /// A share that is still running, by its token.
    async fn live_share(&self, token: &str) -> AppResult<ShareRow> {
        if !looks_like_token(token) {
            return Err(share_not_found());
        }
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {SHARE_COLUMNS} FROM geo_live_shares WHERE token = $1 AND ended_at IS NULL AND expires_at > now()"
        )))
        .bind(token)
        .fetch_optional(&self.core.db)
        .await?
        .ok_or_else(share_not_found)
    }

    /// The sharer's pin as a share link shows it.
    async fn shared_location(&self, owner: Uuid, ja: bool) -> AppResult<Option<FriendLocation>> {
        let ctx = self.ctx_of(owner).await?;
        let state = self.load_state(owner).await?;
        let pin = state.as_ref().filter(|state| state.located()).map(|state| &state.pin);
        let subject = Subject { id: owner, pin, ctx: &ctx, online: false, weather: None };
        Ok(visibility::share_view(&self.view_env(Utc::now()), &subject, ja))
    }

    /// Public: what a share link shows. `client_ip` rate-limits lookups and
    /// counts each visitor once an hour.
    pub(crate) async fn get_live_share(
        &self,
        token: &str,
        client_ip: &str,
        ja: bool,
    ) -> AppResult<GetLiveShareResponse> {
        self.core.cache.rate_limit(&format!("geo-share-view:{client_ip}"), 120, Duration::from_secs(60)).await?;
        let share = self.live_share(token).await?;
        let user = tokuly_identity::summary(&self.core, share.user_id).await?;
        ensure!(!user.deleted, not_found, "SHARE_NOT_FOUND", "this share link does not exist or has ended");
        if self
            .core
            .cache
            .set_nx(&format!("geo:share:seen:{}:{client_ip}", share.id.simple()), "1", Duration::from_secs(3600))
            .await?
        {
            sqlx::query("UPDATE geo_live_shares SET view_count = view_count + 1 WHERE id = $1")
                .bind(share.id)
                .execute(&self.core.db)
                .await?;
        }
        Ok(GetLiveShareResponse {
            user: user.into(),
            label: share.label,
            expires_at: ts(share.expires_at),
            location: self.shared_location(share.user_id, ja).await?.into(),
            ..Default::default()
        })
    }

    /// Public: the current position, then every update until the share ends.
    pub(crate) async fn watch_live_share(
        &self,
        token: &str,
        client_ip: &str,
        ja: bool,
    ) -> AppResult<ServiceStream<WatchLiveShareResponse>> {
        self.core.cache.rate_limit(&format!("geo-share-watch:{client_ip}"), 30, Duration::from_secs(60)).await?;
        let share = self.live_share(token).await?;
        // Listen before reading the current position, so no update falls in between.
        let mut updates = self.core.bus.subscribe(realtime::topic_subject(&topic(&share.token))).await?;
        let current = self.shared_location(share.user_id, ja).await?;
        let (sender, receiver) = mpsc::channel(16);
        let shutdown = self.core.shutdown.clone();
        tokio::spawn(async move {
            if let Some(location) = current {
                if sender
                    .send(WatchLiveShareResponse { location: location.into(), ..Default::default() })
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let remaining = (share.expires_at - Utc::now()).to_std().unwrap_or_default();
            let expiry = tokio::time::sleep(remaining);
            tokio::pin!(expiry);
            let ended = WatchLiveShareResponse { ended: true, ..Default::default() };
            loop {
                tokio::select! {
                    _ = sender.closed() => return,
                    _ = shutdown.cancelled() => return,
                    _ = &mut expiry => {
                        let _ = sender.send(ended).await;
                        return;
                    }
                    message = updates.next() => {
                        let Some(message) = message else { return };
                        let Ok(frame) = ServerFrame::decode_from_slice(&message.payload) else { continue };
                        let Some(Body::Geo(event)) = frame.body else { continue };
                        match event.event {
                            Some(Event::Location(location)) => {
                                if sender.send(WatchLiveShareResponse { location: Some(*location).into(), ..Default::default() }).await.is_err() {
                                    return;
                                }
                            }
                            Some(Event::LocationRemovedUserId(_)) => {
                                let _ = sender.send(ended).await;
                                return;
                            }
                            _ => {}
                        }
                    }
                }
            }
        });
        Ok(Box::pin(ReceiverStream::new(receiver).map(Ok::<_, ConnectError>)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_32_base62_characters() {
        let token = ids::random_base62(TOKEN_CHARS);
        assert!(looks_like_token(&token));
        assert!(!looks_like_token("short"));
        assert!(!looks_like_token(&"a".repeat(31)));
        assert!(!looks_like_token(&format!("{}.>", "a".repeat(29))), "nothing that could be a bus wildcard");
        assert_eq!(topic("abc"), "share:abc");
    }
}
