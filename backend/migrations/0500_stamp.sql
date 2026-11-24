-- Stamp: packs, stamps, each user's library (installs, purchases, favourites,
-- recently used), the usage log and the numbers derived from it.

-- People who make packs. A row appears with a user's first pack, or when a
-- moderator flags them before that.
CREATE TABLE stamp_creators (
    user_id    uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    -- Trusted creators publish without review.
    trusted    boolean     NOT NULL DEFAULT false,
    trusted_by uuid REFERENCES users (id) ON DELETE SET NULL,
    trusted_at timestamptz,
    attrs      jsonb       NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER stamp_creators_updated_at
    BEFORE UPDATE ON stamp_creators
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE TABLE stamp_packs (
    id                 uuid PRIMARY KEY DEFAULT uuidv7(),
    short_id           text             NOT NULL UNIQUE,
    -- NULL for official packs: they belong to the platform, not to a user.
    creator_id         uuid REFERENCES users (id) ON DELETE CASCADE,
    official           boolean          NOT NULL DEFAULT false,
    -- Stable name of a pack created by the official seeder; makes seeding idempotent.
    official_key       text UNIQUE,
    name               text             NOT NULL,
    description        text             NOT NULL DEFAULT '',
    -- Lowercase, without '#'.
    tags               text[]           NOT NULL DEFAULT '{}',
    -- Coins; 0 = free.
    price              integer          NOT NULL DEFAULT 0 CHECK (price >= 0),
    -- tokuly.stamp.v1.PackStatus
    status             integer          NOT NULL DEFAULT 1 CHECK (status BETWEEN 1 AND 5),
    -- Submitting must go through a moderator: stamps were added since the last
    -- approval, or the last decision was a rejection or a takedown.
    needs_review       boolean          NOT NULL DEFAULT true,
    -- The moderator's reason while the pack is rejected or removed.
    review_note        text             NOT NULL DEFAULT '',
    submitted_at       timestamptz,
    reviewed_at        timestamptz,
    -- When the pack last went live; kept after unpublishing.
    published_at       timestamptz,
    -- Non-NULL = on the featured shelf (most recently featured first).
    featured_at        timestamptz,

    -- The creator's choice of cover; NULL = the first stamp.
    cover_stamp_id     uuid,
    -- Derived from the stamps by `packs::refresh_derived`:
    -- tokuly.common.v1.Image of the cover as proto JSON, and counts.
    cover              jsonb,
    stamp_count        integer          NOT NULL DEFAULT 0,
    approved_count     integer          NOT NULL DEFAULT 0,
    animated           boolean          NOT NULL DEFAULT false,

    -- Current installs, not counting people who manage the pack.
    install_count      bigint           NOT NULL DEFAULT 0,
    purchase_count     bigint           NOT NULL DEFAULT 0,
    like_count         bigint           NOT NULL DEFAULT 0,
    comment_count      bigint           NOT NULL DEFAULT 0,
    -- Refreshed from stamp_pack_daily by the ranking worker.
    send_count         bigint           NOT NULL DEFAULT 0,
    popular_score      double precision NOT NULL DEFAULT 0,

    attrs              jsonb            NOT NULL DEFAULT '{}',
    created_at         timestamptz      NOT NULL DEFAULT now(),
    updated_at         timestamptz      NOT NULL DEFAULT now(),

    CONSTRAINT stamp_packs_official_has_no_creator CHECK (official = (creator_id IS NULL))
);

CREATE INDEX stamp_packs_creator ON stamp_packs (creator_id, id DESC) WHERE creator_id IS NOT NULL;
-- Store shelves only ever look at published packs (status 3).
CREATE INDEX stamp_packs_new ON stamp_packs (published_at DESC, id DESC) WHERE status = 3;
CREATE INDEX stamp_packs_popular ON stamp_packs (popular_score DESC, id DESC) WHERE status = 3;
CREATE INDEX stamp_packs_featured ON stamp_packs (featured_at DESC, id DESC) WHERE status = 3 AND featured_at IS NOT NULL;
CREATE INDEX stamp_packs_tags ON stamp_packs USING gin (tags) WHERE status = 3;
CREATE INDEX stamp_packs_name_trgm ON stamp_packs USING gin (name gin_trgm_ops) WHERE status = 3;
-- The review queue (status 2), oldest submission first.
CREATE INDEX stamp_packs_review_queue ON stamp_packs (submitted_at, id) WHERE status = 2;

-- `updated_at` means "the creator's content changed": counter updates do not touch it.
CREATE TRIGGER stamp_packs_updated_at
    BEFORE UPDATE OF name, description, tags, price, status, cover_stamp_id, stamp_count ON stamp_packs
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- A stamp belongs either to a pack or (personal stamps) to a user.
-- The picture of a stamp never changes after it is created.
CREATE TABLE stamps (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    pack_id       uuid REFERENCES stamp_packs (id) ON DELETE CASCADE,
    owner_id      uuid REFERENCES users (id) ON DELETE CASCADE,
    name          text        NOT NULL DEFAULT '',
    emoji         text        NOT NULL DEFAULT '',
    keywords      text[]      NOT NULL DEFAULT '{}',
    -- tokuly.common.v1.StampFormat
    format        integer     NOT NULL,
    -- The media asset the picture comes from; NULL for the built-in official
    -- art, which is not an upload. The stamp holds the asset in the media
    -- domain as `stamp:<id>`, so the upload lives as long as the stamp does.
    -- A personal copy made with SaveStamp points at (and holds) the same
    -- asset, which is how every stamp showing one picture is found when
    -- that picture is taken down.
    asset_id      uuid,
    -- tokuly.common.v1.Image as proto JSON (static preview with variants).
    image         jsonb       NOT NULL,
    -- The fields of tokuly.common.v1.StampRef, ready to serve.
    image_url     text        NOT NULL,
    animation_url text        NOT NULL DEFAULT '',
    width         integer     NOT NULL DEFAULT 0,
    height        integer     NOT NULL DEFAULT 0,
    -- Order within the pack, or within the owner's personal stamps.
    position      integer     NOT NULL DEFAULT 0,
    -- Pack stamps: when the stamp passed review. People other than the pack's
    -- managers only see approved stamps. Always NULL for personal stamps.
    approved_at   timestamptz,
    send_count    bigint      NOT NULL DEFAULT 0,
    attrs         jsonb       NOT NULL DEFAULT '{}',
    created_at    timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT stamps_pack_xor_owner CHECK ((pack_id IS NULL) <> (owner_id IS NULL))
);

CREATE INDEX stamps_pack ON stamps (pack_id, position, id) WHERE pack_id IS NOT NULL;
CREATE INDEX stamps_owner ON stamps (owner_id, position, id) WHERE owner_id IS NOT NULL;
CREATE INDEX stamps_asset ON stamps (asset_id) WHERE asset_id IS NOT NULL;
CREATE INDEX stamps_keywords ON stamps USING gin (keywords);

ALTER TABLE stamp_packs
    ADD CONSTRAINT stamp_packs_cover_stamp FOREIGN KEY (cover_stamp_id) REFERENCES stamps (id) ON DELETE SET NULL;

-- Packs in a user's picker, in the user's order.
CREATE TABLE stamp_pack_installs (
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    pack_id      uuid        NOT NULL REFERENCES stamp_packs (id) ON DELETE CASCADE,
    position     integer     NOT NULL,
    installed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, pack_id)
);

CREATE INDEX stamp_pack_installs_order ON stamp_pack_installs (user_id, position, pack_id);
CREATE INDEX stamp_pack_installs_pack ON stamp_pack_installs (pack_id);

-- Paid packs a user bought. Ownership survives uninstalling and unpublishing.
CREATE TABLE stamp_pack_purchases (
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    pack_id      uuid        NOT NULL REFERENCES stamp_packs (id) ON DELETE CASCADE,
    -- Coins paid, and what the creator received after the platform fee.
    price        integer     NOT NULL,
    net          integer     NOT NULL,
    -- The wallet ledger entry of the payment.
    entry_id     uuid        NOT NULL,
    purchased_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, pack_id)
);

CREATE INDEX stamp_pack_purchases_recent ON stamp_pack_purchases (user_id, purchased_at DESC, pack_id DESC);
CREATE INDEX stamp_pack_purchases_pack ON stamp_pack_purchases (pack_id);

-- An order: a purchase that has started but is not recorded yet, at the
-- price the buyer confirmed. It keeps the creator from deleting the pack
-- until it is settled. Each order pays under a ledger key of its own, so
-- whatever happened to an earlier order (refunded, refused) never blocks a
-- later one. Settling looks the key up in the ledger: paid orders are
-- recorded at `price` (refunded only if moderators took the pack down),
-- unpaid ones are paid at the buyer's request or ended by the worker. One
-- settler at a time holds an order, through `leased_until`.
CREATE TABLE stamp_purchase_intents (
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    pack_id      uuid        NOT NULL REFERENCES stamp_packs (id) ON DELETE CASCADE,
    -- The price the buyer agreed to; the order pays exactly this.
    price        integer     NOT NULL CHECK (price > 0),
    -- The ledger idempotency key of this order's payment.
    payment_key  text        NOT NULL UNIQUE,
    -- Held by a settler until then (a request, or the worker); a worker
    -- attempt that could not finish leaves it set as its back-off.
    leased_until timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, pack_id)
);

CREATE INDEX stamp_purchase_intents_pack ON stamp_purchase_intents (pack_id);

CREATE TABLE stamp_favorites (
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    stamp_id   uuid        NOT NULL REFERENCES stamps (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, stamp_id)
);

CREATE INDEX stamp_favorites_recent ON stamp_favorites (user_id, created_at DESC, stamp_id DESC);
CREATE INDEX stamp_favorites_stamp ON stamp_favorites (stamp_id);

-- The last stamps each user sent (capped per user by tokuly-stamp).
CREATE TABLE stamp_recents (
    user_id   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    stamp_id  uuid        NOT NULL REFERENCES stamps (id) ON DELETE CASCADE,
    used_at   timestamptz NOT NULL DEFAULT now(),
    use_count integer     NOT NULL DEFAULT 1,
    PRIMARY KEY (user_id, stamp_id)
);

CREATE INDEX stamp_recents_last_used ON stamp_recents (user_id, used_at DESC);
CREATE INDEX stamp_recents_stamp ON stamp_recents (stamp_id);

-- The usage log: one row per stamp sent. Pruned after a retention period;
-- long-term numbers live in stamp_pack_daily and stamps.send_count.
CREATE TABLE stamp_usage (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    stamp_id   uuid        NOT NULL REFERENCES stamps (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.stamp.v1.UsageContext
    context    integer     NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX stamp_usage_user ON stamp_usage (user_id, id DESC);
CREATE INDEX stamp_usage_stamp ON stamp_usage (stamp_id);
CREATE INDEX stamp_usage_created ON stamp_usage (created_at);

-- Per-pack numbers by day (Japan time): the source of creator statistics and
-- of the popularity ranking.
CREATE TABLE stamp_pack_daily (
    pack_id         uuid   NOT NULL REFERENCES stamp_packs (id) ON DELETE CASCADE,
    day             date   NOT NULL,
    chat_sends      bigint NOT NULL DEFAULT 0,
    live_chat_sends bigint NOT NULL DEFAULT 0,
    comment_sends   bigint NOT NULL DEFAULT 0,
    note_sends      bigint NOT NULL DEFAULT 0,
    map_sends       bigint NOT NULL DEFAULT 0,
    sends           bigint GENERATED ALWAYS AS (chat_sends + live_chat_sends + comment_sends + note_sends + map_sends) STORED,
    installs        bigint NOT NULL DEFAULT 0,
    purchases       bigint NOT NULL DEFAULT 0,
    -- Coins the creator earned that day, after the platform fee.
    earnings        bigint NOT NULL DEFAULT 0,
    PRIMARY KEY (pack_id, day)
);

CREATE INDEX stamp_pack_daily_day ON stamp_pack_daily (day);

-- Moderation decisions on packs, for the creator and for later reviews.
CREATE TABLE stamp_pack_reviews (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    pack_id      uuid        NOT NULL REFERENCES stamp_packs (id) ON DELETE CASCADE,
    moderator_id uuid REFERENCES users (id) ON DELETE SET NULL,
    -- tokuly.stamp.v1.ReviewDecision
    decision     integer     NOT NULL,
    note         text        NOT NULL DEFAULT '',
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX stamp_pack_reviews_pack ON stamp_pack_reviews (pack_id, id DESC);
