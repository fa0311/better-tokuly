-- Wallet: coin balances and the double-entry ledger.

CREATE TABLE wallet_accounts (
    user_id    uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    -- Spendable coins (bought or granted).
    coins      bigint      NOT NULL DEFAULT 0 CHECK (coins >= 0),
    -- Coins received from others (gifts, stamp sales); kept apart for payouts.
    earnings   bigint      NOT NULL DEFAULT 0 CHECK (earnings >= 0),
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- One row per money movement. `idempotency_key` makes retries safe.
CREATE TABLE wallet_entries (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    -- 'grant' | 'spend' | 'transfer' | 'refund' | 'payout' | 'payout_settlement' |
    -- 'adjustment' | 'closure'
    kind            text        NOT NULL,
    -- What it was for, e.g. 'purchase', 'daily_bonus', 'live.gift', 'stamp.pack'.
    reason          text        NOT NULL,
    idempotency_key text        NOT NULL UNIQUE,
    from_user_id    uuid REFERENCES users (id) ON DELETE SET NULL,
    to_user_id      uuid REFERENCES users (id) ON DELETE SET NULL,
    amount          bigint      NOT NULL CHECK (amount > 0),
    -- Platform fee taken out of `amount` on transfers.
    fee             bigint      NOT NULL DEFAULT 0 CHECK (fee >= 0),
    -- Set on the original entry when it has been refunded.
    refunded_at     timestamptz,
    -- For 'refund' entries: the entry being reversed.
    refund_of       uuid REFERENCES wallet_entries (id),
    memo            jsonb       NOT NULL DEFAULT '{}',
    -- The party of this entry whose account is not shown to the other party
    -- (a channel's owner receiving a gift: the payer sees the channel named
    -- in the memo, never the account behind it). NULL: both see each other.
    -- The nil UUID once that account was deleted: still hidden, naming nobody.
    hidden_user_id  uuid,
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX wallet_entries_from ON wallet_entries (from_user_id, id DESC) WHERE from_user_id IS NOT NULL;
CREATE INDEX wallet_entries_to ON wallet_entries (to_user_id, id DESC) WHERE to_user_id IS NOT NULL;

-- Double-entry lines: the postings of an entry always sum to zero.
-- Accounts: 'user:<uuid>:coins', 'user:<uuid>:earnings', 'system:issued',
-- 'system:spent', 'system:fees', 'system:payout_hold' (earnings locked in open
-- payout requests), 'system:paid_out', 'system:adjustments' (manual
-- corrections by admins), 'system:forfeited' (balances of deleted accounts).
CREATE TABLE wallet_postings (
    id       bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    entry_id uuid   NOT NULL REFERENCES wallet_entries (id) ON DELETE CASCADE,
    account  text   NOT NULL,
    delta    bigint NOT NULL
);

CREATE INDEX wallet_postings_entry ON wallet_postings (entry_id);
CREATE INDEX wallet_postings_account ON wallet_postings (account, id DESC);

-- ───────────────────────────── Coin shop ─────────────────────────────

-- Coin packages sold for money. Never deleted (purchases reference them):
-- retire a package by clearing `active`.
CREATE TABLE wallet_coin_packages (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    -- Stable identifier, also the product ID at payment providers.
    sku             text        NOT NULL UNIQUE,
    coins           bigint      NOT NULL CHECK (coins > 0),
    -- Extra coins granted on top of `coins`.
    bonus_coins     bigint      NOT NULL DEFAULT 0 CHECK (bonus_coins >= 0),
    -- Price in the minor unit of `currency` (ISO 4217).
    price_minor     bigint      NOT NULL CHECK (price_minor > 0),
    currency        text        NOT NULL,
    -- tokuly.wallet.v1.PackageBadge
    badge           integer     NOT NULL DEFAULT 0,
    sort_order      integer     NOT NULL DEFAULT 0,
    active          boolean     NOT NULL DEFAULT true,
    -- Sale window of limited-time packages; NULL bounds are open.
    available_from  timestamptz,
    available_until timestamptz,
    attrs           jsonb       NOT NULL DEFAULT '{}',
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    CHECK (available_from IS NULL OR available_until IS NULL OR available_until > available_from)
);

CREATE TRIGGER wallet_coin_packages_updated_at
    BEFORE UPDATE ON wallet_coin_packages
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- The starting line-up; admins change it with WalletAdminService.SaveCoinPackage.
INSERT INTO wallet_coin_packages (sku, coins, bonus_coins, price_minor, currency, badge, sort_order)
VALUES ('coins_100', 100, 0, 100, 'JPY', 0, 10),
       ('coins_500', 500, 20, 500, 'JPY', 0, 20),
       ('coins_1000', 1000, 60, 1000, 'JPY', 1, 30),
       ('coins_3000', 3000, 240, 3000, 'JPY', 0, 40),
       ('coins_5000', 5000, 500, 5000, 'JPY', 0, 50),
       ('coins_10000', 10000, 1500, 10000, 'JPY', 2, 60);

-- One attempt to buy a package through a payment provider.
CREATE TABLE wallet_purchases (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    -- Purchases are accounting records: they outlive the account, unlinked.
    user_id         uuid REFERENCES users (id) ON DELETE SET NULL,
    package_id      uuid        NOT NULL REFERENCES wallet_coin_packages (id),
    -- Snapshot of the package as it was sold.
    sku             text        NOT NULL,
    coins           bigint      NOT NULL CHECK (coins > 0),
    bonus_coins     bigint      NOT NULL DEFAULT 0 CHECK (bonus_coins >= 0),
    price_minor     bigint      NOT NULL CHECK (price_minor > 0),
    currency        text        NOT NULL,
    -- Name of the payment provider, e.g. 'development'.
    provider        text        NOT NULL,
    -- The provider's identifier of the checkout; empty until it exists.
    provider_ref    text        NOT NULL DEFAULT '',
    checkout_url    text        NOT NULL DEFAULT '',
    status          text        NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'completed', 'failed', 'cancelled', 'expired')),
    failure_reason  text        NOT NULL DEFAULT '',
    -- Retry key chosen by the client; empty when it sent none.
    idempotency_key text        NOT NULL DEFAULT '',
    -- The ledger entry that granted the coins, once completed.
    entry_id        uuid REFERENCES wallet_entries (id),
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    completed_at    timestamptz,
    -- A pending purchase not paid by then is marked expired.
    expires_at      timestamptz NOT NULL
);

CREATE UNIQUE INDEX wallet_purchases_idempotency ON wallet_purchases (user_id, idempotency_key) WHERE idempotency_key <> '';
CREATE UNIQUE INDEX wallet_purchases_provider_ref ON wallet_purchases (provider, provider_ref) WHERE provider_ref <> '';
CREATE INDEX wallet_purchases_user ON wallet_purchases (user_id, id DESC);
CREATE INDEX wallet_purchases_pending ON wallet_purchases (expires_at) WHERE status = 'pending';

CREATE TRIGGER wallet_purchases_updated_at
    BEFORE UPDATE ON wallet_purchases
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- ───────────────────────────── Daily login bonus ─────────────────────────────

-- Streak state per user. The coins themselves are ordinary ledger grants
-- (reason 'daily_bonus').
CREATE TABLE wallet_daily_bonuses (
    user_id         uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    -- Calendar day (in the user's time zone) of the last claim; NULL before
    -- the first. Only ever moves forward, which is what stops a time zone
    -- change from unlocking the same day twice.
    last_claimed_on date,
    -- Consecutive days claimed up to `last_claimed_on`.
    streak          integer     NOT NULL DEFAULT 0,
    longest_streak  integer     NOT NULL DEFAULT 0,
    total_claims    integer     NOT NULL DEFAULT 0,
    total_coins     bigint      NOT NULL DEFAULT 0,
    updated_at      timestamptz NOT NULL DEFAULT now()
);

-- ───────────────────────────── Payouts ─────────────────────────────

-- A creator asking for earned coins to be paid out as money. Requesting moves
-- the coins from the user's earnings to 'system:payout_hold' (`hold_entry_id`);
-- rejecting or cancelling refunds that entry, paying settles it to
-- 'system:paid_out'.
CREATE TABLE wallet_payout_requests (
    id                uuid PRIMARY KEY DEFAULT uuidv7(),
    -- Kept (unlinked) after the account is deleted, for accounting.
    user_id           uuid REFERENCES users (id) ON DELETE SET NULL,
    -- Earned coins locked by this request.
    coins             bigint      NOT NULL CHECK (coins > 0),
    -- Money to send, at the rate in force when requested.
    payout_minor      bigint      NOT NULL CHECK (payout_minor >= 0),
    currency          text        NOT NULL,
    -- tokuly.wallet.v1.PayoutMethod
    method            integer     NOT NULL,
    -- Where to send the money, encrypted (Keys::seal, label
    -- 'wallet/payout-destination'). Erased some time after the request closes
    -- and when the account is deleted.
    destination       bytea,
    -- The requester's message to the reviewer.
    note              text        NOT NULL DEFAULT '',
    status            text        NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'approved', 'rejected', 'paid', 'cancelled')),
    -- Retry key chosen by the client; empty when it sent none.
    idempotency_key   text        NOT NULL DEFAULT '',
    hold_entry_id     uuid        NOT NULL REFERENCES wallet_entries (id),
    -- Who approved or rejected it, when, and their message to the requester.
    reviewed_by       uuid REFERENCES users (id) ON DELETE SET NULL,
    reviewed_at       timestamptz,
    review_note       text        NOT NULL DEFAULT '',
    -- Who recorded the payment, when, and the proof (a transfer ID).
    paid_by           uuid REFERENCES users (id) ON DELETE SET NULL,
    paid_at           timestamptz,
    payment_reference text        NOT NULL DEFAULT '',
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now()
);

-- One open request per user.
CREATE UNIQUE INDEX wallet_payout_requests_open ON wallet_payout_requests (user_id) WHERE status IN ('pending', 'approved');
CREATE UNIQUE INDEX wallet_payout_requests_idempotency ON wallet_payout_requests (user_id, idempotency_key) WHERE idempotency_key <> '';
CREATE INDEX wallet_payout_requests_user ON wallet_payout_requests (user_id, id DESC);
CREATE INDEX wallet_payout_requests_status ON wallet_payout_requests (status, id);

CREATE TRIGGER wallet_payout_requests_updated_at
    BEFORE UPDATE ON wallet_payout_requests
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- ───────────────────────────── Administration ─────────────────────────────

-- Audit log of manual balance corrections: who changed whose balance, by how
-- much and why. Append-only.
CREATE TABLE wallet_adjustments (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    admin_id        uuid REFERENCES users (id) ON DELETE SET NULL,
    user_id         uuid REFERENCES users (id) ON DELETE SET NULL,
    account         text        NOT NULL CHECK (account IN ('coins', 'earnings')),
    -- Signed number of coins added (positive) or removed (negative).
    delta           bigint      NOT NULL CHECK (delta <> 0),
    reason          text        NOT NULL,
    -- Retry key chosen by the admin client; empty when it sent none.
    idempotency_key text        NOT NULL DEFAULT '',
    entry_id        uuid        NOT NULL REFERENCES wallet_entries (id),
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX wallet_adjustments_idempotency ON wallet_adjustments (admin_id, idempotency_key) WHERE idempotency_key <> '';
CREATE INDEX wallet_adjustments_user ON wallet_adjustments (user_id, id DESC);

-- ───────────────────────────── Notifications ─────────────────────────────

-- "Coins received" notifications already requested, per ledger entry and
-- recipient. `wallet.balance.changed` is delivered at least once; this makes
-- the notification exactly once. Rows are pruned after a month.
CREATE TABLE wallet_entry_notices (
    entry_id   uuid        NOT NULL REFERENCES wallet_entries (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (entry_id, user_id)
);

CREATE INDEX wallet_entry_notices_created ON wallet_entry_notices (created_at);
