-- Accounts without a birthdate signed up before the question came, and no
-- birthdate counts as a minor: other people's AI requests leave their messages
-- out. Stored now, while the absence still shows: once a birthdate is given,
-- the sign-up day's age group cannot be told from `users` any more, and a
-- read finding nothing stored would decide by the new date. A choice stays.
INSERT INTO chat_ai_user_settings (user_id, allow_message_use)
SELECT id, false FROM users
WHERE birthdate IS NULL AND NOT bot AND deleted_at IS NULL
ON CONFLICT (user_id) DO UPDATE SET allow_message_use = false, updated_at = now()
WHERE NOT chat_ai_user_settings.message_use_chosen
    AND chat_ai_user_settings.allow_message_use IS DISTINCT FROM false;
