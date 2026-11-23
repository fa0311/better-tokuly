//! Bots: accounts of their own, owned by a person, acting through the API.
//!
//! A bot is a real row in identity's `users` table, so everything that
//! takes a user takes a bot. Identity has no notion of bots yet: no API to
//! create an account that nobody signs in to, no bot flag, no owner link.
//! Until it has, this module is the interim, kept as small as is safe:
//!
//! - It writes the bot's `users` row itself — three statements: the insert
//!   (no email, no password), the profile update, and marking the account
//!   for deletion. Identity's own hourly purge then erases the account and
//!   announces it; nothing here runs that purge.
//! - It emits no event in identity's name. `identity.user.created` promises
//!   consumers a person who signed up (invites, welcome rewards), which a
//!   bot is not; what happens to a bot is announced as `dev.bot.*`.
//! - The owner link lives in `dev_bots`, and outlives the bot for a while.
//! - A bot can address nobody: its tokens get no write scope beyond its own
//!   profile, uploads and inbox (`scopes::allowed_for_bots`), and no wallet
//!   scope. Someone who blocked the owner must not be reachable through the
//!   owner's bots, and only the social domain can check that.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use sqlx::PgConnection;
use tokuly_core::{events, prelude::*};
use tokuly_identity::users;
use tokuly_proto::dev::*;

use crate::{
    Dev,
    events::BotEvent,
    images::{ImageChange, bot_avatar_holder},
    lock_limit,
    secrets::SecretKind,
    tokens::{NewToken, TokenRow},
};

/// Locks the `dev_bots` row of a bot that must still be `owner`'s (and not
/// deleted) for the rest of the transaction: `BOT_NOT_FOUND` otherwise. What
/// was checked before the transaction may be stale by then, after a hand-over.
async fn lock_owned_bot(tx: &mut PgConnection, bot_id: Uuid, owner: Uuid) -> AppResult<()> {
    sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM dev_bots WHERE user_id = $1 AND owner_id = $2 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(bot_id)
    .bind(owner)
    .fetch_optional(&mut *tx)
    .await?
    .or_not_found("BOT_NOT_FOUND", "bot not found")?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct BotRow {
    pub user_id: Uuid,
    pub owner_id: Uuid,
    pub disabled: bool,
    /// The bot's account is suspended by moderation.
    pub suspended: bool,
    pub created_at: Time,
    pub bio: String,
    pub token_count: i64,
    pub last_used_at: Option<Time>,
}

const BOT_COLUMNS: &str = "b.user_id, b.owner_id, b.disabled_at IS NOT NULL AS disabled, u.banned_at IS NOT NULL AS suspended, b.created_at, u.bio,
    (SELECT count(*) FROM dev_tokens t WHERE t.user_id = b.user_id AND t.revoked_at IS NULL AND (t.expires_at IS NULL OR t.expires_at > now())) AS token_count,
    (SELECT max(t.last_used_at) FROM dev_tokens t WHERE t.user_id = b.user_id) AS last_used_at";

impl Dev {
    /// A bot `owner` owns.
    pub(crate) async fn owned_bot(&self, bot_id: Uuid, owner: Uuid) -> AppResult<BotRow> {
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {BOT_COLUMNS} FROM dev_bots b JOIN users u ON u.id = b.user_id
             WHERE b.user_id = $1 AND b.owner_id = $2 AND b.deleted_at IS NULL AND u.deleted_at IS NULL"
        )))
        .bind(bot_id)
        .bind(owner)
        .fetch_optional(&self.core.db)
        .await?
        .or_not_found("BOT_NOT_FOUND", "bot not found")
    }

    async fn bots_of(&self, owner: Uuid) -> AppResult<Vec<BotRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {BOT_COLUMNS} FROM dev_bots b JOIN users u ON u.id = b.user_id
             WHERE b.owner_id = $1 AND b.deleted_at IS NULL AND u.deleted_at IS NULL ORDER BY b.created_at, b.user_id"
        )))
        .bind(owner)
        .fetch_all(&self.core.db)
        .await?)
    }

    /// Who owns — or, for a bot deleted within the retention, owned — a
    /// bot account. For moderation: a report about a bot is a report about
    /// a person, and deleting the bot must not make that person untraceable.
    pub async fn bot_owner(&self, bot_id: Uuid) -> AppResult<Option<Uuid>> {
        Ok(sqlx::query_scalar("SELECT owner_id FROM dev_bots WHERE user_id = $1")
            .bind(bot_id)
            .fetch_optional(&self.core.db)
            .await?)
    }

    async fn bot_protos(&self, rows: &[BotRow]) -> AppResult<Vec<Bot>> {
        let ids: Vec<Uuid> = rows.iter().flat_map(|row| [row.user_id, row.owner_id]).collect();
        let summaries = tokuly_identity::summaries(&self.core, &ids).await?;
        Ok(rows
            .iter()
            .map(|row| Bot {
                user: users::pick(&summaries, row.user_id).into(),
                bio: row.bio.clone(),
                disabled: row.disabled,
                created_at: ts(row.created_at),
                token_count: row.token_count.min(i32::MAX as i64) as i32,
                last_used_at: opt_ts(row.last_used_at),
                owner: users::pick(&summaries, row.owner_id).into(),
                suspended: row.suspended,
                ..Default::default()
            })
            .collect())
    }

    async fn bot_proto(&self, bot_id: Uuid, owner: Uuid) -> AppResult<Bot> {
        let row = self.owned_bot(bot_id, owner).await?;
        Ok(self.bot_protos(&[row]).await?.remove(0))
    }

    /// Creates the account of a bot: the insert identity's sign-up makes,
    /// without credentials, in the owner's language and time zone.
    async fn create_bot_account(
        tx: &mut PgConnection,
        owner: Uuid,
        handle: &str,
        display_name: &str,
        bio: &str,
    ) -> AppResult<Uuid> {
        let taken: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE handle = $1::citext)")
            .bind(handle)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(!taken, already_exists, "HANDLE_TAKEN", "this handle is already in use");
        let bot_id: Uuid = sqlx::query_scalar(
            "INSERT INTO users (handle, display_name, bio, locale, time_zone, attrs)
             SELECT $2::citext, $3, $4, o.locale, o.time_zone, jsonb_build_object('bot', true, 'bot_owner_id', o.id) FROM users o WHERE o.id = $1
             RETURNING id",
        )
        .bind(owner)
        .bind(handle)
        .bind(display_name)
        .bind(bio)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            let e = AppError::from(e);
            if e.is_already_exists() { AppError::already_exists("HANDLE_TAKEN", "this handle is already in use") } else { e }
        })?;
        Ok(bot_id)
    }

    /// Deletes a bot: its tokens and webhooks now; its account is marked
    /// for deletion and erased by identity's next purge (within the hour),
    /// which tells every domain to drop the rest.
    ///
    /// The row that says who owned it is kept, marked deleted, for
    /// `TOKULY_DEV_BOT_RECORD_DAYS` counted from now: what the bot sent
    /// stays with its recipients, who can still report it, and a report must
    /// lead to the person behind it ([`Dev::bot_owner`]).
    pub(crate) async fn delete_bot(&self, bot_id: Uuid, owner_id: Uuid, actor: Option<Uuid>) -> AppResult<()> {
        self.revoke_tokens_of(bot_id, "bot_deleted", None).await?;
        let mut tx = self.core.db.begin().await?;
        // Still theirs: a hand-over accepted in the meantime is not undone by the previous owner.
        lock_owned_bot(&mut tx, bot_id, owner_id).await?;
        if let Ok(images) = self.images() {
            images.release(&mut tx, &bot_avatar_holder(bot_id)).await?;
        }
        sqlx::query("DELETE FROM dev_webhooks WHERE account_id = $1").bind(bot_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM dev_tokens WHERE user_id = $1").bind(bot_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM dev_bot_transfers WHERE bot_id = $1 AND accepted_at IS NULL")
            .bind(bot_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE dev_bots SET deleted_at = COALESCE(deleted_at, now()), disabled_at = COALESCE(disabled_at, now()) WHERE user_id = $1")
            .bind(bot_id)
            .execute(&mut *tx)
            .await?;
        // Due at once: identity's next purge anonymizes the row and announces the deletion.
        sqlx::query("UPDATE users SET deletion_scheduled_at = now() WHERE id = $1 AND deleted_at IS NULL")
            .bind(bot_id)
            .execute(&mut *tx)
            .await?;
        events::emit(&mut *tx, "dev.bot.deleted", actor, &BotEvent { bot_id, owner_id }).await?;
        tx.commit().await?;
        self.subscribed.invalidate();
        Ok(())
    }

    /// Drops the ownership records of bots deleted, and the records of
    /// hand-overs made, longer ago than the retention, and offers that ran out.
    pub async fn purge_bot_records(&self) -> AppResult<u64> {
        let days = self.cfg.bot_record_days as i32;
        let transfers = sqlx::query(
            "DELETE FROM dev_bot_transfers
             WHERE (accepted_at IS NULL AND expires_at < now()) OR accepted_at < now() - make_interval(days => $1)",
        )
        .bind(days)
        .execute(&self.core.db)
        .await?;
        let done = sqlx::query("DELETE FROM dev_bots WHERE deleted_at < now() - make_interval(days => $1)")
            .bind(days)
            .execute(&self.core.db)
            .await?;
        Ok(done.rows_affected() + transfers.rows_affected())
    }
}

impl BotService for Dev {
    async fn create_bot(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateBotRequest>,
    ) -> ServiceResult<CreateBotResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let handle = users::validate_handle(request.handle)?;
            let display_name = users::validate_display_name(request.display_name)?;
            let bio = users::validate_bio(request.bio)?;
            self.core.cache.rate_limit(&format!("dev-bot-create:{owner}"), 10, Duration::from_secs(24 * 3600)).await?;
            let mut tx = self.core.db.begin().await?;
            // One at a time per owner, so the limit cannot be raced past.
            // Everything until the commit runs on this transaction.
            lock_limit(&mut tx, "dev-bots", owner).await?;
            let count: i64 =
                sqlx::query_scalar("SELECT count(*) FROM dev_bots WHERE owner_id = $1 AND deleted_at IS NULL")
                    .bind(owner)
                    .fetch_one(&mut *tx)
                    .await?;
            ensure!(count < self.cfg.max_bots, precondition, "TOO_MANY_BOTS", "at most {} bots", self.cfg.max_bots);
            let bot_id = Self::create_bot_account(&mut tx, owner, &handle, &display_name, &bio).await?;
            sqlx::query("INSERT INTO dev_bots (user_id, owner_id) VALUES ($1, $2)")
                .bind(bot_id)
                .bind(owner)
                .execute(&mut *tx)
                .await?;
            events::emit(&mut *tx, "dev.bot.created", Some(owner), &BotEvent { bot_id, owner_id: owner }).await?;
            tx.commit().await?;
            ok(CreateBotResponse { bot: self.bot_proto(bot_id, owner).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn get_bot(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetBotRequest>,
    ) -> ServiceResult<GetBotResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            ok(GetBotResponse {
                bot: self.bot_proto(parse_id(request.bot_id)?, owner).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_bots(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListBotsRequest>,
    ) -> ServiceResult<ListBotsResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let rows = self.bots_of(owner).await?;
            ok(ListBotsResponse {
                bots: self.bot_protos(&rows).await?,
                max_bots: self.cfg.max_bots.min(i32::MAX as i64) as i32,
                ..Default::default()
            })
        })
        .await
    }

    async fn update_bot(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateBotRequest>,
    ) -> ServiceResult<UpdateBotResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let bot = self.owned_bot(parse_id(request.bot_id)?, owner).await?;
            let display_name = request.display_name.map(users::validate_display_name).transpose()?;
            let bio = request.bio.map(users::validate_bio).transpose()?;
            let handle = request.handle.map(users::validate_handle).transpose()?;
            // Each change is announced to every domain that keeps a copy of profiles.
            self.core.cache.rate_limit(&format!("dev-bot-update:{owner}"), 30, Duration::from_secs(3600)).await?;
            if handle.is_some() {
                self.core.cache.rate_limit(&format!("dev-bot-handle:{}", bot.user_id), 5, Duration::from_secs(7 * 24 * 3600)).await?;
            }
            // Looked up before the transaction: it reads through the pool.
            let avatar = ImageChange::resolve(self, owner, request.avatar_asset_id).await?;
            let mut tx = self.core.db.begin().await?;
            lock_owned_bot(&mut tx, bot.user_id, owner).await?;
            if let Some(handle) = &handle {
                let taken: bool =
                    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE handle = $1::citext AND id <> $2)").bind(handle).bind(bot.user_id).fetch_one(&mut *tx).await?;
                ensure!(!taken, already_exists, "HANDLE_TAKEN", "this handle is already in use");
            }
            // `Some(None)` removes the avatar, `Some(Some(image))` replaces it.
            let avatar = avatar.apply(self, &mut tx, &bot_avatar_holder(bot.user_id)).await?;
            sqlx::query(
                "UPDATE users SET display_name = COALESCE($2, display_name), handle = COALESCE($3::citext, handle), bio = COALESCE($4, bio),
                        avatar = CASE WHEN $5 THEN $6 ELSE avatar END
                 WHERE id = $1",
            )
            .bind(bot.user_id)
            .bind(display_name)
            .bind(handle)
            .bind(bio)
            .bind(avatar.is_some())
            .bind(avatar.flatten())
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                let e = AppError::from(e);
                if e.is_already_exists() { AppError::already_exists("HANDLE_TAKEN", "this handle is already in use") } else { e }
            })?;
            events::emit(&mut *tx, "dev.bot.updated", Some(owner), &BotEvent { bot_id: bot.user_id, owner_id: owner }).await?;
            tx.commit().await?;
            ok(UpdateBotResponse { bot: self.bot_proto(bot.user_id, owner).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn set_bot_disabled(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetBotDisabledRequest>,
    ) -> ServiceResult<SetBotDisabledResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let bot = self.owned_bot(parse_id(request.bot_id)?, owner).await?;
            self.core.cache.rate_limit(&format!("dev-bot-switch:{owner}"), 60, Duration::from_secs(3600)).await?;
            let done = sqlx::query(
                "UPDATE dev_bots SET disabled_at = CASE WHEN $2 THEN COALESCE(disabled_at, now()) END WHERE user_id = $1 AND owner_id = $3",
            )
            .bind(bot.user_id)
            .bind(request.disabled)
            .bind(owner)
            .execute(&self.core.db)
            .await?;
            ensure!(done.rows_affected() == 1, not_found, "BOT_NOT_FOUND", "bot not found");
            // Its tokens are judged afresh on their next request.
            self.forget_account_tokens(bot.user_id).await?;
            ok(SetBotDisabledResponse { bot: self.bot_proto(bot.user_id, owner).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_bot(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteBotRequest>,
    ) -> ServiceResult<DeleteBotResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let bot = self.owned_bot(parse_id(request.bot_id)?, owner).await?;
            // A suspension is not something its subject can delete away, and
            // a suspended bot keeps its place in the owner's allowance.
            ensure!(!bot.suspended, precondition, "BOT_SUSPENDED", "a suspended bot cannot be deleted");
            self.delete_bot(bot.user_id, owner, Some(owner)).await?;
            ok(DeleteBotResponse::default())
        })
        .await
    }

    async fn create_bot_token(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateBotTokenRequest>,
    ) -> ServiceResult<CreateBotTokenResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let req = request.to_owned_message();
            let bot = self.owned_bot(parse_id(&req.bot_id)?, owner).await?;
            self.core.cache.rate_limit(&format!("dev-token-create:{owner}"), 20, Duration::from_secs(3600)).await?;
            let mut tx = self.core.db.begin().await?;
            let new =
                NewToken { kind: SecretKind::Bot, user_id: bot.user_id, created_by: owner, name: &req.name, scopes: &req.scopes, expiry: req.expiry, limit: self.cfg.max_bot_tokens };
            let (token, secret) = self.create_token(&mut tx, new).await?;
            tx.commit().await?;
            let attrs = serde_json::json!({ "token_id": token.id, "name": token.name, "bot_id": bot.user_id, "scopes": token.scopes });
            self.identity.log_security_event(owner, "BOT_TOKEN_CREATED", &ctx.client(), "", attrs).await;
            ok(CreateBotTokenResponse { token: token.to_proto().into(), secret, ..Default::default() })
        })
        .await
    }

    async fn list_bot_tokens(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListBotTokensRequest>,
    ) -> ServiceResult<ListBotTokensResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let bot = self.owned_bot(parse_id(request.bot_id)?, owner).await?;
            // A bot has a handful of tokens at most: one page holds them all.
            let rows = self.list_tokens(bot.user_id, "bot", &Page { limit: 200, cursor: None }).await?;
            ok(ListBotTokensResponse { tokens: rows.iter().map(TokenRow::to_proto).collect(), ..Default::default() })
        })
        .await
    }

    async fn revoke_bot_token(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RevokeBotTokenRequest>,
    ) -> ServiceResult<RevokeBotTokenResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let token = self.owned_token(parse_id(request.token_id)?, owner, "bot").await?;
            self.revoke_token(token.id, owner).await?;
            ok(RevokeBotTokenResponse::default())
        })
        .await
    }

    async fn offer_bot_transfer(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, OfferBotTransferRequest>,
    ) -> ServiceResult<OfferBotTransferResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let transfer = self.offer_bot_transfer(owner, parse_id(request.bot_id)?, request.user_id).await?;
            ok(OfferBotTransferResponse { transfer: transfer.into(), ..Default::default() })
        })
        .await
    }

    async fn list_bot_transfers(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListBotTransfersRequest>,
    ) -> ServiceResult<ListBotTransfersResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            let (outgoing, incoming) = self.bot_transfers_of(user_id).await?;
            ok(ListBotTransfersResponse { outgoing, incoming, ..Default::default() })
        })
        .await
    }

    async fn accept_bot_transfer(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AcceptBotTransferRequest>,
    ) -> ServiceResult<AcceptBotTransferResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            let transfer_id = Uuid::parse_str(request.transfer_id.trim())
                .map_err(|_| AppError::not_found("TRANSFER_NOT_FOUND", "no such offer"))?;
            let bot_id = self.accept_bot_transfer(user_id, transfer_id).await?;
            let attrs = serde_json::json!({ "bot_id": bot_id, "transfer_id": transfer_id });
            self.identity.log_security_event(user_id, "BOT_TRANSFER_ACCEPTED", &ctx.client(), "", attrs).await;
            ok(AcceptBotTransferResponse { bot: self.bot_proto(bot_id, user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn cancel_bot_transfer(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CancelBotTransferRequest>,
    ) -> ServiceResult<CancelBotTransferResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            let transfer_id = Uuid::parse_str(request.transfer_id.trim())
                .map_err(|_| AppError::not_found("TRANSFER_NOT_FOUND", "no such offer"))?;
            self.cancel_bot_transfer(user_id, transfer_id).await?;
            ok(CancelBotTransferResponse::default())
        })
        .await
    }
}
