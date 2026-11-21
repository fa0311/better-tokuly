-- Search: the document registry behind the Meilisearch indexes, per-user
-- search history, popular queries (trending and completions) and the word
-- lists moderators maintain.

-- Every searchable document, as the search domain last heard of it. This is
-- the desired state of the indexes: the indexer copies rows whose
-- `generation` is ahead of `synced_generation` into Meilisearch, so the
-- engine can be rebuilt from here at any time.
CREATE TABLE search_documents (
    -- `user`, `channel`, `note`, `video`... (tokuly_core::outbound::SearchDoc::kind).
    kind              text        NOT NULL,
    doc_id            uuid        NOT NULL,
    -- ID of the newest change applied. Event IDs are UUIDv7, so byte order
    -- is time order: a change older than this is discarded, which is what
    -- keeps a late upsert from undoing a newer delete.
    version           uuid        NOT NULL,
    -- Owner account, when the document has one. No foreign key: official
    -- content is owned by the nil UUID. Documents whose owner is suspended
    -- or deleted are held here but kept out of the index.
    owner_id          uuid,
    -- The document as indexed (see `document.rs`); NULL = removed. Removed
    -- rows stay as tombstones until no older event can arrive any more.
    doc               jsonb,
    -- Bumped whenever the row has to be written to the index again.
    generation        bigint      NOT NULL DEFAULT 1,
    -- The generation the index reflects.
    synced_generation bigint      NOT NULL DEFAULT 0,
    -- When the row last became pending; the indexer works oldest first.
    queued_at         timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (kind, doc_id)
);

CREATE INDEX search_documents_pending ON search_documents (queued_at) WHERE synced_generation < generation;
CREATE INDEX search_documents_owner ON search_documents (owner_id) WHERE owner_id IS NOT NULL;
-- Tombstone sweep.
CREATE INDEX search_documents_removed ON search_documents (updated_at) WHERE doc IS NULL;

-- A user's recent searches: one row per distinct query, newest 50 kept.
CREATE TABLE search_recent (
    id          uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The query as searched (trimmed, Unicode-normalized, case kept).
    query       text        NOT NULL,
    -- Lowercased `query`: searching the same words again moves the row up
    -- instead of adding one.
    query_key   text        NOT NULL COLLATE "C",
    -- The tab it was last searched in; '' = all kinds.
    kind        text        NOT NULL DEFAULT '',
    searched_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (user_id, query_key)
);

CREATE INDEX search_recent_newest ON search_recent (user_id, searched_at DESC, id DESC);

-- Per-user search settings. A missing row means defaults (history on).
CREATE TABLE search_user_settings (
    user_id        uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    -- While set, searches are neither remembered nor counted towards trending.
    history_paused boolean     NOT NULL DEFAULT false,
    updated_at     timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER search_user_settings_updated_at
    BEFORE UPDATE ON search_user_settings
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Queries that enough different users searched. Raw per-hour counters live
-- in Valkey; the trending job folds them into this table, which serves both
-- the trending list (`trending_rank`) and popular completions (prefix scans
-- on `query`, hence the byte-order collation).
CREATE TABLE search_popular_queries (
    -- Normalized: lowercase, width-folded, single spaces.
    query         text COLLATE "C" PRIMARY KEY,
    -- Distinct signed-in users who searched it within the trending window,
    -- as of the last time it qualified.
    searchers     integer          NOT NULL,
    -- Recency-weighted searcher count; orders completions and trending.
    weight        double precision NOT NULL,
    -- Position in the current trending list (1 = top); NULL = not trending.
    trending_rank integer,
    first_seen_at timestamptz      NOT NULL DEFAULT now(),
    -- Last time the query qualified; rows that stop qualifying age out.
    last_seen_at  timestamptz      NOT NULL DEFAULT now()
);

CREATE INDEX search_popular_queries_trending ON search_popular_queries (trending_rank) WHERE trending_rank IS NOT NULL;
CREATE INDEX search_popular_queries_age ON search_popular_queries (last_seen_at);

-- Terms that keep a query out of trending and out of popular completions.
-- A query is blocked when its normalized form contains a term.
CREATE TABLE search_blocked_terms (
    -- Normalized like `search_popular_queries.query`.
    term       text COLLATE "C" PRIMARY KEY,
    -- Why it is blocked; for other moderators.
    note       text        NOT NULL DEFAULT '',
    added_by   uuid        REFERENCES users (id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Groups of words that find each other's documents; applied to every index
-- as Meilisearch synonyms.
CREATE TABLE search_synonym_groups (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    -- 2-20 normalized words.
    words      text[]      NOT NULL,
    updated_by uuid        REFERENCES users (id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER search_synonym_groups_updated_at
    BEFORE UPDATE ON search_synonym_groups
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Small pieces of domain-wide state, e.g. `trending` (when the trending
-- list was last computed).
CREATE TABLE search_state (
    key        text PRIMARY KEY,
    value      jsonb       NOT NULL DEFAULT '{}',
    updated_at timestamptz NOT NULL DEFAULT now()
);
