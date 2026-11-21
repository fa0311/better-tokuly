-- Raw location history of the geo domain (tokuly_geo::history): one row per
-- accepted location sample of a user who records history. Day timelines and
-- route replay read it; `DeleteHistory`, per-user retention
-- (`location_history_days`) and account deletion remove rows with lightweight
-- deletes.
--
-- A sample re-sent by a device replaces the earlier row (same user and time).
-- Unknown values: `altitude` NaN, `heading` and `battery` -1.
-- `flags`: 1 mocked, 2 foreground, 4 charging.
CREATE TABLE IF NOT EXISTS geo_points
(
    user_id     UUID,
    recorded_at DateTime64(3, 'UTC'),
    lat         Float64,
    lng         Float64,
    accuracy    Float32,
    altitude    Float32,
    speed       Float32,
    heading     Float32,
    activity    UInt8,
    battery     Int8,
    flags       UInt8
)
ENGINE = ReplacingMergeTree
PARTITION BY toYYYYMM(recorded_at)
ORDER BY (user_id, recorded_at)
