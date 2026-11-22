//! Per-user settings: the box, the word filter and compliment polls.
//!
//! A user without a row has the defaults, which are the cautious ones: the
//! box is closed, and once opened it takes questions from friends only.
use sqlx::PgExecutor;
use tokuly_core::prelude::*;
use tokuly_proto::ask::{AskerAudience, Settings, UpdateSettingsRequest};

use crate::text;

const SIGNED_IN: i16 = AskerAudience::ASKER_AUDIENCE_SIGNED_IN as i16;
const FRIENDS: i16 = AskerAudience::ASKER_AUDIENCE_FRIENDS as i16;

/// The line a box shows when its owner wrote none.
pub(crate) const DEFAULT_BOX_PROMPT: (&str, &str) = ("なんでも質問してね!", "Ask me anything!");

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct SettingsRow {
    pub box_open: bool,
    /// `tokuly.ask.v1.AskerAudience`.
    pub audience: i16,
    pub box_prompt: String,
    pub allow_signed: bool,
    pub allow_images: bool,
    pub muted_words: Vec<String>,
    pub in_polls: bool,
    pub allow_hints: bool,
}

impl Default for SettingsRow {
    fn default() -> Self {
        Self {
            box_open: false,
            audience: FRIENDS,
            box_prompt: String::new(),
            allow_signed: true,
            allow_images: false,
            muted_words: Vec::new(),
            in_polls: true,
            allow_hints: true,
        }
    }
}

impl SettingsRow {
    pub(crate) fn friends_only(&self) -> bool {
        self.audience != SIGNED_IN
    }

    pub(crate) fn to_proto(&self) -> Settings {
        Settings {
            box_open: self.box_open,
            audience: i32::from(self.audience).into(),
            box_prompt: self.box_prompt.clone(),
            allow_signed: self.allow_signed,
            allow_images: self.allow_images,
            muted_words: self.muted_words.clone(),
            in_polls: self.in_polls,
            allow_hints: self.allow_hints,
            ..Default::default()
        }
    }

    /// Applies the set fields of an update, validating each.
    pub(crate) fn apply(&mut self, change: &UpdateSettingsRequest) -> AppResult<()> {
        if let Some(open) = change.box_open {
            self.box_open = open;
        }
        if let Some(audience) = change.audience {
            self.audience = match audience.as_known() {
                Some(AskerAudience::ASKER_AUDIENCE_SIGNED_IN) => SIGNED_IN,
                Some(AskerAudience::ASKER_AUDIENCE_FRIENDS) => FRIENDS,
                _ => bail!(invalid, "AUDIENCE_INVALID", "choose who may ask: signed-in users or friends"),
            };
        }
        if let Some(prompt) = &change.box_prompt {
            self.box_prompt = text::single_line(&text::optional(
                prompt,
                text::BOX_PROMPT_MAX,
                "BOX_PROMPT_INVALID",
                "the box prompt",
            )?);
        }
        if let Some(allow) = change.allow_signed {
            self.allow_signed = allow;
        }
        if let Some(allow) = change.allow_images {
            self.allow_images = allow;
        }
        if let Some(words) = change.muted_words.as_option() {
            self.muted_words = text::muted_words(&words.words)?;
        }
        if let Some(in_polls) = change.in_polls {
            self.in_polls = in_polls;
        }
        if let Some(allow) = change.allow_hints {
            self.allow_hints = allow;
        }
        Ok(())
    }
}

const COLUMNS: &str = "box_open, audience, box_prompt, allow_signed, allow_images, muted_words, in_polls, allow_hints";

/// A user's settings; the defaults when they never changed any.
pub(crate) async fn load<'e, E: PgExecutor<'e>>(executor: E, user_id: Uuid) -> AppResult<SettingsRow> {
    let row: Option<SettingsRow> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM ask_settings WHERE user_id = $1")))
            .bind(user_id)
            .fetch_optional(executor)
            .await?;
    Ok(row.unwrap_or_default())
}

pub(crate) async fn save<'e, E: PgExecutor<'e>>(executor: E, user_id: Uuid, settings: &SettingsRow) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO ask_settings (user_id, box_open, audience, box_prompt, allow_signed, allow_images, muted_words, in_polls, allow_hints)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (user_id) DO UPDATE SET
             box_open = EXCLUDED.box_open, audience = EXCLUDED.audience, box_prompt = EXCLUDED.box_prompt,
             allow_signed = EXCLUDED.allow_signed, allow_images = EXCLUDED.allow_images, muted_words = EXCLUDED.muted_words,
             in_polls = EXCLUDED.in_polls, allow_hints = EXCLUDED.allow_hints",
    )
    .bind(user_id)
    .bind(settings.box_open)
    .bind(settings.audience)
    .bind(&settings.box_prompt)
    .bind(settings.allow_signed)
    .bind(settings.allow_images)
    .bind(&settings.muted_words)
    .bind(settings.in_polls)
    .bind(settings.allow_hints)
    .execute(executor)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tokuly_proto::ask::MutedWords;

    use super::*;

    #[test]
    fn defaults_are_closed_and_friends_only() {
        let settings = SettingsRow::default();
        assert!(!settings.box_open && settings.friends_only());
        assert!(!settings.allow_images, "images are opt-in");
        assert!(settings.in_polls && settings.allow_hints);
        assert_eq!(settings.to_proto().audience, AskerAudience::ASKER_AUDIENCE_FRIENDS);
    }

    #[test]
    fn apply_changes_only_what_is_set() {
        let mut settings = SettingsRow::default();
        settings
            .apply(&UpdateSettingsRequest {
                box_open: Some(true),
                box_prompt: Some("  なんでも\nどうぞ ".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(settings.box_open);
        assert_eq!(settings.box_prompt, "なんでも どうぞ");
        assert!(settings.friends_only() && settings.allow_signed, "untouched fields keep their value");

        settings
            .apply(&UpdateSettingsRequest {
                audience: Some(AskerAudience::ASKER_AUDIENCE_SIGNED_IN.into()),
                muted_words: MutedWords { words: vec!["うざい".into(), "うざい".into()], ..Default::default() }.into(),
                ..Default::default()
            })
            .unwrap();
        assert!(!settings.friends_only());
        assert_eq!(settings.muted_words, vec!["うざい"]);
    }

    #[test]
    fn apply_rejects_bad_values() {
        let mut settings = SettingsRow::default();
        let err = settings
            .apply(&UpdateSettingsRequest {
                audience: Some(AskerAudience::ASKER_AUDIENCE_UNSPECIFIED.into()),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(err.reason, "AUDIENCE_INVALID");
        let err = settings
            .apply(&UpdateSettingsRequest { box_prompt: Some("あ".repeat(81)), ..Default::default() })
            .unwrap_err();
        assert_eq!(err.reason, "BOX_PROMPT_INVALID");
    }
}
