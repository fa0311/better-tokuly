//! Background work: the event consumer and the maintenance timer.
//!
//! The wallet listens to its own `wallet.balance.changed` events. The ledger
//! is called from many places (this crate, live gifts, stamp sales) and only
//! records the movement; turning a movement into a realtime push and, where
//! it makes sense, a "coins received" notification happens here, once, for
//! all of them.
use std::time::Duration;

use tokuly_core::{
    events::{self, ConsumeOptions, Event},
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_identity::maintenance::UserRef;
use tokuly_proto::wallet::{self as pb, wallet_event};

use crate::{
    Wallet, describe,
    entries::EntryRow,
    events::{BALANCE_CHANGED, BalanceChanged, USER_DELETED},
    money::format_coins,
};

/// Days a closed payout request keeps its (encrypted) destination.
const DESTINATION_RETENTION_DAYS: i32 = 90;
/// Days a "notification sent" marker is kept; far longer than any event redelivery.
const NOTICE_RETENTION_DAYS: i32 = 30;
/// The maintenance timer fires every minute; housekeeping that is not urgent
/// runs on every this-many-th tick.
const HOUSEKEEPING_EVERY_TICKS: u64 = 60;

impl Wallet {
    /// Starts the wallet's workers: the event consumer and the maintenance timer.
    pub fn spawn_workers(&self) {
        let wallet = self.clone();
        tokio::spawn(async move {
            let handler = wallet.clone();
            events::consume(wallet.core.bus.clone(), "wallet", &[BALANCE_CHANGED, USER_DELETED], ConsumeOptions::default(), wallet.core.shutdown.clone(), move |event| {
                let wallet = handler.clone();
                async move { wallet.handle_event(event).await }
            })
            .await;
        });

        let wallet = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            let mut ticks: u64 = 0;
            loop {
                tokio::select! {
                    _ = wallet.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        wallet.run_maintenance(ticks.is_multiple_of(HOUSEKEEPING_EVERY_TICKS)).await;
                        ticks += 1;
                    }
                }
            }
        });
    }

    /// Handles one consumed event. Events are delivered at least once, so
    /// everything behind this is safe to repeat.
    pub async fn handle_event(&self, event: Event) -> AppResult<()> {
        match event.topic.as_str() {
            BALANCE_CHANGED => self.on_balance_changed(event.data()?).await,
            USER_DELETED => self.on_user_deleted(event.data::<UserRef>()?.user_id).await,
            _ => Ok(()),
        }
    }

    /// One round of periodic work. Failures are logged and retried next round.
    pub async fn run_maintenance(&self, housekeeping: bool) {
        match self.expire_due_purchases().await {
            Ok(0) => {}
            Ok(count) => tracing::info!(count, "closed overdue purchases"),
            Err(e) => tracing::error!(error = %e, "closing overdue purchases failed"),
        }
        if !housekeeping {
            return;
        }
        match self.erase_old_destinations(DESTINATION_RETENTION_DAYS).await {
            Ok(0) => {}
            Ok(count) => tracing::info!(count, "erased payout destinations of long-closed requests"),
            Err(e) => tracing::error!(error = %e, "erasing old payout destinations failed"),
        }
        if let Err(e) = self.prune_notices(NOTICE_RETENTION_DAYS).await {
            tracing::error!(error = %e, "pruning notification markers failed");
        }
    }

    /// A balance changed: push it to the user's devices and, when someone
    /// else put coins into their wallet, tell them.
    async fn on_balance_changed(&self, change: BalanceChanged) -> AppResult<()> {
        let locale = match self.user_prefs(change.user_id).await {
            Ok(prefs) => prefs.locale,
            // The account is gone for good; there is nobody to tell.
            Err(e) if e.is_not_found() => return Ok(()),
            Err(e) => return Err(e),
        };
        let Some(entry) = self.find_entry(change.entry_id, change.user_id).await? else {
            // Cannot happen while entries are never deleted; if it does, the
            // current balance is still worth pushing.
            let balance = self.balance_snapshot(change.user_id).await?;
            self.push(change.user_id, wallet_event::Event::BalanceChanged(Box::new(pb::BalanceChanged { balance: balance.into(), ..Default::default() }))).await;
            return Ok(());
        };
        // The event carries the balance right after this entry, and the
        // entry's revision orders it against every other snapshot the client
        // has seen, so a late or repeated delivery cannot roll the display back.
        let balance = pb::Balance { coins: change.coins, earnings: change.earnings, revision: entry.revision, ..Default::default() };
        let mut entries = self.entries_to_proto(std::slice::from_ref(&entry), change.user_id, &locale).await?;
        let pushed = pb::BalanceChanged { balance: balance.into(), entry: entries.pop().into(), ..Default::default() };
        self.push(change.user_id, wallet_event::Event::BalanceChanged(Box::new(pushed))).await;

        if let Some(notice) = self.received_notice(&entry, change.user_id).await? {
            self.notify_once(entry.id, change.user_id, &notice).await?;
        }
        Ok(())
    }

    /// The "coins received" notification an entry deserves for `user_id`, if any.
    async fn received_notice(&self, entry: &EntryRow, user_id: Uuid) -> AppResult<Option<NotificationRequest>> {
        let Some(received) = Received::of(entry, user_id) else { return Ok(None) };
        let description = describe::describe(&entry.kind, &entry.reason, entry.coins_delta, entry.earnings_delta);
        let data = serde_json::json!({ "entry_id": entry.id, "reason": entry.reason, "coins": received.coins() });
        let notice = match received {
            Received::Transfer { from, net } => {
                let amount = format_coins(net);
                let title = Localized::new("コインを受け取りました!", "You received coins!");
                let (body, collapse) = match from {
                    Some(from) => {
                        let sender = tokuly_identity::summary(&self.core, from).await?;
                        let name = if sender.display_name.is_empty() { sender.handle } else { sender.display_name };
                        let body = Localized::new(format!("{name}さんから +{amount}コイン（{}）", description.ja), format!("+{amount} coins from {name} ({})", description.en));
                        (body, format!("wallet.received:{}:{from}", entry.reason))
                    }
                    None => (Localized::new(format!("+{amount}コイン（{}）", description.ja), format!("+{amount} coins ({})", description.en)), format!("wallet.received:{}", entry.reason)),
                };
                let notice = NotificationRequest::new(user_id, "wallet.received", title, body, "/wallet/earnings").collapse(collapse);
                match from {
                    Some(from) => notice.actor(from),
                    None => notice,
                }
            }
            Received::Refund { coins } => {
                let amount = format_coins(coins);
                NotificationRequest::new(
                    user_id,
                    "wallet.received",
                    Localized::new("コインが返金されました", "Coins refunded"),
                    Localized::new(format!("+{amount}コインがウォレットに戻ったよ。"), format!("{amount} coins are back in your wallet.")),
                    "/wallet",
                )
            }
            Received::Adjustment { coins, earnings } => {
                let amount = format_coins(coins);
                let (ja_balance, en_balance, path) = if earnings { ("売上", "earnings", "/wallet/earnings") } else { ("ウォレット", "wallet", "/wallet") };
                NotificationRequest::new(
                    user_id,
                    "wallet.received",
                    Localized::new("コインが届きました!", "Coins added!"),
                    Localized::new(format!("運営から +{amount}コインが{ja_balance}に追加されたよ!"), format!("Tokuly staff added {amount} coins to your {en_balance}.")),
                    path,
                )
            }
        };
        Ok(Some(notice.data(data)))
    }

    /// Requests a notification about an entry at most once per recipient,
    /// however often the triggering event is redelivered: the marker row and
    /// the notification commit together.
    async fn notify_once(&self, entry_id: Uuid, user_id: Uuid, notice: &NotificationRequest) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let first = sqlx::query("INSERT INTO wallet_entry_notices (entry_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(entry_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 1;
        if first {
            outbound::notify(&mut *tx, notice).await?;
            tx.commit().await?;
        }
        Ok(())
    }

    async fn prune_notices(&self, days: i32) -> AppResult<u64> {
        let pruned = sqlx::query("DELETE FROM wallet_entry_notices WHERE created_at < now() - make_interval(days => $1)").bind(days).execute(&self.core.db).await?;
        Ok(pruned.rows_affected())
    }
}

/// Coins that reached a user through someone else's action — the cases worth
/// a notification. Movements the user made themselves (purchases, the daily
/// bonus, their own payout requests) and money leaving are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Received {
    /// Another user sent coins; `net` is what arrived in earnings.
    Transfer { from: Option<Uuid>, net: i64 },
    /// A spend or transfer was reversed and the coins came back.
    Refund { coins: i64 },
    /// Staff added coins (`earnings`: to the earnings balance).
    Adjustment { coins: i64, earnings: bool },
}

impl Received {
    fn of(entry: &EntryRow, user_id: Uuid) -> Option<Self> {
        match entry.kind.as_str() {
            "transfer" if entry.to_user_id == Some(user_id) && entry.earnings_delta > 0 => Some(Self::Transfer { from: entry.from_user_id, net: entry.earnings_delta }),
            // Payout holds coming back raise earnings, not coins, and have
            // their own `wallet.payout` notification.
            "refund" if entry.coins_delta > 0 => Some(Self::Refund { coins: entry.coins_delta }),
            "adjustment" if entry.coins_delta > 0 => Some(Self::Adjustment { coins: entry.coins_delta, earnings: false }),
            "adjustment" if entry.earnings_delta > 0 => Some(Self::Adjustment { coins: entry.earnings_delta, earnings: true }),
            _ => None,
        }
    }

    fn coins(self) -> i64 {
        match self {
            Self::Transfer { net, .. } => net,
            Self::Refund { coins } | Self::Adjustment { coins, .. } => coins,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &str, reason: &str, from: Option<Uuid>, to: Option<Uuid>, coins_delta: i64, earnings_delta: i64) -> EntryRow {
        EntryRow {
            id: new_id(),
            kind: kind.into(),
            reason: reason.into(),
            from_user_id: from,
            to_user_id: to,
            amount: coins_delta.abs().max(earnings_delta.abs()).max(1),
            fee: 0,
            refunded: false,
            refund_of: None,
            memo: serde_json::json!({}),
            created_at: now(),
            coins_delta,
            earnings_delta,
            revision: 1,
            position: 1,
        }
    }

    #[test]
    fn only_the_recipient_of_a_transfer_is_told() {
        let (payer, recipient) = (new_id(), new_id());
        let received = entry("transfer", "live.gift", Some(payer), Some(recipient), 0, 210);
        assert_eq!(Received::of(&received, recipient), Some(Received::Transfer { from: Some(payer), net: 210 }));
        let sent = entry("transfer", "live.gift", Some(payer), Some(recipient), -300, 0);
        assert_eq!(Received::of(&sent, payer), None);
    }

    #[test]
    fn refunds_and_staff_credits_are_announced() {
        let user = new_id();
        assert_eq!(Received::of(&entry("refund", "live.gift.returned", None, Some(user), 300, 0), user), Some(Received::Refund { coins: 300 }));
        assert_eq!(Received::of(&entry("adjustment", "adjustment", None, Some(user), 50, 0), user), Some(Received::Adjustment { coins: 50, earnings: false }));
        assert_eq!(Received::of(&entry("adjustment", "adjustment", None, Some(user), 0, 70), user), Some(Received::Adjustment { coins: 70, earnings: true }));
    }

    #[test]
    fn own_actions_and_losses_stay_quiet() {
        let user = new_id();
        let quiet = [
            entry("grant", "purchase", None, Some(user), 500, 0),
            entry("grant", "daily_bonus", None, Some(user), 10, 0),
            entry("spend", "stamp.pack", Some(user), None, -120, 0),
            entry("payout", "payout", Some(user), None, 0, -1000),
            entry("refund", "payout.rejected", None, Some(user), 0, 1000),
            entry("refund", "live.gift.returned", Some(user), None, 0, -210),
            entry("adjustment", "adjustment", Some(user), None, -50, 0),
        ];
        for e in quiet {
            assert_eq!(Received::of(&e, user), None, "{} / {}", e.kind, e.reason);
        }
    }
}
