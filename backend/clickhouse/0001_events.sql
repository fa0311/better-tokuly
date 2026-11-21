-- Product analytics events (tokuly_core::analytics). One row per tracked
-- event; creator dashboards and admin statistics aggregate over it.
--
-- `owner_id` is the creator/owner the event counts for, `subject_*` what it
-- is about, `user_id` who did it. Absent IDs are stored as the zero UUID.
CREATE TABLE IF NOT EXISTS events
(
    id           UUID,
    name         LowCardinality(String),
    user_id      UUID,
    subject_type LowCardinality(String),
    subject_id   UUID,
    owner_id     UUID,
    at           DateTime64(3, 'UTC'),
    value        Float64,
    props        String CODEC(ZSTD(3))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(at)
ORDER BY (owner_id, name, subject_id, at)
TTL toDateTime(at) + INTERVAL 2 YEAR
