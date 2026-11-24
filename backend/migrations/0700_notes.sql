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

-- ───────────────────────────── Channels ─────────────────────────────

-- Topic timelines anyone can post into. Defined before `notes`, which
-- names the channel a note was posted into.
CREATE TABLE note_channels (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    owner_id        uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name            text        NOT NULL,
    description     text        NOT NULL DEFAULT '',
    -- A NOTE_IMAGE upload (held as `notes:channel:<id>`) and its picture
    -- (tokuly.common.v1.Image as proto JSON).
    banner_asset_id uuid,
    banner          jsonb,
    -- `#rrggbb`, or '' for the default.
    color           text        NOT NULL DEFAULT '',
    follower_count  bigint      NOT NULL DEFAULT 0,
    -- Notes in the channel (not taken out by a moderator).
    note_count      bigint      NOT NULL DEFAULT 0,
    archived_at     timestamptz,
    last_note_at    timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX note_channels_owner ON note_channels (owner_id, id);
-- "Featured": active channels by followers.
CREATE INDEX note_channels_featured ON note_channels (follower_count DESC, id) WHERE archived_at IS NULL;
CREATE INDEX note_channels_name ON note_channels (lower(name) text_pattern_ops);

CREATE TRIGGER note_channels_updated_at
    BEFORE UPDATE OF name, description, banner_asset_id, color, archived_at ON note_channels
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE TABLE note_channel_moderators (
    channel_id uuid        NOT NULL REFERENCES note_channels (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    added_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, user_id)
);

CREATE INDEX note_channel_moderators_user ON note_channel_moderators (user_id);

-- `id` is time-ordered: the keyset of "channels I follow".
CREATE TABLE note_channel_follows (
    channel_id  uuid        NOT NULL REFERENCES note_channels (id) ON DELETE CASCADE,
    user_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    id          uuid        NOT NULL UNIQUE DEFAULT uuidv7(),
    followed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, user_id)
);

CREATE INDEX note_channel_follows_user ON note_channel_follows (user_id, id DESC);

-- ───────────────────────────── Notes ─────────────────────────────

-- A note changes after it is posted only through its author (audience,
-- pin, an edit within the edit window), a channel moderator (taking it out
-- of the channel), and its counters; there is no `updated_at` (the
-- counters change with every like, and nothing reads it).
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
    -- tokuly.common.v1.Visibility: 1 public, 4 friends, 5 private (direct
    -- when `direct_user_ids` names people), 6 followers, 7 close friends,
    -- 8 circle (`circle_id`, a tokuly.social.v1 circle of the author).
    visibility         integer     NOT NULL CHECK (visibility IN (1, 4, 5, 6, 7, 8)),
    circle_id          uuid,
    -- Everyone may see the note: public, and the author's account is not
    -- locked (`note_authors`). Listed notes are the ones in search, in the
    -- hashtag counts and trends, and in the global timeline.
    listed             boolean     NOT NULL DEFAULT false,
    -- The people a direct note is addressed to (visibility 5); empty otherwise.
    direct_user_ids    uuid[]      NOT NULL DEFAULT '{}',
    -- Folded behind a content warning; `content_warning_text` is its line.
    content_warning    boolean     NOT NULL DEFAULT false,
    content_warning_text text      NOT NULL DEFAULT '',
    -- tokuly.notes.v1.AttachmentDetails[] as proto JSON (alt text and the
    -- sensitive mark), in the order of `attachments`.
    attachment_details jsonb       NOT NULL DEFAULT '[]',
    -- `text` parsed into tokuly.notes.v1.RichText (proto JSON), mention user
    -- IDs and emoji pictures left out (they are looked up when shown).
    body               jsonb       NOT NULL DEFAULT '{}',
    -- `ja`, `en`, or '' when it cannot be told (see `text::language`).
    language           text        NOT NULL DEFAULT '',
    channel_id         uuid REFERENCES note_channels (id) ON DELETE SET NULL,
    -- A channel moderator took the note out of its channel.
    channel_removed_at timestamptz,

    -- The note this one answers and that note's author. Deliberately not
    -- foreign keys to `notes`: a reply outlives the note it answers and
    -- keeps saying what it was a reply to.
    reply_to_id        uuid,
    reply_to_author_id uuid REFERENCES users (id) ON DELETE SET NULL,
    -- The note that started the conversation (itself for a note that
    -- answers nothing): what muting a conversation mutes.
    thread_root_id     uuid,
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
    -- Public clips holding the note.
    clip_count         bigint      NOT NULL DEFAULT 0,

    -- Non-NULL = pinned to the author's profile (at most five, latest first).
    pinned_at          timestamptz,
    edited_at          timestamptz,
    edit_count         integer     NOT NULL DEFAULT 0,
    -- The draft this note was posted from (no foreign key: the draft is
    -- deleted once posted). Unique, so a draft is posted at most once.
    draft_id           uuid        UNIQUE,
    attrs              jsonb       NOT NULL DEFAULT '{}',
    created_at         timestamptz NOT NULL DEFAULT now(),
    CHECK ((visibility = 8) = (circle_id IS NOT NULL)),
    CHECK (visibility = 5 OR direct_user_ids = '{}'),
    CHECK (visibility = 1 OR NOT listed)
);

-- Profile and home timelines.
CREATE INDEX notes_author ON notes (author_id, id DESC);
-- Profile "media" tab.
CREATE INDEX notes_author_media ON notes (author_id, id DESC) WHERE asset_ids <> '{}';
-- Global timeline.
CREATE INDEX notes_listed ON notes (id DESC) WHERE listed AND reply_to_id IS NULL;
-- An author's public notes, when their account is locked or unlocked.
CREATE INDEX notes_author_public ON notes (author_id, id) WHERE visibility = 1;
-- Replies of a thread, oldest first.
CREATE INDEX notes_reply_to ON notes (reply_to_id, id) WHERE reply_to_id IS NOT NULL;
-- "Replies to me" in the mentions timeline.
CREATE INDEX notes_reply_to_author ON notes (reply_to_author_id, id DESC) WHERE reply_to_author_id IS NOT NULL;
CREATE INDEX notes_quote_of ON notes (quote_of_id, id DESC) WHERE quote_of_id IS NOT NULL;
-- Finding the notes that show a deleted upload.
CREATE INDEX notes_asset_ids ON notes USING gin (asset_ids);
-- Pinned notes of a profile.
CREATE INDEX notes_pinned ON notes (author_id, pinned_at DESC) WHERE pinned_at IS NOT NULL;
-- A channel's timeline.
CREATE INDEX notes_channel ON notes (channel_id, id DESC) WHERE channel_id IS NOT NULL AND channel_removed_at IS NULL;
CREATE INDEX notes_thread_root ON notes (thread_root_id);
-- Direct notes addressed to someone (the mentions timeline).
CREATE INDEX notes_direct ON notes USING gin (direct_user_ids) WHERE direct_user_ids <> '{}';

-- Whether an author's account is locked, as this domain last learnt it from
-- the social domain (`social.follow.settings_changed`; a first note records
-- it too). The public notes of a locked account reach only its followers
-- and friends: `notes.listed` follows this copy (reads double-check the
-- social domain for the moment before a change arrives here).
CREATE TABLE note_authors (
    user_id    uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    locked     boolean     NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

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

-- The people a note was announced to (a reply, quote or mention
-- notification). What reached someone stays reportable by them after the
-- author hid it from them (made it private, or a block).
CREATE TABLE note_recipients (
    note_id uuid NOT NULL REFERENCES notes (id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (note_id, user_id)
);

CREATE INDEX note_recipients_user ON note_recipients (user_id);

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

-- The trending rankings (one for all notes, language '', and one per
-- language), replaced as a whole by the trending worker.
CREATE TABLE hashtag_trends (
    language          text             NOT NULL,
    hashtag_id        uuid             NOT NULL REFERENCES hashtags (id) ON DELETE CASCADE,
    -- 1 = hottest.
    rank              integer          NOT NULL,
    score             double precision NOT NULL,
    -- Listed notes carrying the hashtag within the trend window.
    recent_note_count bigint           NOT NULL,
    computed_at       timestamptz      NOT NULL,
    PRIMARY KEY (language, hashtag_id)
);

CREATE INDEX hashtag_trends_rank ON hashtag_trends (language, rank);

-- Notes getting attention right now, ranked like `hashtag_trends`.
CREATE TABLE note_trends (
    language    text             NOT NULL,
    note_id     uuid             NOT NULL REFERENCES notes (id) ON DELETE CASCADE,
    rank        integer          NOT NULL,
    score       double precision NOT NULL,
    computed_at timestamptz      NOT NULL,
    PRIMARY KEY (language, note_id)
);

CREATE INDEX note_trends_rank ON note_trends (language, rank);

-- ───────────────────────────── Edits ─────────────────────────────

-- Earlier versions of edited notes (version 1 = as first posted).
CREATE TABLE note_edits (
    note_id              uuid        NOT NULL REFERENCES notes (id) ON DELETE CASCADE,
    version              integer     NOT NULL,
    text                 text        NOT NULL,
    body                 jsonb       NOT NULL,
    content_warning      boolean     NOT NULL,
    content_warning_text text        NOT NULL,
    replaced_at          timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (note_id, version)
);

-- ───────────────────────────── Polls ─────────────────────────────

CREATE TABLE note_polls (
    note_id            uuid PRIMARY KEY REFERENCES notes (id) ON DELETE CASCADE,
    -- 2–10 choices, in order, and the votes each got.
    choices            text[]      NOT NULL,
    votes              bigint[]    NOT NULL,
    multiple           boolean     NOT NULL,
    -- tokuly.notes.v1.PollResultsVisibility: 1 always, 2 after voting, 3 after the end.
    results_visibility integer     NOT NULL CHECK (results_visibility IN (1, 2, 3)),
    ends_at            timestamptz NOT NULL,
    voter_count        bigint      NOT NULL DEFAULT 0,
    -- Set by the closing worker once it told the voters.
    closed_at          timestamptz,
    CHECK (cardinality(choices) BETWEEN 2 AND 10 AND cardinality(votes) = cardinality(choices))
);

-- Work list of the closing worker.
CREATE INDEX note_polls_open ON note_polls (ends_at) WHERE closed_at IS NULL;

-- Who voted, and for what: never shown to anyone (the counts in
-- `note_polls.votes` are), kept so a vote is final and voters can be told
-- when the poll ends.
CREATE TABLE note_poll_votes (
    note_id  uuid        NOT NULL REFERENCES note_polls (note_id) ON DELETE CASCADE,
    user_id  uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    choices  integer[]   NOT NULL,
    voted_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (note_id, user_id)
);

CREATE INDEX note_poll_votes_user ON note_poll_votes (user_id);

-- ───────────────────────────── Drafts ─────────────────────────────

CREATE TABLE note_drafts (
    id             uuid PRIMARY KEY DEFAULT uuidv7(),
    author_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.notes.v1.DraftContent as proto JSON, and the uploads it names
    -- (held as `notes:draft:<id>`).
    content        jsonb       NOT NULL,
    asset_ids      uuid[]      NOT NULL DEFAULT '{}',
    scheduled_at   timestamptz,
    -- Why posting it when scheduled failed (an error reason); '' otherwise.
    failure_reason text        NOT NULL DEFAULT '',
    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX note_drafts_author ON note_drafts (author_id, updated_at DESC, id DESC);
-- Work list of the scheduler.
CREATE INDEX note_drafts_due ON note_drafts (scheduled_at) WHERE scheduled_at IS NOT NULL;
CREATE INDEX note_drafts_asset_ids ON note_drafts USING gin (asset_ids);

-- ───────────────────────────── Lists ─────────────────────────────

CREATE TABLE note_lists (
    id             uuid PRIMARY KEY DEFAULT uuidv7(),
    owner_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name           text        NOT NULL,
    description    text        NOT NULL DEFAULT '',
    public         boolean     NOT NULL DEFAULT false,
    follower_count bigint      NOT NULL DEFAULT 0,
    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX note_lists_owner ON note_lists (owner_id, id);

CREATE TRIGGER note_lists_updated_at
    BEFORE UPDATE OF name, description, public ON note_lists
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- `id` is time-ordered: the keyset of the member list.
CREATE TABLE note_list_members (
    list_id  uuid        NOT NULL REFERENCES note_lists (id) ON DELETE CASCADE,
    user_id  uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    id       uuid        NOT NULL UNIQUE DEFAULT uuidv7(),
    added_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (list_id, user_id)
);

CREATE INDEX note_list_members_user ON note_list_members (user_id);
CREATE INDEX note_list_members_recent ON note_list_members (list_id, id DESC);

CREATE TABLE note_list_follows (
    list_id     uuid        NOT NULL REFERENCES note_lists (id) ON DELETE CASCADE,
    user_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    id          uuid        NOT NULL UNIQUE DEFAULT uuidv7(),
    followed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (list_id, user_id)
);

CREATE INDEX note_list_follows_user ON note_list_follows (user_id, id DESC);

-- ───────────────────────────── Antennas ─────────────────────────────

CREATE TABLE antennas (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    owner_id        uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name            text        NOT NULL,
    -- tokuly.notes.v1.KeywordGroup[] as proto JSON (OR of ANDs), folded
    -- (NFKC, lowercase).
    keywords        jsonb       NOT NULL DEFAULT '[]',
    exclude_words   text[]      NOT NULL DEFAULT '{}',
    -- tokuly.notes.v1.AntennaSource: 1 everyone, 2 following, 3 list, 4 users.
    source          integer     NOT NULL CHECK (source BETWEEN 1 AND 4),
    list_id         uuid REFERENCES note_lists (id) ON DELETE CASCADE,
    user_ids        uuid[]      NOT NULL DEFAULT '{}',
    with_replies    boolean     NOT NULL DEFAULT false,
    media_only      boolean     NOT NULL DEFAULT false,
    notify          boolean     NOT NULL DEFAULT false,
    last_matched_at timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX antennas_owner ON antennas (owner_id, id);

CREATE TRIGGER antennas_updated_at
    BEFORE UPDATE OF name, keywords, exclude_words, source, list_id, user_ids, with_replies, media_only, notify ON antennas
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- What each antenna collected (the latest 1000 are kept).
CREATE TABLE antenna_notes (
    antenna_id uuid        NOT NULL REFERENCES antennas (id) ON DELETE CASCADE,
    note_id    uuid        NOT NULL REFERENCES notes (id) ON DELETE CASCADE,
    matched_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (antenna_id, note_id)
);

CREATE INDEX antenna_notes_recent ON antenna_notes (antenna_id, note_id DESC);
CREATE INDEX antenna_notes_note ON antenna_notes (note_id);

-- ───────────────────────────── Channel pins ─────────────────────────────

CREATE TABLE note_channel_pins (
    channel_id uuid        NOT NULL REFERENCES note_channels (id) ON DELETE CASCADE,
    note_id    uuid        NOT NULL REFERENCES notes (id) ON DELETE CASCADE,
    pinned_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, note_id)
);

-- ───────────────────────────── Clips ─────────────────────────────

CREATE TABLE note_clips (
    id          uuid PRIMARY KEY DEFAULT uuidv7(),
    owner_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name        text        NOT NULL,
    description text        NOT NULL DEFAULT '',
    public      boolean     NOT NULL DEFAULT false,
    created_at  timestamptz NOT NULL DEFAULT now(),
    -- Changes with the clip's name, description, audience and notes.
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX note_clips_owner ON note_clips (owner_id, updated_at DESC, id DESC);

-- `id` is time-ordered: the keyset of a clip's notes.
CREATE TABLE note_clip_items (
    clip_id  uuid        NOT NULL REFERENCES note_clips (id) ON DELETE CASCADE,
    note_id  uuid        NOT NULL REFERENCES notes (id) ON DELETE CASCADE,
    id       uuid        NOT NULL UNIQUE DEFAULT uuidv7(),
    added_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (clip_id, note_id)
);

CREATE INDEX note_clip_items_recent ON note_clip_items (clip_id, id DESC);
CREATE INDEX note_clip_items_note ON note_clip_items (note_id);

-- ───────────────────────────── Muting ─────────────────────────────

-- tokuly.notes.v1.MutedWord[] as proto JSON.
CREATE TABLE note_muted_words (
    user_id    uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    words      jsonb       NOT NULL DEFAULT '[]',
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE note_thread_mutes (
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- `notes.thread_root_id` of the conversation; may outlive the note.
    root_note_id uuid        NOT NULL,
    muted_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, root_note_id)
);

CREATE INDEX note_thread_mutes_root ON note_thread_mutes (root_note_id);

CREATE TABLE note_renote_mutes (
    user_id   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    target_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    muted_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, target_id)
);

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
