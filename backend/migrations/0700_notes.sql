-- Notes: short posts with their likes, renotes, hashtags and mentions; the
-- posting rules people agree to; and stories (記憶) with their audiences,
-- viewers and highlights.

-- ───────────────────────────── Rules ─────────────────────────────

-- The version of the posting rules each user agreed to last.
CREATE TABLE note_rule_agreements (
    user_id   uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    version   integer     NOT NULL,
    agreed_at timestamptz NOT NULL DEFAULT now()
);

-- ───────────────────────────── Notes ─────────────────────────────

-- A note is immutable once posted, apart from its visibility, its pin and
-- its counters; there is no `updated_at` (the counters change with every
-- like, and nothing reads it).
CREATE TABLE notes (
    id                 uuid PRIMARY KEY DEFAULT uuidv7(),
    author_id          uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    text               text        NOT NULL DEFAULT '',
    -- tokuly.media.v1.Attachment[] as proto JSON, in display order, and the
    -- media assets behind them (held as `notes:note:<id>`).
    attachments        jsonb       NOT NULL DEFAULT '[]',
    asset_ids          uuid[]      NOT NULL DEFAULT '{}',
    stamp_id           uuid,
    -- Place tag; all NULL when the note has none. `place_id` is an opaque
    -- reference into the map's place directory.
    place_name         text,
    place_lat          double precision,
    place_lng          double precision,
    place_id           uuid,
    -- tokuly.common.v1.Visibility: 1 public, 4 friends, 5 private.
    visibility         integer     NOT NULL CHECK (visibility IN (1, 4, 5)),
    content_warning    boolean     NOT NULL DEFAULT false,

    -- The note this one answers and that note's author. Deliberately not
    -- foreign keys to `notes`: a reply outlives the note it answers and
    -- keeps saying what it was a reply to.
    reply_to_id        uuid,
    reply_to_author_id uuid REFERENCES users (id) ON DELETE SET NULL,
    -- The note this one quotes; may point at a deleted note for the same reason.
    quote_of_id        uuid,

    -- Parsed from the text when the note was posted: normalized hashtags in
    -- order of appearance, and tokuly.notes.v1.Mention[] as proto JSON.
    hashtags           text[]      NOT NULL DEFAULT '{}',
    mentions           jsonb       NOT NULL DEFAULT '[]',
    -- First link of the text and its tokuly.media.v1.LinkPreview as proto
    -- JSON (NULL until resolved, or when the page has nothing to show).
    link_url           text,
    link_preview       jsonb,

    reply_count        bigint      NOT NULL DEFAULT 0,
    renote_count       bigint      NOT NULL DEFAULT 0,
    quote_count        bigint      NOT NULL DEFAULT 0,
    like_count         bigint      NOT NULL DEFAULT 0,
    view_count         bigint      NOT NULL DEFAULT 0,

    -- Non-NULL = pinned to the author's profile.
    pinned_at          timestamptz,
    attrs              jsonb       NOT NULL DEFAULT '{}',
    created_at         timestamptz NOT NULL DEFAULT now()
);

-- Profile and home timelines.
CREATE INDEX notes_author ON notes (author_id, id DESC);
-- Profile "media" tab.
CREATE INDEX notes_author_media ON notes (author_id, id DESC) WHERE asset_ids <> '{}';
-- Global timeline.
CREATE INDEX notes_public ON notes (id DESC) WHERE visibility = 1 AND reply_to_id IS NULL;
-- Replies of a thread, oldest first.
CREATE INDEX notes_reply_to ON notes (reply_to_id, id) WHERE reply_to_id IS NOT NULL;
-- "Replies to me" in the mentions timeline.
CREATE INDEX notes_reply_to_author ON notes (reply_to_author_id, id DESC) WHERE reply_to_author_id IS NOT NULL;
CREATE INDEX notes_quote_of ON notes (quote_of_id, id DESC) WHERE quote_of_id IS NOT NULL;
-- Finding the notes that show a deleted upload.
CREATE INDEX notes_asset_ids ON notes USING gin (asset_ids);
-- One pinned note per profile.
CREATE UNIQUE INDEX notes_pinned ON notes (author_id) WHERE pinned_at IS NOT NULL;

-- `id` is time-ordered and is the keyset of both "who liked this note" and
-- "what did this user like".
CREATE TABLE note_likes (
    note_id    uuid        NOT NULL REFERENCES notes (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    id         uuid        NOT NULL UNIQUE DEFAULT uuidv7(),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (note_id, user_id)
);

CREATE INDEX note_likes_note ON note_likes (note_id, id DESC);
CREATE INDEX note_likes_user ON note_likes (user_id, id DESC);

-- One renote per user per note. A renote is a timeline entry of its own:
-- `id` sorts it among the notes of the people who see it.
CREATE TABLE note_renotes (
    note_id    uuid        NOT NULL REFERENCES notes (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    id         uuid        NOT NULL UNIQUE DEFAULT uuidv7(),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (note_id, user_id)
);

CREATE INDEX note_renotes_note ON note_renotes (note_id, id DESC);
CREATE INDEX note_renotes_user ON note_renotes (user_id, id DESC);

-- Who a note mentions, for the mentions timeline. `handle` is the handle as
-- written in the text.
CREATE TABLE note_mentions (
    note_id uuid NOT NULL REFERENCES notes (id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    handle  text NOT NULL,
    PRIMARY KEY (note_id, user_id)
);

CREATE INDEX note_mentions_user ON note_mentions (user_id, note_id DESC);

CREATE TABLE hashtags (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    -- Normalized: NFKC, lowercase, without '#'.
    tag        text        NOT NULL UNIQUE,
    -- Public notes carrying the hashtag.
    note_count bigint      NOT NULL DEFAULT 0,
    -- `note_count` as last sent to the search index (the trending worker
    -- re-indexes hashtags whose count moved since).
    indexed_note_count bigint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Completion of a typed prefix.
CREATE INDEX hashtags_prefix ON hashtags (tag text_pattern_ops);

-- The primary key doubles as the hashtag timeline (notes of a hashtag, newest first).
CREATE TABLE note_hashtags (
    hashtag_id uuid NOT NULL REFERENCES hashtags (id) ON DELETE CASCADE,
    note_id    uuid NOT NULL REFERENCES notes (id) ON DELETE CASCADE,
    PRIMARY KEY (hashtag_id, note_id)
);

CREATE INDEX note_hashtags_note ON note_hashtags (note_id);

-- The trending ranking, replaced as a whole by the trending worker.
CREATE TABLE hashtag_trends (
    hashtag_id        uuid PRIMARY KEY REFERENCES hashtags (id) ON DELETE CASCADE,
    -- 1 = hottest.
    rank              integer          NOT NULL,
    score             double precision NOT NULL,
    -- Public notes carrying the hashtag within the trend window.
    recent_note_count bigint           NOT NULL,
    computed_at       timestamptz      NOT NULL
);

CREATE INDEX hashtag_trends_rank ON hashtag_trends (rank);

-- ───────────────────────────── Stories ─────────────────────────────

CREATE TABLE stories (
    id             uuid PRIMARY KEY DEFAULT uuidv7(),
    owner_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title          text        NOT NULL DEFAULT '',
    -- tokuly.notes.v1.StoryItem[] as proto JSON, in playing order, and the
    -- media assets behind them (held as `notes:story:<id>`).
    items          jsonb       NOT NULL,
    asset_ids      uuid[]      NOT NULL,
    -- tokuly.notes.v1.StoryAudience: 1 friends, 2 close friends (`story_audience`).
    audience       integer     NOT NULL DEFAULT 1 CHECK (audience IN (1, 2)),
    view_count     integer     NOT NULL DEFAULT 0,
    reaction_count integer     NOT NULL DEFAULT 0,
    -- Friends see the story until then; afterwards only the owner does
    -- (and friends through a highlight).
    expires_at     timestamptz NOT NULL,
    -- Set by the expiry worker once it has announced the expiry
    -- (`notes.story.expired`). Visibility never depends on it.
    archived_at    timestamptz,
    attrs          jsonb       NOT NULL DEFAULT '{}',
    created_at     timestamptz NOT NULL DEFAULT now()
);

-- The archive, and the live stories of a set of friends (the tray).
CREATE INDEX stories_owner ON stories (owner_id, id DESC);
CREATE INDEX stories_live ON stories (owner_id, expires_at);
-- Work list of the expiry worker.
CREATE INDEX stories_expiring ON stories (expires_at) WHERE archived_at IS NULL;
CREATE INDEX stories_asset_ids ON stories USING gin (asset_ids);

-- The friends chosen for a close-friends story.
CREATE TABLE story_audience (
    story_id uuid NOT NULL REFERENCES stories (id) ON DELETE CASCADE,
    user_id  uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (story_id, user_id)
);

CREATE INDEX story_audience_user ON story_audience (user_id);

-- Who watched a story, and their reaction (one emoji; '' = none).
CREATE TABLE story_views (
    story_id   uuid        NOT NULL REFERENCES stories (id) ON DELETE CASCADE,
    viewer_id  uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    viewed_at  timestamptz NOT NULL DEFAULT now(),
    reaction   text        NOT NULL DEFAULT '',
    reacted_at timestamptz,
    PRIMARY KEY (story_id, viewer_id)
);

-- The viewer list, most recent first.
CREATE INDEX story_views_recent ON story_views (story_id, viewed_at DESC, viewer_id DESC);
CREATE INDEX story_views_viewer ON story_views (viewer_id);

-- Titled groups of stories pinned to a profile.
CREATE TABLE story_highlights (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    owner_id   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title      text        NOT NULL,
    -- Order on the profile, ascending.
    position   integer     NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX story_highlights_owner ON story_highlights (owner_id, position, id);

-- `updated_at` is when the highlight's content (title, stories) last changed;
-- every such change writes `title`. Moving a highlight on the profile
-- (`position`) is not a change of the highlight.
CREATE TRIGGER story_highlights_updated_at
    BEFORE UPDATE OF title ON story_highlights
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE TABLE story_highlight_items (
    highlight_id uuid    NOT NULL REFERENCES story_highlights (id) ON DELETE CASCADE,
    story_id     uuid    NOT NULL REFERENCES stories (id) ON DELETE CASCADE,
    -- Playing order within the highlight, ascending.
    position     integer NOT NULL,
    PRIMARY KEY (highlight_id, story_id)
);

CREATE INDEX story_highlight_items_story ON story_highlight_items (story_id);
