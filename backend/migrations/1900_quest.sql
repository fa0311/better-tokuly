-- Quest: XP and levels, missions, badges, friend streaks, leaderboards, seasons.
--
-- Rules, XP sources, the mission pool and the badge catalogue are data in the
-- crate (`rules.rs`, `missions/pool.rs`, `badges/catalog.rs`); the tables
-- below hold what players earned. Text columns named `source`, `metric`,
-- `mission_key` and `badge_key` hold keys of that data. A key that no longer
-- exists in the code is ignored when read, so removing a rule or a badge
-- needs no migration.
--
-- "day" columns are calendar dates in the player's own time zone.

-- One row per player, created the first time they earn or open anything.
CREATE TABLE quest_profiles (
    user_id              uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    -- Total XP; only ever grows.
    xp                   bigint      NOT NULL DEFAULT 0 CHECK (xp >= 0),
    -- Derived from `xp` (see `levels.rs`); stored so level-ups are detected
    -- exactly once and lists need no arithmetic.
    level                integer     NOT NULL DEFAULT 1 CHECK (level >= 1),
    -- Highest level whose coin reward was granted. Trails `level` only
    -- between a level-up and its grant; whatever is missing is granted the
    -- next time the player earns something.
    rewarded_level       integer     NOT NULL DEFAULT 1,
    -- Badge whose title is shown next to the name; '' = none.
    title_badge          text        NOT NULL DEFAULT '',
    -- Up to 5 badge keys shown on the profile, in order.
    showcase             text[]      NOT NULL DEFAULT '{}',
    -- Latest local day on which the player earned XP ("active players" for
    -- badge rarity, the comeback badge).
    last_active_day      date,
    show_on_leaderboards boolean     NOT NULL DEFAULT true,
    show_on_profile      boolean     NOT NULL DEFAULT true,
    streak_reminders     boolean     NOT NULL DEFAULT true,
    attrs                jsonb       NOT NULL DEFAULT '{}',
    created_at           timestamptz NOT NULL DEFAULT now(),
    updated_at           timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX quest_profiles_active ON quest_profiles (last_active_day) WHERE last_active_day IS NOT NULL;
CREATE INDEX quest_profiles_unrewarded ON quest_profiles (user_id) WHERE rewarded_level < level;

CREATE TRIGGER quest_profiles_updated_at
    BEFORE UPDATE ON quest_profiles
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- XP earned per player, local day and source. This is the XP history, the
-- daily caps (a source never exceeds its cap within a day) and the base of
-- the weekly and seasonal XP rankings.
CREATE TABLE quest_xp_days (
    user_id uuid    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    day     date    NOT NULL,
    source  text    NOT NULL,
    xp      integer NOT NULL CHECK (xp >= 0),
    PRIMARY KEY (user_id, day, source)
);

-- Season snapshots read a whole month across players.
CREATE INDEX quest_xp_days_day ON quest_xp_days (day);

-- What a player did per local day, per metric (messages sent, minutes
-- studied, ...). Mission progress and the metric leaderboards are sums over
-- these rows; old days are pruned.
CREATE TABLE quest_metric_days (
    user_id uuid   NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    day     date   NOT NULL,
    metric  text   NOT NULL,
    value   bigint NOT NULL CHECK (value >= 0),
    PRIMARY KEY (user_id, day, metric)
);

CREATE INDEX quest_metric_days_day ON quest_metric_days (day);

-- Lifetime totals per metric: badge progress and all-time leaderboards.
CREATE TABLE quest_counters (
    user_id uuid   NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    metric  text   NOT NULL,
    value   bigint NOT NULL CHECK (value >= 0),
    PRIMARY KEY (user_id, metric)
);

-- Badges a player owns.
CREATE TABLE quest_badges (
    user_id   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    badge_key text        NOT NULL,
    earned_at timestamptz NOT NULL DEFAULT now(),
    -- Season badges can be won again; everything else stays at 1.
    times     integer     NOT NULL DEFAULT 1 CHECK (times >= 1),
    -- Context of the award, e.g. {"season": "2026-11", "rank": 2}.
    data      jsonb       NOT NULL DEFAULT '{}',
    PRIMARY KEY (user_id, badge_key)
);

CREATE INDEX quest_badges_key ON quest_badges (badge_key);

-- Rarity: how many players own each badge, recomputed by a worker.
CREATE TABLE quest_badge_stats (
    badge_key      text PRIMARY KEY,
    -- Everyone who owns it.
    owners         bigint           NOT NULL DEFAULT 0,
    -- Owners among the active players (XP within the last 30 local days).
    active_owners  bigint           NOT NULL DEFAULT 0,
    active_players bigint           NOT NULL DEFAULT 0,
    -- active_owners / active_players, 0-1.
    share          double precision NOT NULL DEFAULT 0,
    computed_at    timestamptz      NOT NULL DEFAULT now()
);

-- A player's missions. A daily mission belongs to one local day, a weekly
-- one to the week starting on `period_start` (a Monday). The definition is
-- copied in when the mission is drawn, so later changes to the pool never
-- alter a mission somebody is working on.
CREATE TABLE quest_missions (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.quest.v1.MissionPeriod: 1 daily, 2 weekly.
    period       smallint    NOT NULL CHECK (period IN (1, 2)),
    period_start date        NOT NULL,
    -- Position within the period's set (0-2 daily, 0 weekly).
    slot         smallint    NOT NULL,
    mission_key  text        NOT NULL,
    metric       text        NOT NULL,
    target       bigint      NOT NULL CHECK (target > 0),
    -- Value of the period's metric counter when the mission started counting:
    -- 0 for a drawn mission (the whole period counts), the counter at that
    -- moment for a rerolled one (it starts from zero).
    baseline     bigint      NOT NULL DEFAULT 0,
    progress     bigint      NOT NULL DEFAULT 0,
    reward_coins integer     NOT NULL DEFAULT 0,
    reward_xp    integer     NOT NULL DEFAULT 0,
    -- Badge granted with the claim; '' = none.
    reward_badge text        NOT NULL DEFAULT '',
    rerolled     boolean     NOT NULL DEFAULT false,
    completed_at timestamptz,
    -- The claim: XP, counters and the badge are granted in its transaction.
    claimed_at   timestamptz,
    -- The coins of the claim reached the wallet. Trails `claimed_at` only
    -- for a moment; a worker pays whatever stayed behind.
    paid_at      timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (user_id, period, period_start, slot)
);

-- Completed missions whose reward still waits.
CREATE INDEX quest_missions_unclaimed ON quest_missions (user_id, completed_at)
    WHERE completed_at IS NOT NULL AND claimed_at IS NULL;
CREATE INDEX quest_missions_period_start ON quest_missions (period_start);
CREATE INDEX quest_missions_unpaid ON quest_missions (user_id, claimed_at) WHERE claimed_at IS NOT NULL AND paid_at IS NULL;

-- Per player and local day: the reroll allowance and the bonus for claiming
-- all three daily missions.
CREATE TABLE quest_mission_days (
    user_id          uuid     NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    day              date     NOT NULL,
    rerolls_used     smallint NOT NULL DEFAULT 0,
    bonus_granted_at timestamptz,
    -- What the bonus pays, fixed when it was granted.
    bonus_coins      integer  NOT NULL DEFAULT 0,
    -- The bonus coins reached the wallet (see quest_missions.paid_at).
    bonus_paid_at    timestamptz,
    PRIMARY KEY (user_id, day)
);

CREATE INDEX quest_mission_days_unpaid ON quest_mission_days (user_id, bonus_granted_at) WHERE bonus_granted_at IS NOT NULL AND bonus_paid_at IS NULL;

-- One row per pair of friends who ever did something towards each other.
--
-- A streak day is a calendar date; each side's part counts on the date of
-- their own time zone. Dates are settled in order: a date is complete when
-- both did their part, and missed when a side that did not has run out of
-- day. `lo_days` / `hi_days` hold the dates after `settled_day` on which
-- that side already did their part (the two may be on different dates, so
-- a side can be ahead). See `streaks/days.rs` for the whole state machine.
CREATE TABLE quest_streaks (
    user_lo       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    user_hi       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Consecutive completed dates; 0 = no streak is running.
    count         integer     NOT NULL DEFAULT 0 CHECK (count >= 0),
    -- Longest the two ever reached.
    best          integer     NOT NULL DEFAULT 0,
    -- Every date up to this one is settled (completed or missed for good);
    -- parts reported for it or anything earlier are ignored. While a streak
    -- runs this is its latest completed date.
    settled_day   date,
    -- Date of the first completed day of the running streak.
    started_on    date,
    lo_days       date[]      NOT NULL DEFAULT '{}',
    hi_days       date[]      NOT NULL DEFAULT '{}',
    -- When the running streak ends unless the missing part arrives: the
    -- earliest local midnight among the sides that have not done their part
    -- for the date after `settled_day`. NULL while count = 0.
    expires_at    timestamptz,
    -- Date for which each side was already warned ("expires soon").
    lo_warned     date,
    hi_warned     date,
    -- The streak that ended last, restorable for 24 hours after `lost_at`.
    lost_count    integer     NOT NULL DEFAULT 0,
    lost_at       timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_lo, user_hi),
    CHECK (user_lo < user_hi)
);

CREATE INDEX quest_streaks_hi ON quest_streaks (user_hi);
-- The expiry sweep and the "expires soon" reminders.
CREATE INDEX quest_streaks_expiry ON quest_streaks (expires_at) WHERE count > 0;

CREATE TRIGGER quest_streaks_updated_at
    BEFORE UPDATE ON quest_streaks
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Paid streak restores: at most one per player and calendar month (the
-- unique key is both the limit and what makes a repeated request harmless).
--
-- A row is an order. It is placed, with what it is for, before any coins
-- move; then the wallet is charged (idempotent on the order's ID) and the
-- streak restored, which sets `applied_at`. An order that stopped half way
-- is finished from wherever it is seen next: it is either applied, or — when
-- there is nothing left to restore — refunded and deleted.
CREATE TABLE quest_streak_restores (
    id             uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id        uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- First day of the month, in the player's time zone.
    month          date        NOT NULL,
    friend_id      uuid        NOT NULL,
    cost           bigint      NOT NULL,
    -- The loss the order is for: the streak's `lost_at` when it was placed.
    lost_at        timestamptz NOT NULL,
    -- Length of the streak that came back (set when applied).
    restored_count integer     NOT NULL DEFAULT 0,
    -- The wallet entry that paid for it.
    entry_id       uuid,
    -- NULL while the order is placed but not finished.
    applied_at     timestamptz,
    created_at     timestamptz NOT NULL DEFAULT now(),
    UNIQUE (user_id, month)
);

CREATE INDEX quest_streak_restores_pending ON quest_streak_restores (created_at) WHERE applied_at IS NULL;

-- When each rule last fired for anyone. Missions and leaderboards that
-- depend on a rule nobody triggers (a domain that does not exist yet, a
-- feature nobody uses) are not offered.
CREATE TABLE quest_rule_activity (
    rule         text PRIMARY KEY,
    last_seen_at timestamptz NOT NULL
);

-- Finished seasons (calendar months) and every participant's final result.
CREATE TABLE quest_seasons (
    -- First day of the month.
    month        date PRIMARY KEY,
    participants integer     NOT NULL DEFAULT 0,
    -- NULL while the final ranking is being computed.
    finalized_at timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE quest_season_results (
    month      date    NOT NULL REFERENCES quest_seasons (month) ON DELETE CASCADE,
    user_id    uuid    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    xp         bigint  NOT NULL,
    -- Rank among the player and their friends who took part; group_size
    -- counts them (the player included).
    rank       integer NOT NULL,
    group_size integer NOT NULL,
    badge_won  boolean NOT NULL DEFAULT false,
    PRIMARY KEY (month, user_id)
);

CREATE INDEX quest_season_results_user ON quest_season_results (user_id, month DESC);

-- Who the two members of a direct conversation are, learned from
-- `chat.conversation.created`. The chat domain announces messages without
-- their recipient; this lets a direct message count towards the streak of
-- the two. No foreign key on the conversation: chat's tables are not ours.
CREATE TABLE quest_direct_chats (
    conversation_id uuid PRIMARY KEY,
    user_lo         uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    user_hi         uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE
);

CREATE INDEX quest_direct_chats_lo ON quest_direct_chats (user_lo);
CREATE INDEX quest_direct_chats_hi ON quest_direct_chats (user_hi);
