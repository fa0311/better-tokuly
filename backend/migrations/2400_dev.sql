-- Developer platform: personal access tokens, OAuth applications, bots and webhooks.
--
-- Secrets (tokens, client secrets, refresh tokens) are stored as keyed
-- hashes only (tokuly_core::keys::Keys::token_hash). The one exception is a
-- webhook's signing secret, which has to be read back to sign deliveries
-- and is stored encrypted (Keys::seal).

-- Personal access tokens (kind 'pat') and bot tokens (kind 'bot').
CREATE TABLE dev_tokens (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    kind          text        NOT NULL CHECK (kind IN ('pat', 'bot')),
    -- The account the token acts as: its owner, or the bot.
    user_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The person who created it (the bot's owner for a bot token).
    created_by    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name          text        NOT NULL,
    token_hash    text        NOT NULL UNIQUE,
    -- Display only: prefix and the last characters of the secret.
    hint          text        NOT NULL,
    scopes        text[]      NOT NULL,
    rate_tier     text        NOT NULL DEFAULT 'standard',
    -- Fingerprint of the creator's password hash when the token was made: a
    -- different one at verification means the password changed since, which
    -- revokes the token. NULL for accounts without a password.
    password_mark text,
    expires_at    timestamptz,
    last_used_at  timestamptz,
    last_used_ip  text        NOT NULL DEFAULT '',
    revoked_at    timestamptz,
    -- 'owner' | 'password_changed' | 'banned' | 'bot_deleted'
    revoked_reason text       NOT NULL DEFAULT '',
    created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX dev_tokens_user ON dev_tokens (user_id, id DESC) WHERE revoked_at IS NULL;
CREATE INDEX dev_tokens_creator ON dev_tokens (created_by) WHERE revoked_at IS NULL;

-- OAuth applications. The id is the client_id.
CREATE TABLE dev_apps (
    id                         uuid PRIMARY KEY DEFAULT uuidv7(),
    owner_id                   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name                       text        NOT NULL,
    description                text        NOT NULL DEFAULT '',
    -- tokuly.common.v1.Image as proto JSON.
    icon                       jsonb,
    homepage_url               text        NOT NULL DEFAULT '',
    privacy_policy_url         text        NOT NULL DEFAULT '',
    redirect_uris              text[]      NOT NULL,
    scopes                     text[]      NOT NULL,
    confidential               boolean     NOT NULL,
    -- Whether the device flow (RFC 8628) may be used; off unless asked for.
    device_flow                boolean     NOT NULL DEFAULT false,
    secret_hash                text,
    secret_hint                text        NOT NULL DEFAULT '',
    -- The secret replaced by the last rotation, honoured until it expires.
    previous_secret_hash       text,
    previous_secret_expires_at timestamptz,
    rate_tier                  text        NOT NULL DEFAULT 'standard',
    -- 'active' | 'disabled' (moderation)
    status                     text        NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    disabled_reason            text        NOT NULL DEFAULT '',
    attrs                      jsonb       NOT NULL DEFAULT '{}',
    created_at                 timestamptz NOT NULL DEFAULT now(),
    updated_at                 timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX dev_apps_owner ON dev_apps (owner_id, id DESC);

CREATE TRIGGER dev_apps_updated_at
    BEFORE UPDATE ON dev_apps
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- A user's standing permission for an application. Access and refresh
-- tokens are issued under it and die with it.
CREATE TABLE dev_authorizations (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    app_id       uuid        NOT NULL REFERENCES dev_apps (id) ON DELETE CASCADE,
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Everything the user agreed to so far.
    scopes       text[]      NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz,
    UNIQUE (app_id, user_id)
);

CREATE INDEX dev_authorizations_user ON dev_authorizations (user_id, id DESC);
CREATE INDEX dev_authorizations_activity ON dev_authorizations (app_id, last_used_at DESC NULLS LAST, id DESC);

-- Issued OAuth token pairs. Using a refresh token retires its row
-- (rotated_at) and adds the next one of the same family; presenting a
-- retired refresh token again revokes the whole family.
CREATE TABLE dev_oauth_tokens (
    id                 uuid PRIMARY KEY DEFAULT uuidv7(),
    authorization_id   uuid        NOT NULL REFERENCES dev_authorizations (id) ON DELETE CASCADE,
    -- The first row of the chain this pair descends from.
    family_id          uuid        NOT NULL,
    access_hash        text        NOT NULL UNIQUE,
    access_expires_at  timestamptz NOT NULL,
    refresh_hash       text        NOT NULL UNIQUE,
    refresh_expires_at timestamptz NOT NULL,
    scopes             text[]      NOT NULL,
    rotated_at         timestamptz,
    revoked_at         timestamptz,
    created_at         timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX dev_oauth_tokens_authorization ON dev_oauth_tokens (authorization_id);
CREATE INDEX dev_oauth_tokens_family ON dev_oauth_tokens (family_id);
CREATE INDEX dev_oauth_tokens_expiry ON dev_oauth_tokens (refresh_expires_at);

-- Bots: which accounts are bots and whose they are. (Identity has no bot
-- flag or owner link yet; this table is that link.)
CREATE TABLE dev_bots (
    user_id     uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    owner_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    disabled_at timestamptz,
    -- A deleted bot keeps its row for a while (counted from the deletion),
    -- so that a report about what it did still leads to its owner.
    deleted_at  timestamptz,
    attrs       jsonb       NOT NULL DEFAULT '{}',
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX dev_bots_owner ON dev_bots (owner_id, created_at) WHERE deleted_at IS NULL;
CREATE INDEX dev_bots_deleted ON dev_bots (deleted_at) WHERE deleted_at IS NOT NULL;

CREATE TABLE dev_webhooks (
    id                         uuid PRIMARY KEY DEFAULT uuidv7(),
    -- The account whose events are delivered (a person or a bot).
    account_id                 uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    url                        text        NOT NULL,
    description                text        NOT NULL DEFAULT '',
    event_types                text[]      NOT NULL,
    secret_sealed              bytea       NOT NULL,
    secret_hint                text        NOT NULL,
    previous_secret_sealed     bytea,
    previous_secret_expires_at timestamptz,
    -- 'active' | 'paused' (by the owner) | 'disabled' (kept failing)
    status                     text        NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'paused', 'disabled')),
    disabled_reason            text        NOT NULL DEFAULT '',
    -- Failed attempts since the last success, and when that run began.
    consecutive_failures       integer     NOT NULL DEFAULT 0,
    failing_since              timestamptz,
    last_success_at            timestamptz,
    last_failure_at            timestamptz,
    attrs                      jsonb       NOT NULL DEFAULT '{}',
    created_at                 timestamptz NOT NULL DEFAULT now(),
    updated_at                 timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX dev_webhooks_account ON dev_webhooks (account_id, id);
-- "Who wants this event type for this account" is the hot lookup.
CREATE INDEX dev_webhooks_events ON dev_webhooks USING gin (event_types) WHERE status = 'active';

CREATE TRIGGER dev_webhooks_updated_at
    BEFORE UPDATE ON dev_webhooks
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- The delivery log, and the queue of deliveries still to be made.
CREATE TABLE dev_webhook_deliveries (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    webhook_id      uuid        NOT NULL REFERENCES dev_webhooks (id) ON DELETE CASCADE,
    -- The event's id in the envelope; shared by retries and replays.
    event_id        uuid        NOT NULL,
    event_type      text        NOT NULL,
    -- The envelope { id, type, time, data } as sent.
    payload         jsonb       NOT NULL,
    -- 'pending' | 'retrying' | 'succeeded' | 'failed'
    status          text        NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'retrying', 'succeeded', 'failed')),
    -- [{ at, status_code, latency_ms, response_snippet, error }], oldest first.
    attempts        jsonb       NOT NULL DEFAULT '[]',
    attempt_count   integer     NOT NULL DEFAULT 0,
    -- When the next attempt is due; NULL once the delivery is settled.
    next_attempt_at timestamptz,
    -- Set while a worker is sending it, so two never send the same attempt.
    claimed_at      timestamptz,
    replay_of       uuid,
    created_at      timestamptz NOT NULL DEFAULT now(),
    completed_at    timestamptz
);

CREATE INDEX dev_webhook_deliveries_log ON dev_webhook_deliveries (webhook_id, id DESC);
CREATE INDEX dev_webhook_deliveries_due ON dev_webhook_deliveries (next_attempt_at) WHERE next_attempt_at IS NOT NULL;
-- An event reaches a webhook once, however often the bus redelivers it.
CREATE UNIQUE INDEX dev_webhook_deliveries_once ON dev_webhook_deliveries (webhook_id, event_id, event_type) WHERE replay_of IS NULL;
