//! Reading the ledger: balance snapshots and per-user history.
//!
//! A history line is a ledger entry seen from one user's side: how much each
//! of *their* balances moved (taken from the postings, so it is exact for
//! every kind of entry, present and future) and who the other party was.
//!
//! History is read through the postings of the user's two accounts. Posting
//! IDs come from a sequence and are handed out while the user's account row
//! is locked, so for one user they increase in exactly the order the
//! movements happened. That makes a posting ID both the sort key of the
//! history and the `revision` of the balance right after the movement.
use std::collections::HashMap;

use tokuly_core::prelude::*;
use tokuly_proto::{
    common::{PageInfo, UserSummary},
    wallet::{Balance, LedgerAccount, LedgerEntry},
};

use crate::{Wallet, describe, postings::Account};

/// A ledger entry joined with what it did to one user's balances.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct EntryRow {
    pub id: Uuid,
    pub kind: String,
    pub reason: String,
    pub from_user_id: Option<Uuid>,
    pub to_user_id: Option<Uuid>,
    pub amount: i64,
    pub fee: i64,
    pub refunded: bool,
    pub refund_of: Option<Uuid>,
    pub memo: serde_json::Value,
    /// The party whose account the other party does not get to see.
    pub hidden_user_id: Option<Uuid>,
    pub created_at: Time,
    /// Signed change of the viewer's spendable coins.
    pub coins_delta: i64,
    /// Signed change of the viewer's earnings.
    pub earnings_delta: i64,
    /// ID of the entry's last posting on the viewer's accounts (0 when it
    /// has none): the `revision` of the balance right after this entry.
    pub revision: i64,
    /// Where the entry sits in the list it was read for: the ID of its last
    /// posting on the accounts that list covers. The pagination key.
    pub position: i64,
}

/// [`EntryRow`] columns (all but `position`) for an entry `e` joined with
/// [`viewer_postings!`] as `p`.
macro_rules! entry_columns {
    () => {
        "e.id, e.kind, e.reason, e.from_user_id, e.to_user_id, e.amount, e.fee,
         e.refunded_at IS NOT NULL AS refunded, e.refund_of, e.memo, e.hidden_user_id, e.created_at,
         COALESCE(p.coins_delta, 0)::bigint AS coins_delta,
         COALESCE(p.earnings_delta, 0)::bigint AS earnings_delta,
         COALESCE(p.revision, 0) AS revision"
    };
}

/// Join computing what entry `e` did to the viewer, whose posting accounts
/// are bound as `$1` (coins) and `$2` (earnings).
macro_rules! viewer_postings {
    () => {
        " LEFT JOIN LATERAL (
              SELECT sum(delta) FILTER (WHERE account = $1) AS coins_delta,
                     sum(delta) FILTER (WHERE account = $2) AS earnings_delta,
                     max(id) FILTER (WHERE account IN ($1, $2)) AS revision
              FROM wallet_postings
              WHERE entry_id = e.id
          ) p ON true "
    };
}

impl EntryRow {
    /// The other user of a movement between two users, seen from `owner`'s
    /// side of it, whether or not `owner` is told who that is.
    fn other_party(&self, owner: Uuid) -> Option<Uuid> {
        match (self.from_user_id, self.to_user_id) {
            (Some(from), Some(to)) if from == owner => Some(to),
            (Some(from), Some(to)) if to == owner => Some(from),
            _ => None,
        }
    }

    /// The other user of a movement between two users, from `viewer`'s side.
    fn counterpart_id(&self, viewer: Uuid) -> Option<Uuid> {
        // A hidden party: the viewer is told what they paid for (the memo), not which account was paid.
        self.other_party(viewer).filter(|other| !self.hides(*other, viewer))
    }

    /// Whether `user_id` is the entry's hidden party as far as `viewer` is
    /// concerned (the hidden party sees itself, of course).
    fn hides(&self, user_id: Uuid, viewer: Uuid) -> bool {
        self.hidden_user_id == Some(user_id) && viewer != user_id
    }

    /// Whether the other side of this entry is kept from `viewer`.
    fn counterpart_hidden(&self, viewer: Uuid) -> bool {
        [self.from_user_id, self.to_user_id].into_iter().flatten().any(|party| self.hides(party, viewer))
    }

    /// Whether `user_id` is a party of this entry from whom its other party
    /// is hidden (the payer of a gift to a channel).
    fn keeps_party_from(&self, user_id: Uuid) -> bool {
        self.other_party(user_id).is_some() && self.counterpart_hidden(user_id)
    }

    /// The entry's payer and recipient as `viewer` may know them.
    pub(crate) fn parties_for(&self, viewer: Uuid) -> (Option<Uuid>, Option<Uuid>) {
        let shown = |party: Option<Uuid>| party.filter(|party| !self.hides(*party, viewer));
        (shown(self.from_user_id), shown(self.to_user_id))
    }

    /// The entry in `viewer`'s history. `reader` says who is looking at it:
    /// staff tracing a payment are shown both parties, also the one the
    /// account's owner is not told (`counterpart_hidden` stays set, so they
    /// know the owner does not see it).
    fn to_proto(&self, viewer: Uuid, reader: Reader, locale: &str, users: &HashMap<Uuid, UserSummary>) -> LedgerEntry {
        let counterpart = match reader {
            Reader::Owner => self.counterpart_id(viewer),
            Reader::Staff => self.other_party(viewer),
        };
        LedgerEntry {
            id: self.id.to_string(),
            kind: describe::entry_kind(&self.kind).into(),
            reason: self.reason.clone(),
            description: describe::describe(&self.kind, &self.reason, self.coins_delta, self.earnings_delta).pick(locale).to_owned(),
            coins_delta: self.coins_delta,
            earnings_delta: self.earnings_delta,
            amount: self.amount,
            fee: self.fee,
            counterpart: counterpart.map(|id| tokuly_identity::users::pick(users, id)).into(),
            counterpart_hidden: self.counterpart_hidden(viewer),
            refunded: self.refunded,
            refund_of_entry_id: self.refund_of.map(|id| id.to_string()).unwrap_or_default(),
            memo_json: self.memo.to_string(),
            created_at: ts(self.created_at),
            ..Default::default()
        }
    }
}

/// Which of a user's balances a history list covers.
/// Who a history is rendered for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reader {
    /// The account's owner: a hidden counterpart stays hidden.
    Owner,
    /// Staff looking into the account: every party is named.
    Staff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Scope {
    pub coins: bool,
    pub earnings: bool,
}

impl Scope {
    pub const ALL: Self = Self { coins: true, earnings: true };

    /// The scope a request's account filter asks for; unspecified = both.
    pub fn from_filter(account: Option<LedgerAccount>) -> Self {
        match account {
            Some(LedgerAccount::LEDGER_ACCOUNT_COINS) => Self { coins: true, earnings: false },
            Some(LedgerAccount::LEDGER_ACCOUNT_EARNINGS) => Self { coins: false, earnings: true },
            _ => Self::ALL,
        }
    }
}

/// Up to `limit` entries of `user_id`'s history that touched the balances in
/// `scope`, newest first, starting below position `before` (`None` = from
/// the newest).
pub(crate) async fn fetch_entries(db: &sqlx::PgPool, user_id: Uuid, scope: Scope, before: Option<i64>, limit: i64) -> AppResult<Vec<EntryRow>> {
    // One index range scan per account (`wallet_postings_account`), newest
    // first, merged. An entry appears once, at its last posting within the
    // scope — the NOT EXISTS skips any earlier posting of the same entry.
    // The entry itself is then fetched by primary key; `OFFSET 0` keeps the
    // planner from turning that lookup into a join over the whole table.
    Ok(sqlx::query_as(concat!(
        "WITH mine AS (
             (SELECT c.id AS position, c.entry_id FROM wallet_postings c
               WHERE $4 AND c.account = $1 AND c.id < $3
                 AND NOT EXISTS (SELECT 1 FROM wallet_postings later
                                 WHERE later.entry_id = c.entry_id AND later.id > c.id AND (later.account = $1 OR ($5 AND later.account = $2)))
               ORDER BY c.id DESC LIMIT $6)
             UNION ALL
             (SELECT n.id AS position, n.entry_id FROM wallet_postings n
               WHERE $5 AND n.account = $2 AND n.id < $3
                 AND NOT EXISTS (SELECT 1 FROM wallet_postings later
                                 WHERE later.entry_id = n.entry_id AND later.id > n.id AND (later.account = $2 OR ($4 AND later.account = $1)))
               ORDER BY n.id DESC LIMIT $6)
         )
         SELECT ",
        entry_columns!(),
        ", mine.position
         FROM mine
         CROSS JOIN LATERAL (SELECT * FROM wallet_entries WHERE id = mine.entry_id OFFSET 0) e",
        viewer_postings!(),
        "ORDER BY mine.position DESC LIMIT $6"
    ))
    .bind(Account::Coins.of(user_id))
    .bind(Account::Earnings.of(user_id))
    // A concrete bound rather than NULL, so `id < $3` is an index condition.
    .bind(before.unwrap_or(i64::MAX))
    .bind(scope.coins)
    .bind(scope.earnings)
    .bind(limit)
    .fetch_all(db)
    .await?)
}

/// Keyset cursor meaning "start from the newest row": greater than every
/// real (UUIDv7) ID. Lists ordered by ID descending use it in place of a
/// missing cursor, so the comparison stays an index condition.
pub(crate) const NEWEST: Uuid = Uuid::max();

impl Wallet {
    /// A user's balances with their revision, read in one statement so the
    /// three values belong to the same instant.
    pub async fn balance_snapshot(&self, user_id: Uuid) -> AppResult<Balance> {
        let (coins, earnings, revision): (i64, i64, i64) = sqlx::query_as(
            "SELECT COALESCE((SELECT coins FROM wallet_accounts WHERE user_id = $1), 0),
                    COALESCE((SELECT earnings FROM wallet_accounts WHERE user_id = $1), 0),
                    GREATEST(COALESCE((SELECT max(id) FROM wallet_postings WHERE account = $2), 0),
                             COALESCE((SELECT max(id) FROM wallet_postings WHERE account = $3), 0))",
        )
        .bind(user_id)
        .bind(Account::Coins.of(user_id))
        .bind(Account::Earnings.of(user_id))
        .fetch_one(&self.core.db)
        .await?;
        Ok(Balance { coins, earnings, revision, ..Default::default() })
    }

    /// One page of `user_id`'s history within `scope`, newest first, worded
    /// for `locale`.
    pub(crate) async fn list_entries(&self, user_id: Uuid, reader: Reader, scope: Scope, page: &Page, locale: &str) -> AppResult<(Vec<LedgerEntry>, PageInfo)> {
        let mut rows = fetch_entries(&self.core.db, user_id, scope, page.decode()?, page.fetch()).await?;
        let info = page.finish(&mut rows, |row| row.position);
        Ok((self.entries_to_proto(&rows, user_id, reader, locale).await?, info))
    }

    /// One page of `user_id`'s history as staff member `staff_id` sees it
    /// (`Reader::Staff`: every party named) — except what the staff member
    /// would learn about a payment of their own that hides its other party.
    /// Their own wallet reads as it does for its owner, and in anyone
    /// else's wallet the entries of such a payment are left out: finding
    /// their gift there would tell them whose channel it went to.
    pub(crate) async fn list_entries_for_staff(&self, staff_id: Uuid, user_id: Uuid, scope: Scope, page: &Page, locale: &str) -> AppResult<(Vec<LedgerEntry>, PageInfo)> {
        if staff_id == user_id {
            return self.list_entries(user_id, Reader::Owner, scope, page, locale).await;
        }
        let mut rows = fetch_entries(&self.core.db, user_id, scope, page.decode()?, page.fetch()).await?;
        // The cursor is taken before filtering, so the next page starts after every row read.
        let info = page.finish(&mut rows, |row| row.position);
        rows.retain(|row| !row.keeps_party_from(staff_id));
        Ok((self.entries_to_proto(&rows, user_id, Reader::Staff, locale).await?, info))
    }

    /// One entry as `user_id` sees it; `None` when it does not exist.
    pub(crate) async fn find_entry(&self, entry_id: Uuid, user_id: Uuid) -> AppResult<Option<EntryRow>> {
        Ok(sqlx::query_as(concat!("SELECT ", entry_columns!(), ", COALESCE(p.revision, 0) AS position FROM wallet_entries e", viewer_postings!(), "WHERE e.id = $3"))
            .bind(Account::Coins.of(user_id))
            .bind(Account::Earnings.of(user_id))
            .bind(entry_id)
            .fetch_optional(&self.core.db)
            .await?)
    }

    /// Converts rows to API entries, resolving every counterpart in one query.
    pub(crate) async fn entries_to_proto(&self, rows: &[EntryRow], viewer: Uuid, reader: Reader, locale: &str) -> AppResult<Vec<LedgerEntry>> {
        let counterparts: Vec<Uuid> = rows.iter().filter_map(|row| row.other_party(viewer)).collect();
        let users = tokuly_identity::summaries(&self.core, &counterparts).await?;
        Ok(rows.iter().map(|row| row.to_proto(viewer, reader, locale, &users)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(from: Option<Uuid>, to: Option<Uuid>) -> EntryRow {
        EntryRow {
            id: new_id(),
            kind: "transfer".into(),
            reason: "live.gift".into(),
            from_user_id: from,
            to_user_id: to,
            amount: 300,
            fee: 90,
            refunded: false,
            refund_of: None,
            memo: serde_json::json!({ "stream_id": "s1" }),
            hidden_user_id: None,
            created_at: now(),
            coins_delta: -300,
            earnings_delta: 0,
            revision: 7,
            position: 7,
        }
    }

    #[test]
    fn a_hidden_party_is_not_shown_to_the_other_side() {
        let (payer, recipient) = (new_id(), new_id());
        let gift = EntryRow { hidden_user_id: Some(recipient), ..row(Some(payer), Some(recipient)) };
        // The payer does not learn which account was paid...
        assert_eq!(gift.counterpart_id(payer), None);
        assert!(gift.counterpart_hidden(payer));
        assert_eq!(gift.parties_for(payer), (Some(payer), None));
        let shown = gift.to_proto(payer, Reader::Owner, "ja", &HashMap::new());
        assert!(!shown.counterpart.is_set() && shown.counterpart_hidden);
        // Staff looking into the payer's wallet are shown the recipient, and that the payer is not.
        let traced = gift.to_proto(payer, Reader::Staff, "ja", &HashMap::new());
        assert!(traced.counterpart.is_set() && traced.counterpart_hidden);
        // ...while the recipient sees who paid, and both sides of their own entry.
        assert_eq!(gift.counterpart_id(recipient), Some(payer));
        assert!(!gift.counterpart_hidden(recipient));
        assert_eq!(gift.parties_for(recipient), (Some(payer), Some(recipient)));
        // The refund of such a gift runs the other way and hides the same account.
        let refund = EntryRow { hidden_user_id: Some(recipient), ..row(Some(recipient), Some(payer)) };
        assert_eq!(refund.counterpart_id(payer), None);
        assert_eq!(refund.parties_for(payer), (None, Some(payer)));
    }

    #[test]
    fn counterpart_is_the_other_side_of_a_two_party_entry() {
        let (payer, recipient, stranger) = (new_id(), new_id(), new_id());
        let transfer = row(Some(payer), Some(recipient));
        assert_eq!(transfer.counterpart_id(payer), Some(recipient));
        assert_eq!(transfer.counterpart_id(recipient), Some(payer));
        assert_eq!(transfer.counterpart_id(stranger), None);
        assert_eq!(row(Some(payer), None).counterpart_id(payer), None, "platform movements have no counterpart");
        assert_eq!(row(None, Some(recipient)).counterpart_id(recipient), None, "an unlinked (deleted) payer is not named");
    }

    #[test]
    fn entries_are_worded_for_the_viewer() {
        let (payer, recipient) = (new_id(), new_id());
        let entry = row(Some(payer), Some(recipient)).to_proto(payer, Reader::Owner, "en", &HashMap::new());
        assert_eq!(entry.description, "Sent a live gift");
        assert_eq!((entry.coins_delta, entry.amount, entry.fee), (-300, 300, 90));
        assert_eq!(entry.memo_json, r#"{"stream_id":"s1"}"#);
        let counterpart = entry.counterpart.as_option().unwrap();
        assert_eq!(counterpart.id, recipient.to_string());
        assert!(counterpart.deleted, "an unknown counterpart shows as a deleted-user placeholder");
    }

    #[test]
    fn the_account_filter_selects_the_scope() {
        assert_eq!(Scope::from_filter(Some(LedgerAccount::LEDGER_ACCOUNT_COINS)), Scope { coins: true, earnings: false });
        assert_eq!(Scope::from_filter(Some(LedgerAccount::LEDGER_ACCOUNT_EARNINGS)), Scope { coins: false, earnings: true });
        assert_eq!(Scope::from_filter(Some(LedgerAccount::LEDGER_ACCOUNT_UNSPECIFIED)), Scope::ALL);
        assert_eq!(Scope::from_filter(None), Scope::ALL);
    }
}
