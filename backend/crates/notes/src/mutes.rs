//! What someone does not want to see: muted words (plain, or regular
//! expressions), muted conversations, people whose renotes they hide — and
//! the accounts they muted in the social domain (follow mutes), which this
//! domain only reads.
//!
//! Regular expressions run on the `regex` crate, whose matching time grows
//! linearly with the text (no backtracking), compiled with small size
//! limits: a pattern cannot make reading a timeline slow.
//!
//! Where it applies ([`TimelineFilter`]): the home, following, global,
//! hashtag, list, antenna and channel timelines and trending notes. A
//! profile, a thread, the mentions timeline and a note opened directly
//! show everything (the person chose to look there). Muted conversations
//! stop reply and mention notifications from them.
use std::{collections::HashSet, time::Duration};

use regex::{Regex, RegexBuilder};
use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_proto::{common::UserSummary, notes::MutedWord};
use tokuly_social::graph;

use crate::{Notes, hydrate::EntryRow, model::NoteRow, text, visibility::Viewer};

pub const MAX_MUTED_WORDS: usize = 100;
pub const MAX_MUTED_WORD_CHARS: usize = 100;
pub const MAX_MUTED_THREADS: i64 = 1000;
pub const MAX_RENOTE_MUTES: i64 = 1000;
/// Compiled size limits of one muted pattern.
const REGEX_SIZE_LIMIT: usize = 64 * 1024;
const REGEX_DFA_LIMIT: usize = 256 * 1024;
/// Mute changes one user may make per minute.
const CHANGES_PER_MINUTE: u32 = 60;

/// One muted word, ready to match folded text.
#[derive(Debug, Clone)]
enum Matcher {
    Plain(String),
    Pattern(Regex),
}

fn fold(input: &str) -> String {
    use unicode_normalization::UnicodeNormalization as _;
    input.nfkc().collect::<String>().to_lowercase()
}

fn compile(pattern: &str) -> Option<Regex> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_DFA_LIMIT)
        .nest_limit(32)
        .build()
        .ok()
}

/// Checks and normalizes muted words (`INVALID_MUTED_WORD`, `TOO_MANY_MUTED_WORDS`).
pub(crate) fn check_words(words: &[MutedWord]) -> AppResult<Vec<MutedWord>> {
    ensure!(words.len() <= MAX_MUTED_WORDS, invalid, "TOO_MANY_MUTED_WORDS", "at most {MAX_MUTED_WORDS} muted words");
    let mut out: Vec<MutedWord> = Vec::with_capacity(words.len());
    for word in words {
        let text = text::clean(&word.text);
        let text = if word.regex { text } else { text.split_whitespace().collect::<Vec<_>>().join(" ") };
        ensure!(
            !text.is_empty() && text::fits(&text, MAX_MUTED_WORD_CHARS) && (!word.regex || compile(&text).is_some()),
            invalid,
            "INVALID_MUTED_WORD",
            "a muted word has 1-{MAX_MUTED_WORD_CHARS} characters, and a pattern must be a valid regular expression"
        );
        if !out.iter().any(|seen| seen.text == text && seen.regex == word.regex) {
            out.push(MutedWord { text, regex: word.regex, ..Default::default() });
        }
    }
    Ok(out)
}

/// What one viewer keeps out of their timelines.
#[derive(Debug, Clone, Default)]
pub(crate) struct TimelineFilter {
    words: Vec<Matcher>,
    renotes_hidden_from: HashSet<Uuid>,
    muted_accounts: HashSet<Uuid>,
    viewer: Option<Uuid>,
}

impl TimelineFilter {
    pub async fn load(core: &Core, viewer: &Viewer) -> AppResult<Self> {
        let Some(user_id) = viewer.id else { return Ok(Self::default()) };
        let (words, renotes, muted) = tokio::try_join!(
            muted_words(core, user_id),
            async {
                let ids: Vec<Uuid> = sqlx::query_scalar("SELECT target_id FROM note_renote_mutes WHERE user_id = $1")
                    .bind(user_id)
                    .fetch_all(&core.db)
                    .await?;
                AppResult::Ok(ids)
            },
            graph::muted_followee_ids(core, user_id),
        )?;
        let words = words
            .iter()
            .filter_map(|word| {
                if word.regex {
                    compile(&word.text).map(Matcher::Pattern)
                } else {
                    Some(Matcher::Plain(fold(&word.text)))
                }
            })
            .collect();
        Ok(Self {
            words,
            renotes_hidden_from: renotes.into_iter().collect(),
            muted_accounts: muted.into_iter().collect(),
            viewer: Some(user_id),
        })
    }

    fn is_empty(&self) -> bool {
        self.words.is_empty() && self.renotes_hidden_from.is_empty() && self.muted_accounts.is_empty()
    }

    /// Whether the note's words contain a muted word.
    pub fn mutes_words_of(&self, note: &NoteRow) -> bool {
        if self.words.is_empty() || Some(note.author_id) == self.viewer {
            return false;
        }
        let haystack = fold(&format!("{}\n{}", note.content_warning_text, note.text));
        self.words.iter().any(|word| match word {
            Matcher::Plain(plain) => haystack.contains(plain.as_str()),
            Matcher::Pattern(pattern) => pattern.is_match(&haystack),
        })
    }

    /// Whether a timeline row stays out.
    pub fn hides(&self, row: &EntryRow) -> bool {
        if let Some(renoter) = row.renoter_id
            && (self.renotes_hidden_from.contains(&renoter) || self.muted_accounts.contains(&renoter))
        {
            return true;
        }
        let author = row.note.author_id;
        (Some(author) != self.viewer && self.muted_accounts.contains(&author)) || self.mutes_words_of(&row.note)
    }

    pub fn apply(&self, rows: &mut Vec<EntryRow>) {
        if !self.is_empty() {
            rows.retain(|row| !self.hides(row));
        }
    }
}

async fn muted_words(core: &Core, user_id: Uuid) -> AppResult<Vec<MutedWord>> {
    let words: Option<serde_json::Value> = sqlx::query_scalar("SELECT words FROM note_muted_words WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(&core.db)
        .await?;
    Ok(words.and_then(|words| serde_json::from_value(words).ok()).unwrap_or_default())
}

async fn lock_mutes(tx: &mut Transaction<'_, Postgres>, user_id: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("notes:mutes:{user_id}"))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The caller's mute settings, as `GetMuteSettings` returns them.
pub(crate) struct MuteSettings {
    pub words: Vec<MutedWord>,
    pub renotes_hidden_from: Vec<UserSummary>,
    pub muted_thread_ids: Vec<Uuid>,
}

impl Notes {
    async fn admit_mute_change(&self, user_id: Uuid) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("notes:mutes:{user_id}"), CHANGES_PER_MINUTE, Duration::from_secs(60)).await
    }

    pub(crate) async fn mute_settings(&self, user_id: Uuid) -> AppResult<MuteSettings> {
        let core = &self.core;
        let words = muted_words(core, user_id).await?;
        let hidden: Vec<Uuid> =
            sqlx::query_scalar("SELECT target_id FROM note_renote_mutes WHERE user_id = $1 ORDER BY muted_at DESC")
                .bind(user_id)
                .fetch_all(&core.db)
                .await?;
        let threads: Vec<Uuid> = sqlx::query_scalar(
            "SELECT root_note_id FROM note_thread_mutes WHERE user_id = $1 ORDER BY muted_at DESC LIMIT 100",
        )
        .bind(user_id)
        .fetch_all(&core.db)
        .await?;
        Ok(MuteSettings {
            words,
            renotes_hidden_from: tokuly_identity::summaries_ordered(core, &hidden).await?,
            muted_thread_ids: threads,
        })
    }

    pub(crate) async fn update_muted_words(&self, user_id: Uuid, words: &[MutedWord]) -> AppResult<Vec<MutedWord>> {
        self.admit_mute_change(user_id).await?;
        let words = check_words(words)?;
        sqlx::query(
            "INSERT INTO note_muted_words (user_id, words) VALUES ($1, $2)
             ON CONFLICT (user_id) DO UPDATE SET words = EXCLUDED.words, updated_at = now()",
        )
        .bind(user_id)
        .bind(crate::model::to_json(&words)?)
        .execute(&self.core.db)
        .await?;
        Ok(words)
    }

    /// Mutes the conversation a note the caller can see is in.
    pub(crate) async fn mute_thread(&self, viewer: &Viewer, user_id: Uuid, note_id: Uuid) -> AppResult<()> {
        self.admit_mute_change(user_id).await?;
        let note = self.require_note(viewer, note_id).await?;
        let mut tx = self.core.db.begin().await?;
        lock_mutes(&mut tx, user_id).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM note_thread_mutes WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
        let inserted =
            sqlx::query("INSERT INTO note_thread_mutes (user_id, root_note_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(user_id)
                .bind(note.thread_root())
                .execute(&mut *tx)
                .await?
                .rows_affected();
        ensure!(
            inserted == 0 || count < MAX_MUTED_THREADS,
            precondition,
            "TOO_MANY_MUTED_THREADS",
            "at most {MAX_MUTED_THREADS} muted conversations"
        );
        tx.commit().await?;
        Ok(())
    }

    /// Unmutes a conversation, named by any of its notes (or its root, even
    /// after that note is gone). Never refused.
    pub(crate) async fn unmute_thread(&self, user_id: Uuid, note_id: Uuid) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM note_thread_mutes WHERE user_id = $1
               AND root_note_id IN ($2, COALESCE((SELECT thread_root_id FROM notes WHERE id = $2), $2))",
        )
        .bind(user_id)
        .bind(note_id)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    pub(crate) async fn hide_renotes_from(&self, viewer: &Viewer, user_id: Uuid, target: Uuid) -> AppResult<()> {
        self.admit_mute_change(user_id).await?;
        ensure!(user_id != target, invalid, "USER_NOT_FOUND", "user not found");
        self.require_user(viewer, target).await?;
        let mut tx = self.core.db.begin().await?;
        lock_mutes(&mut tx, user_id).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM note_renote_mutes WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
        let inserted =
            sqlx::query("INSERT INTO note_renote_mutes (user_id, target_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(user_id)
                .bind(target)
                .execute(&mut *tx)
                .await?
                .rows_affected();
        ensure!(
            inserted == 0 || count < MAX_RENOTE_MUTES,
            precondition,
            "TOO_MANY_RENOTE_MUTES",
            "renotes can be hidden from at most {MAX_RENOTE_MUTES} people"
        );
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn show_renotes_from(&self, user_id: Uuid, target: Uuid) -> AppResult<()> {
        sqlx::query("DELETE FROM note_renote_mutes WHERE user_id = $1 AND target_id = $2")
            .bind(user_id)
            .bind(target)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// Of these notes, the conversations the viewer muted (by root).
    pub(crate) async fn muted_thread_roots(&self, viewer: &Viewer, notes: &[&NoteRow]) -> AppResult<HashSet<Uuid>> {
        let Some(user_id) = viewer.id else { return Ok(HashSet::new()) };
        let mut roots: Vec<Uuid> = notes.iter().map(|note| note.thread_root()).collect();
        if roots.is_empty() {
            return Ok(HashSet::new());
        }
        roots.sort();
        roots.dedup();
        let muted: Vec<Uuid> = sqlx::query_scalar(
            "SELECT root_note_id FROM note_thread_mutes WHERE user_id = $1 AND root_note_id = ANY($2)",
        )
        .bind(user_id)
        .bind(&roots)
        .fetch_all(&self.core.db)
        .await?;
        Ok(muted.into_iter().collect())
    }

    /// Of `people`, those who muted the conversation `note` is in (or that a
    /// reply to `note` would be in).
    pub(crate) async fn thread_muters(&self, note: &NoteRow, people: &[Uuid]) -> AppResult<HashSet<Uuid>> {
        if people.is_empty() {
            return Ok(HashSet::new());
        }
        let muted: Vec<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM note_thread_mutes WHERE root_note_id = $1 AND user_id = ANY($2)")
                .bind(note.thread_root())
                .bind(people)
                .fetch_all(&self.core.db)
                .await?;
        Ok(muted.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, regex: bool) -> MutedWord {
        MutedWord { text: text.into(), regex, ..Default::default() }
    }

    #[test]
    fn muted_words_are_checked_and_patterns_compiled_within_limits() {
        let words =
            check_words(&[word("  ネタバレ  ", false), word("ネタバレ", false), word("(?i)spoil(er)?s?", true)])
                .unwrap();
        assert_eq!(words.len(), 2);
        for bad in [
            word("", false),
            word("(", true),
            word(&"a".repeat(MAX_MUTED_WORD_CHARS + 1), false),
            word("a{1000}{1000}", true),
        ] {
            assert_eq!(check_words(std::slice::from_ref(&bad)).unwrap_err().reason, "INVALID_MUTED_WORD", "{bad:?}");
        }
        let many: Vec<MutedWord> = (0..=MAX_MUTED_WORDS).map(|i| word(&format!("w{i}"), false)).collect();
        assert_eq!(check_words(&many).unwrap_err().reason, "TOO_MANY_MUTED_WORDS");
    }

    #[test]
    fn folding_matches_width_and_case() {
        let filter =
            TimelineFilter { words: vec![Matcher::Plain(fold("ＳＰＯＩＬＥＲ"))], ..Default::default() };
        assert!(
            filter
                .words
                .iter()
                .any(|w| matches!(w, Matcher::Plain(p) if fold("a big spoiler here").contains(p.as_str())))
        );
    }
}
