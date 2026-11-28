-- Moments: prompt regions become a few bands of UTC offsets, and every
-- account has a day of its own, fixed when the day's prompt reaches it (or
-- at its first post). A change of time zone takes effect from the next day.

-- Which region each time zone is in, by its UTC offset right now (the
-- planner keeps it up to date, daylight saving included).
CREATE TABLE moment_zones (
    time_zone   text        PRIMARY KEY,
    region      text        NOT NULL REFERENCES moment_regions (region) ON DELETE CASCADE,
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX moment_zones_region ON moment_zones (region);
CREATE TRIGGER moment_zones_updated_at BEFORE UPDATE ON moment_zones
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Every account's current day: the latest prompt that reached it. `date`
-- only ever moves forward, so nobody has two days, prompts or moments of
-- one date.
CREATE TABLE moment_people (
    user_id     uuid        PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    day_id      uuid        NOT NULL REFERENCES moment_days (id),
    date        date        NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX moment_people_day ON moment_people (day_id);
CREATE TRIGGER moment_people_updated_at BEFORE UPDATE ON moment_people
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Everyone keeps the day their time zone was on.
INSERT INTO moment_people (user_id, day_id, date)
SELECT u.id, d.id, d.date
FROM users u
JOIN moment_regions r ON r.region = u.time_zone
JOIN moment_days d ON d.id = r.current_day_id
WHERE u.deleted_at IS NULL AND NOT u.bot;

-- The days drawn for the time-zone regions that have not gone out never
-- will: the planner draws the bands instead. Past days stay with their
-- moments.
DELETE FROM moment_days WHERE announced_at IS NULL;
