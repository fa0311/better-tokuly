-- Whether the owner chose `push_preview` themselves (UpdatePreferences). An
-- account that gives a minor's birthdate later (`identity.user.birthdate_given`)
-- gets the minors' default unless it did. Choices made before this column read
-- as not chosen.
ALTER TABLE notify_settings ADD COLUMN push_preview_chosen boolean NOT NULL DEFAULT false;
