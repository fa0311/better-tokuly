-- Accounts without a birthdate signed up before the question came, and no
-- birthdate counts as a minor: their pushes start with the sender only. Stored
-- now, while the absence still shows: once a birthdate is given, the sign-up
-- day's age group cannot be told from `users` any more, and a push finding
-- nothing stored would decide by the new date. As migration 1201, a FULL
-- stored by a default counts as not chosen; nothing widens.
INSERT INTO notify_settings (user_id, push_preview)
SELECT id, 2 FROM users
WHERE birthdate IS NULL AND NOT bot AND deleted_at IS NULL
ON CONFLICT (user_id) DO UPDATE SET push_preview = EXCLUDED.push_preview
WHERE NOT notify_settings.push_preview_chosen AND COALESCE(notify_settings.push_preview, 1) = 1;
