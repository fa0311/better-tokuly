-- Moments: one prompt a day per region, the photo pairs posted for it,
-- reactions (RealMoji and stamps) and every account's RealMoji library.
-- A region is an IANA time zone (`users.time_zone`).

-- Every region that has accounts, and the day it is on: the latest prompt
-- that went out there.
CREATE TABLE moment_regions (
    region          text        PRIMARY KEY,
    current_day_id  uuid,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);
CREATE TRIGGER moment_regions_updated_at BEFORE UPDATE ON moment_regions
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- One row per region and local date, made a day ahead with its prompt time
-- drawn at random. A day runs from its announcement to the next one.
CREATE TABLE moment_days (
    id            uuid        PRIMARY KEY DEFAULT uuidv7(),
    region        text        NOT NULL REFERENCES moment_regions (region) ON DELETE CASCADE,
    date          date        NOT NULL,
    -- NULL: no prompt that date (the region was first seen after its window).
    prompt_at     timestamptz,
    -- When the prompt went out; NULL until then, and for a date that was
    -- skipped because a later one went out first.
    announced_at  timestamptz,
    -- The fan-out of the prompt: the last account told (in ID order), and
    -- when everyone was. A page of notifications and this cursor are
    -- committed together, so nobody is told twice.
    told_through  uuid,
    told_at       timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now(),
    UNIQUE (region, date)
);
CREATE INDEX moment_days_due ON moment_days (prompt_at) WHERE told_at IS NULL AND prompt_at IS NOT NULL;

ALTER TABLE moment_regions
    ADD CONSTRAINT moment_regions_current_day FOREIGN KEY (current_day_id) REFERENCES moment_days (id);

CREATE TABLE moments (
    id              uuid        PRIMARY KEY DEFAULT uuidv7(),
    user_id         uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    day_id          uuid        NOT NULL REFERENCES moment_days (id),
    -- tokuly.common.v1.Image as proto JSON, copied from the media assets.
    back            jsonb       NOT NULL,
    front           jsonb       NOT NULL,
    back_asset_id   uuid        NOT NULL,
    front_asset_id  uuid        NOT NULL,
    caption         text        NOT NULL DEFAULT '',
    lat             double precision,
    lng             double precision,
    retakes         smallint    NOT NULL DEFAULT 0,
    -- 0 on time; seconds after the window otherwise.
    late_seconds    bigint      NOT NULL DEFAULT 0,
    -- tokuly.moments.v1.MomentAudience as its number.
    audience        smallint    NOT NULL,
    posted_at       timestamptz NOT NULL DEFAULT now(),
    -- A deleted moment is kept (for reports) until the evidence period is
    -- over, counted from here; it still uses up its day.
    deleted_at      timestamptz,
    delete_reason   text,
    attrs           jsonb       NOT NULL DEFAULT '{}',
    UNIQUE (user_id, day_id),
    CHECK ((lat IS NULL) = (lng IS NULL))
);
CREATE INDEX moments_day ON moments (day_id, posted_at DESC, id DESC) WHERE deleted_at IS NULL;
CREATE INDEX moments_user ON moments (user_id, posted_at DESC);
CREATE INDEX moments_deleted ON moments (deleted_at) WHERE deleted_at IS NOT NULL;
CREATE INDEX moments_back_asset ON moments (back_asset_id);
CREATE INDEX moments_front_asset ON moments (front_asset_id);

-- One live reaction per person and moment. A replaced or taken back
-- reaction stays (removed_at) for the evidence period: it was seen.
CREATE TABLE moment_reactions (
    id          uuid        PRIMARY KEY DEFAULT uuidv7(),
    moment_id   uuid        NOT NULL REFERENCES moments (id) ON DELETE CASCADE,
    user_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.moments.v1.RealmojiEmoji as its number, with its picture...
    realmoji    smallint,
    image       jsonb,
    asset_id    uuid,
    -- ...or a stamp.
    stamp_id    uuid,
    created_at  timestamptz NOT NULL DEFAULT now(),
    removed_at  timestamptz,
    CHECK ((realmoji IS NOT NULL AND image IS NOT NULL AND asset_id IS NOT NULL AND stamp_id IS NULL)
        OR (realmoji IS NULL AND image IS NULL AND asset_id IS NULL AND stamp_id IS NOT NULL))
);
CREATE UNIQUE INDEX moment_reactions_live ON moment_reactions (moment_id, user_id) WHERE removed_at IS NULL;
CREATE INDEX moment_reactions_list ON moment_reactions (moment_id, created_at DESC, id DESC) WHERE removed_at IS NULL;
CREATE INDEX moment_reactions_user ON moment_reactions (user_id);
CREATE INDEX moment_reactions_asset ON moment_reactions (asset_id) WHERE asset_id IS NOT NULL;
CREATE INDEX moment_reactions_removed ON moment_reactions (removed_at) WHERE removed_at IS NOT NULL;

-- The RealMoji library: one selfie per account and emoji.
CREATE TABLE moment_realmojis (
    user_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    emoji       smallint    NOT NULL,
    asset_id    uuid        NOT NULL,
    image       jsonb       NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, emoji)
);
CREATE INDEX moment_realmojis_asset ON moment_realmojis (asset_id);
CREATE TRIGGER moment_realmojis_updated_at BEFORE UPDATE ON moment_realmojis
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Per account. No row reads as the defaults for minors (no location), so
-- an account whose sign-up event is still on its way is protected.
CREATE TABLE moment_settings (
    user_id          uuid        PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    share_location   boolean     NOT NULL,
    -- The owner set `share_location` themselves: age defaults leave it alone.
    location_chosen  boolean     NOT NULL DEFAULT false,
    audience         smallint    NOT NULL DEFAULT 1,
    prompts          boolean     NOT NULL DEFAULT true,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now()
);
CREATE TRIGGER moment_settings_updated_at BEFORE UPDATE ON moment_settings
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Who was shown a moment unlocked: they can still report it, and its
-- reactions, after it is deleted or its day is over.
CREATE TABLE moment_deliveries (
    moment_id  uuid        NOT NULL REFERENCES moments (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    shown_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (moment_id, user_id)
);
CREATE INDEX moment_deliveries_user ON moment_deliveries (user_id);

-- The accounts that exist already start with the defaults of their age
-- group on this day, decided once and stored (as at sign-up): location on
-- for adults, off for minors and for accounts without a birthdate.
INSERT INTO moment_settings (user_id, share_location)
SELECT id, birthdate IS NOT NULL AND birthdate <= current_date - make_interval(years => 18)
FROM users
WHERE deleted_at IS NULL AND NOT bot;
