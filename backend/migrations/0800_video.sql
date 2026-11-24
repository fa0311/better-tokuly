-- Video: channels and subscriptions, videos with subtitles and ratings,
-- playback history, playlists, feed preferences and watch parties.

-- ───────────────────────────── Channels ─────────────────────────────

-- A creator identity. An account owns a handful of them (the limit is
-- configuration, enforced in code) and publishes as one.
CREATE TABLE channels (
    id               uuid PRIMARY KEY DEFAULT uuidv7(),
    owner_id         uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- A namespace of its own: a channel and a user may share a handle.
    handle           citext      NOT NULL UNIQUE,
    name             text        NOT NULL,
    description      text        NOT NULL DEFAULT '',
    -- tokuly.common.v1.Image as proto JSON, with the upload it came from.
    icon             jsonb,
    icon_asset_id    uuid,
    banner           jsonb,
    banner_asset_id  uuid,
    -- [tokuly.video.v1.ChannelLink] as proto JSON.
    links            jsonb       NOT NULL DEFAULT '[]',
    verified         boolean     NOT NULL DEFAULT false,
    -- The owner's posting identity; exactly one per owner who has channels.
    is_default       boolean     NOT NULL DEFAULT false,
    -- Featured video; the foreign key is added below, once videos exist.
    trailer_video_id uuid,
    subscriber_count bigint      NOT NULL DEFAULT 0,
    -- Views of all the channel's videos.
    view_count       bigint      NOT NULL DEFAULT 0,
    -- The owner's account is suspended (mirrors identity.user.banned /
    -- unbanned): the channel and everything on it is hidden from everyone else.
    suspended        boolean     NOT NULL DEFAULT false,
    attrs            jsonb       NOT NULL DEFAULT '{}',
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX channels_owner ON channels (owner_id, id);
CREATE UNIQUE INDEX channels_one_default ON channels (owner_id) WHERE is_default;
CREATE INDEX channels_popular ON channels (subscriber_count DESC, id DESC) WHERE NOT suspended;
CREATE INDEX channels_icon_asset ON channels (icon_asset_id) WHERE icon_asset_id IS NOT NULL;
CREATE INDEX channels_banner_asset ON channels (banner_asset_id) WHERE banner_asset_id IS NOT NULL;

-- `updated_at` means "the page changed": counters do not touch it.
CREATE TRIGGER channels_updated_at
    BEFORE UPDATE OF handle, name, description, icon, banner, links, trailer_video_id ON channels
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE TABLE channel_subscriptions (
    channel_id   uuid        NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.video.v1.NotificationLevel: 1 none, 2 personalized, 3 all.
    notify_level integer     NOT NULL DEFAULT 2 CHECK (notify_level BETWEEN 1 AND 3),
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, user_id)
);

CREATE INDEX channel_subscriptions_user ON channel_subscriptions (user_id, created_at DESC, channel_id);
-- The owner's subscriber list, newest first.
CREATE INDEX channel_subscriptions_recent ON channel_subscriptions (channel_id, created_at DESC, user_id);

-- ───────────────────────────── Videos ─────────────────────────────

CREATE TABLE videos (
    id                  uuid PRIMARY KEY DEFAULT uuidv7(),
    short_id            text             NOT NULL UNIQUE,
    channel_id          uuid             NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    -- The channel's owner, copied here because every visibility rule is about them.
    owner_id            uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The media upload being played. One video per upload.
    asset_id            uuid             NOT NULL UNIQUE,
    -- tokuly.video.v1.VideoOrigin, and for live archives the stream's ID.
    origin              integer          NOT NULL DEFAULT 1 CHECK (origin BETWEEN 1 AND 2),
    origin_id           uuid,

    title               text             NOT NULL,
    description         text             NOT NULL DEFAULT '',
    -- Lowercase, without '#'.
    tags                text[]           NOT NULL DEFAULT '{}',
    -- tokuly.video.v1.VideoCategory (0 = none).
    category            integer          NOT NULL DEFAULT 0,
    game                text             NOT NULL DEFAULT '',
    -- [tokuly.video.v1.Chapter] as proto JSON, sorted by start.
    chapters            jsonb            NOT NULL DEFAULT '[]',
    -- The picture on cards (tokuly.common.v1.Image as proto JSON): the custom
    -- thumbnail when `thumbnail_asset_id` is set, else the poster frame.
    thumbnail           jsonb,
    thumbnail_asset_id  uuid,

    -- tokuly.common.v1.Visibility: 1 public, 2 no feed, 3 unlisted, 4 friends,
    -- 5 private, 6 followers, 7 close friends, 8 circle (`circle_id`).
    visibility          integer          NOT NULL DEFAULT 5 CHECK (visibility BETWEEN 1 AND 8),
    -- VISIBILITY_CIRCLE only: one of the owner's circles (tokuly.social.v1.Circle).
    circle_id           uuid,
    -- FRIENDS visibility only: anyone with the link may watch as well.
    allow_link          boolean          NOT NULL DEFAULT false,
    age_restricted      boolean          NOT NULL DEFAULT false,
    comments_enabled    boolean          NOT NULL DEFAULT true,
    notify_subscribers  boolean          NOT NULL DEFAULT true,

    -- tokuly.video.v1.VideoStatus: 1 draft, 2 scheduled, 3 published.
    status              integer          NOT NULL DEFAULT 1 CHECK (status BETWEEN 1 AND 3),
    -- tokuly.video.v1.ProcessingState: 1 processing, 2 ready, 3 failed.
    processing          integer          NOT NULL DEFAULT 1 CHECK (processing BETWEEN 1 AND 3),
    processing_error    text             NOT NULL DEFAULT '',
    scheduled_at        timestamptz,
    -- When it first went live; kept when it is unpublished again.
    published_at        timestamptz,
    -- When subscribers and the activity feed were told; at most once per video.
    announced_at        timestamptz,

    -- From the processed upload: tokuly.media.v1.VideoInfo as proto JSON.
    playback            jsonb,
    duration_seconds    double precision NOT NULL DEFAULT 0,

    view_count          bigint           NOT NULL DEFAULT 0,
    like_count          bigint           NOT NULL DEFAULT 0,
    dislike_count       bigint           NOT NULL DEFAULT 0,
    comment_count       bigint           NOT NULL DEFAULT 0,
    -- Maintained by the trending worker from video_hourly_stats.
    trending_score      double precision NOT NULL DEFAULT 0,
    -- Copy of channels.suspended, so listings need no join.
    suspended           boolean          NOT NULL DEFAULT false,
    -- The owner's account is locked (copy of the social domain's follow
    -- settings, kept by social.follow.settings_changed): open videos reach
    -- only the owner's followers and friends.
    owner_locked        boolean          NOT NULL DEFAULT false,

    attrs               jsonb            NOT NULL DEFAULT '{}',
    created_at          timestamptz      NOT NULL DEFAULT now(),
    updated_at          timestamptz      NOT NULL DEFAULT now(),

    -- A published video is always playable.
    CONSTRAINT videos_published_is_ready CHECK (status <> 3 OR (processing = 2 AND published_at IS NOT NULL)),
    CONSTRAINT videos_circle_audience CHECK ((visibility = 8) = (circle_id IS NOT NULL))
);

ALTER TABLE channels
    ADD CONSTRAINT channels_trailer_video FOREIGN KEY (trailer_video_id) REFERENCES videos (id) ON DELETE SET NULL;

-- The studio list.
CREATE INDEX videos_owner ON videos (owner_id, id DESC);
CREATE INDEX videos_channel ON videos (channel_id, id DESC);
-- Channel pages and the subscriptions feed (published videos only).
CREATE INDEX videos_channel_published ON videos (channel_id, published_at DESC, id DESC) WHERE status = 3;
CREATE INDEX videos_channel_popular ON videos (channel_id, view_count DESC, id DESC) WHERE status = 3;
-- Public listings: newest, trending, by tag.
CREATE INDEX videos_public_new ON videos (published_at DESC, id DESC) WHERE status = 3 AND visibility = 1;
CREATE INDEX videos_public_trending ON videos (trending_score DESC, id DESC) WHERE status = 3 AND visibility = 1;
CREATE INDEX videos_public_tags ON videos USING gin (tags) WHERE status = 3 AND visibility = 1;
CREATE INDEX videos_public_category ON videos (category, trending_score DESC, id DESC) WHERE status = 3 AND visibility = 1;
-- Friends' uploads in the home feed.
CREATE INDEX videos_owner_published ON videos (owner_id, published_at DESC) WHERE status = 3;
-- Related videos by game.
CREATE INDEX videos_public_game ON videos (game) WHERE status = 3 AND visibility = 1 AND game <> '';
-- The publishing worker.
CREATE INDEX videos_scheduled ON videos (scheduled_at) WHERE status = 2;
CREATE INDEX videos_thumbnail_asset ON videos (thumbnail_asset_id) WHERE thumbnail_asset_id IS NOT NULL;

-- `updated_at` means "the creator's content changed": counters and scores do not touch it.
CREATE TRIGGER videos_updated_at
    BEFORE UPDATE OF title, description, tags, category, game, chapters, thumbnail, visibility, circle_id, allow_link,
        age_restricted, comments_enabled, status, processing, scheduled_at ON videos
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Subtitle tracks. The WebVTT file itself is an object in the media bucket
-- (`object_key`); at most one track per video is the default.
CREATE TABLE video_subtitles (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    video_id        uuid        NOT NULL REFERENCES videos (id) ON DELETE CASCADE,
    -- BCP 47 tag; one track per language.
    language        text        NOT NULL,
    label           text        NOT NULL,
    object_key      text        NOT NULL,
    is_default      boolean     NOT NULL DEFAULT false,
    -- tokuly.video.v1.SubtitleFormat of the uploaded file: 1 vtt, 2 srt.
    original_format integer     NOT NULL CHECK (original_format BETWEEN 1 AND 2),
    cue_count       integer     NOT NULL DEFAULT 0,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (video_id, language)
);

CREATE UNIQUE INDEX video_subtitles_one_default ON video_subtitles (video_id) WHERE is_default;

CREATE TRIGGER video_subtitles_updated_at
    BEFORE UPDATE ON video_subtitles
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Likes and dislikes.
CREATE TABLE video_reactions (
    video_id   uuid        NOT NULL REFERENCES videos (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.video.v1.Rating: 2 like, 3 dislike.
    rating     integer     NOT NULL CHECK (rating BETWEEN 2 AND 3),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (video_id, user_id)
);

CREATE INDEX video_reactions_user ON video_reactions (user_id, created_at DESC);

-- Views and likes per video and hour: what the trending score is computed
-- from. Pruned by the trending worker.
CREATE TABLE video_hourly_stats (
    video_id uuid        NOT NULL REFERENCES videos (id) ON DELETE CASCADE,
    hour     timestamptz NOT NULL,
    views    integer     NOT NULL DEFAULT 0,
    likes    integer     NOT NULL DEFAULT 0,
    PRIMARY KEY (video_id, hour)
);

CREATE INDEX video_hourly_stats_hour ON video_hourly_stats (hour);

-- Game titles for the "game" field's suggestions: seeded below and grown by
-- the titles of published videos and live streams.
CREATE TABLE video_games (
    title      citext PRIMARY KEY,
    use_count  bigint      NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX video_games_trgm ON video_games USING gin ((title::text) gin_trgm_ops);
CREATE INDEX video_games_popular ON video_games (use_count DESC, title);

INSERT INTO video_games (title)
VALUES ('Minecraft'), ('Fortnite'), ('Apex Legends'), ('VALORANT'), ('League of Legends'), ('Overwatch 2'),
       ('原神'), ('崩壊:スターレイル'), ('ゼンレスゾーンゼロ'), ('スプラトゥーン3'), ('マリオカート ワールド'),
       ('大乱闘スマッシュブラザーズ SPECIAL'), ('あつまれ どうぶつの森'), ('ポケットモンスター スカーレット・バイオレット'),
       ('Pokémon LEGENDS Z-A'), ('ゼルダの伝説 ティアーズ オブ ザ キングダム'), ('モンスターハンターワイルズ'),
       ('ストリートファイター6'), ('鉄拳8'), ('ELDEN RING'), ('ファイナルファンタジーXIV'), ('ドラゴンクエストX'),
       ('Among Us'), ('Fall Guys'), ('Rocket League'), ('Call of Duty: Warzone'), ('PUBG: BATTLEGROUNDS'),
       ('Dead by Daylight'), ('Phasmophobia'), ('Lethal Company'), ('Escape from Tarkov'), ('Rainbow Six Siege'),
       ('Counter-Strike 2'), ('Dota 2'), ('Teamfight Tactics'), ('ウマ娘 プリティーダービー'), ('プロジェクトセカイ'),
       ('雀魂'), ('荒野行動'), ('第五人格'), ('Roblox'), ('The Sims 4'), ('Stardew Valley'), ('Terraria'),
       ('Palworld'), ('Hollow Knight: Silksong'), ('Grand Theft Auto V'), ('Cyberpunk 2077'), ('シャドウバース'),
       ('パズル&ドラゴンズ'), ('モンスターストライク');

-- ───────────────────────────── Playback history ─────────────────────────────

-- One row per user and video they started watching: resume position and
-- total time watched. Not written while the user paused their history.
CREATE TABLE watch_history (
    user_id          uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    video_id         uuid             NOT NULL REFERENCES videos (id) ON DELETE CASCADE,
    position_seconds double precision NOT NULL DEFAULT 0,
    watched_seconds  double precision NOT NULL DEFAULT 0,
    -- Watched to the end at least once.
    completed        boolean          NOT NULL DEFAULT false,
    last_watched_at  timestamptz      NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, video_id)
);

CREATE INDEX watch_history_recent ON watch_history (user_id, last_watched_at DESC, video_id);
CREATE INDEX watch_history_video ON watch_history (video_id);

-- Per-user switches of the video domain.
CREATE TABLE video_user_settings (
    user_id        uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    history_paused boolean     NOT NULL DEFAULT false,
    updated_at     timestamptz NOT NULL DEFAULT now()
);

-- "Not interested": videos and whole channels a user keeps out of their home feed.
CREATE TABLE video_feed_hidden (
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    video_id   uuid REFERENCES videos (id) ON DELETE CASCADE,
    channel_id uuid REFERENCES channels (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT video_feed_hidden_one_target CHECK ((video_id IS NULL) <> (channel_id IS NULL))
);

CREATE UNIQUE INDEX video_feed_hidden_video ON video_feed_hidden (user_id, video_id) WHERE video_id IS NOT NULL;
CREATE UNIQUE INDEX video_feed_hidden_channel ON video_feed_hidden (user_id, channel_id) WHERE channel_id IS NOT NULL;

-- ───────────────────────────── Playlists ─────────────────────────────

CREATE TABLE playlists (
    id          uuid PRIMARY KEY DEFAULT uuidv7(),
    short_id    text        NOT NULL UNIQUE,
    owner_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The channel whose page shows the playlist; NULL for personal playlists.
    channel_id  uuid REFERENCES channels (id) ON DELETE CASCADE,
    -- tokuly.video.v1.PlaylistKind: 1 custom, 2 watch later, 3 liked videos.
    kind        integer     NOT NULL DEFAULT 1 CHECK (kind BETWEEN 1 AND 3),
    title       text        NOT NULL DEFAULT '',
    description text        NOT NULL DEFAULT '',
    -- tokuly.common.v1.Visibility: 1 public, 3 unlisted, 5 private.
    visibility  integer     NOT NULL DEFAULT 5 CHECK (visibility IN (1, 3, 5)),
    item_count  integer     NOT NULL DEFAULT 0,
    like_count  bigint      NOT NULL DEFAULT 0,
    -- The owner's account is locked (as videos.owner_locked): a public or
    -- unlisted playlist reaches only the owner's followers and friends.
    owner_locked boolean    NOT NULL DEFAULT false,
    attrs       jsonb       NOT NULL DEFAULT '{}',
    created_at  timestamptz NOT NULL DEFAULT now(),
    -- Also moves when videos are added, removed or reordered.
    updated_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT playlists_built_in_is_private CHECK (kind = 1 OR (visibility = 5 AND channel_id IS NULL))
);

-- One watch-later and one liked list per user.
CREATE UNIQUE INDEX playlists_built_in ON playlists (owner_id, kind) WHERE kind <> 1;
CREATE INDEX playlists_owner ON playlists (owner_id, kind DESC, updated_at DESC);
CREATE INDEX playlists_channel ON playlists (channel_id, updated_at DESC, id DESC) WHERE channel_id IS NOT NULL AND visibility = 1;

CREATE TABLE playlist_items (
    playlist_id uuid        NOT NULL REFERENCES playlists (id) ON DELETE CASCADE,
    video_id    uuid        NOT NULL REFERENCES videos (id) ON DELETE CASCADE,
    -- Order within the list, ascending. Not unique on purpose: moving an
    -- item shifts a range of positions in one statement. Custom lists append
    -- (max + 1); built-in lists store the negated insertion time, so the same
    -- ordering shows them newest first.
    position    bigint      NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (playlist_id, video_id)
);

CREATE INDEX playlist_items_order ON playlist_items (playlist_id, position, video_id);
CREATE INDEX playlist_items_video ON playlist_items (video_id);

-- Keeps playlists.item_count exact whatever removes an item, including the
-- cascade from a deleted video.
CREATE FUNCTION playlist_items_count() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    IF TG_OP = 'INSERT' THEN
        UPDATE playlists SET item_count = item_count + 1, updated_at = now() WHERE id = NEW.playlist_id;
    ELSE
        UPDATE playlists SET item_count = GREATEST(item_count - 1, 0), updated_at = now() WHERE id = OLD.playlist_id;
    END IF;
    RETURN NULL;
END
$$;

CREATE TRIGGER playlist_items_count
    AFTER INSERT OR DELETE ON playlist_items
    FOR EACH ROW
EXECUTE FUNCTION playlist_items_count();

-- ───────────────────────────── Watch parties ─────────────────────────────

CREATE TABLE watch_parties (
    id               uuid PRIMARY KEY DEFAULT uuidv7(),
    -- What invite links carry. Knowing it is what lets someone join.
    code             text             NOT NULL UNIQUE,
    video_id         uuid             NOT NULL REFERENCES videos (id) ON DELETE CASCADE,
    playlist_id      uuid REFERENCES playlists (id) ON DELETE SET NULL,
    allow_guests     boolean          NOT NULL DEFAULT true,
    -- The authoritative playback state: where the video was at `state_at`.
    playing          boolean          NOT NULL DEFAULT false,
    position_seconds double precision NOT NULL DEFAULT 0,
    rate             double precision NOT NULL DEFAULT 1,
    state_at         timestamptz      NOT NULL DEFAULT now(),
    state_seq        bigint           NOT NULL DEFAULT 1,
    state_by         uuid,
    created_by       uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at       timestamptz      NOT NULL DEFAULT now(),
    ended_at         timestamptz,
    -- tokuly.video.v1.PartyEndReason.
    end_reason       integer          NOT NULL DEFAULT 0
);

CREATE INDEX watch_parties_open ON watch_parties (created_at) WHERE ended_at IS NULL;
CREATE INDEX watch_parties_ended ON watch_parties (ended_at) WHERE ended_at IS NOT NULL;
CREATE INDEX watch_parties_creator ON watch_parties (created_by) WHERE ended_at IS NULL;
CREATE INDEX watch_parties_video ON watch_parties (video_id) WHERE ended_at IS NULL;

-- The seats of a party: signed-in members and guests.
CREATE TABLE watch_party_members (
    id               uuid PRIMARY KEY DEFAULT uuidv7(),
    party_id         uuid        NOT NULL REFERENCES watch_parties (id) ON DELETE CASCADE,
    -- NULL for guests.
    user_id          uuid REFERENCES users (id) ON DELETE CASCADE,
    -- Guests: the name they chose, the keyed hash of their seat token and the
    -- secret segment of the realtime topic they follow (`party:<id>:<key>`).
    -- Every seat: the keyed hash of the network address it was taken from
    -- (for bans: a removed account cannot come back as a guest).
    guest_name       text        NOT NULL DEFAULT '',
    guest_token_hash text,
    topic_key        text,
    ip_hash          text        NOT NULL DEFAULT '',
    -- tokuly.video.v1.PartyRole: 1 host, 2 co-host, 3 member.
    role             integer     NOT NULL DEFAULT 3 CHECK (role BETWEEN 1 AND 3),
    joined_at        timestamptz NOT NULL DEFAULT now(),
    last_seen_at     timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT watch_party_members_user_or_guest CHECK ((user_id IS NULL) = (guest_token_hash IS NOT NULL)),
    CONSTRAINT watch_party_members_guests_do_not_host CHECK (user_id IS NOT NULL OR role = 3)
);

CREATE UNIQUE INDEX watch_party_members_user ON watch_party_members (party_id, user_id) WHERE user_id IS NOT NULL;
CREATE UNIQUE INDEX watch_party_members_guest ON watch_party_members (party_id, guest_token_hash) WHERE guest_token_hash IS NOT NULL;
CREATE UNIQUE INDEX watch_party_members_topic ON watch_party_members (party_id, topic_key) WHERE topic_key IS NOT NULL;
CREATE UNIQUE INDEX watch_party_members_one_host ON watch_party_members (party_id) WHERE role = 1;
CREATE INDEX watch_party_members_party ON watch_party_members (party_id, role, joined_at);
CREATE INDEX watch_party_members_stale ON watch_party_members (last_seen_at);
CREATE INDEX watch_party_members_of_user ON watch_party_members (user_id) WHERE user_id IS NOT NULL;

-- Who the hosts removed: accounts by ID, guests by the keyed hash of their
-- network address. They cannot rejoin.
CREATE TABLE watch_party_bans (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    party_id   uuid        NOT NULL REFERENCES watch_parties (id) ON DELETE CASCADE,
    user_id    uuid REFERENCES users (id) ON DELETE CASCADE,
    ip_hash    text,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT watch_party_bans_one_target CHECK ((user_id IS NULL) <> (ip_hash IS NULL))
);

CREATE UNIQUE INDEX watch_party_bans_user ON watch_party_bans (party_id, user_id) WHERE user_id IS NOT NULL;
CREATE UNIQUE INDEX watch_party_bans_ip ON watch_party_bans (party_id, ip_hash) WHERE ip_hash IS NOT NULL;

-- Party chat. Deleted with the party, a day after it ended.
CREATE TABLE watch_party_messages (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    party_id   uuid        NOT NULL REFERENCES watch_parties (id) ON DELETE CASCADE,
    -- The sender's seat and how they appeared when they sent it: seats come and go.
    member_id  uuid        NOT NULL,
    user_id    uuid REFERENCES users (id) ON DELETE CASCADE,
    name       text        NOT NULL DEFAULT '',
    role       integer     NOT NULL DEFAULT 3,
    text       text        NOT NULL DEFAULT '',
    -- tokuly.common.v1.StampRef as proto JSON.
    stamp      jsonb,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX watch_party_messages_party ON watch_party_messages (party_id, id DESC);
