-- Feed: the activity log, map-story seen state, news and recap notices.

-- ───────────────────────────── Activity log ─────────────────────────────

-- One thing a user did, as reported by the owning domain through
-- `activity.recorded`. Stored once; the owner's log, the profile list, the
-- friends' feed, map stories, the home feed and recaps are all reads of this
-- table, filtered by audience at read time.
CREATE TABLE activities (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id       uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Dotted kind chosen by the producing domain, e.g. `social.bumped`.
    kind          text             NOT NULL,
    -- tokuly.feed.v1.ActivityAudience: 1 private, 2 friends, 3 public.
    audience      smallint         NOT NULL CHECK (audience BETWEEN 1 AND 3),
    -- tokuly.common.v1.SubjectType and the subject's ID, when given. No
    -- foreign key: subjects belong to other domains.
    subject_type  integer,
    subject_id    uuid,
    -- Other users involved (the friend who was bumped...). No foreign key;
    -- entries naming a deleted user are removed on `identity.user.deleted`.
    with_user_ids uuid[]           NOT NULL DEFAULT '{}',
    -- The summary in both languages; the reader's language picks one.
    summary_ja    text             NOT NULL,
    summary_en    text             NOT NULL,
    -- Path in the app to open when tapped; may be empty.
    path          text             NOT NULL DEFAULT '',
    image_url     text,
    -- Where it happened. Entries with a location are map-story material.
    lat           double precision,
    lng           double precision,
    -- Extra structured data from the producing domain.
    data          jsonb            NOT NULL DEFAULT 'null',
    -- When it happened (the event's time); every list sorts by it.
    occurred_at   timestamptz      NOT NULL,
    created_at    timestamptz      NOT NULL DEFAULT now(),
    CHECK ((subject_type IS NULL) = (subject_id IS NULL)),
    CHECK ((lat IS NULL) = (lng IS NULL))
);

-- A user's log and profile list; also drives the friends' feed (one range per friend).
CREATE INDEX activities_log ON activities (user_id, occurred_at DESC, id DESC);
-- "Is there a newer entry of the same kind by the same person?" (burst collapsing).
CREATE INDEX activities_burst ON activities (user_id, kind, occurred_at DESC, id DESC);
-- Map stories: recent entries with a location.
CREATE INDEX activities_located ON activities (user_id, occurred_at DESC) WHERE lat IS NOT NULL;
-- Takedowns (`admin.content.removed`) and deleted users as subjects.
CREATE INDEX activities_subject ON activities (subject_type, subject_id) WHERE subject_id IS NOT NULL;
-- Entries naming a user (account deletion).
CREATE INDEX activities_with_users ON activities USING gin (with_user_ids);
-- Retention sweep.
CREATE INDEX activities_age ON activities (occurred_at);

-- Kinds (or kind prefixes such as `geo`) a user does not show to anyone
-- else. Applied when entries are read, so hiding and showing again never
-- loses anything.
CREATE TABLE activity_hidden_kinds (
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind       text        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, kind)
);

-- IDs of events that were already handled. Event delivery is at-least-once;
-- a row here makes a redelivery a no-op (and keeps an entry the user deleted
-- from coming back). Rows outlive the event stream's retention (7 days) and
-- are then swept.
CREATE TABLE feed_processed_events (
    event_id     uuid PRIMARY KEY,
    processed_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX feed_processed_events_age ON feed_processed_events (processed_at);

-- ───────────────────────────── Map stories ─────────────────────────────

-- Which map-story items a viewer has watched. Private to the viewer.
CREATE TABLE map_story_views (
    viewer_id   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    activity_id uuid        NOT NULL REFERENCES activities (id) ON DELETE CASCADE,
    -- Whose story it was, so the rows between two people can be dropped when
    -- they stop being friends.
    author_id   uuid        NOT NULL,
    seen_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (viewer_id, activity_id)
);

CREATE INDEX map_story_views_activity ON map_story_views (activity_id);
CREATE INDEX map_story_views_author ON map_story_views (author_id, viewer_id);
-- Retention sweep.
CREATE INDEX map_story_views_age ON map_story_views (seen_at);

-- ───────────────────────────── News ─────────────────────────────

-- An RSS/Atom feed articles are collected from. Managed by admins; sources
-- listed in TOKULY_NEWS_FEEDS are created at startup (`from_config`).
CREATE TABLE news_sources (
    id                   uuid PRIMARY KEY DEFAULT uuidv7(),
    name                 text        NOT NULL,
    feed_url             text        NOT NULL UNIQUE,
    site_url             text        NOT NULL DEFAULT '',
    favicon_url          text        NOT NULL DEFAULT '',
    -- BCP 47 tag of the source's language.
    language             text        NOT NULL DEFAULT 'ja',
    -- Tags given to every article of the source.
    default_tags         text[]      NOT NULL DEFAULT '{}',
    enabled              boolean     NOT NULL DEFAULT true,
    from_config          boolean     NOT NULL DEFAULT false,
    -- Nobody chose the name / site: each fetch refreshes them from the feed.
    name_from_feed       boolean     NOT NULL DEFAULT false,
    site_from_feed       boolean     NOT NULL DEFAULT false,
    -- Validators of the last successful response, for conditional requests.
    etag                 text,
    last_modified        text,
    -- Last attempt, successful or not.
    last_fetched_at      timestamptz,
    -- Last attempt that worked (a document or "304 Not Modified").
    last_success_at      timestamptz,
    -- When the feed document was last downloaded and read (a 200 response).
    -- Articles not seen since then (see news_articles.last_seen_at) have
    -- left the feed.
    document_at          timestamptz,
    -- Why the last attempt failed; empty after a success.
    last_error           text        NOT NULL DEFAULT '',
    consecutive_failures integer     NOT NULL DEFAULT 0,
    -- When the scheduler fetches the source next (pushed back after failures).
    next_fetch_at        timestamptz NOT NULL DEFAULT now(),
    attrs                jsonb       NOT NULL DEFAULT '{}',
    created_at           timestamptz NOT NULL DEFAULT now(),
    updated_at           timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX news_sources_due ON news_sources (next_fetch_at) WHERE enabled;

CREATE TRIGGER news_sources_updated_at
    BEFORE UPDATE ON news_sources
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE TABLE news_articles (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    source_id     uuid        NOT NULL REFERENCES news_sources (id) ON DELETE CASCADE,
    -- The link without fragment and tracking parameters: one row per article
    -- even when several feeds (or one feed, twice) carry it.
    canonical_url text        NOT NULL UNIQUE,
    -- The link as the feed gave it; this is what readers open.
    url           text        NOT NULL,
    title         text        NOT NULL,
    -- Plain text (markup stripped).
    summary       text        NOT NULL DEFAULT '',
    image_url     text        NOT NULL DEFAULT '',
    -- Lowercase: source tags, feed categories and keyword matches.
    tags          text[]      NOT NULL DEFAULT '{}',
    -- From the feed; the time of the first fetch for entries without a date.
    published_at  timestamptz NOT NULL,
    -- Last time the article was found in a downloaded feed document
    -- (refreshed at most twice a day). Old articles are pruned only once they
    -- left the feed (last_seen_at well before news_sources.document_at), so
    -- an entry without a date is not re-imported as new after every prune.
    last_seen_at  timestamptz NOT NULL DEFAULT now(),
    click_count   bigint      NOT NULL DEFAULT 0,
    -- Maintained by the engagement domain through the subject resolver.
    like_count    bigint      NOT NULL DEFAULT 0,
    comment_count bigint      NOT NULL DEFAULT 0,
    -- Taken down by a moderator: hidden everywhere, kept so it is not fetched again.
    removed_at    timestamptz,
    attrs         jsonb       NOT NULL DEFAULT '{}',
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX news_articles_latest ON news_articles (published_at DESC, id DESC) WHERE removed_at IS NULL;
CREATE INDEX news_articles_source ON news_articles (source_id, published_at DESC);
CREATE INDEX news_articles_tags ON news_articles USING gin (tags);

CREATE TRIGGER news_articles_updated_at
    BEFORE UPDATE ON news_articles
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Which articles a reader opened. One row per reader and article; the first
-- click is what moves the reader's interests.
CREATE TABLE news_clicks (
    user_id          uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    article_id       uuid        NOT NULL REFERENCES news_articles (id) ON DELETE CASCADE,
    click_count      integer     NOT NULL DEFAULT 1,
    first_clicked_at timestamptz NOT NULL DEFAULT now(),
    last_clicked_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, article_id)
);

CREATE INDEX news_clicks_article ON news_clicks (article_id);

-- Explicit "more / less like this" on an article.
CREATE TABLE news_feedback (
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    article_id uuid        NOT NULL REFERENCES news_articles (id) ON DELETE CASCADE,
    -- tokuly.feed.v1.NewsFeedback: 1 more, 2 less.
    feedback   smallint    NOT NULL CHECK (feedback IN (1, 2)),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, article_id)
);

CREATE INDEX news_feedback_article ON news_feedback (article_id);

-- How much a reader cares about a tag. `score` is the value at `updated_at`;
-- it halves every TOKULY_NEWS_AFFINITY_HALF_LIFE_DAYS, which is applied when
-- the row is read or added to (no periodic rewrite of every row).
CREATE TABLE news_tag_affinity (
    user_id    uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    tag        text             NOT NULL,
    score      double precision NOT NULL,
    updated_at timestamptz      NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, tag)
);

-- Retention sweep (interests that faded to nothing).
CREATE INDEX news_tag_affinity_age ON news_tag_affinity (updated_at);

-- ───────────────────────────── Recaps ─────────────────────────────

-- "Your recap is ready" was sent to a user for the month starting at
-- `period_start`: at most one notice per user and month.
CREATE TABLE feed_recap_notices (
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    period_start date        NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, period_start)
);

CREATE INDEX feed_recap_notices_period ON feed_recap_notices (period_start);
