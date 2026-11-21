-- School: timetables, study sessions and study rooms, exams and assignments.
--
-- Calendar dates (`date` columns) and times of day are in the owner's time
-- zone (`users.time_zone`); instants are `timestamptz`.

-- ───────────────────────────── settings ─────────────────────────────

-- One row per user who changed a setting (or has a pending class reminder);
-- everyone else has the defaults.
CREATE TABLE school_settings (
    user_id              uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    -- Minutes before the first class of the day for `school.class_soon`; 0 = off.
    class_soon_minutes   integer     NOT NULL DEFAULT 0 CHECK (class_soon_minutes BETWEEN 0 AND 120),
    -- When the reminder worker looks at this user next (NULL = never: off).
    class_soon_due_at    timestamptz,
    -- The local date the last reminder was sent for (one per day).
    class_soon_sent_on   date,
    -- tokuly.school.v1.StudyVisibility: 1 friends, 2 nobody.
    study_visibility     smallint    NOT NULL DEFAULT 1,
    share_study_activity boolean     NOT NULL DEFAULT true,
    join_ranking         boolean     NOT NULL DEFAULT false,
    cheer_notifications  boolean     NOT NULL DEFAULT false,
    daily_goal_minutes   integer     NOT NULL DEFAULT 0 CHECK (daily_goal_minutes BETWEEN 0 AND 1440),
    -- Default pomodoro lengths in minutes.
    pomodoro_work        integer     NOT NULL DEFAULT 25,
    pomodoro_break       integer     NOT NULL DEFAULT 5,
    pomodoro_long_break  integer     NOT NULL DEFAULT 15,
    pomodoro_rounds      integer     NOT NULL DEFAULT 4,
    assignment_reminders boolean     NOT NULL DEFAULT true,
    attrs                jsonb       NOT NULL DEFAULT '{}',
    created_at           timestamptz NOT NULL DEFAULT now(),
    updated_at           timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX school_settings_class_soon ON school_settings (class_soon_due_at) WHERE class_soon_due_at IS NOT NULL;

CREATE TRIGGER school_settings_updated_at
    BEFORE UPDATE ON school_settings
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- ───────────────────────────── subjects ─────────────────────────────

-- A user's subjects, shared by their terms, sessions, exams and assignments.
CREATE TABLE school_subjects (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name       text        NOT NULL,
    short_name text        NOT NULL DEFAULT '',
    -- #rrggbb
    color      text        NOT NULL,
    emoji      text        NOT NULL DEFAULT '',
    teacher    text        NOT NULL DEFAULT '',
    room       text        NOT NULL DEFAULT '',
    -- Hidden from pickers but kept for the history that refers to it.
    archived   boolean     NOT NULL DEFAULT false,
    attrs      jsonb       NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX school_subjects_name ON school_subjects (user_id, lower(name));

CREATE TRIGGER school_subjects_updated_at
    BEFORE UPDATE ON school_subjects
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- ───────────────────────────── timetable ─────────────────────────────

CREATE TABLE school_terms (
    id                      uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id                 uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name                    text        NOT NULL,
    start_date              date        NOT NULL,
    end_date                date        NOT NULL,
    is_current              boolean     NOT NULL DEFAULT false,
    -- tokuly.school.v1.PeriodLayout as proto JSON.
    layout                  jsonb       NOT NULL,
    alternating_weeks       boolean     NOT NULL DEFAULT false,
    -- A date inside a week that runs as week A.
    week_a_date             date,
    observe_public_holidays boolean     NOT NULL DEFAULT true,
    -- tokuly.school.v1.TimetableAudience: 1 nobody, 2 chosen, 3 friends.
    audience                smallint    NOT NULL DEFAULT 1,
    copied_from_user_id     uuid REFERENCES users (id) ON DELETE SET NULL,
    attrs                   jsonb       NOT NULL DEFAULT '{}',
    created_at              timestamptz NOT NULL DEFAULT now(),
    updated_at              timestamptz NOT NULL DEFAULT now(),
    CHECK (end_date >= start_date)
);

CREATE INDEX school_terms_owner ON school_terms (user_id, start_date DESC, id DESC);
CREATE UNIQUE INDEX school_terms_current ON school_terms (user_id) WHERE is_current;

CREATE TRIGGER school_terms_updated_at
    BEFORE UPDATE ON school_terms
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- The chosen friends of a term whose audience is "chosen".
CREATE TABLE school_term_shares (
    term_id    uuid        NOT NULL REFERENCES school_terms (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (term_id, user_id)
);

CREATE INDEX school_term_shares_user ON school_term_shares (user_id);

-- The weekly grid. A slot (weekday, period) holds one class for every week,
-- or one for week A and/or one for week B (enforced by the code that writes).
CREATE TABLE school_entries (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    term_id      uuid        NOT NULL REFERENCES school_terms (id) ON DELETE CASCADE,
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- 1 = Monday ... 7 = Sunday.
    weekday      smallint    NOT NULL CHECK (weekday BETWEEN 1 AND 7),
    period       smallint    NOT NULL CHECK (period BETWEEN 1 AND 12),
    -- tokuly.school.v1.WeekPattern: 1 every week, 2 week A, 3 week B.
    week_pattern smallint    NOT NULL DEFAULT 1 CHECK (week_pattern BETWEEN 1 AND 3),
    subject_id   uuid        NOT NULL REFERENCES school_subjects (id) ON DELETE CASCADE,
    -- Empty = as the subject says.
    teacher      text        NOT NULL DEFAULT '',
    room         text        NOT NULL DEFAULT '',
    color        text        NOT NULL DEFAULT '',
    memo         text        NOT NULL DEFAULT '',
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (term_id, weekday, period, week_pattern)
);

CREATE INDEX school_entries_subject ON school_entries (subject_id);
CREATE INDEX school_entries_user ON school_entries (user_id);

CREATE TRIGGER school_entries_updated_at
    BEFORE UPDATE ON school_entries
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- One-off changes: at most one per slot and date.
CREATE TABLE school_changes (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    term_id    uuid        NOT NULL REFERENCES school_terms (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    date       date        NOT NULL,
    period     smallint    NOT NULL CHECK (period BETWEEN 1 AND 12),
    -- tokuly.school.v1.ChangeKind: 1 cancelled, 2 moved, 3 replaced, 4 extra.
    kind       smallint    NOT NULL CHECK (kind BETWEEN 1 AND 4),
    subject_id uuid REFERENCES school_subjects (id) ON DELETE CASCADE,
    teacher    text        NOT NULL DEFAULT '',
    room       text        NOT NULL DEFAULT '',
    memo       text        NOT NULL DEFAULT '',
    -- Moved: the slot the class goes to.
    to_date    date,
    to_period  smallint CHECK (to_period BETWEEN 1 AND 12),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (term_id, date, period),
    CHECK ((kind = 2) = (to_date IS NOT NULL AND to_period IS NOT NULL))
);

CREATE INDEX school_changes_target ON school_changes (term_id, to_date) WHERE to_date IS NOT NULL;
CREATE INDEX school_changes_subject ON school_changes (subject_id) WHERE subject_id IS NOT NULL;
CREATE INDEX school_changes_user ON school_changes (user_id);

-- Holidays, class days and pinned A/B weeks, for a date or a range.
CREATE TABLE school_day_overrides (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    term_id         uuid        NOT NULL REFERENCES school_terms (id) ON DELETE CASCADE,
    user_id         uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    start_date      date        NOT NULL,
    end_date        date        NOT NULL,
    -- tokuly.school.v1.DayOverrideKind: 1 holiday, 2 class day, 3 week pattern.
    kind            smallint    NOT NULL CHECK (kind BETWEEN 1 AND 3),
    name            text        NOT NULL DEFAULT '',
    follows_weekday smallint CHECK (follows_weekday BETWEEN 1 AND 7),
    week_pattern    smallint CHECK (week_pattern BETWEEN 2 AND 3),
    created_at      timestamptz NOT NULL DEFAULT now(),
    CHECK (end_date >= start_date)
);

CREATE INDEX school_day_overrides_term ON school_day_overrides (term_id, start_date);
CREATE INDEX school_day_overrides_user ON school_day_overrides (user_id);

-- ───────────────────────────── study rooms ─────────────────────────────

-- A room's members are the active study sessions that point at it
-- (`school_study_sessions.room_id`), so presence and study time cannot
-- disagree. The timer is a position on a timeline of work and break phases:
-- while running, the position at time t is `timer_offset_ms + (t - timer_anchor_at)`.
CREATE TABLE school_rooms (
    id                 uuid PRIMARY KEY DEFAULT uuidv7(),
    host_id            uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title              text        NOT NULL,
    -- tokuly.school.v1.RoomAudience: 1 public, 2 friends of the host, 3 invited.
    audience           smallint    NOT NULL CHECK (audience BETWEEN 1 AND 3),
    member_cap         integer     NOT NULL,
    -- tokuly.school.v1.StudyMode: 1 stopwatch (open-ended), 2 pomodoro.
    mode               smallint    NOT NULL CHECK (mode BETWEEN 1 AND 2),
    work_seconds       integer     NOT NULL DEFAULT 0,
    break_seconds      integer     NOT NULL DEFAULT 0,
    long_break_seconds integer     NOT NULL DEFAULT 0,
    rounds             integer     NOT NULL DEFAULT 0,
    -- tokuly.school.v1.TimerState: 1 idle, 2 running, 3 paused.
    timer_state        smallint    NOT NULL DEFAULT 1 CHECK (timer_state BETWEEN 1 AND 3),
    timer_anchor_at    timestamptz,
    timer_offset_ms    bigint      NOT NULL DEFAULT 0,
    -- Running pomodoro: when the current phase ends (the worker announces it).
    phase_ends_at      timestamptz,
    -- Kept in step with the active sessions, under the room's row lock.
    member_count       integer     NOT NULL DEFAULT 0,
    -- Since when nobody is inside; the room closes after a while.
    empty_since        timestamptz,
    closed_at          timestamptz,
    -- tokuly.school.v1.RoomCloseReason.
    close_reason       smallint,
    attrs              jsonb       NOT NULL DEFAULT '{}',
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now()
);

-- One open room per host.
CREATE UNIQUE INDEX school_rooms_host_open ON school_rooms (host_id) WHERE closed_at IS NULL;
CREATE INDEX school_rooms_public ON school_rooms (member_count DESC, id DESC) WHERE closed_at IS NULL AND audience = 1;
CREATE INDEX school_rooms_phase ON school_rooms (phase_ends_at) WHERE closed_at IS NULL AND phase_ends_at IS NOT NULL;
CREATE INDEX school_rooms_empty ON school_rooms (empty_since) WHERE closed_at IS NULL AND empty_since IS NOT NULL;

CREATE TRIGGER school_rooms_updated_at
    BEFORE UPDATE ON school_rooms
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE TABLE school_room_invites (
    room_id    uuid        NOT NULL REFERENCES school_rooms (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (room_id, user_id)
);

CREATE INDEX school_room_invites_user ON school_room_invites (user_id);

-- People the host removed; they cannot come back to that room.
CREATE TABLE school_room_bans (
    room_id    uuid        NOT NULL REFERENCES school_rooms (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (room_id, user_id)
);

CREATE INDEX school_room_bans_user ON school_room_bans (user_id);

-- ───────────────────────────── study sessions ─────────────────────────────

-- A session and its timer. Study time is counted into `school_study_segments`
-- up to `checkpoint_at`. `heartbeat_at` is when the client was last heard
-- from: a session the server has to end itself (the client vanished, the
-- room closed) keeps the time counted up to that moment and nothing after.
CREATE TABLE school_study_sessions (
    id                 uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id            uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    subject_id         uuid REFERENCES school_subjects (id),
    memo               text        NOT NULL DEFAULT '',
    -- tokuly.school.v1.SessionSource: 1 timer, 2 room, 3 manual.
    source             smallint    NOT NULL CHECK (source BETWEEN 1 AND 3),
    -- tokuly.school.v1.SessionState: 1 running, 2 paused, 3 ended.
    state              smallint    NOT NULL CHECK (state BETWEEN 1 AND 3),
    started_at         timestamptz NOT NULL,
    ended_at           timestamptz,
    -- The session's own timer (source 1). Room sessions follow the room's.
    -- tokuly.school.v1.StudyMode: 1 stopwatch, 2 pomodoro.
    mode               smallint    NOT NULL DEFAULT 1 CHECK (mode BETWEEN 1 AND 2),
    work_seconds       integer     NOT NULL DEFAULT 0,
    break_seconds      integer     NOT NULL DEFAULT 0,
    long_break_seconds integer     NOT NULL DEFAULT 0,
    rounds             integer     NOT NULL DEFAULT 0,
    timer_anchor_at    timestamptz,
    timer_offset_ms    bigint      NOT NULL DEFAULT 0,
    -- Study time is counted up to here.
    checkpoint_at      timestamptz NOT NULL,
    -- The last sign of life from the client (start, heartbeat, pause, resume...).
    heartbeat_at       timestamptz NOT NULL,
    paused_at          timestamptz,
    -- Sum of the session's segments, in whole seconds.
    counted_seconds    integer     NOT NULL DEFAULT 0,
    auto_stopped       boolean     NOT NULL DEFAULT false,
    cheer_count        integer     NOT NULL DEFAULT 0,
    room_id            uuid REFERENCES school_rooms (id) ON DELETE SET NULL,
    -- When the user entered the room (survives a change of subject, which
    -- starts a new session).
    room_joined_at     timestamptz,
    attrs              jsonb       NOT NULL DEFAULT '{}',
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now(),
    CHECK ((state = 3) = (ended_at IS NOT NULL))
);

-- At most one active session per user.
CREATE UNIQUE INDEX school_study_sessions_active ON school_study_sessions (user_id) WHERE state <> 3;
CREATE INDEX school_study_sessions_log ON school_study_sessions (user_id, started_at DESC, id DESC);
CREATE INDEX school_study_sessions_idle ON school_study_sessions (heartbeat_at) WHERE state <> 3;
CREATE INDEX school_study_sessions_room ON school_study_sessions (room_id) WHERE state <> 3 AND room_id IS NOT NULL;
CREATE INDEX school_study_sessions_subject ON school_study_sessions (subject_id) WHERE subject_id IS NOT NULL;

CREATE TRIGGER school_study_sessions_updated_at
    BEFORE UPDATE ON school_study_sessions
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Stretches of counted study time. Every total (day, week, subject, ranking,
-- heatmap) is a sum over these, cut at the local midnights of whoever asks.
CREATE TABLE school_study_segments (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    session_id uuid        NOT NULL REFERENCES school_study_sessions (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    started_at timestamptz NOT NULL,
    ended_at   timestamptz NOT NULL,
    CHECK (ended_at > started_at)
);

CREATE INDEX school_study_segments_user ON school_study_segments (user_id, started_at);
CREATE INDEX school_study_segments_session ON school_study_segments (session_id, ended_at);

CREATE TABLE school_study_cheers (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    session_id   uuid        NOT NULL REFERENCES school_study_sessions (id) ON DELETE CASCADE,
    from_user_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    to_user_id   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX school_study_cheers_session ON school_study_cheers (session_id, id DESC);
CREATE INDEX school_study_cheers_from ON school_study_cheers (from_user_id);
CREATE INDEX school_study_cheers_to ON school_study_cheers (to_user_id);

-- ───────────────────────────── exams ─────────────────────────────

CREATE TABLE school_exams (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id         uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name            text        NOT NULL,
    start_date      date        NOT NULL,
    end_date        date        NOT NULL,
    -- Friends see "テスト期間" on the profile.
    show_on_profile boolean     NOT NULL DEFAULT false,
    memo            text        NOT NULL DEFAULT '',
    attrs           jsonb       NOT NULL DEFAULT '{}',
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    CHECK (end_date >= start_date)
);

CREATE INDEX school_exams_owner ON school_exams (user_id, start_date);

CREATE TRIGGER school_exams_updated_at
    BEFORE UPDATE ON school_exams
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE TABLE school_exam_subjects (
    id          uuid PRIMARY KEY DEFAULT uuidv7(),
    exam_id     uuid        NOT NULL REFERENCES school_exams (id) ON DELETE CASCADE,
    user_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    subject_id  uuid        NOT NULL REFERENCES school_subjects (id),
    date        date,
    -- 0 = not known.
    period      smallint    NOT NULL DEFAULT 0 CHECK (period BETWEEN 0 AND 12),
    scope       text        NOT NULL DEFAULT '',
    -- The private result; `has_result` tells "no result" from zero points.
    has_result  boolean     NOT NULL DEFAULT false,
    score       integer,
    max_score   integer     NOT NULL DEFAULT 100,
    average     double precision,
    rank        integer,
    result_memo text        NOT NULL DEFAULT '',
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (exam_id, subject_id)
);

CREATE INDEX school_exam_subjects_subject ON school_exam_subjects (subject_id);
CREATE INDEX school_exam_subjects_user ON school_exam_subjects (user_id);

CREATE TRIGGER school_exam_subjects_updated_at
    BEFORE UPDATE ON school_exam_subjects
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- ───────────────────────────── assignments ─────────────────────────────

CREATE TABLE school_assignments (
    id                uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id           uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    subject_id        uuid REFERENCES school_subjects (id),
    title             text        NOT NULL,
    memo              text        NOT NULL DEFAULT '',
    due_at            timestamptz NOT NULL,
    done_at           timestamptz,
    remind            boolean     NOT NULL DEFAULT true,
    -- Pending reminders: the evening before and one hour before. NULL once
    -- sent, or when the moment had passed already when the assignment was saved.
    remind_evening_at timestamptz,
    remind_hour_at    timestamptz,
    attrs             jsonb       NOT NULL DEFAULT '{}',
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX school_assignments_open ON school_assignments (user_id, due_at, id) WHERE done_at IS NULL;
CREATE INDEX school_assignments_done ON school_assignments (user_id, done_at DESC, id DESC) WHERE done_at IS NOT NULL;
CREATE INDEX school_assignments_subject ON school_assignments (subject_id) WHERE subject_id IS NOT NULL;
CREATE INDEX school_assignments_evening ON school_assignments (remind_evening_at) WHERE remind_evening_at IS NOT NULL AND done_at IS NULL;
CREATE INDEX school_assignments_hour ON school_assignments (remind_hour_at) WHERE remind_hour_at IS NOT NULL AND done_at IS NULL;

CREATE TRIGGER school_assignments_updated_at
    BEFORE UPDATE ON school_assignments
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();
