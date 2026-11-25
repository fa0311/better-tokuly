-- Foundation: extensions, the event outbox and shared helpers.

CREATE EXTENSION IF NOT EXISTS citext;
CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE EXTENSION IF NOT EXISTS btree_gist;
CREATE EXTENSION IF NOT EXISTS postgis;

-- Transactional outbox: domain events written in the same transaction as the
-- change they describe, relayed to the EVENTS stream by tokuly-core.
CREATE TABLE outbox (
    id          uuid PRIMARY KEY DEFAULT uuidv7(),
    topic       text        NOT NULL,
    actor_id    uuid,
    data        jsonb       NOT NULL,
    occurred_at timestamptz NOT NULL DEFAULT now()
);

CREATE FUNCTION outbox_notify() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    PERFORM pg_notify('tokuly_outbox', '');
    RETURN NULL;
END
$$;

CREATE TRIGGER outbox_notify
    AFTER INSERT ON outbox
    FOR EACH STATEMENT
EXECUTE FUNCTION outbox_notify();

-- Keeps `updated_at` current on any table that has the column.
CREATE FUNCTION set_updated_at() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END
$$;

-- Feature flags, evaluated per user by tokuly-core and managed from the admin console.
CREATE TABLE feature_flags (
    name            text PRIMARY KEY,
    description     text        NOT NULL DEFAULT '',
    enabled         boolean     NOT NULL DEFAULT false,
    -- 0-100: share of users (by stable hash) who get the flag when `enabled`.
    rollout_percent integer     NOT NULL DEFAULT 100 CHECK (rollout_percent BETWEEN 0 AND 100),
    -- Users who always get the flag, regardless of `enabled`.
    allow_user_ids  uuid[]      NOT NULL DEFAULT '{}',
    updated_at      timestamptz NOT NULL DEFAULT now()
);

-- Server settings saved from the staff console (「サーバー設定」), one row per
-- setting that differs from the lower layers. `tokuly.toml` and the
-- environment win over these rows (see `tokuly_core::settings`).
CREATE TABLE server_settings (
    section    text        NOT NULL,
    key        text        NOT NULL,
    -- The value as the command line would take it; NULL for secrets.
    value      text,
    -- Secrets, sealed with the core keys (`Keys::seal`); NULL otherwise.
    sealed     bytea,
    -- The last four characters of a secret, all the console shows of it.
    last4      text        NOT NULL DEFAULT '',
    updated_by uuid,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (section, key),
    CHECK ((value IS NULL) <> (sealed IS NULL))
);

-- Who changed which setting from the console, when, and from what to what.
-- Secrets are recorded as set or cleared, never with their value.
CREATE TABLE server_setting_changes (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    section    text        NOT NULL,
    key        text        NOT NULL,
    -- NULL: the console had no value (the lower layers applied).
    old_value  text,
    new_value  text,
    secret     boolean     NOT NULL DEFAULT false,
    actor_id   uuid,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX server_setting_changes_key_idx ON server_setting_changes (section, key, id DESC);
