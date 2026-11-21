-- Emoji: custom emoji and their review, the catalogue clients keep in sync,
-- usage numbers, each user's deck and recently used list, and the import and
-- export of whole sets.

-- The managed list of categories, in display order. An emoji either names
-- one of these or has no category.
CREATE TABLE emoji_categories (
    id         uuid PRIMARY KEY     DEFAULT uuidv7(),
    name       text        NOT NULL UNIQUE,
    position   integer     NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- "Cats" and "cats" are the same category.
CREATE UNIQUE INDEX emoji_categories_lower_name ON emoji_categories (lower(name));

CREATE TRIGGER emoji_categories_updated_at
    BEFORE UPDATE ON emoji_categories
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE TABLE emojis (
    id              uuid PRIMARY KEY     DEFAULT uuidv7(),
    -- The shortcode, lowercase, without colons.
    name            text        NOT NULL CHECK (name ~ '^[a-z0-9_]{2,32}$'),
    -- tokuly.emoji.v1.EmojiState: 1 pending, 2 approved, 3 rejected, 4 disabled.
    state           smallint    NOT NULL DEFAULT 1 CHECK (state BETWEEN 1 AND 4),
    -- The media asset the picture comes from; held as `emoji:<id>`.
    asset_id        uuid        NOT NULL,
    -- tokuly.common.v1.Image as proto JSON: `url` is the animated rendition
    -- of an animated emoji, `variants` are stills.
    image           jsonb       NOT NULL,
    animated        boolean     NOT NULL DEFAULT false,
    -- NULL = no category. Follows renames; deleting a category clears it.
    category        text REFERENCES emoji_categories (name) ON UPDATE CASCADE ON DELETE SET NULL,
    -- Search words, lowercase.
    aliases         text[]      NOT NULL DEFAULT '{}',
    license         text        NOT NULL DEFAULT '',
    sensitive       boolean     NOT NULL DEFAULT false,
    local_only      boolean     NOT NULL DEFAULT false,
    -- The user credited for the emoji (its submitter, unless a moderator
    -- named someone else). The emoji outlives the account.
    author_id       uuid REFERENCES users (id) ON DELETE SET NULL,
    -- The moderator's reason while the emoji is rejected or disabled.
    review_note     text        NOT NULL DEFAULT '',
    reviewed_at     timestamptz,
    -- When it first joined the catalogue.
    approved_at     timestamptz,
    -- The catalogue version of its last change while approved (see emoji_catalog).
    catalog_version bigint      NOT NULL DEFAULT 0,
    attrs           jsonb       NOT NULL DEFAULT '{}',
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);

-- A name belongs to every emoji that is usable or may become usable again;
-- a rejected emoji gives its name back.
CREATE UNIQUE INDEX emojis_name ON emojis (name) WHERE state <> 3;
-- The review queue and the moderators' listing.
CREATE INDEX emojis_state ON emojis (state, created_at, id);
CREATE INDEX emojis_author ON emojis (author_id, id) WHERE author_id IS NOT NULL;
CREATE INDEX emojis_asset ON emojis (asset_id);
CREATE INDEX emojis_category ON emojis (category) WHERE category IS NOT NULL;
-- "What changed since version N" and the newest list.
CREATE INDEX emojis_catalog ON emojis (catalog_version) WHERE state = 2;

CREATE TRIGGER emojis_updated_at
    BEFORE UPDATE ON emojis
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- The catalogue's version counter. Every change to the set of approved
-- emoji or to the categories takes the next version while holding this row,
-- so versions are handed out in commit order.
CREATE TABLE emoji_catalog (
    singleton     boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    -- Names this database in Valkey keys: a cache filled from another
    -- database (a restored backup, a test run) is never mistaken for ours.
    epoch         uuid        NOT NULL DEFAULT uuidv7(),
    version       bigint      NOT NULL DEFAULT 0,
    -- Differences can be answered for versions from here on; removals older
    -- than this were pruned.
    floor_version bigint      NOT NULL DEFAULT 0,
    updated_at    timestamptz NOT NULL DEFAULT now()
);

INSERT INTO emoji_catalog DEFAULT VALUES;

-- Emoji that left the catalogue (disabled or deleted), so clients holding
-- an older version learn what to drop. A row goes away when the emoji comes
-- back, and after the retention time.
CREATE TABLE emoji_catalog_removals (
    emoji_id   uuid PRIMARY KEY,
    version    bigint      NOT NULL,
    removed_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX emoji_catalog_removals_removed_at ON emoji_catalog_removals (removed_at);

-- What happened to each emoji and who did it (tokuly.emoji.v1.ReviewAction).
CREATE TABLE emoji_reviews (
    id         uuid PRIMARY KEY     DEFAULT uuidv7(),
    emoji_id   uuid        NOT NULL REFERENCES emojis (id) ON DELETE CASCADE,
    actor_id   uuid REFERENCES users (id) ON DELETE SET NULL,
    action     smallint    NOT NULL,
    note       text        NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX emoji_reviews_emoji ON emoji_reviews (emoji_id, id DESC);

-- People a moderator flagged: trusted submitters skip the review.
CREATE TABLE emoji_submitters (
    user_id    uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    trusted    boolean     NOT NULL DEFAULT false,
    trusted_by uuid REFERENCES users (id) ON DELETE SET NULL,
    trusted_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX emoji_submitters_trusted ON emoji_submitters (trusted_at DESC, user_id) WHERE trusted;

CREATE TRIGGER emoji_submitters_updated_at
    BEFORE UPDATE ON emoji_submitters
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Usage numbers, kept apart from `emojis` so that counting never touches
-- (or locks) what moderators edit.
CREATE TABLE emoji_stats (
    emoji_id   uuid PRIMARY KEY REFERENCES emojis (id) ON DELETE CASCADE,
    -- All time.
    use_count  bigint           NOT NULL DEFAULT 0,
    -- Recent uses, older days weighing less; recomputed by the usage worker.
    popularity double precision NOT NULL DEFAULT 0
);

CREATE INDEX emoji_stats_popularity ON emoji_stats (popularity DESC) WHERE popularity > 0;

-- Uses per emoji and day (days in Japan time).
CREATE TABLE emoji_usage_daily (
    emoji_id uuid   NOT NULL REFERENCES emojis (id) ON DELETE CASCADE,
    day      date   NOT NULL,
    uses     bigint NOT NULL DEFAULT 0,
    PRIMARY KEY (emoji_id, day)
);

CREATE INDEX emoji_usage_daily_day ON emoji_usage_daily (day);

-- Batches of counters the usage worker moved from Valkey into the tables
-- above; a batch is applied once even if the worker is interrupted.
CREATE TABLE emoji_usage_batches (
    batch_id   uuid PRIMARY KEY,
    applied_at timestamptz NOT NULL DEFAULT now()
);

-- Reaction decks: the emoji a user pinned, in order. An item is `:name:`
-- for a custom emoji and the character itself for a Unicode emoji.
CREATE TABLE emoji_decks (
    user_id    uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    items      text[]      NOT NULL DEFAULT '{}',
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- Finds the decks to rewrite when an emoji is renamed.
CREATE INDEX emoji_decks_items ON emoji_decks USING gin (items);

CREATE TRIGGER emoji_decks_updated_at
    BEFORE UPDATE ON emoji_decks
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Recently used emoji per user (same item format as decks).
CREATE TABLE emoji_recents (
    user_id   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    item      text        NOT NULL,
    used_at   timestamptz NOT NULL DEFAULT now(),
    use_count integer     NOT NULL DEFAULT 1,
    PRIMARY KEY (user_id, item)
);

CREATE INDEX emoji_recents_user ON emoji_recents (user_id, used_at DESC);
-- Finds the rows to rewrite when an emoji is renamed.
CREATE INDEX emoji_recents_item ON emoji_recents (item);

-- Imports of emoji packs (ZIP archives in Misskey's export format).
CREATE TABLE emoji_imports (
    id              uuid PRIMARY KEY     DEFAULT uuidv7(),
    requested_by    uuid REFERENCES users (id) ON DELETE SET NULL,
    -- The uploaded archive; held as `emoji:import:<id>` until the import ends.
    asset_id        uuid        NOT NULL,
    -- tokuly.emoji.v1.ImportConflictPolicy: 1 skip, 2 rename.
    conflict_policy smallint    NOT NULL DEFAULT 1,
    -- Category every entry goes into; NULL = the pack's own categories.
    category        text,
    -- tokuly.emoji.v1.ImportStatus: 1 queued, 2 running, 3 completed, 4 failed.
    status          smallint    NOT NULL DEFAULT 1,
    error           text        NOT NULL DEFAULT '',
    source_host     text        NOT NULL DEFAULT '',
    total_count     integer     NOT NULL DEFAULT 0,
    imported_count  integer     NOT NULL DEFAULT 0,
    skipped_count   integer     NOT NULL DEFAULT 0,
    failed_count    integer     NOT NULL DEFAULT 0,
    created_at      timestamptz NOT NULL DEFAULT now(),
    -- When the entries were read from the pack.
    planned_at      timestamptz,
    completed_at    timestamptz
);

CREATE INDEX emoji_imports_open ON emoji_imports (created_at) WHERE status IN (1, 2);

CREATE TABLE emoji_import_entries (
    import_id   uuid     NOT NULL REFERENCES emoji_imports (id) ON DELETE CASCADE,
    -- Index in the pack's `emojis` array.
    position    integer  NOT NULL,
    source_name text     NOT NULL,
    file_name   text     NOT NULL,
    -- The name here (normalized, possibly numbered); '' when none could be made.
    name        text     NOT NULL DEFAULT '',
    -- tokuly.emoji.v1.ImportEntryStatus: 1 processing, 2 imported, 3 renamed,
    -- 4 skipped, 5 failed.
    status      smallint NOT NULL,
    reason      text     NOT NULL DEFAULT '',
    -- What the pack says about the emoji: category, aliases, license, flags.
    meta        jsonb    NOT NULL DEFAULT '{}',
    -- The upload made from the entry's file, once it is extracted.
    asset_id    uuid,
    -- The emoji it became.
    emoji_id    uuid,
    -- When its picture was handed to the media domain.
    extracted_at timestamptz,
    PRIMARY KEY (import_id, position)
);

-- Entries waiting for their picture, found by the upload that just finished.
CREATE INDEX emoji_import_entries_asset ON emoji_import_entries (asset_id) WHERE status = 1;

-- Exports of the catalogue to a ZIP in the private bucket.
CREATE TABLE emoji_exports (
    id           uuid PRIMARY KEY     DEFAULT uuidv7(),
    requested_by uuid REFERENCES users (id) ON DELETE SET NULL,
    -- Only this category ('' = emoji without one); NULL = everything.
    category     text,
    -- tokuly.emoji.v1.ExportStatus: 1 queued, 2 running, 3 ready, 4 failed, 5 expired.
    status       smallint    NOT NULL DEFAULT 1,
    object_key   text        NOT NULL DEFAULT '',
    size_bytes   bigint      NOT NULL DEFAULT 0,
    emoji_count  integer     NOT NULL DEFAULT 0,
    error        text        NOT NULL DEFAULT '',
    created_at   timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz,
    expires_at   timestamptz
);

CREATE INDEX emoji_exports_expiry ON emoji_exports (expires_at) WHERE status = 3;
