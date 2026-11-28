-- Tokuly Bot: the AI account of the server that answers in chat.

-- 「Tokuly Bot と話す」, stored like `allow_message_use`: when the account is
-- created (identity.user.created), off for a minor, on for an adult, by the
-- age at creation, and kept until the owner chooses. NULL (a lost event) is
-- filled the same way the first time it is read.
ALTER TABLE chat_ai_user_settings
    ADD COLUMN bot_enabled boolean,
    ADD COLUMN bot_chosen  boolean NOT NULL DEFAULT false;

-- Accounts that existed before get the default the lazy fill would give
-- them: by their age when the account was created. Unknown age: off.
UPDATE chat_ai_user_settings s
SET bot_enabled = u.birthdate IS NOT NULL AND u.birthdate <= (u.created_at - interval '18 years')::date
FROM users u
WHERE u.id = s.user_id AND s.bot_enabled IS NULL;

-- Which messages Tokuly Bot took up: one answer per message, however often
-- the job runs. A row is written before the AI service is asked.
CREATE TABLE chat_bot_replies (
    message_id       uuid        PRIMARY KEY REFERENCES chat_messages (id) ON DELETE CASCADE,
    -- The answer, once posted; NULL when the bot stayed silent (switched
    -- off, limited, failed).
    reply_message_id uuid        REFERENCES chat_messages (id) ON DELETE SET NULL,
    created_at       timestamptz NOT NULL DEFAULT now()
);
