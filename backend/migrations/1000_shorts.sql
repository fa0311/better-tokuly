-- Shorts: vertical videos with their sounds and hashtags, what viewers did
-- with them (saves, likes, "not interested") and the hourly signals the
-- for-you ranking is computed from.

-- ───────────────────────────── Sounds ─────────────────────────────

-- What a short is set to: the audio of a short ("original sound"), which
-- other shorts may reuse, or a track from outside named as text. A sound is
-- attribution, not audio: the client mixed the audio into its upload.
CREATE TABLE short_sounds (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    -- tokuly.shorts.v1.SoundKind: 1 original, 2 external.
    kind            integer     NOT NULL CHECK (kind BETWEEN 1 AND 2),
    title           text        NOT NULL DEFAULT '',
    artist          text        NOT NULL DEFAULT '',
    -- External tracks: normalized "title\nartist", so shorts naming the
    -- same track share one sound. NULL for original sounds.
    track_key       text UNIQUE,
    -- Original sounds: the short the audio comes from; NULL once that short
    -- is gone (the foreign key is added below, once shorts exist).
    origin_short_id uuid UNIQUE,
    -- Public shorts using the sound.
    use_count       bigint      NOT NULL DEFAULT 0,
    created_at      timestamptz NOT NULL DEFAULT now(),
    CHECK ((kind = 2) = (track_key IS NOT NULL))
);

CREATE INDEX short_sounds_popular ON short_sounds (use_count DESC, id DESC) WHERE use_count > 0;

-- ───────────────────────────── Shorts ─────────────────────────────

CREATE TABLE shorts (
    id                     uuid PRIMARY KEY DEFAULT uuidv7(),
    short_id               text             NOT NULL UNIQUE,
    -- A channel of the video domain. Deliberately not a foreign key: the
    -- video domain deletes channels on its own, and the shorts on them are
    -- cleaned up (uploads, hashtag counts, search) on `video.channel.deleted`.
    channel_id             uuid             NOT NULL,
    -- The channel's owner, copied here because every visibility rule is about them.
    owner_id               uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The media asset being played. One short per asset.
    asset_id               uuid             NOT NULL UNIQUE,
    -- The asset is the short's own upload and is deleted with it; false for
    -- the media of a clip, which the short only holds.
    owns_asset             boolean          NOT NULL DEFAULT true,
    caption                text             NOT NULL DEFAULT '',
    -- Normalized hashtags of the caption, in order of appearance.
    hashtags               text[]           NOT NULL DEFAULT '{}',
    -- [{ "user_id", "handle" }]: the caption's mentions that name an account.
    mentions               jsonb            NOT NULL DEFAULT '[]',
    -- Accounts that have been told they are mentioned in this short. Only
    -- ever grows: taking a mention out and putting it back tells nobody twice.
    mentions_told          uuid[]           NOT NULL DEFAULT '{}',
    -- tokuly.common.v1.Image as proto JSON: what `cover_kind` says.
    cover                  jsonb,
    -- tokuly.shorts.v1.CoverKind: 1 poster frame, 2 frame at an offset, 3 uploaded image.
    cover_kind             integer          NOT NULL DEFAULT 1 CHECK (cover_kind BETWEEN 1 AND 3),
    -- The image asset behind a FRAME cover (made by this domain, deleted
    -- with it) or an IMAGE cover (the creator's upload).
    cover_asset_id         uuid,
    -- FRAME: the offset asked for.
    cover_offset_ms        integer,
    -- FRAME: the extracted frame while the media domain processes it.
    cover_pending_asset_id uuid,
    visibility             integer          NOT NULL DEFAULT 4 CHECK (visibility BETWEEN 1 AND 5),
    comments_enabled       boolean          NOT NULL DEFAULT true,
    allow_remix            boolean          NOT NULL DEFAULT true,
    allow_duet             boolean          NOT NULL DEFAULT true,
    notify_subscribers     boolean          NOT NULL DEFAULT true,
    sound_id               uuid             REFERENCES short_sounds (id) ON DELETE SET NULL,
    -- The short this one is a remix or duet of (tokuly.shorts.v1.RemixKind).
    remix_of_id            uuid             REFERENCES shorts (id) ON DELETE SET NULL,
    remix_kind             integer          NOT NULL DEFAULT 0 CHECK (remix_kind BETWEEN 0 AND 2),
    -- What the short was made from (tokuly.common.v1.SubjectRef), 0 = nothing.
    source_type            integer          NOT NULL DEFAULT 0,
    source_id              uuid,
    -- tokuly.shorts.v1.ShortStatus: 1 processing (draft), 2 published, 3 failed,
    -- 4 withdrawn (deleted by its owner after it was reported: gone for
    -- everyone but moderators until `withdrawn_at` + the retention period).
    status                 integer          NOT NULL DEFAULT 1 CHECK (status BETWEEN 1 AND 4),
    -- When it was last reported (`engagement.report.created`).
    reported_at            timestamptz,
    withdrawn_at           timestamptz,
    processing_error       text             NOT NULL DEFAULT '',
    -- tokuly.media.v1.VideoInfo as proto JSON, once processed.
    playback               jsonb,
    duration_seconds       double precision NOT NULL DEFAULT 0,
    width                  integer          NOT NULL DEFAULT 0,
    height                 integer          NOT NULL DEFAULT 0,
    view_count             bigint           NOT NULL DEFAULT 0,
    like_count             bigint           NOT NULL DEFAULT 0,
    comment_count          bigint           NOT NULL DEFAULT 0,
    share_count            bigint           NOT NULL DEFAULT 0,
    save_count             bigint           NOT NULL DEFAULT 0,
    -- Replays past the end.
    loop_count             bigint           NOT NULL DEFAULT 0,
    -- Public remixes and duets made of it.
    remix_count            bigint           NOT NULL DEFAULT 0,
    -- Sum and number of the watch-through ratios (0-1) reported for it.
    watch_ratio_sum        double precision NOT NULL DEFAULT 0,
    watch_samples          bigint           NOT NULL DEFAULT 0,
    -- The for-you base score and the decayed engagement it is built on,
    -- both written by the ranking worker.
    score                  double precision NOT NULL DEFAULT 0,
    trending               double precision NOT NULL DEFAULT 0,
    scored_at              timestamptz,
    -- Highest view milestone the owner has been told about.
    milestone              bigint           NOT NULL DEFAULT 0,
    -- Pinned to the channel grid (the newest pin first); NULL = not pinned.
    pinned_at              timestamptz,
    -- The owner's account is suspended (mirrors identity.user.banned /
    -- unbanned): hidden from everyone else.
    suspended              boolean          NOT NULL DEFAULT false,
    -- The short is public right now and therefore counted in its hashtags'
    -- and its sound's `use_count` and in the `remix_count` of its original.
    listed                 boolean          NOT NULL DEFAULT false,
    -- When friends were told and the subscriber notifications were set off;
    -- set once.
    announced_at           timestamptz,
    -- How far the subscriber notifications got: the last subscriber told.
    -- NULL = not decided yet, the nil UUID = started, the all-ones UUID =
    -- finished, or never started because the channel's daily allowance was
    -- used up. `subscribers_told` says which: it is what the allowance counts.
    announce_cursor        uuid,
    subscribers_told       boolean          NOT NULL DEFAULT false,
    published_at           timestamptz,
    attrs                  jsonb            NOT NULL DEFAULT '{}',
    created_at             timestamptz      NOT NULL DEFAULT now(),
    updated_at             timestamptz      NOT NULL DEFAULT now()
);

ALTER TABLE short_sounds
    ADD CONSTRAINT short_sounds_origin FOREIGN KEY (origin_short_id) REFERENCES shorts (id) ON DELETE SET NULL;

-- The studio list and the friends feed.
CREATE INDEX shorts_owner ON shorts (owner_id, published_at DESC, id DESC);
CREATE INDEX shorts_owner_all ON shorts (owner_id, id DESC);
-- The channel grid and the following feed.
CREATE INDEX shorts_channel ON shorts (channel_id, published_at DESC, id DESC) WHERE status = 2;
CREATE INDEX shorts_channel_popular ON shorts (channel_id, view_count DESC, id DESC) WHERE status = 2;
CREATE INDEX shorts_channel_pins ON shorts (channel_id, pinned_at DESC) WHERE pinned_at IS NOT NULL;
-- Candidates of the for-you feed.
CREATE INDEX shorts_public_recent ON shorts (published_at DESC, id DESC) WHERE listed;
CREATE INDEX shorts_public_top ON shorts (score DESC, id DESC) WHERE listed;
CREATE INDEX shorts_public_hashtags ON shorts USING gin (hashtags) WHERE listed;
-- Sound pages and remix lists.
CREATE INDEX shorts_sound ON shorts (sound_id, published_at DESC, id DESC) WHERE sound_id IS NOT NULL;
CREATE INDEX shorts_remixes ON shorts (remix_of_id, published_at DESC, id DESC) WHERE remix_of_id IS NOT NULL;
-- Lookups by the assets a short holds besides its video.
CREATE INDEX shorts_cover_asset ON shorts (cover_asset_id) WHERE cover_asset_id IS NOT NULL;
CREATE INDEX shorts_cover_pending ON shorts (cover_pending_asset_id) WHERE cover_pending_asset_id IS NOT NULL;
-- Drafts whose processing result may have been missed.
CREATE INDEX shorts_processing ON shorts (created_at) WHERE status = 1;
-- "Who is mentioned" lookups (account deletion).
CREATE INDEX shorts_mentions ON shorts USING gin (mentions jsonb_path_ops);
-- Withdrawn shorts waiting for the end of their retention period.
CREATE INDEX shorts_withdrawn ON shorts (withdrawn_at) WHERE status = 4;
-- A channel's recent subscriber announcements (the daily allowance).
CREATE INDEX shorts_announced ON shorts (channel_id, announced_at) WHERE subscribers_told;
-- The ranking worker's working set, and the shorts behind trending hashtags.
CREATE INDEX shorts_scoring ON shorts (published_at) WHERE status = 2;
CREATE INDEX shorts_active ON shorts (id) WHERE listed AND trending > 0;

-- `updated_at` means "the creator changed it": counters and scores do not touch it.
CREATE TRIGGER shorts_updated_at
    BEFORE UPDATE OF caption, cover, cover_kind, visibility, comments_enabled, allow_remix, allow_duet, pinned_at ON shorts
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- ───────────────────────────── Hashtags ─────────────────────────────

CREATE TABLE short_hashtags (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    -- Normalized: lowercase, compatibility forms folded, without '#'.
    tag           text             NOT NULL UNIQUE,
    -- Public shorts carrying it.
    use_count     bigint           NOT NULL DEFAULT 0,
    -- Views of those shorts, as of the last ranking run.
    view_count    bigint           NOT NULL DEFAULT 0,
    -- Decayed engagement of those shorts, and the 1-based position among
    -- the trending hashtags (0 = not trending); written by the ranking worker.
    trend_score   double precision NOT NULL DEFAULT 0,
    trend_rank    integer          NOT NULL DEFAULT 0,
    -- `use_count` as of the last search document written (-1 = never).
    indexed_count bigint           NOT NULL DEFAULT -1,
    created_at    timestamptz      NOT NULL DEFAULT now()
);

CREATE INDEX short_hashtags_prefix ON short_hashtags (tag text_pattern_ops, use_count DESC) WHERE use_count > 0;
CREATE INDEX short_hashtags_trending ON short_hashtags (trend_rank) WHERE trend_rank > 0;
CREATE INDEX short_hashtags_stale ON short_hashtags (id) WHERE indexed_count <> use_count;

-- One row per public short carrying a hashtag: the hashtag page.
CREATE TABLE short_hashtag_uses (
    hashtag_id   uuid        NOT NULL REFERENCES short_hashtags (id) ON DELETE CASCADE,
    short_id     uuid        NOT NULL REFERENCES shorts (id) ON DELETE CASCADE,
    published_at timestamptz NOT NULL,
    PRIMARY KEY (hashtag_id, short_id)
);

CREATE INDEX short_hashtag_uses_recent ON short_hashtag_uses (hashtag_id, published_at DESC, short_id DESC);
CREATE INDEX short_hashtag_uses_short ON short_hashtag_uses (short_id);

-- ───────────────────────────── Viewers ─────────────────────────────

-- The private saved list.
CREATE TABLE short_saves (
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    short_id   uuid        NOT NULL REFERENCES shorts (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, short_id)
);

CREATE INDEX short_saves_recent ON short_saves (user_id, created_at DESC, short_id DESC);
CREATE INDEX short_saves_short ON short_saves (short_id);

-- Who liked what: a copy of the engagement domain's likes of shorts, kept
-- from `engagement.reaction.changed`, for the viewer state, the "liked"
-- list and the hashtag affinity of the for-you feed.
--
-- Events can arrive twice or out of order, so a row records the latest
-- change it has seen (`changed_at`, the event's time) and a like taken back
-- stays as a row with `liked = false` for a while: an older "liked" event
-- arriving late then changes nothing.
CREATE TABLE short_likes (
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    short_id   uuid        NOT NULL REFERENCES shorts (id) ON DELETE CASCADE,
    liked      boolean     NOT NULL DEFAULT true,
    -- When the like was given.
    created_at timestamptz NOT NULL DEFAULT now(),
    changed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, short_id)
);

CREATE INDEX short_likes_recent ON short_likes (user_id, created_at DESC, short_id DESC) WHERE liked;
CREATE INDEX short_likes_short ON short_likes (short_id);
CREATE INDEX short_likes_taken_back ON short_likes (changed_at) WHERE NOT liked;

-- Who follows which channel: a copy of the video domain's subscriptions,
-- kept from `video.channel.subscribed` / `unsubscribed` (the video domain
-- offers no lookup of a user's subscriptions), for the following feed, the
-- for-you candidates and the viewer state.
CREATE TABLE short_follows (
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    channel_id uuid        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, channel_id)
);

CREATE INDEX short_follows_channel ON short_follows (channel_id);

-- "Not interested": the short is hidden from the user's feeds, and its
-- channel and hashtags (as they were when it was marked) rank lower in
-- their for-you feed. Not a foreign key on the short: the down-weighting
-- outlives a short that is deleted; old marks are pruned by the worker.
CREATE TABLE short_hidden (
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    short_id   uuid        NOT NULL,
    channel_id uuid        NOT NULL,
    hashtags   text[]      NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, short_id)
);

CREATE INDEX short_hidden_recent ON short_hidden (user_id, created_at DESC);
CREATE INDEX short_hidden_age ON short_hidden (created_at);
CREATE INDEX short_hidden_channel ON short_hidden (channel_id);

-- ───────────────────────────── Ranking signals ─────────────────────────────

-- What happened to a short during one hour. Plays arrive here in batches
-- from the Valkey buffer (never one write per impression); likes, comments,
-- shares and saves as they happen. Every signal counts once per account,
-- short and period, so the numbers only ever grow. The ranking worker sums
-- the last week with a time decay and prunes older rows.
CREATE TABLE short_hourly_stats (
    short_id uuid        NOT NULL REFERENCES shorts (id) ON DELETE CASCADE,
    hour     timestamptz NOT NULL,
    views    bigint      NOT NULL DEFAULT 0,
    loops    bigint      NOT NULL DEFAULT 0,
    likes    bigint      NOT NULL DEFAULT 0,
    comments bigint      NOT NULL DEFAULT 0,
    shares   bigint      NOT NULL DEFAULT 0,
    saves    bigint      NOT NULL DEFAULT 0,
    PRIMARY KEY (short_id, hour)
);

CREATE INDEX short_hourly_stats_hour ON short_hourly_stats (hour);
