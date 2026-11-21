-- Social: the friend graph, requests, blocks, best-friend ranking,
-- suggestions, invites, bumps, map reactions and map views.

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
