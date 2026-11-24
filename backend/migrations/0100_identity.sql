-- Identity: accounts, sessions, passkeys, security log, profiles (with the
-- プロフ帳), profile views.

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

    -- A bot account made on the developer platform, and the person it
    -- belongs to (other domains read `bot` in joins: bots never count as people).
    bot                   boolean     NOT NULL DEFAULT false,
    bot_owner_id          uuid REFERENCES users (id) ON DELETE SET NULL,

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

CREATE INDEX users_bot_owner ON users (bot_owner_id) WHERE bot_owner_id IS NOT NULL;

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
    revoked_at            timestamptz,
    -- Account switching: the session was signed in beside another account on
    -- the device, is parked there (another account is active; it refreshes
    -- only through a switch), and when it was last switched to.
    added_by_switch       boolean     NOT NULL DEFAULT false,
    parked_at             timestamptz,
    switched_at           timestamptz,
    -- When this session last passed a second factor (TOTP, recovery code or
    -- passkey). A switch to an account with TOTP on asks again past 12 hours.
    mfa_at                timestamptz
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

-- ───────────────────────────── プロフ帳 ─────────────────────────────

-- The single-valued fields of the プロフ帳, one row per account that filled
-- any of it. `*_visibility` are tokuly.common.v1.Visibility values (public 1,
-- friends 4, private 5, followers 6, close friends 7) of the field next to them.
CREATE TABLE profile_extras (
    user_id                uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    mbti                   text        NOT NULL DEFAULT '',
    blood_type             integer     NOT NULL DEFAULT 0,
    pronouns               text        NOT NULL DEFAULT '',
    long_bio               text        NOT NULL DEFAULT '',
    -- tokuly.identity.v1.ProfileTheme as proto JSON.
    theme                  jsonb       NOT NULL DEFAULT '{}',
    -- tokuly.identity.v1.PinnedSong as proto JSON.
    pinned_song            jsonb,
    mbti_visibility        integer     NOT NULL DEFAULT 1,
    blood_type_visibility  integer     NOT NULL DEFAULT 1,
    pronouns_visibility    integer     NOT NULL DEFAULT 1,
    zodiac_visibility      integer     NOT NULL DEFAULT 5,
    oshi_visibility        integer     NOT NULL DEFAULT 1,
    tags_visibility        integer     NOT NULL DEFAULT 1,
    links_visibility       integer     NOT NULL DEFAULT 1,
    long_bio_visibility    integer     NOT NULL DEFAULT 1,
    pinned_song_visibility integer     NOT NULL DEFAULT 1,
    attrs                  jsonb       NOT NULL DEFAULT '{}',
    created_at             timestamptz NOT NULL DEFAULT now(),
    updated_at             timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER profile_extras_updated_at
    BEFORE UPDATE ON profile_extras
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- 推し, in display order.
CREATE TABLE profile_oshi (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    position   integer     NOT NULL,
    name       text        NOT NULL,
    -- tokuly.common.v1.Image as proto JSON; the asset is held as `identity:oshi:<id>`.
    image      jsonb,
    url        text        NOT NULL DEFAULT '',
    emoji      text        NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX profile_oshi_user ON profile_oshi (user_id, position);

-- Favourite things. `name_key` is the name folded for matching (lowercase,
-- width-normalized) so suggestions count 「ヨルシカ」 and 「ﾖﾙｼｶ」 together.
CREATE TABLE profile_tags (
    user_id  uuid    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    category integer NOT NULL,
    name     text    NOT NULL,
    name_key text    NOT NULL,
    position integer NOT NULL,
    PRIMARY KEY (user_id, category, name_key)
);

CREATE INDEX profile_tags_suggest ON profile_tags (category, name_key text_pattern_ops);

-- Links, in display order. `verified_at` is set while the page links back
-- to the profile with rel="me"; the background check fills `checked_at`.
CREATE TABLE profile_links (
    id          uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    position    integer     NOT NULL,
    title       text        NOT NULL,
    url         text        NOT NULL,
    verified_at timestamptz,
    checked_at  timestamptz,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX profile_links_user ON profile_links (user_id, position);
CREATE INDEX profile_links_checked ON profile_links (checked_at NULLS FIRST);

-- Question-and-answer cards of the プロフ帳, in display order.
CREATE TABLE profile_qa_cards (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    position   integer     NOT NULL,
    -- A prompt of the curated bank (code), or '' for a free question.
    prompt_id  text        NOT NULL DEFAULT '',
    question   text        NOT NULL DEFAULT '',
    answer     text        NOT NULL,
    -- tokuly.common.v1.Visibility; with VISIBILITY_CIRCLE (8), `circle_id`
    -- names one of the owner's circles (social domain; no foreign key).
    visibility integer     NOT NULL DEFAULT 1,
    circle_id  uuid,
    hidden     boolean     NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX profile_qa_cards_user ON profile_qa_cards (user_id, position);

CREATE TRIGGER profile_qa_cards_updated_at
    BEFORE UPDATE ON profile_qa_cards
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();
