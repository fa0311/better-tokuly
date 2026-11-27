# wallet — events, jobs and configuration

Crate `tokuly-wallet`, migrations `1400_wallet.sql`, `1401_wallet_spending_limits.sql`, API `tokuly.wallet.v1`
(`WalletService`, `EarningsService`, `WalletAdminService`).

## How coins move

Every movement is one row in `wallet_entries` plus postings in
`wallet_postings` that sum to zero. There are two ways in:

- `tokuly_wallet::ledger` — what other domains call (`spend`, `transfer`,
  `refund`, `grant`, `balance`). Each call is its own transaction.
- `tokuly_wallet::ledger_tx` — the wallet's own operations, on the caller's
  transaction so that a purchase, payout request or adjustment row commits
  together with its ledger entry.

| Entry kind | Postings | Made by |
|---|---|---|
| `grant` | `system:issued` → `user:coins` | purchases (`purchase`), daily bonus (`daily_bonus`), other domains |
| `spend` | `user:coins` → `system:spent` | other domains |
| `transfer` | `user:coins` → `user:earnings` (+ `system:fees`) | other domains (gifts, stamp sales) |
| `refund` | reverses a `spend`, `transfer` or `payout` | other domains; payout rejected / cancelled |
| `payout` | `user:earnings` → `system:payout_hold` | `EarningsService.RequestPayout` |
| `payout_settlement` | `system:payout_hold` → `system:paid_out` | `WalletAdminService.MarkPayoutPaid` |
| `adjustment` | `system:adjustments` ↔ `user:coins` / `user:earnings` | `WalletAdminService.AdjustBalance` |
| `closure` | `user:coins` + `user:earnings` → `system:forfeited` | account deletion |

Idempotency keys the wallet uses itself: `purchase:<purchase id>`,
`daily_bonus:<user id>:<local date>`, `payout:<payout id>`,
`refund:<entry id>`, `payout.settled:<hold entry id>`,
`adjustment:<adjustment id>`, `closure:<random>`. Other domains should prefix
their keys with their own reason (`live.gift:<id>`) to stay clear of these.

Conventions for callers of the ledger:

- An idempotency key names one movement: its kind, payer, recipient and
  amount. Repeating the call with the same key returns the original entry
  (`applied: false`; a transfer's receipt gives the fee and net of the
  original). A call that uses the key for anything else — another payer,
  recipient, amount or kind — fails with `IDEMPOTENCY_KEY_REUSED`
  (`already_exists`) and moves nothing. A retry of a movement that was
  refunded since fails with `ENTRY_REFUNDED` (`failed_precondition`): the
  coins are back, so treat it as not paid. Build keys from the ID of the thing
  being paid for, and when a price can change between attempts, retry with
  the amount of the first attempt.

- `reason` is `<domain>.<thing>[.<detail>]`. History wording is chosen from
  the first segment (`live` → live gift, `stamp` → stamp pack; see
  `describe.rs`); anything else gets a generic line, so new reasons work
  without touching the wallet.
- `memo` is returned to clients as `LedgerEntry.memo_json`, to **both** parties
  of a transfer. Put IDs for deep links there (`{"stream_id": …}`), nothing
  private.
- When the payer must not learn which account is paid (a gift to a channel
  pays the channel's owner, and a channel does not say who runs it), use
  `Wallet::transfer_with_fee_hiding_recipient` /
  `ledger::transfer_to_hidden_recipient`. The entry remembers the hidden party
  (`wallet_entries.hidden_user_id`): the payer's history, balance push and
  data export carry no counterpart and `LedgerEntry.counterpart_hidden`, and
  so does the refund of such a transfer. The recipient sees the payer as
  usual, and staff see both parties (`WalletAdminService.ListUserLedgerEntries`,
  with `counterpart_hidden` still set) — except a staff member who is the
  payer: their own wallet reads as it does for any payer, and once they
  have paid a hidden recipient (refunded or not) every other wallet is
  refused to them (`GetUserWallet`, `ListUserLedgerEntries`:
  `HIDDEN_PARTY`), whichever account they ask about, because refusing
  only the account they paid would name it. Another staff member looks
  into wallets for them. Say in `memo` what was paid.
- Take the platform fee from `Wallet::transfer_with_fee` /
  `Wallet::platform_fee_percent` rather than hard-coding it.
- A transfer that would take the payer past their monthly spending limit
  (below) fails with `SPENDING_LIMIT_REACHED` (`failed_precondition`) and
  moves nothing. Pass it on to the client unchanged: it is the payer's own
  limit and says nothing about the recipient. A spend is not limited.

## Spending limit

Under 18 (`tokuly_identity::age::is_minor`, on the day of the charge) an
account spends at most `TOKULY_WALLET_MINOR_MONTHLY_LIMIT_YEN` (¥5,000) a
calendar month in Japan time. Anyone may choose a limit of their own
(`wallet_accounts.monthly_limit_yen`; minors only up to the ceiling and
without clearing it); it stays when they turn 18. The lower one is in force.

Spending is the price of coin purchases paid that month and of those still
waiting for their payment (whenever they were started: a purchase started
before the month turned and paid after it counts in the new month), plus the
`transfer` entries the account paid that month and that were not refunded. Coins have no exchange rate: one coin counts as ¥1 (and a
package priced in another currency counts its coins). Grants (the daily bonus,
rewards), spends and everything received are not spending.

`StartPurchase` and every transfer check the limit under the payer's account
row lock, so two charges at once are counted one after the other. A purchase
is checked when it starts, never when it completes: money that was taken
always becomes coins, even past the limit.

## Events emitted

| Topic | Payload | When |
|---|---|---|
| `wallet.balance.changed` | `{ user_id, coins, earnings, entry_id, reason }` (`tokuly_wallet::events::BalanceChanged`) | once per user whose balance an entry changed, in the transaction of the entry. `coins` / `earnings` are the balances right after it; the actor is that user. |

Not emitted for `payout_settlement` and `closure` entries (no live user
balance changes).

## Events consumed

One durable consumer, `wallet` (`Wallet::spawn_workers`):

| Topic | Effect |
|---|---|
| `wallet.balance.changed` | Pushes `WalletEvent.balance_changed` (the balance with its revision, plus the entry worded for the user) to the user's devices. If the entry put coins into the wallet through someone else's action, requests a `wallet.received` notification — at most once per entry and recipient (`wallet_entry_notices`). |
| `identity.user.deleted` | Closes the wallet (below). |

### Account deletion

Bookkeeping stays, unlinked; personal data goes. In one transaction:

1. open payout requests are cancelled and their held coins forfeited;
2. the user is removed from every ledger entry (`from_user_id` / `to_user_id`
   set to `NULL`, as the schema's `ON DELETE SET NULL` would) — counterparts
   keep their side of a transfer, without a name; where the user was the
   hidden party, `hidden_user_id` becomes the nil UUID, so the payer's line
   still reads `counterpart_hidden` exactly as before; such transfers can no
   longer be refunded (`REFUND_IMPOSSIBLE`);
3. purchases, payout requests and adjustment records lose their `user_id`;
   payout destinations and notes are erased; unpaid purchases are cancelled;
4. the daily-bonus row is deleted;
5. remaining coins and earnings are forfeited with a `closure` entry and the
   `wallet_accounts` row is deleted.

Postings keep their account names (`user:<id>:…`), so the closed account
stays auditable by ID through `WalletAdminService.ListUserLedgerEntries`;
the ID belongs to identity's anonymous tombstone by then.

Once identity has erased the account (`users.deleted_at`), the ledger
refuses every new movement that names it — grant, spend, transfer in
either direction — with `ACCOUNT_CLOSED` (`failed_precondition`), and a
refund to or from it with `REFUND_IMPOSSIBLE`, whether or not the consumer
above has run yet. A retry of a movement that happened before is still
answered as a retry.

## Realtime

`WalletEvent`, always user-scoped:

| Event | Sent when |
|---|---|
| `balance_changed` | any ledger entry changed the user's balance (via the consumer above). Clients keep the `Balance` with the highest `revision`, whatever path it arrived by. |
| `purchase_updated` | a purchase completed, failed, was cancelled or expired |
| `payout_updated` | a payout request was approved, rejected, paid or cancelled |

## Notification kinds

| Kind | Recipient | When | Path |
|---|---|---|---|
| `wallet.received` | recipient of a transfer (actor: the sender) | coins arrived in earnings: gift, stamp sale… Collapse key `wallet.received:<reason>:<sender>` | `/wallet/earnings` |
| `wallet.received` | payer | a spend or transfer was refunded | `/wallet` |
| `wallet.received` | user | an admin adjustment added coins or earnings | `/wallet` or `/wallet/earnings` |
| `wallet.purchase` | buyer | a purchase completed. Inbox only when the buyer's own request completed it; inbox + push when a webhook or the expiry check did. Collapse key `wallet.purchase:<id>` | `/wallet` |
| `wallet.payout` | requester | request approved, rejected (with the reason) or paid. Collapse key `wallet.payout:<id>` | `/wallet/payouts/<id>` |

Purchases, the daily bonus, money leaving and the user's own payout requests
produce no `wallet.received`.

## Background work

No named jobs. `Wallet::spawn_workers` runs, besides the consumer, a timer:

| Every | Work |
|---|---|
| 1 min | Purchases past `expires_at`: ask the provider one last time, then complete (paid), fail, or expire them. An unreachable provider is retried for 24 h before the purchase is expired anyway. |
| 1 h | Erase the encrypted destination of payout requests closed more than 90 days ago; prune `wallet_entry_notices` older than 30 days. |

Analytics events: `wallet.purchase` (value = coins granted), `wallet.daily_bonus`.

## Payment providers

`trait tokuly_wallet::PaymentProvider` (`create_checkout`, `confirm`,
`cancel_checkout`). Register implementations at startup with
`Wallet::add_payment_provider`; report webhook results with
`Wallet::settle_purchase(provider, reference, state)`. The first registered
provider is the default. Built in: `development` (settles instantly, no
money involved).

## Roles

| Role | RPCs |
|---|---|
| `moderator` | `ListPayoutQueue`, `GetPayoutRequest`, `ApprovePayout`, `RejectPayout`, `MarkPayoutPaid`, `GetUserWallet`, `ListUserLedgerEntries` |
| `admin` | the above plus `AdjustBalance`, `ListAdjustments`, `ListAllCoinPackages`, `SaveCoinPackage` |

Staff cannot approve, reject or pay their own payout request (`SELF_REVIEW`),
and admins cannot adjust their own balance (`SELF_ADJUSTMENT`). No money
goes to a suspended account (`users.banned_at`): its payout request can be
rejected but not approved or paid (`ACCOUNT_SUSPENDED`) until the
suspension is lifted; staff views flag it (`PayoutRequest.user_suspended`).

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_WALLET_DEV_PROVIDER` | unset: `true` unless `TOKULY_ENV=production` | Enables the `development` payment provider (free coins). |
| `TOKULY_WALLET_PLATFORM_FEE_PERCENT` | `30` | Share of every transfer between users kept by the platform (0–100). |
| `TOKULY_WALLET_DAILY_BONUS_COINS` | `10,10,15,15,20,20,50` | Daily bonus ladder: coins for streak day 1, 2, …; restarts after the last step. Empty disables the bonus. |
| `TOKULY_WALLET_PAYOUT_MIN_COINS` | `1000` | Smallest payout request, in earned coins. |
| `TOKULY_WALLET_PAYOUT_CURRENCY` | `JPY` | Currency of payouts (ISO 4217). |
| `TOKULY_WALLET_PAYOUT_MINOR_PER_100_COINS` | `100` | Money per 100 earned coins, in the currency's minor unit. Fixed on a request when it is made. |
| `TOKULY_WALLET_PURCHASE_TTL_MINUTES` | `60` | How long a purchase may stay unpaid. |
| `TOKULY_WALLET_MINOR_MONTHLY_LIMIT_YEN` | `5000` | Most an account under 18 may spend a month, in yen (see "Spending limit"). Not editable from the console. |

Rate limits (per user): `StartPurchase` 30/h, `ConfirmPurchase` 60/min,
`ClaimDailyBonus` 20/min, `SetSpendingLimit` 30/min, `RequestPayout` 10/day.
