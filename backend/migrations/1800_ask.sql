-- Ask: question boxes, prompts and compliment polls.
--
-- Three kinds of things are sent anonymously here: questions, prompt
-- responses and flames. Each row keeps who sent it (`sender_id`), for
-- moderators only; `sender_id` becomes NULL when that account is deleted.
-- `dropped` marks what was accepted and never reaches its recipient: sent by
-- someone the recipient had blocked, or by an account that was suspended
-- before the item was delivered. The sender is shown the same as for
-- anything else they sent.
--
-- Nothing an anonymous item's recipient sees depends on the state of the
-- sender's account afterwards. Only signed items are hidden while their
-- sender is suspended.

-- ───────────────────────────── settings ─────────────────────────────

-- One row per user who changed a setting; everyone else has the defaults
-- (box closed, friends only).
CREATE TABLE ask_settings (
    user_id      uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    box_open     boolean     NOT NULL DEFAULT false,
    -- tokuly.ask.v1.AskerAudience
    audience     smallint    NOT NULL DEFAULT 2,
    box_prompt   text        NOT NULL DEFAULT '',
    allow_signed boolean     NOT NULL DEFAULT true,
    allow_images boolean     NOT NULL DEFAULT false,
    -- As entered by the owner; matching normalizes both sides.
    muted_words  text[]      NOT NULL DEFAULT '{}',
    in_polls     boolean     NOT NULL DEFAULT true,
    allow_hints  boolean     NOT NULL DEFAULT true,
    attrs        jsonb       NOT NULL DEFAULT '{}',
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER ask_settings_updated_at
    BEFORE UPDATE ON ask_settings
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- ───────────────────────────── question box ─────────────────────────────

CREATE TABLE ask_questions (
    id                    uuid PRIMARY KEY DEFAULT uuidv7(),
    -- Whose box the question is in.
    owner_id              uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    sender_id             uuid        REFERENCES users (id) ON DELETE SET NULL,
    -- The sender chose to show their name.
    signed                boolean     NOT NULL DEFAULT false,
    text                  text        NOT NULL,
    -- tokuly.common.v1.Image as proto JSON.
    image                 jsonb,
    image_asset_id        uuid,
    -- tokuly.ask.v1.QuestionFolder: 1 inbox, 2 answered, 3 archived, 4 filtered.
    folder                smallint    NOT NULL DEFAULT 1 CHECK (folder BETWEEN 1 AND 4),
    dropped               boolean     NOT NULL DEFAULT false,
    read_at               timestamptz,
    -- Set once the delivery worker dealt with it. Until then the question is
    -- in nobody's box; the worker is also where a question whose sender was
    -- suspended in the meantime is stopped (`dropped`).
    delivered_at          timestamptz,
    -- Gone from the box: 1 deleted by the owner (the sender keeps seeing it
    -- as waiting), 2 withdrawn by the sender, 3 taken down by moderation.
    -- The row stays for a while so a report can still be acted on.
    removed_at            timestamptz,
    removed_by            smallint CHECK (removed_by BETWEEN 1 AND 3),
    answer_text           text        NOT NULL DEFAULT '',
    answer_image          jsonb,
    answer_image_asset_id uuid,
    answer_stamp_id       uuid,
    -- tokuly.ask.v1.AnswerAudience: 1 public, 2 friends, 3 private.
    answer_audience       smallint    NOT NULL DEFAULT 0,
    answered_at           timestamptz,
    answer_edited_at      timestamptz,
    -- Denormalized from the engagement domain.
    like_count            integer     NOT NULL DEFAULT 0,
    comment_count         integer     NOT NULL DEFAULT 0,
    attrs                 jsonb       NOT NULL DEFAULT '{}',
    created_at            timestamptz NOT NULL DEFAULT now(),
    updated_at            timestamptz NOT NULL DEFAULT now(),
    CHECK ((removed_at IS NULL) = (removed_by IS NULL)),
    CHECK ((folder = 2) = (answered_at IS NOT NULL))
);

-- The owner's inbox, one folder at a time.
CREATE INDEX ask_questions_inbox ON ask_questions (owner_id, folder, id DESC) WHERE removed_at IS NULL AND NOT dropped;
-- The Q&A page.
CREATE INDEX ask_questions_answers ON ask_questions (owner_id, answered_at DESC, id DESC) WHERE folder = 2 AND removed_at IS NULL;
-- "Questions I asked".
CREATE INDEX ask_questions_sent ON ask_questions (sender_id, id DESC) WHERE sender_id IS NOT NULL;
-- What the maintenance worker purges: a removed question some time after it
-- was removed, a dropped one some time after it was sent.
CREATE INDEX ask_questions_purge ON ask_questions ((COALESCE(removed_at, created_at))) WHERE removed_at IS NOT NULL OR dropped;
CREATE INDEX ask_questions_image ON ask_questions (image_asset_id) WHERE image_asset_id IS NOT NULL;
CREATE INDEX ask_questions_answer_image ON ask_questions (answer_image_asset_id) WHERE answer_image_asset_id IS NOT NULL;

CREATE TRIGGER ask_questions_updated_at
    BEFORE UPDATE ON ask_questions
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- "Block whoever sent this". One row per blocked item, not per sender: two
-- blocks of the same person stay two rows, so the owner's list of blocks
-- cannot tell that two items came from one sender. A sender is blocked
-- while at least one row names them.
CREATE TABLE ask_sender_blocks (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    owner_id   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- NULL when the sender's account is gone; the row stays, because a block
    -- that vanished would tell the owner that the sender left.
    sender_id  uuid        REFERENCES users (id) ON DELETE SET NULL,
    -- tokuly.ask.v1.BlockSource: 1 question, 2 prompt response, 3 flame.
    source     smallint    NOT NULL CHECK (source BETWEEN 1 AND 3),
    source_id  uuid        NOT NULL,
    -- The blocked text, kept as the owner's reminder of what this block is.
    preview    text        NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (owner_id, source, source_id)
);

CREATE INDEX ask_sender_blocks_pair ON ask_sender_blocks (owner_id, sender_id) WHERE sender_id IS NOT NULL;
CREATE INDEX ask_sender_blocks_newest ON ask_sender_blocks (owner_id, id DESC);
CREATE INDEX ask_sender_blocks_sender ON ask_sender_blocks (sender_id) WHERE sender_id IS NOT NULL;

-- ───────────────────────────── prompts ─────────────────────────────

CREATE TABLE ask_prompts (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    owner_id   uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    text       text        NOT NULL,
    emoji      text        NOT NULL DEFAULT '',
    closes_at  timestamptz NOT NULL,
    -- Closed early by the owner.
    closed_at  timestamptz,
    -- 1 deleted by the owner, 3 taken down by moderation.
    removed_at timestamptz,
    removed_by smallint CHECK (removed_by IN (1, 3)),
    attrs      jsonb       NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((removed_at IS NULL) = (removed_by IS NULL))
);

CREATE INDEX ask_prompts_owner ON ask_prompts (owner_id, id DESC) WHERE removed_at IS NULL;
CREATE INDEX ask_prompts_purge ON ask_prompts (removed_at) WHERE removed_at IS NOT NULL;

CREATE TRIGGER ask_prompts_updated_at
    BEFORE UPDATE ON ask_prompts
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE TABLE ask_prompt_responses (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    prompt_id    uuid        NOT NULL REFERENCES ask_prompts (id) ON DELETE CASCADE,
    -- The prompt's owner, repeated here for the owner's queries.
    owner_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    sender_id    uuid        REFERENCES users (id) ON DELETE SET NULL,
    signed       boolean     NOT NULL DEFAULT false,
    text         text        NOT NULL,
    -- tokuly.common.v1.Image as proto JSON.
    image        jsonb,
    image_asset_id uuid,
    -- Held by the word filter until the owner releases it.
    held         boolean     NOT NULL DEFAULT false,
    dropped      boolean     NOT NULL DEFAULT false,
    seen_at      timestamptz,
    delivered_at timestamptz,
    published_at timestamptz,
    -- 1 deleted by the owner (the responder keeps seeing it as sent),
    -- 3 taken down by moderation.
    removed_at   timestamptz,
    removed_by   smallint CHECK (removed_by IN (1, 3)),
    created_at   timestamptz NOT NULL DEFAULT now(),
    -- One response per person and prompt, dropped and deleted ones included.
    UNIQUE (prompt_id, sender_id),
    CHECK ((removed_at IS NULL) = (removed_by IS NULL))
);

CREATE INDEX ask_prompt_responses_received ON ask_prompt_responses (prompt_id, id DESC) WHERE removed_at IS NULL AND NOT dropped;
CREATE INDEX ask_prompt_responses_published ON ask_prompt_responses (prompt_id, published_at DESC, id DESC)
    WHERE published_at IS NOT NULL AND removed_at IS NULL;
CREATE INDEX ask_prompt_responses_sender ON ask_prompt_responses (sender_id) WHERE sender_id IS NOT NULL;
CREATE INDEX ask_prompt_responses_owner ON ask_prompt_responses (owner_id);
CREATE INDEX ask_prompt_responses_image ON ask_prompt_responses (image_asset_id) WHERE image_asset_id IS NOT NULL;

-- ───────────────────────────── compliment polls ─────────────────────────────

-- The question bank. Built-in questions come from the crate (`key` is their
-- stable name there) and are refreshed at startup; staff add their own and
-- switch any of them off.
CREATE TABLE ask_poll_questions (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    key        text UNIQUE,
    text_ja    text        NOT NULL,
    text_en    text        NOT NULL,
    emoji      text        NOT NULL DEFAULT '',
    enabled    boolean     NOT NULL DEFAULT true,
    use_count  bigint      NOT NULL DEFAULT 0,
    created_by uuid        REFERENCES users (id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX ask_poll_questions_enabled ON ask_poll_questions (id) WHERE enabled;

CREATE TABLE ask_rounds (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The questions of the round, in order; its length is the poll count.
    question_ids uuid[]      NOT NULL,
    -- Polls voted on or skipped so far.
    answered     integer     NOT NULL DEFAULT 0,
    votes        integer     NOT NULL DEFAULT 0,
    completed_at timestamptz,
    -- Coins granted for completing it; `rewarded_at` is set once the wallet
    -- confirmed (or there was nothing to pay).
    reward_coins bigint      NOT NULL DEFAULT 0,
    rewarded_at  timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now()
);

-- At most one round in progress per user.
CREATE UNIQUE INDEX ask_rounds_active ON ask_rounds (user_id) WHERE completed_at IS NULL;
CREATE INDEX ask_rounds_latest ON ask_rounds (user_id, id DESC);
CREATE INDEX ask_rounds_completed ON ask_rounds (completed_at) WHERE completed_at IS NOT NULL;

-- A poll is drawn when the round reaches it.
CREATE TABLE ask_polls (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    round_id      uuid        NOT NULL REFERENCES ask_rounds (id) ON DELETE CASCADE,
    user_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- 0-based.
    position      integer     NOT NULL,
    question_id   uuid        NOT NULL REFERENCES ask_poll_questions (id),
    -- The four friends shown.
    choices       uuid[]      NOT NULL,
    shuffles_used integer     NOT NULL DEFAULT 0,
    -- 0 waiting, 1 voted, 2 skipped.
    outcome       smallint    NOT NULL DEFAULT 0 CHECK (outcome BETWEEN 0 AND 2),
    answered_at   timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now(),
    UNIQUE (round_id, position)
);

CREATE INDEX ask_polls_question ON ask_polls (question_id);

-- How often each friend was shown to a voter. Polls draw the friends shown
-- least so far, which is what makes everyone come up over time. A friend
-- gets a row the first time they are eligible for one of the voter's polls,
-- starting at the lowest count among the voter's other friends.
CREATE TABLE ask_poll_exposure (
    user_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    friend_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    shown_count   integer     NOT NULL DEFAULT 0,
    last_shown_at timestamptz NOT NULL DEFAULT now(),
    -- First time this friend could come up: what the "friends for N months"
    -- hint falls back to when the social domain does not know since when.
    created_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, friend_id)
);

CREATE INDEX ask_poll_exposure_friend ON ask_poll_exposure (friend_id);

-- A compliment: `receiver_id` was picked for `question_id`.
CREATE TABLE ask_flames (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    receiver_id  uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    sender_id    uuid        REFERENCES users (id) ON DELETE SET NULL,
    question_id  uuid        NOT NULL REFERENCES ask_poll_questions (id),
    dropped      boolean     NOT NULL DEFAULT false,
    -- The hints this flame sells about its sender, in order, as
    -- `[{"kind": <tokuly.ask.v1.HintKind>, "value": "…"}]`; empty when the
    -- sender had hints switched off. Written when the vote is cast and never
    -- touched again, so that neither the price list nor a purchase depends
    -- on what became of the sender's account.
    hints        jsonb       NOT NULL DEFAULT '[]',
    seen_at      timestamptz,
    delivered_at timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX ask_flames_received ON ask_flames (receiver_id, id DESC);
CREATE INDEX ask_flames_week ON ask_flames (receiver_id, created_at);
CREATE INDEX ask_flames_sender ON ask_flames (sender_id) WHERE sender_id IS NOT NULL;
CREATE INDEX ask_flames_question ON ask_flames (question_id);

-- Hints a receiver bought about the sender of a flame: one row per hint
-- that was paid for, copied from `ask_flames.hints`.
CREATE TABLE ask_flame_hints (
    flame_id     uuid        NOT NULL REFERENCES ask_flames (id) ON DELETE CASCADE,
    -- 0 = first hint.
    position     smallint    NOT NULL,
    -- tokuly.ask.v1.HintKind
    kind         smallint    NOT NULL,
    value        text        NOT NULL,
    price        bigint      NOT NULL,
    -- The buyer's idempotency keys that led to this hint: a retried purchase
    -- finds its hint here instead of buying the next one.
    request_keys text[]      NOT NULL,
    -- The wallet entry that paid for it.
    entry_id     uuid        NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (flame_id, position)
);

-- ───────────────────────────── moderation ─────────────────────────────

-- Every time a moderator looked up who sent something: anonymity is lifted
-- for staff only, and each lift leaves a trace.
CREATE TABLE ask_inspections (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    staff_id   uuid        REFERENCES users (id) ON DELETE SET NULL,
    -- 'question', 'prompt_response' or 'flame'.
    kind       text        NOT NULL,
    target_id  uuid        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX ask_inspections_target ON ask_inspections (target_id);
CREATE INDEX ask_inspections_staff ON ask_inspections (staff_id, id DESC);
