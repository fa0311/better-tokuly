-- Community: communities with members and roles, boards (channels, threads,
-- replies, polls), safety tools (bans, timeouts, review queue, moderation
-- log) and events with RSVP and check-in.
--
-- Enum-like integer columns hold the numbers of the enums in
-- tokuly.community.v1 (CommunityCategory, CommunityVisibility, MemberRole,
-- NotificationLevel, PostingPolicy, PostState, HiddenReason,
-- AttendanceStatus, ModerationAction).

-- ───────────────────────────── communities ─────────────────────────────

CREATE TABLE communities (
    id                        uuid PRIMARY KEY          DEFAULT uuidv7(),
    -- A namespace of its own (not shared with users). A closed community's
    -- handle is replaced with `deleted-<id>` so the name becomes free again.
    handle                    citext           NOT NULL UNIQUE,
    name                      text             NOT NULL,
    description               text             NOT NULL DEFAULT '',
    -- tokuly.common.v1.Image as proto JSON, with the asset that is held for it.
    icon                      jsonb,
    icon_asset_id             uuid,
    banner                    jsonb,
    banner_asset_id           uuid,
    -- [{ "title": …, "body": … }] in display order.
    rules                     jsonb            NOT NULL DEFAULT '[]',
    tags                      text[]           NOT NULL DEFAULT '{}',
    category                  integer          NOT NULL,
    language                  text             NOT NULL DEFAULT 'ja',
    visibility                integer          NOT NULL,
    -- For school and club communities: matched against users.school_name.
    school_name               text             NOT NULL DEFAULT '',
    join_questions            text[]           NOT NULL DEFAULT '{}',
    members_can_invite        boolean          NOT NULL DEFAULT false,
    members_can_create_events boolean          NOT NULL DEFAULT false,
    -- Stored normalized (see text::fold); a post containing one is held for review.
    muted_words               text[]           NOT NULL DEFAULT '{}',
    -- Distinct reporting members that hide a post; 0 = the server's default.
    auto_hide_reports         integer          NOT NULL DEFAULT 0,
    owner_id                  uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    member_count              integer          NOT NULL DEFAULT 0,
    -- Threads in public view (not deleted, not hidden).
    thread_count              integer          NOT NULL DEFAULT 0,
    -- Recent growth and activity; recomputed by a worker.
    trend_score               double precision NOT NULL DEFAULT 0,
    last_activity_at          timestamptz      NOT NULL DEFAULT now(),
    -- Closed. Its posts and events stay readable for moderation (reports,
    -- author lookups) for the retention period, counted from this moment.
    deleted_at                timestamptz,
    -- The retention period is over: its posts and events were marked removed
    -- and are purged by the same pass that purges any removed post. Once
    -- they all are, the row is deleted, and everything of the community
    -- with it (memberships, bans, invites, requests, the log).
    purged_at                 timestamptz,
    attrs                     jsonb            NOT NULL DEFAULT '{}',
    created_at                timestamptz      NOT NULL DEFAULT now(),
    updated_at                timestamptz      NOT NULL DEFAULT now()
);

-- The directory lists communities that are not hidden (visibility 3) and not closed.
CREATE INDEX communities_directory ON communities (category, member_count DESC, id DESC)
    WHERE deleted_at IS NULL AND visibility <> 3;
CREATE INDEX communities_members ON communities (member_count DESC, id DESC)
    WHERE deleted_at IS NULL AND visibility <> 3;
CREATE INDEX communities_active ON communities (last_activity_at DESC, id DESC)
    WHERE deleted_at IS NULL AND visibility <> 3;
CREATE INDEX communities_trending ON communities (trend_score DESC, id DESC)
    WHERE deleted_at IS NULL AND visibility <> 3 AND trend_score > 0;
CREATE INDEX communities_tags ON communities USING gin (tags) WHERE deleted_at IS NULL AND visibility <> 3;
CREATE INDEX communities_school ON communities (school_name, member_count DESC)
    WHERE school_name <> '' AND deleted_at IS NULL AND visibility <> 3;
CREATE INDEX communities_name_trgm ON communities USING gin (name gin_trgm_ops) WHERE deleted_at IS NULL;
CREATE INDEX communities_owner ON communities (owner_id) WHERE deleted_at IS NULL;
CREATE INDEX communities_closed ON communities (deleted_at) WHERE deleted_at IS NOT NULL AND purged_at IS NULL;
-- Finding what shows an upload that was deleted by force (media.asset.deleted).
CREATE INDEX communities_icon_asset ON communities (icon_asset_id) WHERE icon_asset_id IS NOT NULL;
CREATE INDEX communities_banner_asset ON communities (banner_asset_id) WHERE banner_asset_id IS NOT NULL;

CREATE TRIGGER communities_updated_at
    BEFORE UPDATE ON communities
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE TABLE community_members (
    community_id       uuid        NOT NULL REFERENCES communities (id) ON DELETE CASCADE,
    user_id            uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role               integer     NOT NULL DEFAULT 1,
    notification_level integer     NOT NULL DEFAULT 1,
    -- The member chose to show this membership on their profile (public communities only).
    show_on_profile    boolean     NOT NULL DEFAULT false,
    timeout_until      timestamptz,
    timeout_reason     text        NOT NULL DEFAULT '',
    -- "Mark everything as read": nothing older counts as unread.
    read_all_at        timestamptz,
    joined_at          timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, user_id)
);

CREATE INDEX community_members_user ON community_members (user_id, joined_at DESC);
-- The directory: staff first, then by join date.
CREATE INDEX community_members_directory ON community_members (community_id, role DESC, joined_at, user_id);
-- Growth over the last days, for the trending score.
CREATE INDEX community_members_joined ON community_members (joined_at);
CREATE INDEX community_members_shown ON community_members (user_id) WHERE show_on_profile;

-- A request to join a restricted community. A declined request stays (the
-- requester is not told) until it is old enough to be asked again.
CREATE TABLE community_join_requests (
    id           uuid PRIMARY KEY     DEFAULT uuidv7(),
    community_id uuid        NOT NULL REFERENCES communities (id) ON DELETE CASCADE,
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- [{ "question": …, "answer": … }]
    answers      jsonb       NOT NULL DEFAULT '[]',
    declined_at  timestamptz,
    declined_by  uuid,
    created_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (community_id, user_id)
);

CREATE INDEX community_join_requests_pending ON community_join_requests (community_id, id) WHERE declined_at IS NULL;
CREATE INDEX community_join_requests_user ON community_join_requests (user_id);

CREATE TABLE community_invites (
    id           uuid PRIMARY KEY     DEFAULT uuidv7(),
    community_id uuid        NOT NULL REFERENCES communities (id) ON DELETE CASCADE,
    code         text        NOT NULL UNIQUE,
    created_by   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    note         text        NOT NULL DEFAULT '',
    -- 0 = unlimited.
    max_uses     integer     NOT NULL DEFAULT 0,
    use_count    integer     NOT NULL DEFAULT 0,
    expires_at   timestamptz,
    revoked_at   timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX community_invites_community ON community_invites (community_id, id DESC) WHERE revoked_at IS NULL;

CREATE TABLE community_bans (
    community_id uuid        NOT NULL REFERENCES communities (id) ON DELETE CASCADE,
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    reason       text        NOT NULL DEFAULT '',
    banned_by    uuid,
    -- NULL = permanent. Expired bans count as lifted and are pruned by a worker.
    expires_at   timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, user_id)
);

CREATE INDEX community_bans_list ON community_bans (community_id, created_at DESC, user_id);
CREATE INDEX community_bans_user ON community_bans (user_id);
CREATE INDEX community_bans_expiry ON community_bans (expires_at) WHERE expires_at IS NOT NULL;

-- ───────────────────────────── boards ─────────────────────────────

CREATE TABLE community_channels (
    id                uuid PRIMARY KEY     DEFAULT uuidv7(),
    community_id      uuid        NOT NULL REFERENCES communities (id) ON DELETE CASCADE,
    name              text        NOT NULL,
    description       text        NOT NULL DEFAULT '',
    emoji             text        NOT NULL DEFAULT '',
    position          integer     NOT NULL DEFAULT 0,
    posting           integer     NOT NULL DEFAULT 1,
    anonymous         boolean     NOT NULL DEFAULT false,
    slow_mode_seconds integer     NOT NULL DEFAULT 0,
    -- Threads in public view.
    thread_count      integer     NOT NULL DEFAULT 0,
    last_activity_at  timestamptz NOT NULL DEFAULT now(),
    deleted_at        timestamptz,
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX community_channels_list ON community_channels (community_id, position, id) WHERE deleted_at IS NULL;

CREATE TRIGGER community_channels_updated_at
    BEFORE UPDATE ON community_channels
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Rank of a thread in the "hot" order: activity points over age. Age is
-- the mean of the time since the thread was started and the time since its
-- last activity, so a new reply lifts an old thread but never as high as a
-- fresh one. The value changes with the clock: a worker recomputes it for
-- threads active in the last weeks and lets older ones drop to zero.
CREATE FUNCTION community_hot_score(replies integer, likes bigint, comments integer, created timestamptz,
                                    last_activity timestamptz, at timestamptz)
    RETURNS double precision
    LANGUAGE sql
    IMMUTABLE PARALLEL SAFE AS
$$
SELECT (1 + 3 * replies + 2 * likes + comments)::double precision
           / power(2 + (greatest(extract(epoch FROM (at - created)), 0)
                        + greatest(extract(epoch FROM (at - last_activity)), 0)) / 7200.0, 1.5)
$$;

CREATE TABLE community_threads (
    id               uuid PRIMARY KEY          DEFAULT uuidv7(),
    short_id         text             NOT NULL UNIQUE,
    community_id     uuid             NOT NULL REFERENCES communities (id) ON DELETE CASCADE,
    channel_id       uuid             NOT NULL REFERENCES community_channels (id) ON DELETE CASCADE,
    -- Always the real author, also in anonymous channels: moderation needs it.
    -- Nothing member-facing reads it for an anonymous thread.
    author_id        uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Copied from the channel when the thread is started.
    anonymous        boolean          NOT NULL DEFAULT false,
    title            text             NOT NULL,
    body             text             NOT NULL DEFAULT '',
    -- [{ "handle": …, "userId": … }] (tokuly.community.v1.Mention as proto JSON).
    mentions         jsonb            NOT NULL DEFAULT '[]',
    -- [tokuly.common.v1.Image] as proto JSON, with the assets held for them.
    images           jsonb            NOT NULL DEFAULT '[]',
    asset_ids        uuid[]           NOT NULL DEFAULT '{}',
    stamp_id         uuid,
    tags             text[]           NOT NULL DEFAULT '{}',
    pinned_at        timestamptz,
    locked_at        timestamptz,
    announcement     boolean          NOT NULL DEFAULT false,
    -- Set the first time the thread became an announcement: members are
    -- notified once per thread.
    announced_at     timestamptz,
    -- Progress of the notification fan-out: the last member told.
    announce_cursor  uuid,
    announce_done_at timestamptz,
    state            integer          NOT NULL DEFAULT 1,
    hidden_reason    integer          NOT NULL DEFAULT 0,
    hidden_at        timestamptz,
    -- A moderator restored the thread after review: reports no longer hide it.
    reviewed_at      timestamptz,
    -- Replies in public view.
    reply_count      integer          NOT NULL DEFAULT 0,
    -- Number of the latest reply; replies are numbered from 1.
    reply_seq        integer          NOT NULL DEFAULT 0,
    like_count       bigint           NOT NULL DEFAULT 0,
    comment_count    integer          NOT NULL DEFAULT 0,
    hot_score        double precision NOT NULL DEFAULT 0,
    last_activity_at timestamptz      NOT NULL DEFAULT now(),
    edited_at        timestamptz,
    -- Times the title or body was edited; each edit keeps what it replaced
    -- in community_revisions, and the number of edits is capped.
    edit_count       integer          NOT NULL DEFAULT 0,
    -- Removed (by its author, the staff, site moderation, or with its
    -- channel). The row keeps its content and uploads for the retention
    -- period, counted from removal: what was posted stays reportable and
    -- its author can still be looked up and banned.
    deleted_at       timestamptz,
    -- The retention period is over: content and uploads are gone.
    purged_at        timestamptz,
    attrs            jsonb            NOT NULL DEFAULT '{}',
    created_at       timestamptz      NOT NULL DEFAULT now(),
    updated_at       timestamptz      NOT NULL DEFAULT now()
);

-- IDs are UUIDv7, so `id DESC` is "newest first".
CREATE INDEX community_threads_channel_new ON community_threads (channel_id, id DESC) WHERE deleted_at IS NULL;
CREATE INDEX community_threads_community_new ON community_threads (community_id, id DESC) WHERE deleted_at IS NULL;
CREATE INDEX community_threads_channel_hot ON community_threads (channel_id, hot_score DESC, id DESC)
    WHERE deleted_at IS NULL AND state = 1;
CREATE INDEX community_threads_community_hot ON community_threads (community_id, hot_score DESC, id DESC)
    WHERE deleted_at IS NULL AND state = 1;
CREATE INDEX community_threads_community_top ON community_threads (community_id, like_count DESC, reply_count DESC, id DESC)
    WHERE deleted_at IS NULL AND state = 1;
CREATE INDEX community_threads_channel_active ON community_threads (channel_id, last_activity_at DESC)
    WHERE deleted_at IS NULL AND state = 1;
CREATE INDEX community_threads_community_active ON community_threads (community_id, last_activity_at DESC)
    WHERE deleted_at IS NULL AND state = 1;
CREATE INDEX community_threads_pinned ON community_threads (channel_id, pinned_at DESC)
    WHERE pinned_at IS NOT NULL AND deleted_at IS NULL;
CREATE INDEX community_threads_announcements ON community_threads (community_id, announced_at DESC, id DESC)
    WHERE announcement AND deleted_at IS NULL;
CREATE INDEX community_threads_hidden ON community_threads (community_id, hidden_at, id)
    WHERE state = 2 AND deleted_at IS NULL;
CREATE INDEX community_threads_author ON community_threads (author_id, id DESC);
CREATE INDEX community_threads_tags ON community_threads USING gin (tags) WHERE deleted_at IS NULL;
CREATE INDEX community_threads_assets ON community_threads USING gin (asset_ids) WHERE asset_ids <> '{}';
-- What the hot-score worker walks.
CREATE INDEX community_threads_recent ON community_threads (last_activity_at) WHERE deleted_at IS NULL;
-- Activity over the last days, for the trending score.
CREATE INDEX community_threads_created ON community_threads (created_at);
CREATE INDEX community_threads_hot_stale ON community_threads (last_activity_at) WHERE hot_score > 0;
CREATE INDEX community_threads_announcing ON community_threads (announced_at)
    WHERE announced_at IS NOT NULL AND announce_done_at IS NULL;
-- What the retention purge walks.
CREATE INDEX community_threads_removed ON community_threads (deleted_at) WHERE deleted_at IS NOT NULL AND purged_at IS NULL;

CREATE TRIGGER community_threads_updated_at
    BEFORE UPDATE ON community_threads
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE TABLE community_polls (
    thread_id   uuid PRIMARY KEY REFERENCES community_threads (id) ON DELETE CASCADE,
    multiple    boolean     NOT NULL DEFAULT false,
    -- Deadline; NULL = open until closed by hand.
    closes_at   timestamptz,
    closed_at   timestamptz,
    voter_count integer     NOT NULL DEFAULT 0
);

CREATE TABLE community_poll_options (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    thread_id  uuid    NOT NULL REFERENCES community_polls (thread_id) ON DELETE CASCADE,
    position   integer NOT NULL,
    text       text    NOT NULL,
    vote_count integer NOT NULL DEFAULT 0,
    UNIQUE (thread_id, position)
);

CREATE TABLE community_poll_votes (
    thread_id uuid        NOT NULL REFERENCES community_polls (thread_id) ON DELETE CASCADE,
    option_id uuid        NOT NULL REFERENCES community_poll_options (id) ON DELETE CASCADE,
    user_id   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    voted_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (thread_id, user_id, option_id)
);

CREATE INDEX community_poll_votes_user ON community_poll_votes (user_id);

CREATE TABLE community_replies (
    id            uuid PRIMARY KEY     DEFAULT uuidv7(),
    thread_id     uuid        NOT NULL REFERENCES community_threads (id) ON DELETE CASCADE,
    community_id  uuid        NOT NULL REFERENCES communities (id) ON DELETE CASCADE,
    -- The real author; see community_threads.author_id.
    author_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    anonymous     boolean     NOT NULL DEFAULT false,
    -- Position in the thread, from 1.
    seq           integer     NOT NULL,
    body          text        NOT NULL DEFAULT '',
    mentions      jsonb       NOT NULL DEFAULT '[]',
    images        jsonb       NOT NULL DEFAULT '[]',
    asset_ids     uuid[]      NOT NULL DEFAULT '{}',
    stamp_id      uuid,
    -- The reply of the same thread this one answers.
    reply_to_id   uuid        REFERENCES community_replies (id) ON DELETE SET NULL,
    state         integer     NOT NULL DEFAULT 1,
    hidden_reason integer     NOT NULL DEFAULT 0,
    hidden_at     timestamptz,
    reviewed_at   timestamptz,
    like_count    bigint      NOT NULL DEFAULT 0,
    comment_count integer     NOT NULL DEFAULT 0,
    edited_at     timestamptz,
    edit_count    integer     NOT NULL DEFAULT 0,
    -- See community_threads.deleted_at / purged_at.
    deleted_at    timestamptz,
    purged_at     timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    UNIQUE (thread_id, seq)
);

CREATE INDEX community_replies_hidden ON community_replies (community_id, hidden_at, id)
    WHERE state = 2 AND deleted_at IS NULL;
CREATE INDEX community_replies_author ON community_replies (author_id, id DESC);
CREATE INDEX community_replies_reply_to ON community_replies (reply_to_id) WHERE reply_to_id IS NOT NULL;
CREATE INDEX community_replies_assets ON community_replies USING gin (asset_ids) WHERE asset_ids <> '{}';
-- Activity over the last days, for the trending score.
CREATE INDEX community_replies_created ON community_replies (created_at);
CREATE INDEX community_replies_removed ON community_replies (deleted_at) WHERE deleted_at IS NOT NULL AND purged_at IS NULL;

CREATE TRIGGER community_replies_updated_at
    BEFORE UPDATE ON community_replies
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Who follows a thread. A row with `following = false` is a member who
-- unfollowed: replying again does not follow the thread behind their back.
CREATE TABLE community_thread_follows (
    thread_id   uuid        NOT NULL REFERENCES community_threads (id) ON DELETE CASCADE,
    user_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    following   boolean     NOT NULL DEFAULT true,
    -- Set when the follower was notified about a new reply and cleared when
    -- they read the thread: one notification per thread until it is read.
    notified_at timestamptz,
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (thread_id, user_id)
);

CREATE INDEX community_thread_follows_user ON community_thread_follows (user_id) WHERE following;

-- How far a member has read a thread.
CREATE TABLE community_thread_reads (
    thread_id uuid        NOT NULL REFERENCES community_threads (id) ON DELETE CASCADE,
    user_id   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    read_at   timestamptz NOT NULL DEFAULT now(),
    -- The thread's reply_seq when it was read.
    read_seq  integer     NOT NULL DEFAULT 0,
    PRIMARY KEY (thread_id, user_id)
);

CREATE INDEX community_thread_reads_user ON community_thread_reads (user_id);

-- "Mark channel as read".
CREATE TABLE community_channel_reads (
    channel_id uuid        NOT NULL REFERENCES community_channels (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    read_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, user_id)
);

CREATE INDEX community_channel_reads_user ON community_channel_reads (user_id);

-- ───────────────────────────── safety ─────────────────────────────

-- Reports against threads and replies, one row per reporting member
-- (from engagement.report.created). Enough of them hide the post.
CREATE TABLE community_reports (
    subject_type integer     NOT NULL,
    subject_id   uuid        NOT NULL,
    reporter_id  uuid        NOT NULL,
    community_id uuid        NOT NULL REFERENCES communities (id) ON DELETE CASCADE,
    report_id    uuid        NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (subject_type, subject_id, reporter_id)
);

CREATE INDEX community_reports_reporter ON community_reports (reporter_id);

CREATE TABLE community_moderation_log (
    id               uuid PRIMARY KEY     DEFAULT uuidv7(),
    community_id     uuid        NOT NULL REFERENCES communities (id) ON DELETE CASCADE,
    action           integer     NOT NULL,
    -- NULL for automatic actions (and for actors whose account was deleted).
    actor_id         uuid,
    -- The actor used the site-wide moderator role rather than a community role.
    site_moderation  boolean     NOT NULL DEFAULT false,
    target_user_id   uuid,
    -- The action concerns an anonymous post: `target_user_id` is its author,
    -- kept for RevealAuthor's audit trail and never shown in the log.
    target_anonymous boolean     NOT NULL DEFAULT false,
    thread_id        uuid,
    reply_id         uuid,
    channel_id       uuid,
    event_id         uuid,
    reason           text        NOT NULL DEFAULT '',
    details          jsonb       NOT NULL DEFAULT '{}',
    created_at       timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX community_moderation_log_list ON community_moderation_log (community_id, id DESC);
CREATE INDEX community_moderation_log_action ON community_moderation_log (community_id, action, id DESC);
CREATE INDEX community_moderation_log_actor ON community_moderation_log (actor_id) WHERE actor_id IS NOT NULL;
CREATE INDEX community_moderation_log_target ON community_moderation_log (target_user_id) WHERE target_user_id IS NOT NULL;

-- What a thread, a reply or an event said before each edit of its title or
-- text. Members only see that something was edited; the staff and site
-- moderation read the earlier versions, so an edit cannot take back what
-- was reported. Rows go when the post is purged.
CREATE TABLE community_revisions (
    id          uuid PRIMARY KEY     DEFAULT uuidv7(),
    -- 1 = thread, 2 = reply, 3 = event.
    kind        integer     NOT NULL,
    content_id  uuid        NOT NULL,
    title       text        NOT NULL DEFAULT '',
    body        text        NOT NULL DEFAULT '',
    -- When this version was replaced.
    replaced_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX community_revisions_content ON community_revisions (kind, content_id, id);

-- ───────────────────────────── events ─────────────────────────────

CREATE TABLE community_events (
    id                      uuid PRIMARY KEY     DEFAULT uuidv7(),
    community_id            uuid        NOT NULL REFERENCES communities (id) ON DELETE CASCADE,
    organizer_id            uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title                   text        NOT NULL,
    description             text        NOT NULL DEFAULT '',
    starts_at               timestamptz NOT NULL,
    ends_at                 timestamptz NOT NULL,
    online                  boolean     NOT NULL DEFAULT false,
    online_url              text        NOT NULL DEFAULT '',
    place_name              text        NOT NULL DEFAULT '',
    place_address           text        NOT NULL DEFAULT '',
    place_lat               double precision,
    place_lng               double precision,
    cover                   jsonb,
    cover_asset_id          uuid,
    -- 0 = unlimited.
    capacity                integer     NOT NULL DEFAULT 0,
    going_count             integer     NOT NULL DEFAULT 0,
    waitlist_count          integer     NOT NULL DEFAULT 0,
    interested_count        integer     NOT NULL DEFAULT 0,
    attended_count          integer     NOT NULL DEFAULT 0,
    like_count              bigint      NOT NULL DEFAULT 0,
    comment_count           integer     NOT NULL DEFAULT 0,
    -- Reminders already sent: the day before and the hour before.
    reminded_day_at         timestamptz,
    reminded_hour_at        timestamptz,
    cancelled_at            timestamptz,
    cancel_reason           text        NOT NULL DEFAULT '',
    edit_count              integer     NOT NULL DEFAULT 0,
    -- See community_threads.deleted_at / purged_at.
    deleted_at              timestamptz,
    purged_at               timestamptz,
    created_at              timestamptz NOT NULL DEFAULT now(),
    updated_at              timestamptz NOT NULL DEFAULT now(),
    CHECK (ends_at > starts_at)
);

CREATE INDEX community_events_calendar ON community_events (community_id, starts_at, id) WHERE deleted_at IS NULL;
CREATE INDEX community_events_ending ON community_events (community_id, ends_at DESC, id DESC) WHERE deleted_at IS NULL;
-- What the reminder worker scans.
CREATE INDEX community_events_upcoming ON community_events (starts_at)
    WHERE deleted_at IS NULL AND cancelled_at IS NULL AND reminded_hour_at IS NULL;
CREATE INDEX community_events_organizer ON community_events (organizer_id);
CREATE INDEX community_events_cover_asset ON community_events (cover_asset_id) WHERE cover_asset_id IS NOT NULL;
CREATE INDEX community_events_removed ON community_events (deleted_at) WHERE deleted_at IS NOT NULL AND purged_at IS NULL;

CREATE TRIGGER community_events_updated_at
    BEFORE UPDATE ON community_events
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE TABLE community_event_attendees (
    event_id      uuid        NOT NULL REFERENCES community_events (id) ON DELETE CASCADE,
    user_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    status        integer     NOT NULL,
    responded_at  timestamptz NOT NULL DEFAULT now(),
    -- When the member joined the waitlist: first come, first promoted.
    queued_at     timestamptz,
    checked_in_at timestamptz,
    -- The organiser who checked the member in by hand; NULL for a code check-in.
    checked_in_by uuid,
    PRIMARY KEY (event_id, user_id)
);

CREATE INDEX community_event_attendees_status ON community_event_attendees (event_id, status, responded_at, user_id);
CREATE INDEX community_event_attendees_waitlist ON community_event_attendees (event_id, queued_at, user_id) WHERE status = 2;
CREATE INDEX community_event_attendees_user ON community_event_attendees (user_id, status);
