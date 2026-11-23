-- spaces: audio rooms with a stage and an audience, and always-on hangouts
-- of friend groups (たまり場). Audio itself runs on LiveKit; these tables are
-- the truth about who is where and may do what.

-- ───────────────────────────── rooms ─────────────────────────────

CREATE TABLE spaces (
    id                uuid PRIMARY KEY     DEFAULT uuidv7(),
    -- The current host. Changes when a room is handed over.
    host_id           uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title             text        NOT NULL,
    -- Lowercase, without '#'.
    tags              text[]      NOT NULL DEFAULT '{}',
    -- BCP 47 primary tag, or '' when unknown.
    language          text        NOT NULL DEFAULT '',
    -- tokuly.spaces.v1.SpaceAudience: 1 public, 2 friends of the host, 3 invited only.
    audience          smallint    NOT NULL,
    -- tokuly.spaces.v1.SpaceState: 1 scheduled, 2 live, 3 ended, 4 cancelled.
    state             smallint    NOT NULL,
    -- tokuly.spaces.v1.SpaceEndReason; 0 until the room ended or was cancelled.
    end_reason        smallint    NOT NULL DEFAULT 0,
    -- LiveKit room name.
    room              text        NOT NULL,
    scheduled_at      timestamptz,
    started_at        timestamptz,
    ended_at          timestamptz,
    -- Counters over space_participants, kept in the transaction that changes
    -- them (every change to a room happens under a lock on its row).
    listener_count    integer     NOT NULL DEFAULT 0,
    speaker_count     integer     NOT NULL DEFAULT 0,
    hand_count        integer     NOT NULL DEFAULT 0,
    peak_listeners    integer     NOT NULL DEFAULT 0,
    total_listeners   integer     NOT NULL DEFAULT 0,
    reminder_count    integer     NOT NULL DEFAULT 0,
    pin_text          text        NOT NULL DEFAULT '',
    pin_url           text        NOT NULL DEFAULT '',
    pinned_by         uuid,
    pinned_at         timestamptz,
    -- Since when the host of a live room is not inside; after the grace
    -- period the room is handed to a co-host or ended.
    host_absent_since timestamptz,
    -- Since when nobody is inside a live room.
    empty_since       timestamptz,
    -- When the host was told that the scheduled time has come.
    host_nudged_at    timestamptz,
    -- When the LiveKit room of an ended room was closed (retried until it works).
    room_closed_at    timestamptz,
    -- When the host removed the room from their history. It reads as gone
    -- from then on, but stays for a while: people who were in it can still
    -- report it, and moderators see who was there.
    deleted_at        timestamptz,
    attrs             jsonb       NOT NULL DEFAULT '{}',
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER spaces_updated_at
    BEFORE UPDATE ON spaces
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- A host's rooms, newest first.
CREATE INDEX spaces_host ON spaces (host_id, id DESC);
-- One live room per host; also the list of live rooms for the workers.
CREATE UNIQUE INDEX spaces_live_host ON spaces (host_id) WHERE state = 2;
-- Public rooms that are live, ranked by listeners.
CREATE INDEX spaces_live_public ON spaces (listener_count DESC, id DESC) WHERE state = 2 AND audience = 1;
-- Upcoming rooms, soonest first.
CREATE INDEX spaces_scheduled ON spaces (scheduled_at, id) WHERE state = 1;
-- Ended rooms whose LiveKit room still has to be closed.
CREATE INDEX spaces_unclosed ON spaces (ended_at) WHERE state = 3 AND room_closed_at IS NULL;
-- Finished rooms by age, for retention.
CREATE INDEX spaces_finished ON spaces (ended_at) WHERE state IN (3, 4);

-- Everyone who has been inside a room. A row is "inside" while left_at is
-- NULL; it stays afterwards (history, and the role a host or co-host comes
-- back with).
CREATE TABLE space_participants (
    space_id         uuid        NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
    user_id          uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.spaces.v1.SpaceRole: 1 listener, 2 speaker, 3 co-host, 4 host.
    role             smallint    NOT NULL DEFAULT 1,
    -- The highest role the person ever held in the room. A speaker goes
    -- back to listener on leaving; what they were stays on record.
    top_role         smallint    NOT NULL DEFAULT 1,
    -- Start of the current (or last) stay.
    joined_at        timestamptz NOT NULL DEFAULT now(),
    left_at          timestamptz,
    -- tokuly.spaces.v1.SpaceLeaveReason of the last departure.
    leave_reason     smallint    NOT NULL DEFAULT 0,
    -- Reconciliation with LiveKit: when the participant was last found
    -- connected during this stay, and since when they are found missing.
    seen_at          timestamptz,
    missing_since    timestamptz,
    hand_raised_at   timestamptz,
    -- The raised hand may be shown to everyone in the room: it went up
    -- while its owner was visible. A hand raised by a listener who hides
    -- their online status is for the moderators only, also if they stop
    -- hiding later.
    hand_public      boolean     NOT NULL DEFAULT false,
    -- When they last came on stage; NULL off stage. What people other than
    -- the moderators see instead of `joined_at`, which may fall in a time
    -- the person was hidden.
    stage_since      timestamptz,
    -- When they last left the room from the stage (in plain view). Kept
    -- through later stays, so nothing about a later, perhaps hidden, stay
    -- changes what others see of it.
    left_stage_at    timestamptz,
    -- tokuly.spaces.v1.SpaceLeaveReason of that departure.
    left_stage_reason smallint   NOT NULL DEFAULT 0,
    stage_invited_at timestamptz,
    stage_invited_by uuid,
    -- Seconds spent inside, summed over every stay that ended.
    seconds          integer     NOT NULL DEFAULT 0,
    first_joined_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (space_id, user_id)
);

-- Who is inside a room: the stage first, then by arrival.
CREATE INDEX space_participants_inside ON space_participants (space_id, role DESC, joined_at, user_id) WHERE left_at IS NULL;
-- Which room someone is inside.
CREATE INDEX space_participants_user ON space_participants (user_id) WHERE left_at IS NULL;
-- Raised hands, first raised first.
CREATE INDEX space_participants_hands ON space_participants (space_id, hand_raised_at) WHERE left_at IS NULL AND hand_raised_at IS NOT NULL;
-- Who was inside and is gone, last to leave first.
CREATE INDEX space_participants_past ON space_participants (space_id, left_at DESC, user_id) WHERE left_at IS NOT NULL;

CREATE TABLE space_bans (
    space_id   uuid        NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    banned_by  uuid,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (space_id, user_id)
);

CREATE INDEX space_bans_user ON space_bans (user_id);

-- Invitations: the way into friends and invited-only rooms.
CREATE TABLE space_invites (
    space_id   uuid        NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    invited_by uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    -- When the invitation was taken back. It opens no door any more, but
    -- the row stays: whoever received an invitation can still report the
    -- room it was for.
    revoked_at timestamptz,
    PRIMARY KEY (space_id, user_id)
);

CREATE INDEX space_invites_user ON space_invites (user_id, space_id);
CREATE INDEX space_invites_inviter ON space_invites (invited_by);

-- "Remind me when it starts."
CREATE TABLE space_reminders (
    space_id   uuid        NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (space_id, user_id)
);

CREATE INDEX space_reminders_user ON space_reminders (user_id, space_id);

-- ───────────────────────────── hangouts ─────────────────────────────

CREATE TABLE hangouts (
    id                 uuid PRIMARY KEY     DEFAULT uuidv7(),
    owner_id           uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name               text        NOT NULL,
    emoji              text        NOT NULL DEFAULT '',
    members_can_invite boolean     NOT NULL DEFAULT false,
    -- LiveKit room name.
    room               text        NOT NULL,
    -- Members (state 2), the owner included; kept with hangout_members.
    member_count       integer     NOT NULL DEFAULT 1,
    -- People in the voice room right now; kept with hangout_presence.
    inside_count       integer     NOT NULL DEFAULT 0,
    last_active_at     timestamptz,
    -- When members were last told that somebody is inside (throttle).
    last_announced_at  timestamptz,
    -- When the owner closed the hangout. It reads as gone from then on;
    -- the row stays for a while so that it can still be reported.
    deleted_at         timestamptz,
    attrs              jsonb       NOT NULL DEFAULT '{}',
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER hangouts_updated_at
    BEFORE UPDATE ON hangouts
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE INDEX hangouts_owner ON hangouts (owner_id) WHERE deleted_at IS NULL;
-- Closed hangouts by age, for the purge.
CREATE INDEX hangouts_closed ON hangouts (deleted_at) WHERE deleted_at IS NOT NULL;
-- Hangouts with somebody inside, for the reconciliation with LiveKit.
CREATE INDEX hangouts_open ON hangouts (id) WHERE inside_count > 0;
-- Hangouts that were in use a moment ago (old tokens may still connect).
CREATE INDEX hangouts_active ON hangouts (last_active_at);

CREATE TABLE hangout_members (
    hangout_id uuid        NOT NULL REFERENCES hangouts (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.spaces.v1.HangoutMemberState: 1 invited, 2 member; and 3 for
    -- someone who was invited or a member and is not any more (never shown
    -- in the API: it lets them report a hangout they were part of).
    state      smallint    NOT NULL,
    invited_by uuid,
    -- The member silenced "someone is inside" notifications.
    muted      boolean     NOT NULL DEFAULT false,
    -- When the invitation was accepted (the owner: when the hangout was created).
    joined_at  timestamptz,
    -- When they stopped being part of the hangout (state 3).
    left_at    timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (hangout_id, user_id)
);

CREATE INDEX hangout_members_user ON hangout_members (user_id, state);
CREATE INDEX hangout_members_inviter ON hangout_members (invited_by) WHERE state = 1;
-- Former members and invitees by age, for the purge.
CREATE INDEX hangout_members_former ON hangout_members (left_at) WHERE state = 3;

-- Who is in a hangout's voice room right now. Rows go when people leave.
CREATE TABLE hangout_presence (
    hangout_id    uuid        NOT NULL REFERENCES hangouts (id) ON DELETE CASCADE,
    user_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    joined_at     timestamptz NOT NULL DEFAULT now(),
    -- Reconciliation with LiveKit, as in space_participants.
    seen_at       timestamptz,
    missing_since timestamptz,
    PRIMARY KEY (hangout_id, user_id)
);

CREATE INDEX hangout_presence_user ON hangout_presence (user_id);
