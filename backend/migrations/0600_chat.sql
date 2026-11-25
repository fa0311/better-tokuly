-- Chat: direct and group conversations, messages, reactions, pins, read
-- state, POV requests and calls.

-- ───────────────────────────── conversations ─────────────────────────────

CREATE TABLE chat_conversations (
    id               uuid PRIMARY KEY     DEFAULT uuidv7(),
    -- tokuly.chat.v1.ConversationKind: 1 direct, 2 group.
    kind             smallint    NOT NULL,
    -- Direct conversations only: the two users in canonical order, which is
    -- what makes a direct conversation unique per pair. The columns survive
    -- a membership being removed (account deletion).
    user_lo          uuid REFERENCES users (id) ON DELETE CASCADE,
    user_hi          uuid REFERENCES users (id) ON DELETE CASCADE,
    -- Groups only.
    name             text        NOT NULL DEFAULT '',
    -- tokuly.common.v1.Image as proto JSON, and the asset it came from.
    icon             jsonb,
    icon_asset_id    uuid,
    created_by       uuid        REFERENCES users (id) ON DELETE SET NULL,
    member_count     integer     NOT NULL DEFAULT 0,
    -- Sequence number of the newest message; the row lock taken to advance
    -- it is what orders concurrent sends.
    last_seq         bigint      NOT NULL DEFAULT 0,
    last_message_id  uuid,
    last_activity_at timestamptz NOT NULL DEFAULT now(),
    -- Direct only: nobody can send (a block between the two, or one account
    -- was deleted). Blocks are also checked live when sending; this flag is
    -- what lists show.
    read_only        boolean     NOT NULL DEFAULT false,
    attrs            jsonb       NOT NULL DEFAULT '{}',
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CHECK ((kind = 1 AND user_lo IS NOT NULL AND user_hi IS NOT NULL AND user_lo < user_hi)
        OR (kind = 2 AND user_lo IS NULL AND user_hi IS NULL))
);

CREATE UNIQUE INDEX chat_conversations_pair ON chat_conversations (user_lo, user_hi) WHERE kind = 1;
CREATE INDEX chat_conversations_user_hi ON chat_conversations (user_hi) WHERE kind = 1;

CREATE TRIGGER chat_conversations_updated_at
    BEFORE UPDATE ON chat_conversations
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- One row per member: role, where the conversation stands for them, their
-- read cursor and their private preferences.
CREATE TABLE chat_members (
    conversation_id      uuid        NOT NULL REFERENCES chat_conversations (id) ON DELETE CASCADE,
    user_id              uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.chat.v1.MemberRole: 1 member, 2 admin, 3 owner.
    role                 smallint    NOT NULL DEFAULT 1,
    -- tokuly.chat.v1.RequestState: 1 accepted, 2 pending request, 3 declined.
    request_state        smallint    NOT NULL DEFAULT 1,
    -- Messages this member sent while the other side had not accepted.
    request_messages     integer     NOT NULL DEFAULT 0,
    read_seq             bigint      NOT NULL DEFAULT 0,
    read_at              timestamptz,
    unread_count         integer     NOT NULL DEFAULT 0,
    unread_mention_count integer     NOT NULL DEFAULT 0,
    -- Messages with seq <= this are invisible to the member: they joined
    -- later, or cleared the history.
    history_from_seq     bigint      NOT NULL DEFAULT 0,
    pinned_at            timestamptz,
    muted                boolean     NOT NULL DEFAULT false,
    muted_until          timestamptz,
    archived_at          timestamptz,
    -- Hidden until the next message arrives.
    hidden_at            timestamptz,
    added_by             uuid,
    joined_at            timestamptz NOT NULL DEFAULT now(),
    attrs                jsonb       NOT NULL DEFAULT '{}',
    PRIMARY KEY (conversation_id, user_id)
);

CREATE INDEX chat_members_user ON chat_members (user_id, conversation_id);

-- ───────────────────────────── messages ─────────────────────────────

CREATE TABLE chat_messages (
    id              uuid PRIMARY KEY     DEFAULT uuidv7(),
    conversation_id uuid        NOT NULL REFERENCES chat_conversations (id) ON DELETE CASCADE,
    seq             bigint      NOT NULL,
    -- NULL for system messages. Kept when the account is deleted (the
    -- message then shows "deleted user").
    sender_id       uuid        REFERENCES users (id) ON DELETE SET NULL,
    -- tokuly.chat.v1.MessageKind.
    kind            smallint    NOT NULL,
    -- Idempotency key chosen by the sending client.
    client_id       text,
    text            text        NOT NULL DEFAULT '',
    -- [{ "user_id": …, "handle": … }]: members mentioned as @handle.
    mentions        jsonb       NOT NULL DEFAULT '[]',
    mention_all     boolean     NOT NULL DEFAULT false,
    stamp_id        uuid,
    -- [tokuly.media.v1.Attachment as proto JSON]; chat_message_assets indexes them.
    attachments     jsonb       NOT NULL DEFAULT '[]',
    -- tokuly.chat.v1.LocationShare as proto JSON.
    location        jsonb,
    -- First link in the text; its preview is fetched by a job.
    link_url        text,
    -- tokuly.media.v1.LinkPreview as proto JSON; NULL when the page offers none.
    link_preview    jsonb,
    -- When the preview job looked the link up (whatever the outcome).
    link_checked_at timestamptz,
    reply_to_id     uuid,
    forwarded       boolean     NOT NULL DEFAULT false,
    -- System messages: { "type": SystemEventType, "actor_id", "target_ids", "text", "message_id" }.
    system          jsonb,
    call_id         uuid,
    -- Set on a POV request message and on the media message answering it.
    pov_request_id  uuid,
    edited_at       timestamptz,
    deleted_at      timestamptz,
    deleted_by      uuid,
    attrs           jsonb       NOT NULL DEFAULT '{}',
    created_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (conversation_id, seq)
);

CREATE UNIQUE INDEX chat_messages_client ON chat_messages (conversation_id, sender_id, client_id) WHERE client_id IS NOT NULL;
CREATE INDEX chat_messages_sender ON chat_messages (sender_id) WHERE sender_id IS NOT NULL;
-- Per-conversation substring search.
CREATE INDEX chat_messages_text_trgm ON chat_messages USING gin (text gin_trgm_ops) WHERE deleted_at IS NULL AND text <> '';
-- Messages still waiting for their link preview (a sweep retries lost jobs).
CREATE INDEX chat_messages_link_pending ON chat_messages (created_at) WHERE link_url IS NOT NULL AND link_checked_at IS NULL;

-- Which uploads a message shows: lets the domain find the messages of an
-- asset (processing finished, asset deleted) and list a conversation's media.
CREATE TABLE chat_message_assets (
    message_id      uuid     NOT NULL REFERENCES chat_messages (id) ON DELETE CASCADE,
    asset_id        uuid     NOT NULL,
    conversation_id uuid     NOT NULL REFERENCES chat_conversations (id) ON DELETE CASCADE,
    seq             bigint   NOT NULL,
    position        smallint NOT NULL DEFAULT 0,
    -- tokuly.media.v1.AssetKind.
    kind            smallint NOT NULL,
    PRIMARY KEY (message_id, asset_id)
);

CREATE INDEX chat_message_assets_asset ON chat_message_assets (asset_id);
CREATE INDEX chat_message_assets_gallery ON chat_message_assets (conversation_id, seq DESC, position DESC);

CREATE TABLE chat_reactions (
    message_id uuid        NOT NULL REFERENCES chat_messages (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    emoji      text        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (message_id, user_id, emoji)
);

CREATE INDEX chat_reactions_newest ON chat_reactions (message_id, created_at DESC, user_id);
CREATE INDEX chat_reactions_user ON chat_reactions (user_id);

CREATE TABLE chat_pins (
    message_id      uuid PRIMARY KEY REFERENCES chat_messages (id) ON DELETE CASCADE,
    conversation_id uuid        NOT NULL REFERENCES chat_conversations (id) ON DELETE CASCADE,
    pinned_by       uuid,
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX chat_pins_conversation ON chat_pins (conversation_id, created_at DESC, message_id DESC);

-- What a message said before its sender edited or unsent it, kept only so
-- that the people it reached can still report it: the subject resolver is
-- the one reader. Purged 30 days after the removal.
CREATE TABLE chat_removed_texts (
    id         uuid PRIMARY KEY     DEFAULT uuidv7(),
    message_id uuid        NOT NULL REFERENCES chat_messages (id) ON DELETE CASCADE,
    -- The text; for a message without one, the line notifications describe it with.
    text       text        NOT NULL,
    -- 1 replaced by an edit, 2 deleted for everyone.
    reason     smallint    NOT NULL,
    removed_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX chat_removed_texts_message ON chat_removed_texts (message_id, removed_at);
CREATE INDEX chat_removed_texts_removed ON chat_removed_texts (removed_at);

-- ───────────────────────────── POV requests ─────────────────────────────

CREATE TABLE chat_pov_requests (
    id                 uuid PRIMARY KEY     DEFAULT uuidv7(),
    conversation_id    uuid        NOT NULL REFERENCES chat_conversations (id) ON DELETE CASCADE,
    requester_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    target_id          uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    note               text        NOT NULL DEFAULT '',
    -- tokuly.chat.v1.PovState: 1 open, 2 answered, 3 declined, 4 cancelled, 5 expired.
    state              smallint    NOT NULL DEFAULT 1,
    request_message_id uuid,
    reply_message_id   uuid,
    -- Until when the request can be answered.
    expires_at         timestamptz NOT NULL,
    answered_at        timestamptz,
    -- When the unsaved answer disappears.
    reply_expires_at   timestamptz,
    saved_at           timestamptz,
    reply_expired_at   timestamptz,
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now()
);

-- One open request per conversation.
CREATE UNIQUE INDEX chat_pov_open ON chat_pov_requests (conversation_id) WHERE state = 1;
CREATE INDEX chat_pov_answer_due ON chat_pov_requests (expires_at) WHERE state = 1;
CREATE INDEX chat_pov_reply_due ON chat_pov_requests (reply_expires_at)
    WHERE state = 2 AND saved_at IS NULL AND reply_expired_at IS NULL;

CREATE TRIGGER chat_pov_requests_updated_at
    BEFORE UPDATE ON chat_pov_requests
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- ───────────────────────────── calls ─────────────────────────────

CREATE TABLE chat_calls (
    id              uuid PRIMARY KEY     DEFAULT uuidv7(),
    conversation_id uuid        NOT NULL REFERENCES chat_conversations (id) ON DELETE CASCADE,
    started_by      uuid        REFERENCES users (id) ON DELETE SET NULL,
    video           boolean     NOT NULL DEFAULT false,
    -- tokuly.chat.v1.CallState: 1 ringing, 2 active, 3 ended.
    state           smallint    NOT NULL DEFAULT 1,
    -- tokuly.chat.v1.CallEndReason; 0 while the call is live.
    end_reason      smallint    NOT NULL DEFAULT 0,
    -- LiveKit room name.
    room            text        NOT NULL,
    -- The CALL message in the conversation.
    message_id      uuid,
    ring_expires_at timestamptz NOT NULL,
    answered_at     timestamptz,
    ended_at        timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now()
);

-- One live call per conversation. The index doubles as the list of live
-- calls for the ringing timeout and the reconciliation with LiveKit.
CREATE UNIQUE INDEX chat_calls_live ON chat_calls (conversation_id) WHERE state <> 3;

CREATE TABLE chat_call_participants (
    call_id   uuid        NOT NULL REFERENCES chat_calls (id) ON DELETE CASCADE,
    user_id   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.chat.v1.CallParticipantState: 1 ringing, 2 joined, 3 declined, 4 left, 5 missed.
    state     smallint    NOT NULL DEFAULT 1,
    joined_at timestamptz,
    left_at   timestamptz,
    -- Seconds spent connected, summed over every time the user joined.
    seconds   integer     NOT NULL DEFAULT 0,
    -- Whether the user was rung (false for someone who joined a group call
    -- nobody was rung for, or who was added to the group during the call).
    rung      boolean     NOT NULL DEFAULT true,
    PRIMARY KEY (call_id, user_id)
);

CREATE INDEX chat_call_participants_user ON chat_call_participants (user_id, call_id DESC);

-- ───────────────────────────── AI assistant ─────────────────────────────

-- Operator settings of one AI provider, once saved in the staff console
-- (AssistAdminService). A provider without a row runs on the environment's
-- values (TOKULY_ANTHROPIC_*, TOKULY_OPENAI_*, TOKULY_OPENAI_COMPAT_*).
CREATE TABLE chat_ai_providers (
    -- tokuly.chat.v1.AiProvider: 1 Anthropic, 2 OpenAI, 3 OpenAI-compatible.
    provider           smallint    PRIMARY KEY CHECK (provider BETWEEN 1 AND 3),
    enabled            boolean     NOT NULL,
    model              text        NOT NULL,
    base_url           text        NOT NULL,
    -- The API key, sealed (core keys, label "chat/ai-key/instance"). Never
    -- returned to a client; only its last four characters are shown.
    api_key_sealed     bytea,
    api_key_last4      text        NOT NULL DEFAULT '',
    -- OpenAI-compatible only: the base URL's host may be on a private
    -- network (the outbound guard's one exception, for that host only).
    allow_private_host boolean     NOT NULL DEFAULT false CHECK (NOT allow_private_host OR provider = 3),
    updated_by         uuid        REFERENCES users (id) ON DELETE SET NULL,
    updated_at         timestamptz NOT NULL DEFAULT now()
);

-- The instance default, once chosen in the staff console (one row).
CREATE TABLE chat_ai_instance (
    singleton        boolean     PRIMARY KEY DEFAULT true CHECK (singleton),
    -- tokuly.chat.v1.AiProvider; 0: the first enabled provider.
    default_provider smallint    NOT NULL DEFAULT 0,
    updated_by       uuid        REFERENCES users (id) ON DELETE SET NULL,
    updated_at       timestamptz NOT NULL DEFAULT now()
);

-- Each user's assistant settings. No row: the defaults (the instance
-- default provider, on, consent by age).
CREATE TABLE chat_ai_user_settings (
    user_id           uuid        PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.chat.v1.AiProvider; 0: the instance default.
    provider          smallint    NOT NULL DEFAULT 0,
    off               boolean     NOT NULL DEFAULT false,
    -- 「AI に自分のメッセージを使ってもいい」. Stored when the account is
    -- created (identity.user.created): off for an account of a minor, on
    -- for an adult, by the age at creation, and kept until the owner
    -- chooses. Never derived from today's date, so turning 18 changes
    -- nothing anybody can see. NULL (a lost event) is filled the same way
    -- the first time it is read.
    allow_message_use  boolean,
    -- The owner chose `allow_message_use` themselves.
    message_use_chosen boolean     NOT NULL DEFAULT false,
    updated_at         timestamptz NOT NULL DEFAULT now()
);

-- Accounts that existed before get the default for their age at this
-- migration, once. Unknown age: off.
INSERT INTO chat_ai_user_settings (user_id, allow_message_use)
SELECT id, birthdate IS NOT NULL AND birthdate <= (now() - interval '18 years')::date FROM users
ON CONFLICT (user_id) DO UPDATE
    SET allow_message_use = COALESCE(chat_ai_user_settings.allow_message_use, EXCLUDED.allow_message_use);

-- API keys users brought for their own requests (Anthropic, OpenAI).
CREATE TABLE chat_ai_user_keys (
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    provider   smallint    NOT NULL CHECK (provider IN (1, 2)),
    -- Sealed (core keys, label "chat/ai-key/user"); never returned.
    key_sealed bytea       NOT NULL,
    last4      text        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, provider)
);
