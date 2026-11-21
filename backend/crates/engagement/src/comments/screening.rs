//! The rules an owner sets for comments on their subjects: review
//! everything, hold links, blocked words, and people who may not comment at
//! all. [`screen`] turns them into the status a new or edited comment gets.
use sqlx::PgExecutor;
use tokuly_core::prelude::*;
use tokuly_proto::engagement::{HoldReason, ModerationSettings};

use super::status;
use crate::{Engagement, text};

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub(crate) struct SettingsRow {
    pub hold_all: bool,
    pub hold_links: bool,
    pub blocked_words: Vec<String>,
}

impl SettingsRow {
    pub(crate) fn to_proto(&self) -> ModerationSettings {
        ModerationSettings {
            hold_all: self.hold_all,
            hold_links: self.hold_links,
            blocked_words: self.blocked_words.clone(),
            ..Default::default()
        }
    }
}

/// An owner's rules; the defaults (nothing is held) when they never set any.
pub(crate) async fn settings_of<'e, E: PgExecutor<'e>>(executor: E, owner_id: Uuid) -> AppResult<SettingsRow> {
    let row: Option<SettingsRow> =
        sqlx::query_as("SELECT hold_all, hold_links, blocked_words FROM comment_settings WHERE user_id = $1")
            .bind(owner_id)
            .fetch_optional(executor)
            .await?;
    Ok(row.unwrap_or_default())
}

pub(crate) async fn save_settings<'e, E: PgExecutor<'e>>(
    executor: E,
    owner_id: Uuid,
    settings: &SettingsRow,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO comment_settings (user_id, hold_all, hold_links, blocked_words) VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id) DO UPDATE SET hold_all = $2, hold_links = $3, blocked_words = $4",
    )
    .bind(owner_id)
    .bind(settings.hold_all)
    .bind(settings.hold_links)
    .bind(&settings.blocked_words)
    .execute(executor)
    .await?;
    Ok(())
}

/// Whether `owner_id` stopped the account `user_id` from commenting, under
/// whichever identity.
pub(crate) async fn is_blocked_commenter<'e, E: PgExecutor<'e>>(
    executor: E,
    owner_id: Uuid,
    user_id: Uuid,
) -> AppResult<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM comment_blocks WHERE owner_id = $1 AND blocked_user_id = $2)")
        .bind(owner_id)
        .bind(user_id)
        .fetch_one(executor)
        .await?)
}

/// Where a comment goes: its status and, unless published, why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Verdict {
    pub status: i32,
    pub reason: HoldReason,
}

impl Verdict {
    pub(crate) const PUBLISH: Self = Self { status: status::PUBLISHED, reason: HoldReason::HOLD_REASON_UNSPECIFIED };

    fn hold(reason: HoldReason) -> Self {
        Self { status: status::HELD, reason }
    }

    pub(crate) fn is_published(&self) -> bool {
        self.status == status::PUBLISHED
    }
}

/// Applies an owner's rules to a comment text. A blocked commenter's
/// comment is hidden outright — only they see it — and is not queued for
/// review; everything else that trips a rule waits for the owner.
pub(crate) fn screen(settings: &SettingsRow, commenter_blocked: bool, text: &str) -> Verdict {
    if commenter_blocked {
        Verdict { status: status::HIDDEN, reason: HoldReason::HOLD_REASON_BLOCKED_COMMENTER }
    } else if settings.hold_all {
        Verdict::hold(HoldReason::HOLD_REASON_REVIEW_ALL)
    } else if text::find_blocked_word(text, &settings.blocked_words).is_some() {
        Verdict::hold(HoldReason::HOLD_REASON_BLOCKED_WORD)
    } else if settings.hold_links && text::contains_link(text) {
        Verdict::hold(HoldReason::HOLD_REASON_LINK)
    } else {
        Verdict::PUBLISH
    }
}

impl Engagement {
    /// Screens a comment `author_id` writes on a subject of `owner_id`. The
    /// owner's own comments, and comments on content that belongs to
    /// nobody, are never screened.
    pub(crate) async fn screen_comment(&self, owner_id: Uuid, author_id: Uuid, text: &str) -> AppResult<Verdict> {
        if owner_id.is_nil() || owner_id == author_id {
            return Ok(Verdict::PUBLISH);
        }
        let settings = settings_of(&self.core.db, owner_id).await?;
        let blocked = is_blocked_commenter(&self.core.db, owner_id, author_id).await?;
        Ok(screen(&settings, blocked, text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(hold_all: bool, hold_links: bool, words: &[&str]) -> SettingsRow {
        SettingsRow { hold_all, hold_links, blocked_words: words.iter().map(|w| w.to_string()).collect() }
    }

    #[test]
    fn without_rules_everything_is_published() {
        assert_eq!(screen(&SettingsRow::default(), false, "https://example.com ばか"), Verdict::PUBLISH);
    }

    #[test]
    fn a_blocked_commenter_is_hidden_before_any_other_rule() {
        let verdict = screen(&settings(true, true, &["x"]), true, "x https://example.com");
        assert_eq!((verdict.status, verdict.reason), (status::HIDDEN, HoldReason::HOLD_REASON_BLOCKED_COMMENTER));
    }

    #[test]
    fn rules_hold_comments_with_the_first_reason_that_applies() {
        let all = settings(true, true, &["spam"]);
        assert_eq!(screen(&all, false, "hello").reason, HoldReason::HOLD_REASON_REVIEW_ALL);

        let words_and_links = settings(false, true, &["spam"]);
        let verdict = screen(&words_and_links, false, "ＳＰＡＭ at example.com");
        assert_eq!((verdict.status, verdict.reason), (status::HELD, HoldReason::HOLD_REASON_BLOCKED_WORD));
        assert_eq!(screen(&words_and_links, false, "see example.com").reason, HoldReason::HOLD_REASON_LINK);
        assert!(screen(&words_and_links, false, "こんにちは").is_published());

        // Links are fine unless the owner holds them.
        assert!(screen(&settings(false, false, &["spam"]), false, "see example.com").is_published());
    }
}
