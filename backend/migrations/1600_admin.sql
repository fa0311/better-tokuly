-- Admin: the moderation queue, sanctions, announcements and the audit log.
--
-- Staff and reporter references use ON DELETE SET NULL (not CASCADE): the
-- moderation record and the audit trail must outlive the accounts involved.

-- ───────────────────────────── moderation queue ─────────────────────────────

-- Reports about one subject are handled together as a case. At most one case
-- per subject is open at a time; a report arriving after a case was resolved
-- opens a new one.
CREATE TABLE moderation_cases (
    id                uuid PRIMARY KEY DEFAULT uuidv7(),
    -- tokuly.common.v1.SubjectType
    subject_type      integer     NOT NULL,
    subject_id        uuid        NOT NULL,
    -- NULL once the owner's account is deleted.
    subject_owner_id  uuid REFERENCES users (id) ON DELETE SET NULL,
    -- The content hides its author (a post in an anonymous channel): the owner
    -- above came from the domain that keeps it, not from the report, and is
    -- shown only to a moderator who is not involved, one case at a time.
    owner_hidden      boolean     NOT NULL DEFAULT false,
    -- Snapshot from the latest report: the content itself lives in its domain.
    subject_title     text        NOT NULL DEFAULT '',
    subject_path      text        NOT NULL DEFAULT '',
    status            text        NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'resolved')),
    report_count      integer     NOT NULL DEFAULT 0,
    -- Distinct report categories (SPAM, HARASSMENT, ...) in the order first seen.
    categories        text[]      NOT NULL DEFAULT '{}',
    assignee_id       uuid REFERENCES users (id) ON DELETE SET NULL,
    assigned_at       timestamptz,
    last_reported_at  timestamptz NOT NULL DEFAULT now(),
    -- tokuly.admin.v1.ModerationActionKind chosen when resolving.
    resolution        integer,
    content_removed   boolean     NOT NULL DEFAULT false,
    resolution_reason text        NOT NULL DEFAULT '',
    resolved_by       uuid REFERENCES users (id) ON DELETE SET NULL,
    resolved_at       timestamptz,
    -- The uploads of the subject are held for the case (media holder
    -- `admin:case:<id>`) until TOKULY_MODERATION_EVIDENCE_DAYS after it is resolved.
    evidence_held     boolean     NOT NULL DEFAULT false,
    attrs             jsonb       NOT NULL DEFAULT '{}',
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX moderation_cases_open_subject ON moderation_cases (subject_type, subject_id) WHERE status = 'open';
CREATE INDEX moderation_cases_subject ON moderation_cases (subject_type, subject_id, id DESC);
CREATE INDEX moderation_cases_recent ON moderation_cases (status, last_reported_at DESC, id DESC);
CREATE INDEX moderation_cases_oldest ON moderation_cases (status, created_at, id);
CREATE INDEX moderation_cases_most_reported ON moderation_cases (status, report_count DESC, id DESC);
CREATE INDEX moderation_cases_owner ON moderation_cases (subject_owner_id, id DESC) WHERE subject_owner_id IS NOT NULL;
CREATE INDEX moderation_cases_assignee ON moderation_cases (assignee_id) WHERE assignee_id IS NOT NULL AND status = 'open';
CREATE INDEX moderation_cases_evidence_due ON moderation_cases (resolved_at) WHERE evidence_held AND status = 'resolved';

CREATE TRIGGER moderation_cases_updated_at
    BEFORE UPDATE ON moderation_cases
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- One row per report received from the engagement domain. The primary key is
-- engagement's report ID, which makes event consumption idempotent.
CREATE TABLE moderation_reports (
    id          uuid PRIMARY KEY,
    case_id     uuid        NOT NULL REFERENCES moderation_cases (id) ON DELETE CASCADE,
    -- NULL once the reporter's account is deleted (the text is cleared too).
    reporter_id uuid REFERENCES users (id) ON DELETE SET NULL,
    category    text        NOT NULL,
    text        text        NOT NULL DEFAULT '',
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX moderation_reports_case ON moderation_reports (case_id, created_at DESC, id DESC);
CREATE INDEX moderation_reports_reporter ON moderation_reports (reporter_id) WHERE reporter_id IS NOT NULL;
CREATE INDEX moderation_reports_created ON moderation_reports (created_at);

-- Internal remarks moderators leave on a case.
CREATE TABLE moderation_case_notes (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    case_id    uuid        NOT NULL REFERENCES moderation_cases (id) ON DELETE CASCADE,
    author_id  uuid REFERENCES users (id) ON DELETE SET NULL,
    body       text        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX moderation_case_notes_case ON moderation_case_notes (case_id, id);

-- The moderation log: every action taken against an account or a piece of
-- content. An account's moderation history is the rows targeting it.
CREATE TABLE moderation_actions (
    id             uuid PRIMARY KEY DEFAULT uuidv7(),
    -- tokuly.admin.v1.ModerationActionKind
    kind           integer     NOT NULL,
    -- NULL for automatic actions (expired suspensions) and deleted staff.
    moderator_id   uuid REFERENCES users (id) ON DELETE SET NULL,
    target_user_id uuid REFERENCES users (id) ON DELETE SET NULL,
    -- The action is about content that hides its author (see
    -- moderation_cases.owner_hidden): the target above is that author, and is
    -- not shown to a moderator who is involved in the content.
    target_hidden  boolean     NOT NULL DEFAULT false,
    subject_type   integer,
    subject_id     uuid,
    subject_title  text        NOT NULL DEFAULT '',
    case_id        uuid REFERENCES moderation_cases (id) ON DELETE SET NULL,
    -- Shown to the affected user.
    reason         text        NOT NULL DEFAULT '',
    -- Staff only.
    note           text        NOT NULL DEFAULT '',
    -- Suspensions: when it ends on its own.
    expires_at     timestamptz,
    created_at     timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX moderation_actions_target ON moderation_actions (target_user_id, id DESC) WHERE target_user_id IS NOT NULL;
CREATE INDEX moderation_actions_moderator ON moderation_actions (moderator_id, id DESC) WHERE moderator_id IS NOT NULL;
CREATE INDEX moderation_actions_case ON moderation_actions (case_id, id DESC) WHERE case_id IS NOT NULL;
CREATE INDEX moderation_actions_kind ON moderation_actions (kind, id DESC);

-- Suspensions and bans imposed through the admin console. `users.banned_at`
-- (identity) is what actually locks the account; this table adds who, why
-- and until when, and drives the automatic lifting of timed suspensions.
CREATE TABLE user_suspensions (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    moderator_id uuid REFERENCES users (id) ON DELETE SET NULL,
    case_id      uuid REFERENCES moderation_cases (id) ON DELETE SET NULL,
    reason       text        NOT NULL,
    -- NULL = permanent ban.
    expires_at   timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now(),
    -- Set when the suspension stopped applying.
    ended_at     timestamptz,
    -- 'lifted' (by staff) | 'expired' | 'superseded' (replaced by a newer one)
    end_cause    text        NOT NULL DEFAULT '',
    ended_by     uuid REFERENCES users (id) ON DELETE SET NULL
);

CREATE UNIQUE INDEX user_suspensions_active ON user_suspensions (user_id) WHERE ended_at IS NULL;
CREATE INDEX user_suspensions_due ON user_suspensions (expires_at) WHERE ended_at IS NULL AND expires_at IS NOT NULL;

-- ───────────────────────────── announcements ─────────────────────────────

-- The product update log. `published_at` decides the state: NULL = draft, in
-- the future = scheduled, otherwise published.
CREATE TABLE announcements (
    id                  uuid PRIMARY KEY DEFAULT uuidv7(),
    -- tokuly.admin.v1.AnnouncementCategory
    category            integer     NOT NULL DEFAULT 1,
    title_ja            text        NOT NULL,
    title_en            text        NOT NULL DEFAULT '',
    summary_ja          text        NOT NULL DEFAULT '',
    summary_en          text        NOT NULL DEFAULT '',
    -- Markdown.
    body_ja             text        NOT NULL,
    body_en             text        NOT NULL DEFAULT '',
    -- tokuly.common.v1.Image as proto JSON.
    image               jsonb,
    pinned              boolean     NOT NULL DEFAULT false,
    published_at        timestamptz,
    -- Notify every active user once the announcement is visible.
    notify_users        boolean     NOT NULL DEFAULT false,
    -- Broadcast progress: when the job was queued, the last user notified
    -- (the job resumes after it), how many so far, and when it finished.
    broadcast_queued_at timestamptz,
    broadcast_cursor    uuid,
    notified_count      integer     NOT NULL DEFAULT 0,
    notified_at         timestamptz,
    author_id           uuid REFERENCES users (id) ON DELETE SET NULL,
    attrs               jsonb       NOT NULL DEFAULT '{}',
    created_at          timestamptz NOT NULL DEFAULT now(),
    updated_at          timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX announcements_feed ON announcements (pinned DESC, published_at DESC, id DESC) WHERE published_at IS NOT NULL;
CREATE INDEX announcements_broadcast_due ON announcements (published_at) WHERE notify_users AND notified_at IS NULL AND published_at IS NOT NULL;

CREATE TRIGGER announcements_updated_at
    BEFORE UPDATE ON announcements
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- ───────────────────────────── audit log ─────────────────────────────

-- Every administrative action: who did what to what, why, and the state
-- before and after. Append-only; rows survive the accounts they mention.
CREATE TABLE admin_audit_log (
    id             uuid PRIMARY KEY DEFAULT uuidv7(),
    -- NULL for automatic actions and deleted staff accounts.
    actor_id       uuid REFERENCES users (id) ON DELETE SET NULL,
    -- Dotted name, e.g. 'case.resolved', 'user.suspended', 'flag.updated'.
    action         text        NOT NULL,
    -- 'user' | 'case' | 'subject' | 'flag' | 'announcement'
    target_type    text        NOT NULL DEFAULT '',
    target_id      text        NOT NULL DEFAULT '',
    -- The account affected, when there is one.
    target_user_id uuid REFERENCES users (id) ON DELETE SET NULL,
    -- The entry is about content that hides its author (see
    -- moderation_cases.owner_hidden), named here: the account above is that
    -- author, and is not shown to an admin who is involved in the content.
    target_hidden  boolean     NOT NULL DEFAULT false,
    subject_type   integer,
    subject_id     uuid,
    reason         text        NOT NULL DEFAULT '',
    before         jsonb,
    after          jsonb,
    ip             text        NOT NULL DEFAULT '',
    created_at     timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX admin_audit_log_actor ON admin_audit_log (actor_id, id DESC) WHERE actor_id IS NOT NULL;
CREATE INDEX admin_audit_log_target_user ON admin_audit_log (target_user_id, id DESC) WHERE target_user_id IS NOT NULL;
CREATE INDEX admin_audit_log_action ON admin_audit_log (action text_pattern_ops, id DESC);
CREATE INDEX admin_audit_log_target ON admin_audit_log (target_type, target_id, id DESC);
