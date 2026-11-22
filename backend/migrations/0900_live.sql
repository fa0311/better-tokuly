-- Live: streams with their ingest sessions and viewer samples, live chat and
-- its moderation, gifts, polls, clips and OBS overlays.
--
-- Channels belong to the video domain. Rows here name a channel without a
-- foreign key: the live domain cleans up after `video.channel.deleted`
-- itself (recordings and uploads need more than a cascade).

-- ───────────────────────────── Streams ─────────────────────────────

-- One broadcast: UPCOMING (1) → LIVE (2) → ENDED (3).
CREATE TABLE live_streams (
    id                 uuid PRIMARY KEY DEFAULT uuidv7(),
    -- Public ID, and the stream's name on the media server (`live/<short_id>`).
    short_id           text        NOT NULL UNIQUE,
    channel_id         uuid        NOT NULL,
    -- The channel's owner.
    owner_id           uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title              text        NOT NULL,
    description        text        NOT NULL DEFAULT '',
    tags               text[]      NOT NULL DEFAULT '{}',
    -- tokuly.video.v1.VideoCategory.
    category           integer     NOT NULL DEFAULT 0,
    game               text        NOT NULL DEFAULT '',
    -- tokuly.common.v1.Image as proto JSON, with the upload it came from.
    thumbnail          jsonb,
    thumbnail_asset_id uuid,
    -- tokuly.common.v1.Visibility.
    visibility         integer     NOT NULL DEFAULT 1 CHECK (visibility BETWEEN 1 AND 5),
    -- tokuly.live.v1.StreamStatus.
    status             integer     NOT NULL DEFAULT 1 CHECK (status BETWEEN 1 AND 3),
    scheduled_at       timestamptz,
    low_latency        boolean     NOT NULL DEFAULT true,
    dvr_enabled        boolean     NOT NULL DEFAULT true,
    auto_archive       boolean     NOT NULL DEFAULT true,
    -- Go on air as soon as the encoder connects.
    auto_start         boolean     NOT NULL DEFAULT true,
    gifts_enabled      boolean     NOT NULL DEFAULT true,
    -- tokuly.live.v1.ChatSettings as proto JSON.
    chat               jsonb       NOT NULL,

    -- The stream key: encrypted (shown to the owner again) and hashed (what
    -- the media server's question is compared with).
    stream_key_sealed  bytea       NOT NULL,
    stream_key_hash    text        NOT NULL,
    key_rotated_at     timestamptz NOT NULL DEFAULT now(),

    -- What the media server last said about the encoder.
    encoder_connected  boolean     NOT NULL DEFAULT false,
    -- "rtmp", "srt" or "webrtc" while connected.
    encoder_protocol   text        NOT NULL DEFAULT '',
    -- When an encoder that was on air went away; the stream ends when it
    -- stays away longer than the reconnect grace.
    encoder_lost_at    timestamptz,

    started_at         timestamptz,
    ended_at           timestamptz,
    -- tokuly.live.v1.EndReason.
    end_reason         integer     NOT NULL DEFAULT 0,

    -- Viewers at the last sample, and the most there ever were.
    viewer_count       bigint      NOT NULL DEFAULT 0,
    -- Signed-in viewers at the last sample: what "live now" is ranked by.
    -- Guests are not counted here, since anyone can pretend to be many.
    ranked_viewer_count bigint     NOT NULL DEFAULT 0,
    peak_viewer_count  bigint      NOT NULL DEFAULT 0,
    like_count         bigint      NOT NULL DEFAULT 0,
    chat_message_count bigint      NOT NULL DEFAULT 0,
    -- Gifts received, before the platform fee and after it.
    gift_coins         bigint      NOT NULL DEFAULT 0,
    gift_net_coins     bigint      NOT NULL DEFAULT 0,
    gift_count         bigint      NOT NULL DEFAULT 0,
    -- Subscriptions the channel gained while this stream was on air.
    new_subscribers    bigint      NOT NULL DEFAULT 0,

    pinned_message_id  uuid,

    -- tokuly.live.v1.ArchiveStatus, the recording's upload and the video.
    archive_status     integer     NOT NULL DEFAULT 1,
    archive_asset_id   uuid,
    archive_video_id   uuid,
    archive_error      text        NOT NULL DEFAULT '',
    -- When the archive entered its current state (stuck jobs are requeued).
    archive_changed_at timestamptz,

    raid_target_id     uuid,
    -- The `live.started` announcement was queued (once per stream).
    announced_at       timestamptz,
    -- The owner's account is suspended: hidden from everyone else.
    suspended          boolean     NOT NULL DEFAULT false,
    -- Deleted (by its owner, with its channel, or by moderation): gone for
    -- everyone, but the row and its chat are kept for a retention period so
    -- that what was said can still be reported and judged. Purged after that.
    deleted_at         timestamptz,
    attrs              jsonb       NOT NULL DEFAULT '{}',
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX live_streams_channel ON live_streams (channel_id, id DESC);
CREATE INDEX live_streams_owner ON live_streams (owner_id, id DESC);
-- "Live now", most viewers first.
CREATE INDEX live_streams_on_air ON live_streams (ranked_viewer_count DESC, id DESC) WHERE status = 2;
-- Upcoming streams by planned start.
CREATE INDEX live_streams_scheduled ON live_streams (scheduled_at, id) WHERE status = 1 AND scheduled_at IS NOT NULL;
-- What the reconciler looks at: streams with an encoder, or on air.
CREATE INDEX live_streams_active ON live_streams (id) WHERE status = 2 OR encoder_connected;
CREATE INDEX live_streams_archive ON live_streams (archive_changed_at) WHERE archive_status IN (2, 3);
CREATE INDEX live_streams_archive_video ON live_streams (archive_video_id) WHERE archive_video_id IS NOT NULL;
CREATE INDEX live_streams_archive_asset ON live_streams (archive_asset_id) WHERE archive_asset_id IS NOT NULL;
CREATE INDEX live_streams_thumbnail_asset ON live_streams (thumbnail_asset_id) WHERE thumbnail_asset_id IS NOT NULL;
CREATE INDEX live_streams_deleted ON live_streams (deleted_at) WHERE deleted_at IS NOT NULL;

-- `updated_at` means "the page changed": counters and encoder state do not touch it.
CREATE TRIGGER live_streams_updated_at
    BEFORE UPDATE OF title, description, tags, category, game, thumbnail, visibility, status, scheduled_at,
        low_latency, dvr_enabled, auto_archive, auto_start, gifts_enabled, chat ON live_streams
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- One uninterrupted connection of the encoder. The media timeline of a
-- stream (chat offsets, clip positions, the archive) is the time covered
-- by its sessions from `started_at` on.
CREATE TABLE live_ingest_sessions (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    stream_id  uuid        NOT NULL REFERENCES live_streams (id) ON DELETE CASCADE,
    protocol   text        NOT NULL DEFAULT '',
    started_at timestamptz NOT NULL DEFAULT now(),
    ended_at   timestamptz
);

CREATE INDEX live_ingest_sessions_stream ON live_ingest_sessions (stream_id, started_at);
CREATE UNIQUE INDEX live_ingest_sessions_open ON live_ingest_sessions (stream_id) WHERE ended_at IS NULL;

-- Viewers once a minute while on air: the timeline, and the average.
CREATE TABLE live_viewer_samples (
    stream_id uuid        NOT NULL REFERENCES live_streams (id) ON DELETE CASCADE,
    at        timestamptz NOT NULL,
    viewers   bigint      NOT NULL,
    PRIMARY KEY (stream_id, at)
);

-- Who subscribed to the channel while this stream was on air: counted (and
-- announced by the alert boxes) once per person, however often they
-- subscribe again or the event is delivered.
CREATE TABLE live_stream_subscribers (
    stream_id  uuid        NOT NULL REFERENCES live_streams (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (stream_id, user_id)
);

CREATE INDEX live_stream_subscribers_user ON live_stream_subscribers (user_id);

-- "Tell me when it starts."
CREATE TABLE live_reminders (
    stream_id  uuid        NOT NULL REFERENCES live_streams (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (stream_id, user_id)
);

CREATE INDEX live_reminders_user ON live_reminders (user_id);

-- ───────────────────────────── Chat ─────────────────────────────

CREATE TABLE live_chat_messages (
    -- UUIDv7: the order of the chat.
    id                uuid PRIMARY KEY DEFAULT uuidv7(),
    stream_id         uuid        NOT NULL REFERENCES live_streams (id) ON DELETE CASCADE,
    -- NULL for notices written by the server.
    user_id           uuid REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.live.v1.ChatMessageKind: 1 text, 2 stamp, 3 gift, 4 notice.
    kind              integer     NOT NULL CHECK (kind BETWEEN 1 AND 4),
    -- tokuly.live.v1.ChatRole of the author when it was written.
    role              integer     NOT NULL DEFAULT 1,
    text              text        NOT NULL DEFAULT '',
    stamp_id          uuid,
    gift_id           uuid,
    gift_coins        bigint      NOT NULL DEFAULT 0,
    gift_tier         integer     NOT NULL DEFAULT 0,
    -- Gift messages stay highlighted above the chat until then.
    highlighted_until timestamptz,
    -- tokuly.live.v1.ChatNotice as proto JSON, for notices.
    notice            jsonb,
    -- Position on the stream's media timeline.
    offset_ms         bigint      NOT NULL DEFAULT 0,
    created_at        timestamptz NOT NULL DEFAULT now(),
    deleted_at        timestamptz
);

CREATE INDEX live_chat_messages_stream ON live_chat_messages (stream_id, id DESC) WHERE deleted_at IS NULL;
CREATE INDEX live_chat_messages_replay ON live_chat_messages (stream_id, offset_ms, id) WHERE deleted_at IS NULL;
CREATE INDEX live_chat_messages_user ON live_chat_messages (user_id, id);
CREATE INDEX live_chat_messages_highlighted ON live_chat_messages (stream_id, highlighted_until)
    WHERE highlighted_until IS NOT NULL AND deleted_at IS NULL;

-- Per-channel chat rules that outlive a stream.
CREATE TABLE live_channel_settings (
    channel_id   uuid PRIMARY KEY,
    owner_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Lower-cased; a message containing one is refused.
    banned_words text[]      NOT NULL DEFAULT '{}',
    updated_at   timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE live_moderators (
    channel_id   uuid        NOT NULL,
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, user_id)
);

CREATE INDEX live_moderators_user ON live_moderators (user_id);

-- Timeouts (with `expires_at`) and bans (without) from a channel's chats.
CREATE TABLE live_chat_restrictions (
    channel_id   uuid        NOT NULL,
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    expires_at   timestamptz,
    reason       text        NOT NULL DEFAULT '',
    -- Who imposed it; NULL once that account is gone.
    moderator_id uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, user_id)
);

CREATE INDEX live_chat_restrictions_user ON live_chat_restrictions (user_id);
CREATE INDEX live_chat_restrictions_list ON live_chat_restrictions (channel_id, created_at DESC, user_id);

-- ───────────────────────────── Gifts ─────────────────────────────

-- A gift is written down before its coins move (PENDING) and becomes real
-- (SETTLED) once the wallet has moved them: a gift whose payment was
-- interrupted is picked up again from this row, so coins never move without
-- a gift or twice for one. What the gift contains (tier, message, how long
-- it is highlighted) is decided when it is written down and kept here.
--
-- A gift is bookkeeping: it stays when its stream is deleted, and when its
-- sender or recipient leaves (without their name).
CREATE TABLE live_gifts (
    id                uuid PRIMARY KEY DEFAULT uuidv7(),
    stream_id         uuid REFERENCES live_streams (id) ON DELETE SET NULL,
    channel_id        uuid        NOT NULL,
    recipient_id      uuid REFERENCES users (id) ON DELETE SET NULL,
    sender_id         uuid REFERENCES users (id) ON DELETE SET NULL,
    -- 1 pending (the payment is not confirmed), 2 settled.
    status            integer     NOT NULL DEFAULT 1 CHECK (status IN (1, 2)),
    coins             bigint      NOT NULL CHECK (coins > 0),
    tier              integer     NOT NULL,
    message           text        NOT NULL DEFAULT '',
    highlight_seconds integer     NOT NULL DEFAULT 0,
    -- The sender's idempotency key.
    client_key        text        NOT NULL,
    -- Set when settled: the ledger entry, and the platform fee it withheld
    -- (the recipient earned `coins - fee`).
    ledger_entry_id   uuid,
    fee               bigint      NOT NULL DEFAULT 0,
    offset_ms         bigint      NOT NULL DEFAULT 0,
    created_at        timestamptz NOT NULL DEFAULT now(),
    settled_at        timestamptz
);

CREATE UNIQUE INDEX live_gifts_idempotency ON live_gifts (sender_id, client_key);
CREATE UNIQUE INDEX live_gifts_entry ON live_gifts (ledger_entry_id) WHERE ledger_entry_id IS NOT NULL;
CREATE INDEX live_gifts_sender ON live_gifts (sender_id, id DESC);
CREATE INDEX live_gifts_channel ON live_gifts (channel_id, id DESC);
CREATE INDEX live_gifts_stream ON live_gifts (stream_id, id DESC);
CREATE INDEX live_gifts_pending ON live_gifts (created_at) WHERE status = 1;

-- ───────────────────────────── Polls ─────────────────────────────

CREATE TABLE live_polls (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    stream_id  uuid        NOT NULL REFERENCES live_streams (id) ON DELETE CASCADE,
    created_by uuid REFERENCES users (id) ON DELETE SET NULL,
    question   text        NOT NULL,
    options    text[]      NOT NULL CHECK (cardinality(options) BETWEEN 2 AND 5),
    -- Votes per option, in the order of `options`.
    votes      bigint[]    NOT NULL,
    started_at timestamptz NOT NULL DEFAULT now(),
    ends_at    timestamptz NOT NULL,
    ended_at   timestamptz
);

CREATE INDEX live_polls_stream ON live_polls (stream_id, id DESC);
-- One open poll per stream.
CREATE UNIQUE INDEX live_polls_open ON live_polls (stream_id) WHERE ended_at IS NULL;
CREATE INDEX live_polls_due ON live_polls (ends_at) WHERE ended_at IS NULL;

CREATE TABLE live_poll_votes (
    poll_id    uuid        NOT NULL REFERENCES live_polls (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- 1-based.
    option     integer     NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (poll_id, user_id)
);

CREATE INDEX live_poll_votes_user ON live_poll_votes (user_id);

-- ───────────────────────────── Clips ─────────────────────────────

-- A short cut of a stream or a video. Its visibility is its source's.
CREATE TABLE live_clips (
    id               uuid PRIMARY KEY DEFAULT uuidv7(),
    short_id         text        NOT NULL UNIQUE,
    creator_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The channel it was cut from, and that channel's owner.
    channel_id       uuid        NOT NULL,
    owner_id         uuid        NOT NULL,
    -- The source: a stream, a video, or both once a stream has its archive.
    stream_id        uuid REFERENCES live_streams (id) ON DELETE SET NULL,
    video_id         uuid,
    title            text        NOT NULL,
    -- Start in the source (media timeline of a stream, or seconds into a
    -- video) and length.
    start_seconds    double precision NOT NULL,
    duration_seconds double precision NOT NULL CHECK (duration_seconds > 0),
    -- tokuly.live.v1.ClipStatus: 1 processing, 2 ready, 3 failed.
    status           integer     NOT NULL DEFAULT 1,
    error            text        NOT NULL DEFAULT '',
    -- The clip's upload, once cut; what it plays, once processed.
    asset_id         uuid,
    -- tokuly.media.v1.VideoInfo / tokuly.common.v1.Image as proto JSON.
    playback         jsonb,
    thumbnail        jsonb,
    like_count       bigint      NOT NULL DEFAULT 0,
    attrs            jsonb       NOT NULL DEFAULT '{}',
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX live_clips_creator ON live_clips (creator_id, id DESC);
CREATE INDEX live_clips_channel ON live_clips (channel_id, id DESC);
CREATE INDEX live_clips_stream ON live_clips (stream_id, id DESC) WHERE stream_id IS NOT NULL;
CREATE INDEX live_clips_video ON live_clips (video_id, id DESC) WHERE video_id IS NOT NULL;
CREATE UNIQUE INDEX live_clips_asset ON live_clips (asset_id) WHERE asset_id IS NOT NULL;
CREATE INDEX live_clips_processing ON live_clips (created_at) WHERE status = 1;

CREATE TRIGGER live_clips_updated_at
    BEFORE UPDATE OF title, status, playback ON live_clips
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- ───────────────────────────── Overlays ─────────────────────────────

-- A widget for OBS, opened with its token instead of a sign-in.
CREATE TABLE live_overlays (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    channel_id   uuid        NOT NULL,
    owner_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.live.v1.OverlayKind: 1 chat box, 2 alert box, 3 goal, 4 poll.
    kind         integer     NOT NULL CHECK (kind BETWEEN 1 AND 4),
    name         text        NOT NULL,
    preset       text        NOT NULL,
    custom_css   text        NOT NULL DEFAULT '',
    -- tokuly.live.v1.OverlaySettings as proto JSON.
    settings     jsonb       NOT NULL,
    -- Subscriber goals: the channel's subscriber count when counting began.
    goal_baseline bigint     NOT NULL DEFAULT 0,
    -- The token: hashed for lookups, encrypted to show its URL again.
    token_hash   text        NOT NULL UNIQUE,
    token_sealed bytea       NOT NULL,
    -- Name of the widget's realtime topic (`overlay:<topic_key>`): derived
    -- from the token with a key, so the token itself never becomes a topic
    -- name (those travel through the bus and into logs).
    topic_key    text        NOT NULL UNIQUE,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX live_overlays_channel ON live_overlays (channel_id, id);

CREATE TRIGGER live_overlays_updated_at
    BEFORE UPDATE ON live_overlays
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();
