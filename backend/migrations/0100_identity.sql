-- Identity: accounts, sessions, passkeys, security log, profile views.

CREATE TABLE users (
    id                    uuid PRIMARY KEY DEFAULT uuidv7(),
    handle                citext      NOT NULL UNIQUE,
    display_name          text        NOT NULL,
    email                 citext UNIQUE,
    email_verified_at     timestamptz,
    password_hash         text,
    bio                   text        NOT NULL DEFAULT '',
    -- tokuly.common.v1.Image as proto JSON; copied from the media asset when set.
    avatar                jsonb,
    avatar_video_url      text,
    banner                jsonb,
    birthdate             date,
    locale                text        NOT NULL DEFAULT 'ja',
    time_zone             text        NOT NULL DEFAULT 'Asia/Tokyo',
    roles                 text[]      NOT NULL DEFAULT '{}',
    verified              boolean     NOT NULL DEFAULT false,
    school_name           text        NOT NULL DEFAULT '',

    status_emoji          text        NOT NULL DEFAULT '',
    status_text           text        NOT NULL DEFAULT '',
    status_set_at         timestamptz,
    status_expires_at     timestamptz,

    totp_secret           text,
    totp_enabled_at       timestamptz,
    -- Keyed hashes of unused recovery codes.
    recovery_code_hashes  text[]      NOT NULL DEFAULT '{}',

    -- tokuly.identity.v1.PrivacySettings / AppearanceSettings as proto JSON.
    -- Missing keys fall back to defaults in code, so new settings need no migration.
    privacy               jsonb       NOT NULL DEFAULT '{}',
    appearance            jsonb       NOT NULL DEFAULT '{}',

    invited_by            uuid REFERENCES users (id) ON DELETE SET NULL,
    profile_view_count    bigint      NOT NULL DEFAULT 0,

    banned_at             timestamptz,
    ban_reason            text        NOT NULL DEFAULT '',
    deletion_scheduled_at timestamptz,
    deleted_at            timestamptz,

    -- Extension point for future per-user data that does not deserve a column yet.
    attrs                 jsonb       NOT NULL DEFAULT '{}',
    created_at            timestamptz NOT NULL DEFAULT now(),
    updated_at            timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX users_handle_trgm ON users USING gin ((handle::text) gin_trgm_ops);
CREATE INDEX users_display_name_trgm ON users USING gin (display_name gin_trgm_ops);
CREATE INDEX users_deletion_scheduled ON users (deletion_scheduled_at) WHERE deletion_scheduled_at IS NOT NULL;

-- Contact matching compares uploaded SHA-256 hashes with verified emails. The
-- function is immutable in practice (fixed encoding), which lets `users` be
-- searched by hash through an index instead of hashing every row per lookup.
CREATE FUNCTION contact_hash(email citext) RETURNS text
    LANGUAGE sql
    IMMUTABLE STRICT PARALLEL SAFE AS
$$
SELECT encode(sha256(convert_to(lower(email::text), 'UTF8')), 'hex')
$$;

CREATE INDEX users_contact_hash ON users (contact_hash(email))
    WHERE email_verified_at IS NOT NULL AND deleted_at IS NULL;

-- "Same school" lookups; almost every row has no school and is skipped.
CREATE INDEX users_school ON users (school_name) WHERE school_name <> '' AND deleted_at IS NULL;

CREATE TRIGGER users_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE TABLE sessions (
    id                    uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id               uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Keyed hash of the current refresh token.
    refresh_hash          text        NOT NULL UNIQUE,
    -- The current refresh token, encrypted. A request that races a rotation and
    -- still presents the previous token (within a short grace period) is handed
    -- this same token again, so concurrent refreshes converge instead of
    -- logging the user out.
    refresh_sealed        bytea       NOT NULL,
    previous_refresh_hash text,
    rotated_at            timestamptz,
    platform              integer     NOT NULL DEFAULT 0,
    device_name           text        NOT NULL DEFAULT '',
    device_id             text        NOT NULL DEFAULT '',
    app_version           text        NOT NULL DEFAULT '',
    ip                    text        NOT NULL DEFAULT '',
    user_agent            text        NOT NULL DEFAULT '',
    location              text        NOT NULL DEFAULT '',
    created_at            timestamptz NOT NULL DEFAULT now(),
    last_active_at        timestamptz NOT NULL DEFAULT now(),
    expires_at            timestamptz NOT NULL,
    revoked_at            timestamptz
);

CREATE INDEX sessions_user ON sessions (user_id) WHERE revoked_at IS NULL;
CREATE INDEX sessions_previous_hash ON sessions (previous_refresh_hash) WHERE previous_refresh_hash IS NOT NULL;

CREATE TABLE passkeys (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name          text        NOT NULL DEFAULT '',
    credential_id bytea       NOT NULL UNIQUE,
    -- Serialized webauthn-rs `Passkey` (public key, counter, backup state).
    credential    jsonb       NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    last_used_at  timestamptz
);

CREATE INDEX passkeys_user ON passkeys (user_id);

-- Single-use tokens mailed to users.
CREATE TABLE email_tokens (
    token_hash text PRIMARY KEY,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- 'verify' | 'reset' | 'change'
    purpose    text        NOT NULL,
    -- The address being verified (for 'change': the new address).
    email      citext      NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    used_at    timestamptz
);

CREATE INDEX email_tokens_user ON email_tokens (user_id, purpose);

CREATE TABLE security_events (
    id          uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    type        text        NOT NULL,
    ip          text        NOT NULL DEFAULT '',
    device_name text        NOT NULL DEFAULT '',
    location    text        NOT NULL DEFAULT '',
    attrs       jsonb       NOT NULL DEFAULT '{}',
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX security_events_user ON security_events (user_id, id DESC);

-- Third-party account links (e.g. Spotify for "now playing").
CREATE TABLE connections (
    user_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    provider      text        NOT NULL,
    account_id    text        NOT NULL DEFAULT '',
    account_name  text        NOT NULL DEFAULT '',
    -- Encrypted with a key derived from the master secret.
    access_token  bytea       NOT NULL,
    refresh_token bytea,
    expires_at    timestamptz,
    scopes        text        NOT NULL DEFAULT '',
    connected_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, provider)
);

-- One row per viewer per day; feeds the profile view counter and viewer list.
CREATE TABLE profile_views (
    viewed_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    viewer_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    day       date        NOT NULL,
    viewed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (viewed_id, viewer_id, day)
);

CREATE INDEX profile_views_recent ON profile_views (viewed_id, viewed_at DESC);

CREATE TABLE data_exports (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- 'pending' | 'ready' | 'failed'
    status       text        NOT NULL DEFAULT 'pending',
    object_key   text,
    size_bytes   bigint,
    created_at   timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz
);

CREATE INDEX data_exports_user ON data_exports (user_id, id DESC);
