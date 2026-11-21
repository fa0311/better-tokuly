-- Engagement: comments, reactions, bookmarks, shares and reports on any
-- subject. A subject is a (tokuly.common.v1.SubjectType, UUID) pair owned by
-- another domain; nothing here has a foreign key to it.

-- ───────────────────────────── comments ─────────────────────────────

-- Rank of a comment in the "top" order. Points are likes minus dislikes, two
-- per reply and ten for the creator's heart. The score is log10(1 + points)
-- plus the comment's age in weeks since the epoch: ten times the points are
-- worth about one week, so newer comments need fewer points than older ones,
-- a single like already counts, and comments without points sort newest
-- first. The value never changes with the clock, which keeps keyset
-- pagination over it stable.
CREATE FUNCTION comment_score(likes bigint, dislikes bigint, replies integer, hearted boolean, created timestamptz)
    RETURNS double precision
    LANGUAGE sql
    IMMUTABLE PARALLEL SAFE AS
$$
SELECT sign(points) * log(1 + abs(points))
           + extract(epoch FROM (created AT TIME ZONE 'UTC')) / 604800.0
FROM (SELECT (likes - dislikes + 2 * replies + CASE WHEN hearted THEN 10 ELSE 0 END)::double precision AS points) AS p
$$;

CREATE TABLE comments (
    id                uuid PRIMARY KEY          DEFAULT uuidv7(),
    subject_type      integer          NOT NULL,
    subject_id        uuid             NOT NULL,
    -- The subject's owner when the comment was written (the nil UUID for
    -- content that belongs to nobody). Lists "comments on my subjects";
    -- permissions are always checked against the subject's current owner.
    subject_owner_id  uuid             NOT NULL,
    -- The top-level comment a reply belongs to. Threads are one level deep.
    parent_id         uuid REFERENCES comments (id) ON DELETE CASCADE,
    -- The reply this reply answers, when it is not the top-level comment itself.
    reply_to_id       uuid             REFERENCES comments (id) ON DELETE SET NULL,
    author_id         uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Set when the comment was written as one of the author's channels.
    channel_id        uuid,
    text              text             NOT NULL DEFAULT '',
    -- Kept after the stamp is deleted, so the comment can say so.
    stamp_id          uuid,
    -- tokuly.common.v1.Image as proto JSON, and the media asset behind it
    -- (held as `engagement:comment:<id>`).
    image             jsonb,
    image_asset_id    uuid,
    -- A moment of the video the comment refers to.
    timestamp_seconds integer,
    -- tokuly.engagement.v1.CommentStatus / HoldReason
    status            integer          NOT NULL DEFAULT 1 CHECK (status BETWEEN 1 AND 3),
    hold_reason       integer          NOT NULL DEFAULT 0,
    like_count        bigint           NOT NULL DEFAULT 0,
    dislike_count     bigint           NOT NULL DEFAULT 0,
    -- Published replies (always 0 on replies).
    reply_count       integer          NOT NULL DEFAULT 0,
    pinned_at         timestamptz,
    hearted_at        timestamptz,
    edited_at         timestamptz,
    -- When the people it replies to and mentions were notified: on
    -- publication, which for a held comment is its approval.
    notified_at       timestamptz,
    score             double precision GENERATED ALWAYS AS (
        comment_score(like_count, dislike_count, reply_count, hearted_at IS NOT NULL, created_at)) STORED,
    attrs             jsonb            NOT NULL DEFAULT '{}',
    created_at        timestamptz      NOT NULL DEFAULT now(),
    updated_at        timestamptz      NOT NULL DEFAULT now(),

    CONSTRAINT comments_one_attachment CHECK (stamp_id IS NULL OR image_asset_id IS NULL),
    CONSTRAINT comments_only_top_level_pinned CHECK (pinned_at IS NULL OR parent_id IS NULL)
);

CREATE TRIGGER comments_updated_at
    BEFORE UPDATE ON comments
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- The two orders of a subject's top-level comments.
CREATE INDEX comments_top ON comments (subject_type, subject_id, score DESC, id DESC) WHERE parent_id IS NULL;
CREATE INDEX comments_newest ON comments (subject_type, subject_id, id DESC) WHERE parent_id IS NULL;
CREATE INDEX comments_replies ON comments (parent_id, id) WHERE parent_id IS NOT NULL;
-- A subject has one pinned comment.
CREATE UNIQUE INDEX comments_pinned ON comments (subject_type, subject_id) WHERE pinned_at IS NOT NULL;
CREATE INDEX comments_author ON comments (author_id, id DESC);
-- The owner's comment manager and review queue.
CREATE INDEX comments_received ON comments (subject_owner_id, id DESC);
CREATE INDEX comments_held ON comments (subject_owner_id, id DESC) WHERE status = 2;
CREATE INDEX comments_image_asset ON comments (image_asset_id) WHERE image_asset_id IS NOT NULL;
CREATE INDEX comments_reply_to ON comments (reply_to_id) WHERE reply_to_id IS NOT NULL;

-- The users a comment's `@handle`s referred to when it was written.
CREATE TABLE comment_mentions (
    comment_id uuid NOT NULL REFERENCES comments (id) ON DELETE CASCADE,
    user_id    uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (comment_id, user_id)
);

CREATE INDEX comment_mentions_user ON comment_mentions (user_id);

-- How a user wants comments on their own subjects screened.
CREATE TABLE comment_settings (
    user_id       uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    hold_all      boolean     NOT NULL DEFAULT false,
    hold_links    boolean     NOT NULL DEFAULT false,
    -- As the owner typed them; matching normalizes both sides.
    blocked_words text[]      NOT NULL DEFAULT '{}',
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER comment_settings_updated_at
    BEFORE UPDATE ON comment_settings
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- People an owner stopped from commenting on their subjects. The block
-- applies to the account whatever identity it comments with; `channel_id`
-- records the identity the owner saw when blocking (NULL: the user), which
-- is the only one shown back to them. One row per identity, so blocking a
-- second identity of the same account does not reveal the connection.
CREATE TABLE comment_blocks (
    id              uuid PRIMARY KEY     DEFAULT uuidv7(),
    owner_id        uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    blocked_user_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    channel_id      uuid,
    created_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE NULLS NOT DISTINCT (owner_id, blocked_user_id, channel_id),
    CHECK (owner_id <> blocked_user_id)
);

CREATE INDEX comment_blocks_newest ON comment_blocks (owner_id, id DESC);
CREATE INDEX comment_blocks_blocked ON comment_blocks (blocked_user_id);

-- ───────────────────────────── reactions ─────────────────────────────

-- A user's one reaction to a subject: a like, a dislike or an emoji.
CREATE TABLE reactions (
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    subject_type integer     NOT NULL,
    subject_id   uuid        NOT NULL,
    -- tokuly.engagement.v1.ReactionKind
    kind         integer     NOT NULL CHECK (kind BETWEEN 1 AND 3),
    emoji        text        NOT NULL DEFAULT '',
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, subject_type, subject_id),
    CONSTRAINT reactions_emoji_iff_kind CHECK ((kind = 3) = (emoji <> ''))
);

-- Who reacted to a subject, and what a user reacted to.
CREATE INDEX reactions_subject ON reactions (subject_type, subject_id, created_at DESC, user_id DESC);
CREATE INDEX reactions_mine ON reactions (user_id, created_at DESC, subject_id DESC);

-- Every number of a subject, kept in step with the rows it counts in the
-- transaction that changes them.
CREATE TABLE subject_stats (
    subject_type   integer     NOT NULL,
    subject_id     uuid        NOT NULL,
    -- The subject's owner when the row was created (nil: nobody); lets the
    -- data of an erased account's subjects be purged.
    owner_id       uuid        NOT NULL,
    like_count     bigint      NOT NULL DEFAULT 0,
    dislike_count  bigint      NOT NULL DEFAULT 0,
    -- Emoji reactions of every emoji together.
    emoji_count    bigint      NOT NULL DEFAULT 0,
    -- Comments in public view: published top-level comments and the
    -- published replies below them.
    comment_count  bigint      NOT NULL DEFAULT 0,
    -- Users who saved the subject.
    bookmark_count bigint      NOT NULL DEFAULT 0,
    share_count    bigint      NOT NULL DEFAULT 0,
    updated_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (subject_type, subject_id)
);

CREATE TRIGGER subject_stats_updated_at
    BEFORE UPDATE ON subject_stats
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE INDEX subject_stats_owner ON subject_stats (owner_id);

CREATE TABLE reaction_emoji_counts (
    subject_type integer NOT NULL,
    subject_id   uuid    NOT NULL,
    emoji        text    NOT NULL,
    count        bigint  NOT NULL DEFAULT 0,
    PRIMARY KEY (subject_type, subject_id, emoji)
);

-- Shares per target (tokuly.engagement.v1.ShareTarget). Single shares are
-- analytics events, not rows.
CREATE TABLE share_counts (
    subject_type integer NOT NULL,
    subject_id   uuid    NOT NULL,
    target       integer NOT NULL,
    count        bigint  NOT NULL DEFAULT 0,
    PRIMARY KEY (subject_type, subject_id, target)
);

-- ───────────────────────────── bookmarks ─────────────────────────────

-- Every user has one default collection ("saved") holding everything they
-- saved; the collections they create hold subsets of it.
CREATE TABLE bookmark_collections (
    id         uuid PRIMARY KEY     DEFAULT uuidv7(),
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name       text        NOT NULL,
    -- The name as compared for uniqueness: NFKC, lowercase.
    name_key   text        NOT NULL,
    is_default boolean     NOT NULL DEFAULT false,
    item_count integer     NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (user_id, name_key)
);

CREATE TRIGGER bookmark_collections_updated_at
    BEFORE UPDATE ON bookmark_collections
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE UNIQUE INDEX bookmark_collections_default ON bookmark_collections (user_id) WHERE is_default;

CREATE TABLE bookmarks (
    collection_id uuid        NOT NULL REFERENCES bookmark_collections (id) ON DELETE CASCADE,
    user_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    subject_type  integer     NOT NULL,
    subject_id    uuid        NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (collection_id, subject_type, subject_id)
);

CREATE INDEX bookmarks_newest ON bookmarks (collection_id, created_at DESC, subject_id DESC);
CREATE INDEX bookmarks_newest_by_type ON bookmarks (collection_id, subject_type, created_at DESC, subject_id DESC);
CREATE INDEX bookmarks_of_user ON bookmarks (user_id, subject_type, subject_id);
CREATE INDEX bookmarks_subject ON bookmarks (subject_type, subject_id);

-- ───────────────────────────── reports ─────────────────────────────

CREATE TABLE reports (
    id               uuid PRIMARY KEY     DEFAULT uuidv7(),
    reporter_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    subject_type     integer     NOT NULL,
    subject_id       uuid        NOT NULL,
    -- As they were when the report was filed; the subject may be gone since.
    subject_owner_id uuid,
    subject_title    text        NOT NULL DEFAULT '',
    subject_path     text        NOT NULL DEFAULT '',
    -- tokuly.engagement.v1.ReportCategory / ReportStatus
    category         integer     NOT NULL CHECK (category BETWEEN 1 AND 10),
    text             text        NOT NULL DEFAULT '',
    status           integer     NOT NULL DEFAULT 1 CHECK (status BETWEEN 1 AND 4),
    created_at       timestamptz NOT NULL DEFAULT now(),
    resolved_at      timestamptz
);

-- A user has one open (submitted) report per subject.
CREATE UNIQUE INDEX reports_open ON reports (reporter_id, subject_type, subject_id) WHERE status = 1;
CREATE INDEX reports_mine ON reports (reporter_id, id DESC);
CREATE INDEX reports_open_by_subject ON reports (subject_type, subject_id) WHERE status = 1;
