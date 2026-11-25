-- Notify: the notification inbox, preferences, mutes and push devices.

-- One inbox entry. Titles and bodies are stored in both languages and picked
-- when read, so a user who switches language sees their whole inbox in it.
CREATE TABLE notifications (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Dotted kind chosen by the producing domain, e.g. `chat.message`.
    kind         text        NOT NULL,
    -- Preference category derived from the kind (see `categories.rs`).
    category     text        NOT NULL,
    -- Who caused it. No foreign key: a request naming an unknown actor must
    -- not poison the consumer; rows are removed on `identity.user.deleted`.
    actor_id     uuid,
    title_ja     text        NOT NULL,
    title_en     text        NOT NULL,
    body_ja      text        NOT NULL,
    body_en      text        NOT NULL,
    image_url    text,
    -- Path in the app to open when tapped.
    path         text        NOT NULL,
    -- tokuly.common.v1.SubjectType and the subject's ID, when given.
    subject_type integer,
    subject_id   uuid,
    -- Entries of one user sharing a key replace each other.
    collapse_key text,
    -- Extra structured data from the producing domain.
    data         jsonb       NOT NULL DEFAULT 'null',
    -- IDs the entry refers to: the subject and every `*_id` value at the top
    -- level of `data` (the comment, the message...). When one of them is
    -- deleted or taken down the entry is withdrawn (see `withdraw.rs`).
    refs         uuid[]      NOT NULL DEFAULT '{}',
    -- The producing domain marked it urgent (incoming call, SOS): its push
    -- was sent even during quiet hours and while pushes were paused.
    urgent       boolean     NOT NULL DEFAULT false,
    -- Events folded into this entry since it was last read.
    group_count  integer     NOT NULL DEFAULT 1 CHECK (group_count >= 1),
    read_at      timestamptz,
    -- Time of the latest event folded into the entry; the inbox sorts by it.
    occurred_at  timestamptz NOT NULL DEFAULT now(),
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX notifications_collapse ON notifications (user_id, collapse_key) WHERE collapse_key IS NOT NULL;
CREATE INDEX notifications_inbox ON notifications (user_id, occurred_at DESC, id DESC);
CREATE INDEX notifications_unread ON notifications (user_id, category) WHERE read_at IS NULL;
CREATE INDEX notifications_actor ON notifications (actor_id) WHERE actor_id IS NOT NULL;
CREATE INDEX notifications_refs ON notifications USING gin (refs);
-- Retention sweep.
CREATE INDEX notifications_age ON notifications (occurred_at);

-- IDs of content that was deleted or taken down. A request about one that
-- arrives after the deletion (events are not ordered) is dropped, and a push
-- still queued for it is not sent. Swept with the handled-event markers.
CREATE TABLE notify_withdrawn (
    ref_id       uuid PRIMARY KEY,
    withdrawn_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX notify_withdrawn_age ON notify_withdrawn (withdrawn_at);

-- The latest edit of each comment notifications quote (`edits.rs`): what
-- they show of it now and whom it still mentions, by version (`edited_at`,
-- the comment's own). A request or push about an older version that is
-- handled after the edit shows this instead. Swept with the handled-event
-- markers.
CREATE TABLE notify_edits (
    ref_id      uuid PRIMARY KEY,
    -- The comment's author; their edits go with their account.
    author_id   uuid        NOT NULL,
    body_ja     text        NOT NULL,
    body_en     text        NOT NULL,
    mentioned   uuid[]      NOT NULL DEFAULT '{}',
    edited_at   timestamptz NOT NULL,
    -- For a top-level comment: its subject's owner, and the kind and title
    -- their mention becomes when the edit took the mention out.
    owner_id       uuid,
    owner_kind     text,
    owner_title_ja text,
    owner_title_en text,
    recorded_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX notify_edits_age ON notify_edits (recorded_at);
CREATE INDEX notify_edits_author ON notify_edits (author_id);
CREATE INDEX notify_edits_mentioned ON notify_edits USING gin (mentioned);

-- The latest change of who may see a piece of content (a hidden comment, a
-- narrowed note; `withdraw.rs`): its owner, who else (`visible_to`, NULL =
-- everyone) and when it changed (the producer's time, which orders two
-- changes). A request or push about it from before the change that is handled
-- after it reaches nobody else. Swept with the handled-event markers.
CREATE TABLE notify_audiences (
    ref_id      uuid PRIMARY KEY,
    owner_id    uuid        NOT NULL,
    visible_to  uuid[],
    changed_at  timestamptz NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX notify_audiences_age ON notify_audiences (recorded_at);
CREATE INDEX notify_audiences_owner ON notify_audiences (owner_id);
CREATE INDEX notify_audiences_visible_to ON notify_audiences USING gin (visible_to);

-- IDs of `notify.requested` events that were already handled. Event delivery
-- is at-least-once; a row here makes a redelivery a no-op. Rows outlive the
-- event stream's retention (7 days) and are then swept.
CREATE TABLE notify_processed_events (
    event_id     uuid PRIMARY KEY,
    processed_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX notify_processed_events_age ON notify_processed_events (processed_at);

-- Per-user settings that are not per category. A missing row means defaults:
-- push not paused, no quiet hours.
CREATE TABLE notify_settings (
    user_id            uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    push_paused        boolean     NOT NULL DEFAULT false,
    -- When a pause ends by itself; NULL while paused = until resumed.
    push_paused_until  timestamptz,
    quiet_enabled      boolean     NOT NULL DEFAULT false,
    -- Minutes after midnight in the user's time zone (`users.time_zone`).
    quiet_start_minute smallint    NOT NULL DEFAULT 1320 CHECK (quiet_start_minute BETWEEN 0 AND 1439),
    quiet_end_minute   smallint    NOT NULL DEFAULT 420 CHECK (quiet_end_minute BETWEEN 0 AND 1439),
    -- What pushes show (tokuly.notify.v1.PushPreview: 1 full, 2 sender only,
    -- 3 hidden). Set when the account is created (2 for minors, 1 for
    -- adults; identity.user.created) and by the owner afterwards; never
    -- derived from the birthdate on a read, so turning 18 changes nothing.
    -- NULL until then, read as full.
    push_preview       smallint    CHECK (push_preview BETWEEN 1 AND 3),
    attrs              jsonb       NOT NULL DEFAULT '{}',
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER notify_settings_updated_at
    BEFORE UPDATE ON notify_settings
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Accounts of minors that exist already show only the sender on the lock screen.
INSERT INTO notify_settings (user_id, push_preview)
SELECT id, 2 FROM users
WHERE birthdate > current_date - interval '18 years' AND deleted_at IS NULL
ON CONFLICT (user_id) DO NOTHING;

-- Per-category toggles. A missing row means both are on.
CREATE TABLE notify_category_prefs (
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    category   text        NOT NULL,
    inbox      boolean     NOT NULL DEFAULT true,
    push       boolean     NOT NULL DEFAULT true,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, category)
);

-- "Do not notify me about what this person does." Written by the mute RPCs
-- and kept in sync with the friend preference of the social domain.
-- Unmuting keeps the row (`muted = false`): changes arrive from two places
-- and events are not ordered, so the latest change (`changed_at`) wins.
CREATE TABLE notify_mutes (
    user_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    muted_user_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    muted         boolean     NOT NULL DEFAULT true,
    -- When the mute was last switched on (the list is sorted by it).
    created_at    timestamptz NOT NULL DEFAULT now(),
    -- Time of the change this row reflects: the RPC's or the event's.
    changed_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, muted_user_id),
    CHECK (user_id <> muted_user_id)
);

CREATE INDEX notify_mutes_list ON notify_mutes (user_id, created_at DESC, muted_user_id DESC) WHERE muted;
CREATE INDEX notify_mutes_muted ON notify_mutes (muted_user_id);

-- Blocks, mirrored from the social domain's events. Nothing is delivered
-- across a block in either direction. Unblocking keeps the row
-- (`blocked = false`) so that events handled out of order can be told apart
-- by `changed_at` (the time of the event).
CREATE TABLE notify_blocks (
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    blocked_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    blocked    boolean     NOT NULL,
    changed_at timestamptz NOT NULL,
    PRIMARY KEY (user_id, blocked_id),
    CHECK (user_id <> blocked_id)
);

CREATE INDEX notify_blocks_blocked ON notify_blocks (blocked_id);

-- Where push notifications go: one registration per session and channel.
-- Rows die with their session (cascade on purge; swept when it is revoked or
-- expires) and when the push service reports the token as gone.
CREATE TABLE push_devices (
    id                uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id           uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    session_id        uuid        NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    -- tokuly.notify.v1.PushChannelKind
    channel           integer     NOT NULL,
    -- tokuly.common.v1.Platform
    platform          integer     NOT NULL DEFAULT 0,
    -- Web Push endpoint URL, FCM registration token or APNs device token (hex).
    token             text        NOT NULL,
    -- SHA-256 of the token, written by the application. Tokens can be long
    -- (endpoint URLs); uniqueness is enforced on the digest.
    token_hash        bytea       NOT NULL,
    -- Web Push only: the browser's P-256 public key (65 bytes) and auth secret (16 bytes).
    p256dh            bytea,
    auth              bytea,
    -- APNs only: the token belongs to the sandbox environment.
    apns_sandbox      boolean     NOT NULL DEFAULT false,
    -- Language of the device; empty = the account's language.
    locale            text        NOT NULL DEFAULT '',
    device_name       text        NOT NULL DEFAULT '',
    -- Consecutive deliveries the push service refused, and what it last
    -- said: for diagnosing a device that stopped receiving. Reset on success.
    rejections        integer     NOT NULL DEFAULT 0,
    last_error        text        NOT NULL DEFAULT '',
    last_delivered_at timestamptz,
    attrs             jsonb       NOT NULL DEFAULT '{}',
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now(),
    UNIQUE (channel, token_hash),
    UNIQUE (session_id, channel),
    CHECK (channel <> 1 OR (p256dh IS NOT NULL AND auth IS NOT NULL))
);

CREATE INDEX push_devices_user ON push_devices (user_id);

CREATE TRIGGER push_devices_updated_at
    BEFORE UPDATE ON push_devices
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();
