-- Social: the friend graph, requests, blocks, best-friend ranking,
-- suggestions, invites, bumps, map reactions and map views; follows, follow
-- requests and circles.

-- ───────────────────────────── friend graph ─────────────────────────────

-- A friendship is two rows, one per direction. "Friends of X" is then a
-- primary-key prefix scan, and each side keeps its own preferences about the
-- other (nickname, favourite, mute) and its own best-friend rank.
CREATE TABLE friendships (
    user_id          uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    friend_id        uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.social.v1.FriendSource: how the two met.
    source           integer     NOT NULL DEFAULT 0,
    -- Private to `user_id`.
    nickname         text        NOT NULL DEFAULT '',
    favorite         boolean     NOT NULL DEFAULT false,
    muted            boolean     NOT NULL DEFAULT false,
    -- 1 = closest; 0 = not among the automatically ranked best friends.
    -- Maintained by the ranking worker from `friend_pairs.score`.
    best_friend_rank integer     NOT NULL DEFAULT 0,
    attrs            jsonb       NOT NULL DEFAULT '{}',
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, friend_id),
    CHECK (user_id <> friend_id)
);

CREATE INDEX friendships_friend ON friendships (friend_id);
CREATE INDEX friendships_newest ON friendships (user_id, created_at DESC, friend_id DESC);

CREATE TRIGGER friendships_updated_at
    BEFORE UPDATE ON friendships
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Pending requests. An accepted request becomes two `friendships` rows and is
-- deleted. A declined one stays, hidden from the recipient, so the sender
-- cannot tell (and cannot simply ask again).
CREATE TABLE friend_requests (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    from_user_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    to_user_id   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    source       integer     NOT NULL DEFAULT 0,
    message      text        NOT NULL DEFAULT '',
    declined_at  timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (from_user_id, to_user_id),
    CHECK (from_user_id <> to_user_id)
);

CREATE INDEX friend_requests_incoming ON friend_requests (to_user_id, id DESC) WHERE declined_at IS NULL;
CREATE INDEX friend_requests_outgoing ON friend_requests (from_user_id, id DESC);
-- Declined requests are dropped after a while (see the maintenance worker).
CREATE INDEX friend_requests_declined ON friend_requests (declined_at) WHERE declined_at IS NOT NULL;

CREATE TABLE user_blocks (
    blocker_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    blocked_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (blocker_id, blocked_id),
    CHECK (blocker_id <> blocked_id)
);

CREATE INDEX user_blocks_blocked ON user_blocks (blocked_id);
CREATE INDEX user_blocks_newest ON user_blocks (blocker_id, created_at DESC, blocked_id DESC);

-- ───────────────────────────── shared history & ranking ─────────────────────────────

-- What two users have done together, one row per pair (user_lo < user_hi).
-- It outlives the friendship: bump counts are history, and friends who
-- reconnect get their past back.
CREATE TABLE friend_pairs (
    user_lo               uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    user_hi               uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    bump_count            integer          NOT NULL DEFAULT 0,
    last_bump_at          timestamptz,
    time_together_seconds bigint           NOT NULL DEFAULT 0,
    nights_together       integer          NOT NULL DEFAULT 0,
    -- Minutes together during the night of `night_date` (local date of the
    -- reporting user); a night counts once it reaches the threshold.
    night_minutes         integer          NOT NULL DEFAULT 0,
    night_date            date,
    -- Consecutive weeks with interaction; `streak_week` is the Monday (UTC) of
    -- the last week that counted. A streak whose week is older than last week
    -- is broken: readers treat it as 0 and the next interaction restarts it.
    streak_weeks          integer          NOT NULL DEFAULT 0,
    streak_week           date,
    -- Decayed 30-day interaction score; orders each side's best friends.
    score                 double precision NOT NULL DEFAULT 0,
    -- Messages + stamps exchanged in the last 30 days.
    interaction_score     integer          NOT NULL DEFAULT 0,
    -- Titles derived from the 30-day window (MUTUAL_LOVE, PARTY_PEOPLE, ...).
    titles                text[]           NOT NULL DEFAULT '{}',
    last_interaction_at   timestamptz,
    -- Set by every recorded interaction, cleared by the ranking worker.
    dirty                 boolean          NOT NULL DEFAULT false,
    scored_at             timestamptz,
    PRIMARY KEY (user_lo, user_hi),
    CHECK (user_lo < user_hi)
);

CREATE INDEX friend_pairs_hi ON friend_pairs (user_hi);
CREATE INDEX friend_pairs_dirty ON friend_pairs (user_lo, user_hi) WHERE dirty;
-- Pairs with a score keep being rescored so the score decays.
CREATE INDEX friend_pairs_decay ON friend_pairs (scored_at) WHERE score > 0;

-- The interaction log, bucketed per pair, day (UTC) and kind. `lo_weight` is
-- what user_lo initiated, `hi_weight` what user_hi initiated. Kinds:
-- 1 message, 2 stamp, 3 bump, 4 map view, 5 map reaction, 6 call (minutes),
-- 7 time together (minutes), 8 night together (derived from 7).
CREATE TABLE friend_interaction_days (
    user_lo   uuid     NOT NULL,
    user_hi   uuid     NOT NULL,
    day       date     NOT NULL,
    kind      smallint NOT NULL,
    lo_weight bigint   NOT NULL DEFAULT 0,
    hi_weight bigint   NOT NULL DEFAULT 0,
    PRIMARY KEY (user_lo, user_hi, day, kind),
    FOREIGN KEY (user_lo, user_hi) REFERENCES friend_pairs (user_lo, user_hi) ON DELETE CASCADE
);

CREATE INDEX friend_interaction_days_hi ON friend_interaction_days (user_hi, kind, day);
CREATE INDEX friend_interaction_days_day ON friend_interaction_days (day);

-- ───────────────────────────── suggestions, invites ─────────────────────────────

-- Reasons to suggest someone that cannot be computed on the fly: a contact
-- match (CONTACT), an invite (INVITED_BY / INVITED), or a hint from another
-- domain (NEARBY_OFTEN). Friends of friends are derived at read time.
CREATE TABLE friend_suggestions (
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    suggested_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    reason       text        NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, suggested_id, reason),
    CHECK (user_id <> suggested_id)
);

CREATE INDEX friend_suggestions_suggested ON friend_suggestions (suggested_id);

CREATE TABLE friend_suggestion_dismissals (
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    dismissed_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, dismissed_id)
);

CREATE INDEX friend_suggestion_dismissals_dismissed ON friend_suggestion_dismissals (dismissed_id);

-- One current invite code per user; resetting replaces it.
CREATE TABLE friend_invites (
    user_id    uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    code       text        NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Who joined through whose invite ("intros"). A user is introduced at most once.
CREATE TABLE friend_intros (
    invitee_id uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    inviter_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (invitee_id <> inviter_id)
);

CREATE INDEX friend_intros_inviter ON friend_intros (inviter_id, created_at DESC, invitee_id DESC);

-- ───────────────────────────── interactions ─────────────────────────────

-- A bump is always between two users (user_lo < user_hi); three phones
-- shaking together produce one bump per pair.
CREATE TABLE bumps (
    id                     uuid PRIMARY KEY DEFAULT uuidv7(),
    user_lo                uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    user_hi                uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    lat                    double precision NOT NULL,
    lng                    double precision NOT NULL,
    -- Filled in by the geo domain when it knows the place.
    place_name             text             NOT NULL DEFAULT '',
    -- The two were not friends yet and this bump sent a friend request.
    friend_request_created boolean          NOT NULL DEFAULT false,
    attrs                  jsonb            NOT NULL DEFAULT '{}',
    created_at             timestamptz      NOT NULL DEFAULT now(),
    CHECK (user_lo < user_hi)
);

CREATE INDEX bumps_lo ON bumps (user_lo, id DESC);
CREATE INDEX bumps_hi ON bumps (user_hi, id DESC);

-- Map reactions stay queued (acked_at IS NULL) until the recipient's client
-- confirms it played them.
CREATE TABLE map_reactions (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    from_user_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    to_user_id   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.social.v1.MapReactionKind
    kind         integer     NOT NULL,
    emoji        text        NOT NULL DEFAULT '',
    -- Stamp ID as sent by the client (the stamp domain is not a dependency).
    stamp_id     uuid,
    intensity    integer     NOT NULL DEFAULT 1,
    created_at   timestamptz NOT NULL DEFAULT now(),
    acked_at     timestamptz
);

CREATE INDEX map_reactions_pending ON map_reactions (to_user_id, id) WHERE acked_at IS NULL;
CREATE INDEX map_reactions_from ON map_reactions (from_user_id);
CREATE INDEX map_reactions_created ON map_reactions (created_at);

-- Who opened whose pin. Only the last 24 hours are shown; older rows are pruned.
CREATE TABLE map_views (
    id        uuid PRIMARY KEY DEFAULT uuidv7(),
    viewed_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    viewer_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    viewed_at timestamptz NOT NULL DEFAULT now(),
    CHECK (viewed_id <> viewer_id)
);

CREATE INDEX map_views_viewed ON map_views (viewed_id, viewed_at DESC);
CREATE INDEX map_views_viewer ON map_views (viewer_id);
CREATE INDEX map_views_at ON map_views (viewed_at);

-- ───────────────────────────── follows ─────────────────────────────

-- Follow settings. Without a row an account is unlocked. Accounts of minors
-- (by birthdate) get `locked = true` when they are created, and keep it until
-- their owner unlocks them: the lock is never derived from today's date, so
-- turning 18 changes nothing anybody can see.
CREATE TABLE follow_settings (
    user_id    uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    locked     boolean     NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER follow_settings_updated_at
    BEFORE UPDATE ON follow_settings
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Accounts of minors that exist already start locked too.
INSERT INTO follow_settings (user_id, locked)
SELECT id, true FROM users
WHERE birthdate > current_date - interval '18 years' AND deleted_at IS NULL
ON CONFLICT (user_id) DO NOTHING;

-- `follower_id` follows `followee_id`. The follower's preferences about the
-- followee (mute, notifications) live on the row and are private to them.
CREATE TABLE follows (
    follower_id  uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    followee_id  uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Muted: left out of the follower's timelines. `muted_until` NULL with
    -- `muted` = until unmuted; a past `muted_until` reads as not muted.
    muted        boolean     NOT NULL DEFAULT false,
    muted_until  timestamptz,
    -- tokuly.social.v1.FollowNotifyLevel (1 none, 2 all posts).
    notify_level smallint    NOT NULL DEFAULT 1,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (follower_id, followee_id),
    CHECK (follower_id <> followee_id)
);

CREATE INDEX follows_followers ON follows (followee_id, created_at DESC, follower_id DESC);
CREATE INDEX follows_following ON follows (follower_id, created_at DESC, followee_id DESC);
CREATE INDEX follows_notify ON follows (followee_id) WHERE notify_level = 2;

CREATE TRIGGER follows_updated_at
    BEFORE UPDATE ON follows
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Requests to follow a locked account. An approved request becomes a
-- `follows` row and is deleted. A declined one stays, hidden from the
-- recipient, so the sender keeps seeing it as pending (and cannot ask again).
CREATE TABLE follow_requests (
    id          uuid PRIMARY KEY DEFAULT uuidv7(),
    follower_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    followee_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    declined_at timestamptz,
    created_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (follower_id, followee_id),
    CHECK (follower_id <> followee_id)
);

CREATE INDEX follow_requests_incoming ON follow_requests (followee_id, id DESC) WHERE declined_at IS NULL;
CREATE INDEX follow_requests_outgoing ON follow_requests (follower_id, id DESC);
CREATE INDEX follow_requests_declined ON follow_requests (declined_at) WHERE declined_at IS NOT NULL;

CREATE TABLE follow_suggestion_dismissals (
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    dismissed_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, dismissed_id)
);

CREATE INDEX follow_suggestion_dismissals_dismissed ON follow_suggestion_dismissals (dismissed_id);

-- ───────────────────────────── circles ─────────────────────────────

-- Lists of people to share with: one "close friends" circle per user
-- (kind 1, created on first use) and custom ones (kind 2).
CREATE TABLE circles (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    owner_id   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.social.v1.CircleKind
    kind       smallint    NOT NULL,
    name       text        NOT NULL DEFAULT '',
    emoji      text        NOT NULL DEFAULT '',
    -- tokuly.social.v1.CircleColor
    color      smallint    NOT NULL DEFAULT 0,
    -- Order of the custom circles (close friends always comes first).
    position   integer     NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (kind IN (1, 2))
);

CREATE INDEX circles_owner ON circles (owner_id, position, id);
CREATE UNIQUE INDEX circles_one_close_friends ON circles (owner_id) WHERE kind = 1;

CREATE TRIGGER circles_updated_at
    BEFORE UPDATE ON circles
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Members of a circle: friends or followers of its owner (`owner_id` is
-- repeated here so a relationship that ends can take its member out of all
-- of the owner's circles in one statement).
CREATE TABLE circle_members (
    circle_id uuid        NOT NULL REFERENCES circles (id) ON DELETE CASCADE,
    owner_id  uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    member_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    added_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (circle_id, member_id),
    CHECK (owner_id <> member_id)
);

CREATE INDEX circle_members_newest ON circle_members (circle_id, added_at DESC, member_id DESC);
CREATE INDEX circle_members_member ON circle_members (member_id);
CREATE INDEX circle_members_owner ON circle_members (owner_id, member_id);
