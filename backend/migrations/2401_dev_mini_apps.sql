-- Mini apps: an application's web page that people open inside Tokuly.
--
-- The owner edits the draft manifest and submits it; staff approve or
-- reject that revision. What people open is the approved copy (`live_*`),
-- so an edit is reviewed before anyone sees it.
CREATE TABLE dev_mini_apps (
    app_id          uuid PRIMARY KEY REFERENCES dev_apps (id) ON DELETE CASCADE,
    -- The draft.
    name            text        NOT NULL,
    -- tokuly.common.v1.Image as proto JSON.
    icon            jsonb,
    launch_url      text        NOT NULL,
    scopes          text[]      NOT NULL,
    -- Grows with every change of the draft; a review names the one it read.
    revision        integer     NOT NULL DEFAULT 1,
    review_state    text        NOT NULL DEFAULT 'draft' CHECK (review_state IN ('draft', 'in_review', 'approved', 'rejected')),
    review_note     text        NOT NULL DEFAULT '',
    submitted_at    timestamptz,
    reviewed_by     uuid        REFERENCES users (id) ON DELETE SET NULL,
    reviewed_at     timestamptz,
    -- The approved copy; NULL until the first approval.
    live_name       text,
    live_icon       jsonb,
    live_launch_url text,
    live_scopes     text[],
    approved_at     timestamptz,
    attrs           jsonb       NOT NULL DEFAULT '{}',
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX dev_mini_apps_queue ON dev_mini_apps (submitted_at, app_id) WHERE review_state = 'in_review';

CREATE TRIGGER dev_mini_apps_updated_at
    BEFORE UPDATE ON dev_mini_apps
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- What a person let a mini app have through the bridge (mini-app scopes,
-- not API scopes: no token is issued under it).
CREATE TABLE dev_mini_app_grants (
    app_id     uuid        NOT NULL REFERENCES dev_mini_apps (app_id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    scopes     text[]      NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (app_id, user_id)
);

CREATE INDEX dev_mini_app_grants_user ON dev_mini_app_grants (user_id);
