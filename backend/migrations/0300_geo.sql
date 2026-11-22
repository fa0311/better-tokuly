-- geo: ghost mode, places, visits, nights, footprints, trips, place alerts,
-- SOS, live shares and gatherings.
--
-- The live state of a user (latest sample, current stay, sleeping...) lives in
-- Valkey; raw location history lives in ClickHouse (`geo_points`,
-- backend/clickhouse/0300_geo.sql). PostgreSQL holds everything derived from
-- it that must last.

-- One row per user who ever reported a location: footprint totals, the recap
-- marker, and a mirror of the privacy setting leaderboards filter on.
CREATE TABLE geo_users (
    user_id             uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    -- PrivacySettings.footprints_visible_to (identity Audience number), kept
    -- in sync on `identity.user.privacy_changed`.
    footprints_audience smallint         NOT NULL DEFAULT 3,
    -- Resolution-9 cells uncovered, and their summed area.
    cell_count          bigint           NOT NULL DEFAULT 0,
    explored_km2        double precision NOT NULL DEFAULT 0,
    -- GetRecap reports what was uncovered after this moment.
    recap_seen_at       timestamptz,
    created_at          timestamptz      NOT NULL DEFAULT now()
);

CREATE INDEX geo_users_rank ON geo_users (cell_count DESC) WHERE cell_count > 0;

-- ───────────────────────────── Ghost mode ─────────────────────────────

-- How precisely a friend sees the owner. No row = precise.
CREATE TABLE geo_ghost_rules (
    owner_id  uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The friend the rule applies to; the nil UUID holds the default for everyone.
    viewer_id uuid        NOT NULL,
    -- GhostMode: 1 precise (only as an exception to a stricter default), 2 blurred, 3 frozen.
    mode      smallint    NOT NULL CHECK (mode BETWEEN 1 AND 3),
    since     timestamptz NOT NULL DEFAULT now(),
    -- NULL = until changed. Expired rules are removed by the sweeper.
    until     timestamptz,
    -- FROZEN: the pin as the viewer saw it when the rule was set (FrozenPin
    -- JSON). NULL when frozen without a known position: the viewer sees nothing.
    frozen    jsonb,
    -- Since when the viewers of this rule have not seen the owner precisely.
    -- It stays put while one non-precise rule replaces another: nothing the
    -- owner did after this moment is shown to them.
    hidden_since timestamptz NOT NULL DEFAULT now(),
    -- The owner's footprint totals at `hidden_since` (FootprintSnapshot JSON):
    -- what these viewers keep seeing instead of the live numbers.
    footprint jsonb,
    PRIMARY KEY (owner_id, viewer_id)
);

CREATE INDEX geo_ghost_rules_until ON geo_ghost_rules (until) WHERE until IS NOT NULL;
CREATE INDEX geo_ghost_rules_viewer ON geo_ghost_rules (viewer_id);

-- ───────────────────────────── Places ─────────────────────────────

CREATE TABLE geo_places (
    id             uuid PRIMARY KEY          DEFAULT uuidv7(),
    owner_id       uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name           text             NOT NULL,
    emoji          text             NOT NULL DEFAULT '',
    -- PlaceLabel: 1 home, 2 work, 3 school, 4 custom.
    label          smallint         NOT NULL DEFAULT 4,
    lat            double precision NOT NULL,
    lng            double precision NOT NULL,
    geog           geography(Point, 4326) GENERATED ALWAYS AS (ST_SetSRID(ST_MakePoint(lng, lat), 4326)::geography) STORED,
    radius_m       double precision NOT NULL DEFAULT 80,
    -- common Visibility: 1 public, 4 friends, 5 private. Home/work/school are always private.
    visibility     smallint         NOT NULL DEFAULT 5,
    address        text             NOT NULL DEFAULT '',
    category       text             NOT NULL DEFAULT '',
    -- Created from a stay-pattern suggestion rather than from scratch.
    auto_detected  boolean          NOT NULL DEFAULT false,
    -- common Image as proto JSON, and the asset it came from.
    photo          jsonb,
    photo_asset_id uuid,
    attrs          jsonb            NOT NULL DEFAULT '{}',
    created_at     timestamptz      NOT NULL DEFAULT now(),
    updated_at     timestamptz      NOT NULL DEFAULT now(),
    CHECK (label BETWEEN 1 AND 4),
    CHECK (visibility IN (1, 4, 5)),
    CHECK (label = 4 OR visibility = 5)
);

CREATE TRIGGER geo_places_updated_at
    BEFORE UPDATE ON geo_places
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE INDEX geo_places_owner ON geo_places (owner_id, created_at);
CREATE INDEX geo_places_geog ON geo_places USING gist (geog);
CREATE INDEX geo_places_public_name ON geo_places USING gin (name gin_trgm_ops) WHERE visibility = 1;
CREATE INDEX geo_places_photo_asset ON geo_places (photo_asset_id) WHERE photo_asset_id IS NOT NULL;
-- One home, one work, one school per account.
CREATE UNIQUE INDEX geo_places_label ON geo_places (owner_id, label) WHERE label IN (1, 2, 3);

CREATE TABLE geo_saved_places (
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    place_id   uuid        NOT NULL REFERENCES geo_places (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, place_id)
);

CREATE INDEX geo_saved_places_recent ON geo_saved_places (user_id, created_at DESC, place_id DESC);
CREATE INDEX geo_saved_places_place ON geo_saved_places (place_id);

-- ───────────────────────────── History ─────────────────────────────

-- A stay (still within ~80 m for 5 minutes or more), or a manual check-in.
CREATE TABLE geo_visits (
    id            uuid PRIMARY KEY          DEFAULT uuidv7(),
    user_id       uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The place the stay matched, if any; `place_name` keeps its name as it was.
    place_id      uuid REFERENCES geo_places (id) ON DELETE SET NULL,
    place_name    text             NOT NULL DEFAULT '',
    lat           double precision NOT NULL,
    lng           double precision NOT NULL,
    arrived_at    timestamptz      NOT NULL,
    -- NULL while the stay is ongoing.
    departed_at   timestamptz,
    -- Friends who were there (gathering members, check-in tags).
    with_user_ids uuid[]           NOT NULL DEFAULT '{}',
    check_in      boolean          NOT NULL DEFAULT false,
    message       text             NOT NULL DEFAULT '',
    -- Coarse area name in English and Japanese.
    locality      text             NOT NULL DEFAULT '',
    locality_ja   text             NOT NULL DEFAULT '',
    country_code  text             NOT NULL DEFAULT '',
    -- The friends who saw the visitor's precise location when the visit began
    -- (visibility::Audience): 0 all friends, 1 all but `audience_ids`, 2 only
    -- `audience_ids`. Nobody else is ever shown that this visit took place.
    audience      smallint         NOT NULL DEFAULT 0 CHECK (audience BETWEEN 0 AND 2),
    audience_ids  uuid[]           NOT NULL DEFAULT '{}',
    created_at    timestamptz      NOT NULL DEFAULT now()
);

-- A stay is identified by when it began: processing the same samples again
-- (a redelivered batch) finds the visit instead of opening a second one.
CREATE UNIQUE INDEX geo_visits_user_time ON geo_visits (user_id, arrived_at);
CREATE INDEX geo_visits_place ON geo_visits (place_id, user_id) WHERE place_id IS NOT NULL;
-- At most one ongoing automatic stay per user.
CREATE UNIQUE INDEX geo_visits_ongoing ON geo_visits (user_id) WHERE departed_at IS NULL;

-- Where the user slept: one row per evening date.
CREATE TABLE geo_nights (
    user_id       uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The evening's date in the time zone of the place.
    night         date             NOT NULL,
    visit_id      uuid REFERENCES geo_visits (id) ON DELETE SET NULL,
    place_id      uuid REFERENCES geo_places (id) ON DELETE SET NULL,
    place_name    text             NOT NULL DEFAULT '',
    lat           double precision NOT NULL,
    lng           double precision NOT NULL,
    started_at    timestamptz      NOT NULL,
    ended_at      timestamptz      NOT NULL,
    with_user_ids uuid[]           NOT NULL DEFAULT '{}',
    at_home       boolean          NOT NULL DEFAULT false,
    locality      text             NOT NULL DEFAULT '',
    locality_ja   text             NOT NULL DEFAULT '',
    PRIMARY KEY (user_id, night)
);

-- Movement per user, hour and activity: the source of day summaries and
-- statistics in any time zone.
CREATE TABLE geo_activity_hours (
    user_id       uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Start of the hour (UTC).
    hour          timestamptz      NOT NULL,
    -- geo Activity number (0 = unknown).
    activity      smallint         NOT NULL,
    distance_m    double precision NOT NULL DEFAULT 0,
    duration_s    integer          NOT NULL DEFAULT 0,
    max_speed_mps real             NOT NULL DEFAULT 0,
    point_count   integer          NOT NULL DEFAULT 0,
    -- Where the user was at the end of the hour.
    locality      text             NOT NULL DEFAULT '',
    locality_ja   text             NOT NULL DEFAULT '',
    country_code  text             NOT NULL DEFAULT '',
    admin1_id     text             NOT NULL DEFAULT '',
    -- Sample time up to which the totals are counted: movement that arrives
    -- a second time (a redelivered batch) is not added again.
    counted_until timestamptz      NOT NULL DEFAULT '-infinity',
    PRIMARY KEY (user_id, hour, activity)
);

-- Diary entry for a day (the user's own calendar day).
CREATE TABLE geo_day_memos (
    user_id         uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    day             date        NOT NULL,
    text            text        NOT NULL DEFAULT '',
    -- common Image messages as proto JSON, and the assets they came from (same order).
    photos          jsonb       NOT NULL DEFAULT '[]',
    photo_asset_ids uuid[]      NOT NULL DEFAULT '{}',
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, day)
);

CREATE TRIGGER geo_day_memos_updated_at
    BEFORE UPDATE ON geo_day_memos
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

CREATE INDEX geo_day_memos_assets ON geo_day_memos USING gin (photo_asset_ids);

-- ───────────────────────────── Footprints ─────────────────────────────

-- Countries and first-level subdivisions, seeded from the boundary data
-- bundled with tokuly-geo (`Geo::seed_regions`).
CREATE TABLE geo_regions (
    id           text PRIMARY KEY,
    -- RegionKind: 2 country, 3 first-level subdivision.
    kind         smallint         NOT NULL,
    name         text             NOT NULL,
    name_ja      text             NOT NULL DEFAULT '',
    -- ISO 3166-1 alpha-2 of the country (the region's own ID for countries).
    country_code text             NOT NULL DEFAULT '',
    -- GeoNames admin1 key (e.g. `JP.40`), linking the offline gazetteer to subdivisions.
    gn_code      text             NOT NULL DEFAULT '',
    area_km2     double precision NOT NULL,
    center_lat   double precision NOT NULL,
    center_lng   double precision NOT NULL,
    geom         geometry(MultiPolygon, 4326) NOT NULL
);

CREATE INDEX geo_regions_geom ON geo_regions USING gist (geom);
CREATE INDEX geo_regions_kind ON geo_regions (kind, country_code);
CREATE INDEX geo_regions_gn ON geo_regions (gn_code) WHERE gn_code <> '';

-- Small key/value store: version of the seeded region data, world land area.
CREATE TABLE geo_meta (
    key   text PRIMARY KEY,
    value text NOT NULL
);

-- Every H3 cell a user has been in. Resolution 9 is the truth; the parents
-- at 7 and 5 are materialized for viewport queries at low zoom.
CREATE TABLE geo_cells (
    user_id    uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    res        smallint         NOT NULL CHECK (res IN (5, 7, 9)),
    -- H3 index.
    cell       bigint           NOT NULL,
    -- Cell center, for viewport filtering.
    lat        double precision NOT NULL,
    lng        double precision NOT NULL,
    -- When the user was first there (device time of the sample)...
    first_at     timestamptz    NOT NULL,
    -- ...and when the server learned of it (a backlog upload arrives later):
    -- the recap reports what was uncovered since it was last looked at.
    uncovered_at timestamptz    NOT NULL DEFAULT now(),
    -- Regions containing the cell center (resolution 9 only; '' = none).
    country_id text             NOT NULL DEFAULT '',
    admin1_id  text             NOT NULL DEFAULT '',
    PRIMARY KEY (user_id, res, cell)
);

CREATE INDEX geo_cells_first ON geo_cells (user_id, first_at) WHERE res = 9;
CREATE INDEX geo_cells_uncovered ON geo_cells (user_id, uncovered_at) WHERE res = 9;

-- Explored cells per user and region.
CREATE TABLE geo_region_stats (
    user_id      uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    region_id    text             NOT NULL REFERENCES geo_regions (id) ON DELETE CASCADE,
    cell_count   bigint           NOT NULL DEFAULT 0,
    explored_km2 double precision NOT NULL DEFAULT 0,
    -- When the user first entered the region.
    first_at     timestamptz      NOT NULL,
    PRIMARY KEY (user_id, region_id)
);

CREATE INDEX geo_region_stats_rank ON geo_region_stats (region_id, cell_count DESC);

-- ───────────────────────────── Trips ─────────────────────────────

CREATE TABLE geo_trips (
    id                   uuid PRIMARY KEY          DEFAULT uuidv7(),
    user_id              uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Exactly one kind of destination: a friend (followed live), a place, or a point.
    dest_user_id         uuid REFERENCES users (id) ON DELETE CASCADE,
    dest_place_id        uuid REFERENCES geo_places (id) ON DELETE SET NULL,
    dest_name            text             NOT NULL DEFAULT '',
    -- Resolved coordinates (for a friend: where the traveller last saw them).
    dest_lat             double precision NOT NULL,
    dest_lng             double precision NOT NULL,
    arrive_radius_m      double precision NOT NULL DEFAULT 60,
    -- The destination's coordinates and the route are for the traveller only
    -- (heading to a friend, or to a private place such as home).
    dest_hidden          boolean          NOT NULL DEFAULT false,
    -- TravelMode / TripStatus numbers.
    mode                 smallint         NOT NULL,
    status               smallint         NOT NULL DEFAULT 1,
    started_at           timestamptz      NOT NULL DEFAULT now(),
    ended_at             timestamptz,
    eta                  timestamptz,
    distance_remaining_m double precision NOT NULL DEFAULT 0,
    route_polyline       text             NOT NULL DEFAULT '',
    -- The plan the live ETA is scaled from: straight-line meters at the
    -- start and the planner's duration for them.
    plan_distance_m      double precision NOT NULL DEFAULT 0,
    plan_duration_s      integer          NOT NULL DEFAULT 0,
    -- Empty = all friends.
    shared_with          uuid[]           NOT NULL DEFAULT '{}',
    notify_on_arrival    boolean          NOT NULL DEFAULT false,
    updated_at           timestamptz      NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX geo_trips_active ON geo_trips (user_id) WHERE status = 1;
CREATE INDEX geo_trips_user ON geo_trips (user_id, started_at DESC);
CREATE INDEX geo_trips_dest_user ON geo_trips (dest_user_id) WHERE status = 1;

-- ───────────────────────────── Safety ─────────────────────────────

-- "Tell me when Aoi gets home."
CREATE TABLE geo_place_alerts (
    id                  uuid PRIMARY KEY     DEFAULT uuidv7(),
    owner_id            uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    subject_id          uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    place_name          text        NOT NULL DEFAULT '',
    -- Where: fixed coordinates, or (subject_place_label <> 0) the subject's
    -- own labelled place, resolved when the alert is evaluated.
    lat                 double precision,
    lng                 double precision,
    radius_m            double precision,
    subject_place_label smallint    NOT NULL DEFAULT 0,
    -- AlertTrigger / AlertStatus numbers.
    trigger             smallint    NOT NULL,
    recurring           boolean     NOT NULL DEFAULT false,
    status              smallint    NOT NULL,
    -- Whether the subject was inside at the last evaluation (NULL = not evaluated yet).
    inside              boolean,
    created_at          timestamptz NOT NULL DEFAULT now(),
    responded_at        timestamptz,
    last_fired_at       timestamptz,
    -- When a pending or one-time alert lapses.
    expires_at          timestamptz,
    CHECK (owner_id <> subject_id),
    CHECK ((subject_place_label <> 0) <> (lat IS NOT NULL AND lng IS NOT NULL AND radius_m IS NOT NULL))
);

CREATE INDEX geo_place_alerts_owner ON geo_place_alerts (owner_id, created_at DESC);
CREATE INDEX geo_place_alerts_subject ON geo_place_alerts (subject_id, created_at DESC);
CREATE INDEX geo_place_alerts_expiry ON geo_place_alerts (expires_at) WHERE status IN (1, 2) AND expires_at IS NOT NULL;

CREATE TABLE geo_sos (
    id              uuid PRIMARY KEY     DEFAULT uuidv7(),
    user_id         uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    message         text        NOT NULL DEFAULT '',
    -- Friends alerted; they see the precise live location until it is resolved.
    notify_user_ids uuid[]      NOT NULL,
    started_at      timestamptz NOT NULL DEFAULT now(),
    resolved_at     timestamptz
);

CREATE UNIQUE INDEX geo_sos_active ON geo_sos (user_id) WHERE resolved_at IS NULL;
CREATE INDEX geo_sos_notified ON geo_sos USING gin (notify_user_ids) WHERE resolved_at IS NULL;
CREATE INDEX geo_sos_user ON geo_sos (user_id, started_at DESC);

-- Time-limited public links to a user's live location.
CREATE TABLE geo_live_shares (
    id         uuid PRIMARY KEY     DEFAULT uuidv7(),
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The capability: whoever knows the token sees the location. Stored as
    -- a keyed hash (lookups) and sealed with the server secret (so that the
    -- owner can be shown their links again and the pipeline can publish on
    -- the share's topic): the table alone opens no link.
    token_hash   text      NOT NULL UNIQUE,
    token_sealed bytea     NOT NULL,
    label      text        NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    -- Set when the share was revoked or its expiry was processed.
    ended_at   timestamptz,
    view_count integer     NOT NULL DEFAULT 0
);

CREATE INDEX geo_live_shares_user ON geo_live_shares (user_id, created_at DESC);
CREATE INDEX geo_live_shares_due ON geo_live_shares (expires_at) WHERE ended_at IS NULL;

-- ───────────────────────────── Gatherings ─────────────────────────────

-- Friends in the same spot (within ~30 m for 2 minutes or more).
CREATE TABLE geo_gatherings (
    id               uuid PRIMARY KEY          DEFAULT uuidv7(),
    lat              double precision NOT NULL,
    lng              double precision NOT NULL,
    -- A public or friends-visible place at the spot, if any.
    place_id         uuid REFERENCES geo_places (id) ON DELETE SET NULL,
    place_name       text             NOT NULL DEFAULT '',
    -- At a member's home, work or school: no "party" treatment, no announcement.
    at_private_place boolean          NOT NULL DEFAULT false,
    started_at       timestamptz      NOT NULL DEFAULT now(),
    ended_at         timestamptz,
    -- Time together has been reported to the social domain up to here.
    credited_until   timestamptz      NOT NULL DEFAULT now(),
    peak_size        integer          NOT NULL DEFAULT 2
);

CREATE INDEX geo_gatherings_active ON geo_gatherings (started_at) WHERE ended_at IS NULL;
CREATE INDEX geo_gatherings_place ON geo_gatherings (place_id) WHERE place_id IS NOT NULL;

CREATE TABLE geo_gathering_members (
    gathering_id uuid        NOT NULL REFERENCES geo_gatherings (id) ON DELETE CASCADE,
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    arrived_at   timestamptz NOT NULL DEFAULT now(),
    left_at      timestamptz,
    -- Last time a sample confirmed the member was still there.
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    -- The other members this one was together with, by their own proximity
    -- check (each sees the other's precise location): who they "met".
    with_user_ids uuid[]     NOT NULL DEFAULT '{}',
    PRIMARY KEY (gathering_id, user_id)
);

-- A user is in at most one gathering at a time.
CREATE UNIQUE INDEX geo_gathering_members_active ON geo_gathering_members (user_id) WHERE left_at IS NULL;
CREATE INDEX geo_gathering_members_user ON geo_gathering_members (user_id, arrived_at DESC);

-- Bumps as announced by the social domain (`social.bump.created`), for history statistics.
CREATE TABLE geo_bumps (
    bump_id    uuid PRIMARY KEY,
    user_a     uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    user_b     uuid             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    lat        double precision NOT NULL,
    lng        double precision NOT NULL,
    place_name text             NOT NULL DEFAULT '',
    created_at timestamptz      NOT NULL DEFAULT now()
);

CREATE INDEX geo_bumps_a ON geo_bumps (user_a, created_at);
CREATE INDEX geo_bumps_b ON geo_bumps (user_b, created_at);
