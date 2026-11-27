-- A running pomodoro phase is believed to its end (#334): a locked phone
-- sends no heartbeats. `heard_phase_ends_at` is, when the client was last
-- heard from, the end of the pomodoro phase then in progress (NULL for a
-- stopwatch or a timer not running). The client is believed up to the later
-- of it and `heartbeat_at`; the idle limit counts from there.
ALTER TABLE school_study_sessions ADD COLUMN heard_phase_ends_at timestamptz;
