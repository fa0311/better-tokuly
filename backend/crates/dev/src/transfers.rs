//! Handing a bot over to another person: the owner offers it, the receiver
//! accepts (or declines), and from then on the bot is theirs.
//!
//! What moves and what does not:
//!
//! - the bot's account moves as it is: profile, avatar, followers, what it
//!   posted;
//! - its tokens are revoked (`bot_transferred`): they were made by the
//!   previous owner and shown to them. Independently of that, the verifier
//!   refuses a bot token whose creator no longer owns the bot, so a token
//!   minted in the moment of the hand-over, or a revocation that failed
//!   after the commit, cannot keep the previous owner in control;
//! - its webhooks are deleted: they point at the previous owner's servers
//!   and are signed with secrets the previous owner knows;
//! - who owned it before stays on record ([`Dev::bot_owner_at`]), so that a
//!   report about what the bot did leads to the person who ran it then.
//!
//! An offer reaches somebody's inbox, so it is refused (as an unknown
//! person, `RECIPIENT_NOT_FOUND`) towards anyone in a block relation with the
//! owner, and limited per owner. A block made after the offer ends it: the
//! accept asks again and answers `TRANSFER_NOT_FOUND`.
//!
//! The new owner gets what the bot received: its direct messages, inbox,
//! notes shared with it, readable with tokens they make. The offer and the
//! accept screens say so. (The bot's conversations are not told: the chat
//! domain has no way for another domain to post a notice into one.)
use std::time::Duration;

use tokuly_core::{
    events,
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_identity::users;
use tokuly_proto::dev::BotTransfer;

use crate::{Dev, events::BotTransferEvent, lock_limit};

/// How long an offer waits for its answer.
const OFFER_DAYS: i32 = 7;
/// Offers one person may make a day.
const OFFERS_PER_DAY: u32 = 20;

#[derive(Debug, Clone, sqlx::FromRow)]
struct TransferRow {
    id: Uuid,
    bot_id: Uuid,
    from_id: Uuid,
    to_id: Uuid,
    created_at: Time,
    expires_at: Time,
}

const TRANSFER_COLUMNS: &str = "t.id, t.bot_id, t.from_id, t.to_id, t.created_at, t.expires_at";

fn transfer_not_found() -> AppError {
    AppError::not_found("TRANSFER_NOT_FOUND", "no such offer")
}

impl Dev {
    async fn transfer_protos(&self, rows: &[TransferRow]) -> AppResult<Vec<BotTransfer>> {
        let ids: Vec<Uuid> = rows.iter().flat_map(|row| [row.bot_id, row.from_id, row.to_id]).collect();
        let summaries = tokuly_identity::summaries(&self.core, &ids).await?;
        Ok(rows
            .iter()
            .map(|row| BotTransfer {
                id: row.id.to_string(),
                bot: users::pick(&summaries, row.bot_id).into(),
                from: users::pick(&summaries, row.from_id).into(),
                to: users::pick(&summaries, row.to_id).into(),
                created_at: ts(row.created_at),
                expires_at: ts(row.expires_at),
                ..Default::default()
            })
            .collect())
    }

    /// Offers `bot_id` of `owner` to `recipient`, replacing an open offer of it.
    pub(crate) async fn offer_bot_transfer(
        &self,
        owner: Uuid,
        bot_id: Uuid,
        recipient: &str,
    ) -> AppResult<BotTransfer> {
        let bot = self.owned_bot(bot_id, owner).await?;
        ensure!(!bot.suspended, precondition, "BOT_SUSPENDED", "a suspended bot cannot be handed over");
        let gone = || AppError::not_found("RECIPIENT_NOT_FOUND", "no such person");
        let to_id = Uuid::parse_str(recipient.trim()).map_err(|_| gone())?;
        let person = users::find_by_id(&self.core, to_id).await?.ok_or_else(gone)?;
        let is_bot: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM dev_bots WHERE user_id = $1)")
            .bind(to_id)
            .fetch_one(&self.core.db)
            .await?;
        let reachable = to_id != owner
            && person.deleted_at.is_none()
            && person.banned_at.is_none()
            && !is_bot
            && self.identity.can_view(Some(owner), to_id).await?;
        ensure!(reachable, not_found, "RECIPIENT_NOT_FOUND", "no such person");
        // Each offer lands in somebody's inbox.
        self.core
            .cache
            .rate_limit(&format!("dev-bot-offer:{owner}"), OFFERS_PER_DAY, Duration::from_secs(24 * 3600))
            .await?;

        // Read before the transaction: it reads through the pool.
        let names = tokuly_identity::summaries(&self.core, &[owner, bot.user_id]).await?;
        let (from, bot_name) = (users::pick(&names, owner).display_name, users::pick(&names, bot.user_id).display_name);
        let mut tx = self.core.db.begin().await?;
        // A new offer replaces the open one, under a new ID: an answer to the old one finds nothing.
        let row: TransferRow = sqlx::query_as(
            "INSERT INTO dev_bot_transfers AS t (bot_id, from_id, to_id, expires_at)
             VALUES ($1, $2, $3, now() + make_interval(days => $4))
             ON CONFLICT (bot_id) WHERE accepted_at IS NULL
             DO UPDATE SET id = uuidv7(), from_id = EXCLUDED.from_id, to_id = EXCLUDED.to_id,
                           created_at = now(), expires_at = EXCLUDED.expires_at
             RETURNING t.id, t.bot_id, t.from_id, t.to_id, t.created_at, t.expires_at",
        )
        .bind(bot.user_id)
        .bind(owner)
        .bind(to_id)
        .bind(OFFER_DAYS)
        .fetch_one(&mut *tx)
        .await?;
        let notice = NotificationRequest::new(
            to_id,
            "dev.bot_transfer_offered",
            Localized::new("ボットを引き継いでほしいみたい!", "Someone wants to hand you a bot"),
            Localized::new(
                format!("{from}さんがボット「{bot_name}」をあなたに引き継ごうとしているよ。受け取るか決めてね!"),
                format!("{from} offers you their bot \"{bot_name}\". Accept or decline it."),
            ),
            "/developers/bots",
        )
        .actor(owner)
        .data(serde_json::json!({ "transfer_id": row.id, "bot_id": bot.user_id }));
        outbound::notify(&mut *tx, &notice).await?;
        tx.commit().await?;
        Ok(self.transfer_protos(&[row]).await?.remove(0))
    }

    /// The open offers `user_id` made and those made to them.
    pub(crate) async fn bot_transfers_of(&self, user_id: Uuid) -> AppResult<(Vec<BotTransfer>, Vec<BotTransfer>)> {
        // Only offers that can still be accepted: in time, of a bot the offerer still owns.
        let rows: Vec<TransferRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {TRANSFER_COLUMNS} FROM dev_bot_transfers t
             JOIN dev_bots b ON b.user_id = t.bot_id AND b.owner_id = t.from_id AND b.deleted_at IS NULL
             WHERE (t.from_id = $1 OR t.to_id = $1) AND t.accepted_at IS NULL AND t.expires_at > now()
             ORDER BY t.created_at DESC, t.id DESC"
        )))
        .bind(user_id)
        .fetch_all(&self.core.db)
        .await?;
        let (outgoing, incoming): (Vec<TransferRow>, Vec<TransferRow>) =
            rows.into_iter().partition(|row| row.from_id == user_id);
        Ok((self.transfer_protos(&outgoing).await?, self.transfer_protos(&incoming).await?))
    }

    /// Takes the bot of an offer made to `user_id` over. Returns the bot's ID.
    pub(crate) async fn accept_bot_transfer(&self, user_id: Uuid, transfer_id: Uuid) -> AppResult<Uuid> {
        // A block made since the offer ends it: neither side is handed anything by the other.
        // Asked before the transaction (it reads through the pool, the social domain's way);
        // the claim below names the same offerer, so the answer is about this very offer.
        let offerer: Uuid = sqlx::query_scalar(
            "SELECT from_id FROM dev_bot_transfers WHERE id = $1 AND to_id = $2 AND accepted_at IS NULL",
        )
        .bind(transfer_id)
        .bind(user_id)
        .fetch_optional(&self.core.db)
        .await?
        .ok_or_else(transfer_not_found)?;
        if !self.identity.can_view(Some(offerer), user_id).await? {
            return Err(transfer_not_found());
        }
        let mut tx = self.core.db.begin().await?;
        // The same lock as creating a bot: the allowance cannot be raced past.
        // Everything until the commit runs on this transaction.
        lock_limit(&mut tx, "dev-bots", user_id).await?;
        // Claimed exactly once: a second accept, a withdrawal or a new offer finds nothing.
        let (bot_id, from_id): (Uuid, Uuid) = sqlx::query_as(
            "UPDATE dev_bot_transfers SET accepted_at = now()
             WHERE id = $1 AND to_id = $2 AND from_id = $3 AND accepted_at IS NULL AND expires_at > now()
             RETURNING bot_id, from_id",
        )
        .bind(transfer_id)
        .bind(user_id)
        .bind(offerer)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(transfer_not_found)?;
        // The bot, locked against its owner's changes: still the offerer's, and both in good standing.
        let state: Option<(bool, bool)> = sqlx::query_as(
            "SELECT u.banned_at IS NOT NULL, o.banned_at IS NOT NULL OR o.deleted_at IS NOT NULL
             FROM dev_bots b JOIN users u ON u.id = b.user_id JOIN users o ON o.id = b.owner_id
             WHERE b.user_id = $1 AND b.owner_id = $2 AND b.deleted_at IS NULL AND u.deleted_at IS NULL
             FOR UPDATE OF b",
        )
        .bind(bot_id)
        .bind(from_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((suspended, offerer_gone)) = state else { return Err(transfer_not_found()) };
        ensure!(!suspended, precondition, "BOT_SUSPENDED", "a suspended bot cannot be handed over");
        if offerer_gone {
            return Err(transfer_not_found());
        }
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM dev_bots WHERE owner_id = $1 AND deleted_at IS NULL")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(count < self.cfg.max_bots, precondition, "TOO_MANY_BOTS", "at most {} bots", self.cfg.max_bots);
        sqlx::query("UPDATE dev_bots SET owner_id = $2 WHERE user_id = $1")
            .bind(bot_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        // The owner on the account itself, which every domain reads.
        tokuly_identity::bots::set_bot_owner(&mut tx, bot_id, user_id).await?;
        sqlx::query("DELETE FROM dev_webhooks WHERE account_id = $1").bind(bot_id).execute(&mut *tx).await?;
        let event = BotTransferEvent { bot_id, from_id, to_id: user_id };
        events::emit(&mut *tx, "dev.bot.transferred", Some(user_id), &event).await?;
        tx.commit().await?;
        self.subscribed.invalidate();
        // Refused already (their creator is not the owner); revoked for the record and the caches.
        self.revoke_tokens_of(bot_id, "bot_transferred", None).await?;
        Ok(bot_id)
    }

    /// Declines (the receiver) or withdraws (the offerer) an open offer.
    pub(crate) async fn cancel_bot_transfer(&self, user_id: Uuid, transfer_id: Uuid) -> AppResult<()> {
        let done = sqlx::query(
            "DELETE FROM dev_bot_transfers WHERE id = $1 AND accepted_at IS NULL AND (from_id = $2 OR to_id = $2)",
        )
        .bind(transfer_id)
        .bind(user_id)
        .execute(&self.core.db)
        .await?;
        ensure!(done.rows_affected() == 1, not_found, "TRANSFER_NOT_FOUND", "no such offer");
        Ok(())
    }

    /// Who owned a bot at `at`: the one who handed it over after that time,
    /// or its owner of record ([`Dev::bot_owner`]). For moderation: a report
    /// about something the bot did leads to whoever ran it then.
    pub async fn bot_owner_at(&self, bot_id: Uuid, at: Time) -> AppResult<Option<Uuid>> {
        let before: Option<Uuid> = sqlx::query_scalar(
            "SELECT from_id FROM dev_bot_transfers WHERE bot_id = $1 AND accepted_at > $2 ORDER BY accepted_at LIMIT 1",
        )
        .bind(bot_id)
        .bind(at)
        .fetch_optional(&self.core.db)
        .await?;
        match before {
            Some(owner) => Ok(Some(owner)),
            None => self.bot_owner(bot_id).await,
        }
    }
}
