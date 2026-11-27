-- Whether the owner chose `push_preview` themselves (UpdatePreferences). An
-- account that gives a minor's birthdate later (`identity.user.birthdate_given`)
-- gets the minors' default unless it did. Values stored before this column read
-- as not chosen: an old FULL cannot be told from the old default, and minors
-- come first (a choice narrowed once can be widened again).
ALTER TABLE notify_settings ADD COLUMN push_preview_chosen boolean NOT NULL DEFAULT false;
