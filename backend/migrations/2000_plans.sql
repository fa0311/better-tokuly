-- Plans: free-time status ("ひま"), plans with RSVP / checklist / cost split,
-- date polls, countdowns, anniversaries and the domain's per-user settings.
--
-- Enum columns hold the numbers of the enums in tokuly.plans.v1.
-- Columns named `*_due_at` drive the workers: they hold the time something is
-- due and are set to NULL once it has been done (or when it no longer applies).

-- ───────────────────────────── ひま ─────────────────────────────

-- At most one status per user; setting a new one replaces the row (and with
-- it the answers to the old one). Expired rows are deleted by the worker.
CREATE TABLE free_statuses (
    id          uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id     uuid        NOT NULL UNIQUE REFERENCES users (id) ON DELETE CASCADE,
    -- FreeWindow the owner picked; the resolved window is starts_at .. ends_at.
    window_kind smallint    NOT NULL,
    starts_at   timestamptz NOT NULL,
    ends_at     timestamptz NOT NULL,
    note        text        NOT NULL DEFAULT '',
    -- Rounded to about 1 km before it is stored; never a precise position.
    area_lat    double precision,
    area_lng    double precision,
    area_name   text        NOT NULL DEFAULT '',
    -- FreeAudience; `chosen_ids` is only used for CHOSEN.
    audience    smallint    NOT NULL,
    chosen_ids  uuid[]      NOT NULL DEFAULT '{}',
    attrs       jsonb       NOT NULL DEFAULT '{}',
    created_at  timestamptz NOT NULL DEFAULT now(),
    CHECK (ends_at > starts_at),
    CHECK ((area_lat IS NULL) = (area_lng IS NULL))
);

CREATE INDEX free_statuses_ends ON free_statuses (ends_at);
CREATE INDEX free_statuses_chosen ON free_statuses USING gin (chosen_ids);

-- "あそぼ!": one answer per friend and status.
CREATE TABLE free_answers (
    status_id  uuid        NOT NULL REFERENCES free_statuses (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    message    text        NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (status_id, user_id)
);

CREATE INDEX free_answers_newest ON free_answers (status_id, created_at DESC, user_id);
CREATE INDEX free_answers_user ON free_answers (user_id);

-- "Tell me when they are free": `user_id` wants to hear about `friend_id`.
-- The last window they were told about keeps it to once per window, and the
-- time they were told to once in a while whatever the windows.
CREATE TABLE free_alerts (
    user_id            uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    friend_id          uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    notified_starts_at timestamptz,
    notified_ends_at   timestamptz,
    notified_at        timestamptz,
    created_at         timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, friend_id),
    CHECK (user_id <> friend_id)
);

CREATE INDEX free_alerts_friend ON free_alerts (friend_id);
CREATE INDEX free_alerts_newest ON free_alerts (user_id, created_at DESC, friend_id);

-- ───────────────────────────── plans ─────────────────────────────

CREATE TABLE plans (
    id                    uuid PRIMARY KEY DEFAULT uuidv7(),
    host_id               uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title                 text        NOT NULL,
    emoji                 text        NOT NULL DEFAULT '',
    description           text        NOT NULL DEFAULT '',
    -- 0 = to be decided, 1 = a time (start_at), 2 = a day (start_date).
    start_kind            smallint    NOT NULL DEFAULT 0,
    start_at              timestamptz,
    start_date            date,
    end_at                timestamptz,
    end_date              date,
    time_zone             text        NOT NULL,
    -- The instant the plan starts (local midnight for a day) and the instant
    -- after which it counts as over; both NULL while the time is undecided.
    sort_at               timestamptz,
    over_at               timestamptz,
    place_name            text        NOT NULL DEFAULT '',
    place_lat             double precision,
    place_lng             double precision,
    place_id              text        NOT NULL DEFAULT '',
    place_address         text        NOT NULL DEFAULT '',
    guests_can_invite     boolean     NOT NULL DEFAULT false,
    -- Most people who can be going; 0 = no limit.
    capacity              integer     NOT NULL DEFAULT 0,
    remind_day_before     boolean     NOT NULL DEFAULT true,
    remind_minutes_before integer     NOT NULL DEFAULT 60,
    -- Reminders and the "attended" events still to be sent (see the worker).
    day_before_due_at     timestamptz,
    before_due_at         timestamptz,
    attended_due_at       timestamptz,
    -- The start time each reminder was last sent for. A reminder that comes
    -- due again for the same start (the host changed the lead time back and
    -- forth) is not sent a second time; one for a new start is.
    day_before_sent_for   timestamptz,
    before_sent_for       timestamptz,
    -- When attendance was announced. Set once: a plan moved to another day
    -- afterwards does not count anyone as having attended a second time.
    attended_at           timestamptz,
    currency              text        NOT NULL DEFAULT 'JPY',
    -- PlanStatus
    status                smallint    NOT NULL DEFAULT 1,
    cancelled_at          timestamptz,
    cancel_reason         text        NOT NULL DEFAULT '',
    -- Taken down by a moderator: invisible to everyone from then on.
    removed_at            timestamptz,
    -- The date poll it came out of (no foreign key: the poll may be deleted).
    poll_id               uuid,
    comment_count         integer     NOT NULL DEFAULT 0,
    attrs                 jsonb       NOT NULL DEFAULT '{}',
    created_at            timestamptz NOT NULL DEFAULT now(),
    updated_at            timestamptz NOT NULL DEFAULT now(),
    CHECK (capacity >= 0),
    CHECK ((place_lat IS NULL) = (place_lng IS NULL))
);

CREATE INDEX plans_host ON plans (host_id);
CREATE INDEX plans_day_before_due ON plans (day_before_due_at) WHERE day_before_due_at IS NOT NULL;
CREATE INDEX plans_before_due ON plans (before_due_at) WHERE before_due_at IS NOT NULL;
CREATE INDEX plans_attended_due ON plans (attended_due_at) WHERE attended_due_at IS NOT NULL;

CREATE TRIGGER plans_updated_at
    BEFORE UPDATE ON plans
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Everyone in a plan, the host included.
CREATE TABLE plan_participants (
    plan_id       uuid        NOT NULL REFERENCES plans (id) ON DELETE CASCADE,
    user_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- PlanRole
    role          smallint    NOT NULL,
    -- Rsvp
    rsvp          smallint    NOT NULL,
    comment       text        NOT NULL DEFAULT '',
    invited_by    uuid,
    responded_at  timestamptz,
    -- Since when they wait for a place (rsvp = WAITLISTED): the queue order.
    waitlisted_at timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (plan_id, user_id)
);

CREATE INDEX plan_participants_user ON plan_participants (user_id, plan_id);

-- People who were in a plan (invited or more) and no longer are: removed,
-- gone by their own choice, or uninvited when a friendship ended or a block
-- was set. Kept so that they can still report the plan, with the title they
-- last saw (see the subject resolver).
CREATE TABLE plan_past_participants (
    plan_id    uuid        NOT NULL REFERENCES plans (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    invited_by uuid,
    title      text        NOT NULL,
    -- They left by themselves (LeavePlan): they are not invited again.
    left_by_choice boolean NOT NULL DEFAULT false,
    left_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (plan_id, user_id)
);

CREATE INDEX plan_past_participants_user ON plan_past_participants (user_id);

-- 持ち物 / やること.
CREATE TABLE plan_checklist_items (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    plan_id    uuid        NOT NULL REFERENCES plans (id) ON DELETE CASCADE,
    -- ChecklistKind
    kind       smallint    NOT NULL,
    text       text        NOT NULL,
    created_by uuid        NOT NULL,
    claimed_by uuid,
    done       boolean     NOT NULL DEFAULT false,
    done_by    uuid,
    done_at    timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX plan_checklist_items_plan ON plan_checklist_items (plan_id, id);

CREATE TRIGGER plan_checklist_items_updated_at
    BEFORE UPDATE ON plan_checklist_items
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- 割り勘: who paid what. Amounts are in the currency's minor unit.
CREATE TABLE plan_expenses (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    plan_id    uuid        NOT NULL REFERENCES plans (id) ON DELETE CASCADE,
    title      text        NOT NULL,
    amount     bigint      NOT NULL,
    currency   text        NOT NULL,
    paid_by    uuid        NOT NULL,
    -- SplitMethod
    method     smallint    NOT NULL,
    created_by uuid        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (amount > 0)
);

CREATE INDEX plan_expenses_plan ON plan_expenses (plan_id, id DESC);

CREATE TRIGGER plan_expenses_updated_at
    BEFORE UPDATE ON plan_expenses
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- What each person owes for an expense; the amounts of an expense's shares
-- always add up to the expense.
CREATE TABLE plan_expense_shares (
    expense_id uuid    NOT NULL REFERENCES plan_expenses (id) ON DELETE CASCADE,
    user_id    uuid    NOT NULL,
    amount     bigint  NOT NULL,
    -- Only for SplitMethod SHARES.
    weight     integer NOT NULL DEFAULT 0,
    PRIMARY KEY (expense_id, user_id),
    CHECK (amount >= 0)
);

CREATE INDEX plan_expense_shares_user ON plan_expense_shares (user_id);

-- Transfers marked as received ("B gave me the 1,200 yen"), recorded by the
-- receiver. One that is taken back stays, with who did it and when, and no
-- longer counts.
CREATE TABLE plan_settlements (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    plan_id      uuid        NOT NULL REFERENCES plans (id) ON DELETE CASCADE,
    from_user_id uuid        NOT NULL,
    to_user_id   uuid        NOT NULL,
    amount       bigint      NOT NULL,
    currency     text        NOT NULL,
    recorded_by  uuid        NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    removed_at   timestamptz,
    removed_by   uuid,
    CHECK (amount > 0),
    CHECK (from_user_id <> to_user_id)
);

CREATE INDEX plan_settlements_plan ON plan_settlements (plan_id, id DESC);

-- ───────────────────────────── date polls ─────────────────────────────

CREATE TABLE date_polls (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    -- The link.
    short_id        text        NOT NULL UNIQUE,
    creator_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title           text        NOT NULL,
    description     text        NOT NULL DEFAULT '',
    time_zone       text        NOT NULL,
    -- Any friend of the creator with the link may answer.
    open_to_friends boolean     NOT NULL DEFAULT false,
    -- PollStatus
    status          smallint    NOT NULL DEFAULT 1,
    deadline        timestamptz,
    -- When the people who have not answered get their automatic reminder.
    remind_due_at   timestamptz,
    -- When that reminder was sent. Once per poll, whatever the deadline is
    -- changed to afterwards.
    reminded_at     timestamptz,
    final_slot_id   uuid,
    plan_id         uuid,
    attrs           jsonb       NOT NULL DEFAULT '{}',
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX date_polls_creator ON date_polls (creator_id);
CREATE INDEX date_polls_deadline ON date_polls (deadline) WHERE status = 1 AND deadline IS NOT NULL;
CREATE INDEX date_polls_remind_due ON date_polls (remind_due_at) WHERE remind_due_at IS NOT NULL;

CREATE TRIGGER date_polls_updated_at
    BEFORE UPDATE ON date_polls
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Candidates: a time (start_at, optional end_at) or a whole day (slot_date).
CREATE TABLE date_poll_slots (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    poll_id    uuid        NOT NULL REFERENCES date_polls (id) ON DELETE CASCADE,
    start_at   timestamptz,
    slot_date  date,
    end_at     timestamptz,
    -- The instant the slot starts (local midnight for a day): the order of
    -- the slots and the "earliest" tie-break of the suggestion.
    sort_at    timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((start_at IS NULL) <> (slot_date IS NULL))
);

CREATE INDEX date_poll_slots_poll ON date_poll_slots (poll_id, sort_at, id);

-- People invited to a poll or who answered it through the link (the creator
-- has a row too, so they can answer like everyone else).
CREATE TABLE date_poll_participants (
    poll_id     uuid        NOT NULL REFERENCES date_polls (id) ON DELETE CASCADE,
    user_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    invited     boolean     NOT NULL DEFAULT false,
    comment     text        NOT NULL DEFAULT '',
    answered_at timestamptz,
    -- The creator was told that this person answered (once, however often
    -- they take their answers back and answer again).
    announced   boolean     NOT NULL DEFAULT false,
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (poll_id, user_id)
);

CREATE INDEX date_poll_participants_user ON date_poll_participants (user_id, poll_id);

-- People who were in a poll and no longer are. Someone who left by
-- themselves is not invited again (through the link of an open poll they
-- can come back on their own); someone the creator removed no longer gets
-- in through the link, until invited again.
CREATE TABLE date_poll_past_participants (
    poll_id uuid        NOT NULL REFERENCES date_polls (id) ON DELETE CASCADE,
    user_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    removed boolean     NOT NULL,
    left_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (poll_id, user_id)
);

-- ○ △ × per slot and person.
CREATE TABLE date_poll_answers (
    slot_id uuid     NOT NULL REFERENCES date_poll_slots (id) ON DELETE CASCADE,
    user_id uuid     NOT NULL,
    poll_id uuid     NOT NULL REFERENCES date_polls (id) ON DELETE CASCADE,
    -- PollAnswer
    answer  smallint NOT NULL,
    PRIMARY KEY (slot_id, user_id)
);

CREATE INDEX date_poll_answers_poll ON date_poll_answers (poll_id, user_id);

-- ───────────────────── countdowns and anniversaries ─────────────────────

CREATE TABLE plan_countdowns (
    id             uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id        uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title          text        NOT NULL,
    emoji          text        NOT NULL DEFAULT '',
    target_date    date        NOT NULL,
    repeats_yearly boolean     NOT NULL DEFAULT false,
    -- CountdownSharing; `shared_ids` is only used for CHOSEN.
    sharing        smallint    NOT NULL DEFAULT 1,
    shared_ids     uuid[]      NOT NULL DEFAULT '{}',
    -- When the owner is told "today is the day" (next occurrence).
    notify_due_at  timestamptz,
    attrs          jsonb       NOT NULL DEFAULT '{}',
    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX plan_countdowns_user ON plan_countdowns (user_id);
CREATE INDEX plan_countdowns_shared ON plan_countdowns USING gin (shared_ids);
CREATE INDEX plan_countdowns_notify_due ON plan_countdowns (notify_due_at) WHERE notify_due_at IS NOT NULL;

CREATE TRIGGER plan_countdowns_updated_at
    BEFORE UPDATE ON plan_countdowns
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- "Days since" counters, one row per pair (user_lo < user_hi) and kind.
-- A couple counter starts as PENDING (an invitation from `created_by`) and
-- only exists for real once the other person accepted.
CREATE TABLE plan_anniversaries (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    -- AnniversaryKind
    kind            smallint    NOT NULL,
    user_lo         uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    user_hi         uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_by      uuid        NOT NULL,
    start_date      date        NOT NULL,
    -- true: the first day is "day 1" (1日目); false: days elapsed.
    count_start_day boolean     NOT NULL DEFAULT true,
    title           text        NOT NULL DEFAULT '',
    emoji           text        NOT NULL DEFAULT '',
    -- AnniversaryStatus
    status          smallint    NOT NULL,
    -- Each side's choice to show a couple counter on their profile.
    show_lo         boolean     NOT NULL DEFAULT false,
    show_hi         boolean     NOT NULL DEFAULT false,
    -- When the next milestone notification is due for either of the two.
    notice_due_at   timestamptz,
    attrs           jsonb       NOT NULL DEFAULT '{}',
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (kind, user_lo, user_hi),
    CHECK (user_lo < user_hi)
);

CREATE INDEX plan_anniversaries_hi ON plan_anniversaries (user_hi);
CREATE INDEX plan_anniversaries_lo ON plan_anniversaries (user_lo);
CREATE INDEX plan_anniversaries_notice_due ON plan_anniversaries (notice_due_at) WHERE notice_due_at IS NOT NULL;

CREATE TRIGGER plan_anniversaries_updated_at
    BEFORE UPDATE ON plan_anniversaries
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Milestones each person was already told about ("d100", "half", "y2"), so
-- nobody hears about one twice. Cleared when the start date changes.
CREATE TABLE plan_anniversary_notices (
    anniversary_id uuid        NOT NULL REFERENCES plan_anniversaries (id) ON DELETE CASCADE,
    user_id        uuid        NOT NULL,
    milestone      text        NOT NULL,
    sent_at        timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (anniversary_id, user_id, milestone)
);

-- ───────────────────────────── settings ─────────────────────────────

CREATE TABLE plan_settings (
    user_id        uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    -- Friends may see my birthday (month and day).
    share_birthday boolean     NOT NULL DEFAULT false,
    attrs          jsonb       NOT NULL DEFAULT '{}',
    updated_at     timestamptz NOT NULL DEFAULT now()
);
