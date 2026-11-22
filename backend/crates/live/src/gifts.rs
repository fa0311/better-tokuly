//! Gifts (super chat): coins from a viewer to the streamer, with a message
//! highlighted in chat according to the gift's tier ([`crate::tiers`]).
//!
//! The coins move through the wallet: the payer's coins become the
//! earnings of the channel's owner, minus the platform fee. The transfer
//! hides its recipient (`Wallet::transfer_with_fee_hiding_recipient`): the
//! sender's wallet history shows what was paid (the memo names the channel
//! and the stream), never the account behind the channel.
//!
//! # Exactly once, and never lost
//!
//! A gift is written down before any coin moves, and finished afterwards:
//!
//! 1. **PENDING**: after validation the gift row is inserted with
//!    everything the gift contains (coins, tier, message, highlight time,
//!    decided now and never recomputed). The row is unique per sender and
//!    idempotency key, so a repeated request finds it instead of making a
//!    second one;
//! 2. **the payment**: the wallet moves the coins under the ledger key
//!    `live.gift:<gift id>`, at most once however often it is asked;
//! 3. **SETTLED**: one transaction flips the row from PENDING to SETTLED
//!    (`UPDATE … WHERE status = 1`, so only one caller gets to do it) and
//!    with it writes the chat line, the stream's counters, the event and
//!    the notification.
//!
//! A request that dies between the steps leaves a PENDING row, and every
//! later entry point finishes it by running steps 2 and 3 again
//! ([`Live::settle_gift`]): the sender's next `SendGift` or
//! `ListSentGifts`, and the worker's sweep. A payment the wallet refuses
//! (not enough coins) removes the PENDING row: nothing happened.
use std::{collections::HashMap, time::Duration};

use tokuly_core::{
    analytics::AnalyticsEvent,
    events as core_events,
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    common::{ChannelSummary, UserSummary},
    live::{Alert, AlertKind, ChatRole, Gift, OverlayKind, Supporter, live_event::Event},
};
use tokuly_video::channels;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    Live, access, audience,
    chat::{self, ChatRow, NewMessage},
    events::{GIFT_SENT, GiftSent},
    overlays,
    streams::{self, StreamRow},
    tiers,
};

pub const PENDING: i32 = 1;
pub const SETTLED: i32 = 2;

/// Pending gifts younger than this are left to the request that is making them.
const RECOVERY_AGE_SECONDS: f64 = 15.0;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GiftRow {
    pub id: Uuid,
    /// `None` once the stream was erased.
    pub stream_id: Option<Uuid>,
    pub stream_title: Option<String>,
    pub channel_id: Uuid,
    pub recipient_id: Option<Uuid>,
    /// `None` once the sender's account was deleted.
    pub sender_id: Option<Uuid>,
    /// PENDING until the wallet has moved the coins.
    pub status: i32,
    pub coins: i64,
    pub fee: i64,
    pub tier: i32,
    pub message: String,
    /// How long the message is highlighted, as decided when the gift was made.
    pub highlight_seconds: i32,
    pub offset_ms: i64,
    pub created_at: Time,
}

macro_rules! gift_columns {
    () => {
        "g.id, g.stream_id, \
         (SELECT s.title FROM live_streams s WHERE s.id = g.stream_id AND s.deleted_at IS NULL) AS stream_title, g.channel_id, \
         g.recipient_id, g.sender_id, g.status, g.coins, g.fee, g.tier, g.message, g.highlight_seconds, g.offset_ms, g.created_at"
    };
}

impl GiftRow {
    pub fn is_settled(&self) -> bool {
        self.status == SETTLED
    }
}

/// The key a client names a gift attempt with: 8-64 characters of letters,
/// digits, `-` and `_`.
pub fn validate_idempotency_key(key: &str) -> AppResult<&str> {
    ensure!(
        (8..=64).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        invalid,
        "IDEMPOTENCY_KEY_INVALID",
        "the idempotency key is 8 to 64 letters, digits, '-' or '_'"
    );
    Ok(key)
}

/// The message of a gift of `tier`: one line, no longer than the tier allows.
pub fn validate_message(input: &str, tier: &tiers::Tier) -> AppResult<String> {
    let message: String = input.chars().map(|c| if c.is_whitespace() { ' ' } else { c }).collect();
    let message = message.trim();
    ensure!(
        !message.chars().any(char::is_control),
        invalid,
        "GIFT_MESSAGE_INVALID",
        "the message contains characters that cannot be shown"
    );
    let length = message.graphemes(true).count();
    ensure!(
        length <= tier.max_message_length,
        invalid,
        "GIFT_MESSAGE_TOO_LONG",
        "a gift of this size can carry a message of up to {} characters",
        tier.max_message_length
    );
    Ok(message.to_owned())
}

/// Gifts as the API returns them. `for_streamer` adds what reached the
/// streamer's earnings.
pub async fn to_protos(core: &Core, rows: &[GiftRow], for_streamer: bool) -> AppResult<Vec<Gift>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut sender_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.sender_id).collect();
    sender_ids.sort_unstable();
    sender_ids.dedup();
    let senders: HashMap<Uuid, UserSummary> = tokuly_identity::summaries(core, &sender_ids).await?;
    let mut channel_ids: Vec<Uuid> = rows.iter().map(|row| row.channel_id).collect();
    channel_ids.sort_unstable();
    channel_ids.dedup();
    let channels: HashMap<Uuid, ChannelSummary> = channels::summaries(core, &channel_ids).await?;
    Ok(rows
        .iter()
        .map(|row| Gift {
            id: row.id.to_string(),
            stream_id: row.stream_id.map(|id| id.to_string()).unwrap_or_default(),
            stream_title: row.stream_title.clone().unwrap_or_default(),
            channel: channels.get(&row.channel_id).cloned().into(),
            sender: row.sender_id.and_then(|id| senders.get(&id).cloned()).into(),
            coins: row.coins,
            tier: row.tier,
            message: row.message.clone(),
            created_at: ts(row.created_at),
            offset_ms: row.offset_ms,
            net_coins: if for_streamer { row.coins - row.fee } else { 0 },
            ..Default::default()
        })
        .collect())
}

async fn find_by_key(core: &Core, sender_id: Uuid, client_key: &str) -> AppResult<Option<GiftRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        gift_columns!(),
        " FROM live_gifts g WHERE g.sender_id = $1 AND g.client_key = $2"
    ))
    .bind(sender_id)
    .bind(client_key)
    .fetch_optional(&core.db)
    .await?)
}

/// A gift by ID, pending or settled.
pub async fn find_by_id(core: &Core, gift_id: Uuid) -> AppResult<Option<GiftRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", gift_columns!(), " FROM live_gifts g WHERE g.id = $1"))
        .bind(gift_id)
        .fetch_optional(&core.db)
        .await?)
}

/// The chat line of a gift, unless a moderator removed it.
pub async fn chat_row_of(core: &Core, gift_id: Uuid) -> AppResult<Option<ChatRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        chat::chat_columns!(),
        " FROM live_chat_messages m WHERE m.gift_id = $1 AND m.deleted_at IS NULL"
    ))
    .bind(gift_id)
    .fetch_optional(&core.db)
    .await?)
}

/// Gifts `sender_id` sent, before `before`, newest first.
pub async fn list_sent(core: &Core, sender_id: Uuid, before: Option<Uuid>, limit: i64) -> AppResult<Vec<GiftRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        gift_columns!(),
        " FROM live_gifts g WHERE g.sender_id = $1 AND g.status = 2 AND ($2::uuid IS NULL OR g.id < $2) ORDER BY g.id DESC LIMIT $3"
    ))
    .bind(sender_id)
    .bind(before)
    .bind(limit)
    .fetch_all(&core.db)
    .await?)
}

/// Gifts a channel received (on one stream, if given), newest first.
pub async fn list_received(
    core: &Core,
    channel_id: Uuid,
    stream_id: Option<Uuid>,
    before: Option<Uuid>,
    limit: i64,
) -> AppResult<Vec<GiftRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        gift_columns!(),
        " FROM live_gifts g
          WHERE g.channel_id = $1 AND g.status = 2 AND ($2::uuid IS NULL OR g.stream_id = $2) AND ($3::uuid IS NULL OR g.id < $3)
          ORDER BY g.id DESC LIMIT $4"
    ))
    .bind(channel_id)
    .bind(stream_id)
    .bind(before)
    .bind(limit)
    .fetch_all(&core.db)
    .await?)
}

/// Coins a channel received (on one stream, if given): before and after the fee.
pub async fn received_totals(core: &Core, channel_id: Uuid, stream_id: Option<Uuid>) -> AppResult<(i64, i64)> {
    Ok(sqlx::query_as(
        "SELECT COALESCE(sum(coins), 0)::bigint, COALESCE(sum(coins - fee), 0)::bigint FROM live_gifts
         WHERE channel_id = $1 AND status = 2 AND ($2::uuid IS NULL OR stream_id = $2)",
    )
    .bind(channel_id)
    .bind(stream_id)
    .fetch_one(&core.db)
    .await?)
}

/// Who gave the most on a stream.
pub async fn top_supporters(core: &Core, stream_id: Uuid, limit: i64) -> AppResult<Vec<Supporter>> {
    let rows: Vec<(Uuid, i64, i64)> = sqlx::query_as(
        "SELECT sender_id, sum(coins)::bigint AS coins, count(*) AS gifts FROM live_gifts
         WHERE stream_id = $1 AND status = 2 AND sender_id IS NOT NULL
         GROUP BY sender_id ORDER BY coins DESC, min(created_at), sender_id LIMIT $2",
    )
    .bind(stream_id)
    .bind(limit)
    .fetch_all(&core.db)
    .await?;
    let ids: Vec<Uuid> = rows.iter().map(|(id, _, _)| *id).collect();
    let users = tokuly_identity::summaries(core, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|(id, coins, gift_count)| Supporter {
            user: Some(tokuly_identity::users::pick(&users, id)).into(),
            coins,
            gift_count,
            ..Default::default()
        })
        .collect())
}

/// Whether the wallet's answer leaves it open if the coins moved (a
/// crash, a timeout) as opposed to a clear refusal (not enough coins).
fn outcome_unknown(error: &AppError) -> bool {
    use connectrpc::ErrorCode;
    matches!(
        error.code,
        ErrorCode::Internal
            | ErrorCode::Unknown
            | ErrorCode::Unavailable
            | ErrorCode::DeadlineExceeded
            | ErrorCode::Aborted
    )
}

impl Live {
    /// Sends a gift from `sender_id` to the streamer of `stream`. Returns
    /// the gift and the chat line it produced (`None` when a moderator has
    /// removed the line since). Repeating the call with the same key
    /// returns the first gift.
    pub async fn give_gift(
        &self,
        stream: &StreamRow,
        sender_id: Uuid,
        coins: i64,
        message: &str,
        client_key: &str,
    ) -> AppResult<(GiftRow, Option<ChatRow>)> {
        let core = &self.core;
        let client_key = validate_idempotency_key(client_key)?;
        // Unfinished business first: a gift of this sender whose payment was interrupted.
        self.settle_pending_gifts(Some(sender_id)).await;
        if let Some(existing) = find_by_key(core, sender_id, client_key).await? {
            let gift = self.settle_gift(existing.id).await?;
            return Ok((gift.clone(), chat_row_of(core, gift.id).await?));
        }

        ensure!(stream.is_live(), precondition, "STREAM_NOT_LIVE", "gifts can be sent while the stream is on air");
        ensure!(stream.gifts_enabled, precondition, "GIFTS_DISABLED", "this stream does not accept gifts");
        ensure!(sender_id != stream.owner_id, invalid, "GIFT_OWN_STREAM", "you cannot send a gift to your own stream");
        let tier = tiers::tier_for(coins).ok_or_else(|| {
            AppError::invalid(
                "GIFT_AMOUNT_INVALID",
                format!("a gift is {} to {} coins", tiers::MIN_GIFT_COINS, tiers::MAX_GIFT_COINS),
            )
        })?;
        let message = validate_message(message, tier)?;
        let standing = access::standing(core, stream.channel_id, stream.owner_id, sender_id).await?;
        // Gifts follow blocks, bans and timeouts, but not the chat's
        // audience modes: a closed chat still takes gifts.
        if let Some(block) =
            chat::chat_block(core, stream, &crate::streams::default_chat_settings(), sender_id, standing).await?
        {
            return Err(block.into_error());
        }
        if !message.is_empty() && !standing.can_moderate() {
            ensure!(
                chat::banned_word(&chat::banned_words(core, stream.channel_id).await?, &message).is_none(),
                invalid,
                "MESSAGE_BLOCKED",
                "this message contains a word the streamer does not allow"
            );
        }
        if !standing.can_moderate() {
            ensure!(
                stream.chat_settings().link_policy != tokuly_proto::live::LinkPolicy::LINK_POLICY_BLOCK
                    || !chat::contains_link(&message),
                invalid,
                "LINKS_NOT_ALLOWED",
                "links are not allowed in this chat"
            );
        }
        core.cache.rate_limit(&format!("live-gift:{sender_id}"), 30, Duration::from_secs(60)).await?;

        // Written down before any coin moves, with everything the gift contains.
        let inserted: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO live_gifts (stream_id, channel_id, recipient_id, sender_id, coins, tier, message, highlight_seconds, client_key)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (sender_id, client_key) DO NOTHING RETURNING id",
        )
        .bind(stream.id)
        .bind(stream.channel_id)
        .bind(stream.owner_id)
        .bind(sender_id)
        .bind(coins)
        .bind(tier.level)
        .bind(&message)
        .bind(tier.highlight_seconds as i32)
        .bind(client_key)
        .fetch_optional(&core.db)
        .await?;
        let gift_id = match inserted {
            Some(gift_id) => gift_id,
            // The same request arrived twice at once: both finish the one gift.
            None => {
                find_by_key(core, sender_id, client_key).await?.or_not_found("GIFT_NOT_FOUND", "gift not found")?.id
            }
        };
        let gift = self.settle_gift(gift_id).await?;
        Ok((gift.clone(), chat_row_of(core, gift.id).await?))
    }

    /// Finishes a gift: has the wallet move its coins (once, however often
    /// this runs) and makes the gift real. Returns the settled gift. A
    /// payment the wallet refuses removes the pending gift and returns the
    /// wallet's error (`INSUFFICIENT_COINS`).
    pub async fn settle_gift(&self, gift_id: Uuid) -> AppResult<GiftRow> {
        let core = &self.core;
        let gone = || AppError::not_found("GIFT_NOT_FOUND", "gift not found");
        let gift = find_by_id(core, gift_id).await?.ok_or_else(gone)?;
        if gift.is_settled() {
            return Ok(gift);
        }
        let cancel = || async {
            sqlx::query("DELETE FROM live_gifts WHERE id = $1 AND status = 1").bind(gift_id).execute(&core.db).await?;
            AppResult::Ok(())
        };
        let (Some(sender_id), Some(recipient_id)) = (gift.sender_id, gift.recipient_id) else {
            // One of the two accounts is gone: there is nobody to pay or to be paid.
            cancel().await?;
            return Err(gone());
        };
        let memo =
            serde_json::json!({ "gift_id": gift.id, "stream_id": gift.stream_id, "channel_id": gift.channel_id });
        let ledger_key = format!("live.gift:{}", gift.id);
        let receipt = match self
            .wallet
            .transfer_with_fee_hiding_recipient(sender_id, recipient_id, gift.coins, "live.gift", &ledger_key, memo)
            .await
        {
            Ok(receipt) => receipt,
            // Whether the coins moved is unknown: the gift stays pending and is finished later.
            Err(error) if outcome_unknown(&error) => return Err(error),
            Err(error) => {
                cancel().await?;
                return Err(error);
            }
        };

        // Everything the last step needs is read before its transaction.
        let stream = match gift.stream_id {
            Some(stream_id) => streams::find_by_id(core, stream_id).await?,
            None => None,
        };
        if stream.is_none() {
            // The stream was deleted while the gift was on its way: there is
            // no chat to show it in and nothing it was given for. The coins
            // go back and the gift is dropped. (If they cannot go back any
            // more, the gift stands as bookkeeping.)
            match tokuly_wallet::ledger::refund(core, receipt.entry_id, "live.gift.returned").await {
                Ok(()) => {
                    cancel().await?;
                    return Err(AppError::precondition(
                        "GIFT_RETURNED",
                        "the stream is gone; your coins were returned",
                    ));
                }
                Err(error) if error.reason == "REFUND_IMPOSSIBLE" => {}
                Err(error) => return Err(error),
            }
        }
        let sender = tokuly_identity::summary(core, sender_id).await?;
        let sender_name =
            if sender.display_name.is_empty() { sender.handle.clone() } else { sender.display_name.clone() };
        let role = match &stream {
            Some(stream) => {
                let standing = access::standing(core, stream.channel_id, stream.owner_id, sender_id).await?;
                chat::role_from_standing(standing, channels::is_subscribed(core, sender_id, stream.channel_id).await?)
            }
            None => ChatRole::CHAT_ROLE_VIEWER,
        };
        let dashboard_open = match &stream {
            Some(stream) => audience::dashboard_open(core, stream.id).await.unwrap_or(false),
            None => true,
        };

        let mut tx = core.db.begin().await?;
        // Only one caller gets to settle a gift.
        let settled = sqlx::query(
            "UPDATE live_gifts SET status = 2, fee = $2, ledger_entry_id = $3, settled_at = now() WHERE id = $1 AND status = 1",
        )
        .bind(gift.id)
        .bind(receipt.fee)
        .bind(receipt.entry_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if !settled {
            drop(tx);
            return find_by_id(core, gift_id).await?.ok_or_else(gone);
        }
        let mut line = None;
        if let Some(stream) = &stream {
            let inserted = chat::insert_in(
                &mut tx,
                NewMessage {
                    stream_id: stream.id,
                    user_id: Some(sender_id),
                    kind: chat::KIND_GIFT,
                    role,
                    text: &gift.message,
                    stamp_id: None,
                    gift: Some((gift.id, gift.coins, gift.tier, i64::from(gift.highlight_seconds))),
                    notice: None,
                },
            )
            .await?;
            sqlx::query("UPDATE live_gifts SET offset_ms = $2 WHERE id = $1")
                .bind(gift.id)
                .bind(inserted.offset_ms)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE live_streams SET gift_coins = gift_coins + $2, gift_net_coins = gift_net_coins + $3, gift_count = gift_count + 1
                 WHERE id = $1",
            )
            .bind(stream.id)
            .bind(gift.coins)
            .bind(receipt.net)
            .execute(&mut *tx)
            .await?;
            if !dashboard_open {
                // The streamer is not looking at their dashboard: tell them.
                let request = NotificationRequest::new(
                    stream.owner_id,
                    "live.gift",
                    Localized::new("ギフトが届いたよ!", "You got a gift!"),
                    Localized::new(
                        format!("{sender_name}さんから{}コインのギフト!「{}」", gift.coins, stream.title),
                        format!("{sender_name} sent {} coins on {}", gift.coins, stream.title),
                    ),
                    stream.path(),
                )
                .actor(sender_id)
                .subject(crate::streams::SUBJECT, stream.id)
                .collapse(format!("live.gift:{}", stream.id))
                .data(serde_json::json!({
                    "stream_id": stream.id, "short_id": stream.short_id, "gift_id": gift.id, "coins": gift.coins, "tier": gift.tier,
                }));
                outbound::notify(&mut *tx, &request).await?;
            }
            line = Some(inserted);
        }
        let event = GiftSent {
            gift_id: gift.id,
            stream_id: gift.stream_id,
            channel_id: gift.channel_id,
            sender_id,
            recipient_id,
            coins: gift.coins,
            net_coins: receipt.net,
            tier: gift.tier,
        };
        core_events::emit(&mut *tx, GIFT_SENT, Some(sender_id), &event).await?;
        tx.commit().await?;

        if let (Some(stream), Some(line)) = (&stream, &line) {
            core.analytics
                .track(
                    AnalyticsEvent::new("live.gift")
                        .user(Some(sender_id))
                        .subject("live_stream", stream.id)
                        .owner(stream.owner_id)
                        .value(gift.coins as f64)
                        .props(serde_json::json!({ "channel_id": stream.channel_id, "tier": gift.tier, "net": receipt.net })),
                )
                .await;
            self.push_chat_message(stream, line).await;
            let alert = Alert {
                kind: AlertKind::ALERT_KIND_GIFT.into(),
                name: sender_name,
                avatar: sender.avatar.clone(),
                coins: gift.coins,
                tier: gift.tier,
                message: gift.message.clone(),
                ..Default::default()
            };
            self.push_to_overlays(
                stream.channel_id,
                &[OverlayKind::OVERLAY_KIND_ALERT_BOX],
                Event::Alert(Box::new(alert)),
            )
            .await;
        }
        overlays::push_goal_progress(self, gift.channel_id).await;
        find_by_id(core, gift_id).await?.ok_or_else(gone)
    }

    /// Finishes the gifts whose payment was interrupted: those of one
    /// sender (called when they come back) or everyone's (the worker's
    /// sweep). Never fails the caller. Returns how many were settled.
    pub async fn settle_pending_gifts(&self, sender_id: Option<Uuid>) -> u64 {
        let pending: Vec<Uuid> = match sqlx::query_scalar(
            "SELECT id FROM live_gifts
             WHERE status = 1 AND created_at < now() - make_interval(secs => $2) AND ($1::uuid IS NULL OR sender_id = $1)
             ORDER BY id LIMIT 200",
        )
        .bind(sender_id)
        .bind(RECOVERY_AGE_SECONDS)
        .fetch_all(&self.core.db)
        .await
        {
            Ok(pending) => pending,
            Err(error) => {
                tracing::warn!(%error, "could not look for unfinished gifts");
                return 0;
            }
        };
        let mut settled = 0;
        for gift_id in pending {
            match self.settle_gift(gift_id).await {
                Ok(gift) if gift.is_settled() => settled += 1,
                Ok(_) => {}
                // Refused by the wallet (and removed), or still not possible: the next sweep looks again.
                Err(error) => tracing::info!(%gift_id, %error, "an unfinished gift could not be settled"),
            }
        }
        settled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idempotency_keys_are_short_url_safe_strings() {
        assert!(validate_idempotency_key("gift-0001_abc").is_ok());
        for bad in ["short", "has space 1", "ümlaut-key-1", &"x".repeat(65), ""] {
            assert_eq!(validate_idempotency_key(bad).unwrap_err().reason, "IDEMPOTENCY_KEY_INVALID", "{bad:?}");
        }
    }

    #[test]
    fn a_gift_message_may_be_as_long_as_its_tier_allows() {
        let smallest = tiers::tier_for(10).unwrap();
        assert_eq!(validate_message("   ", smallest).unwrap(), "");
        assert_eq!(validate_message("がんばれ", smallest).unwrap_err().reason, "GIFT_MESSAGE_TOO_LONG");
        let second = tiers::tier_for(50).unwrap();
        assert_eq!(validate_message(" がんばれ!\n応援してる ", second).unwrap(), "がんばれ! 応援してる");
        assert_eq!(validate_message(&"あ".repeat(51), second).unwrap_err().reason, "GIFT_MESSAGE_TOO_LONG");
        assert_eq!(validate_message("a\u{7}b", second).unwrap_err().reason, "GIFT_MESSAGE_INVALID");
        let biggest = tiers::tier_for(tiers::MAX_GIFT_COINS).unwrap();
        assert!(validate_message(&"あ".repeat(350), biggest).is_ok());
    }
}
