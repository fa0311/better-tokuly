-- no-transaction
-- The sweep looks for sessions by how far their client is believed (2101).
CREATE INDEX CONCURRENTLY IF NOT EXISTS school_study_sessions_believed
    ON school_study_sessions (GREATEST(heartbeat_at, heard_phase_ends_at)) WHERE state <> 3;
