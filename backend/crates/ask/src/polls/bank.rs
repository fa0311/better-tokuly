//! The question bank of the compliment polls.
//!
//! The built-in questions ship with the crate as data
//! (`data/poll_questions.tsv`) and are copied into `ask_poll_questions` at
//! startup; staff add their own and switch any question off by RPC. The
//! copy never switches a question back on, so a question staff disabled
//! stays disabled across releases.
use chrono::{DateTime, Utc};
use tokuly_core::prelude::*;
use tokuly_proto::{
    ask::{PollQuestion, PollQuestionEntry},
    common::{LocalizedText, PageInfo},
};

use crate::{Ask, text};

const DATA: &str = include_str!("../../data/poll_questions.tsv");

/// A built-in question, as written in the data file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuiltIn {
    /// Its permanent name; `ask_poll_questions.key`.
    pub key: &'static str,
    pub emoji: &'static str,
    pub ja: &'static str,
    pub en: &'static str,
}

/// The built-in questions. Lines starting with `#` and empty lines are
/// comments; a line that is not `key<TAB>emoji<TAB>ja<TAB>en` is a mistake
/// in the file, which the tests below catch before it ships.
pub(crate) fn built_in() -> Vec<BuiltIn> {
    DATA.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let question =
                BuiltIn { key: fields.next()?, emoji: fields.next()?, ja: fields.next()?, en: fields.next()? };
            fields.next().is_none().then_some(question)
        })
        .collect()
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct QuestionRow {
    pub id: Uuid,
    pub key: Option<String>,
    pub text_ja: String,
    pub text_en: String,
    pub emoji: String,
    pub enabled: bool,
    pub use_count: i64,
    pub created_at: DateTime<Utc>,
}

const COLUMNS: &str = "id, key, text_ja, text_en, emoji, enabled, use_count, created_at";

/// A bank question as shown in polls and on flames.
pub(crate) fn poll_question(id: Uuid, text_ja: &str, text_en: &str, emoji: &str) -> PollQuestion {
    PollQuestion {
        id: id.to_string(),
        text: LocalizedText { ja: text_ja.to_owned(), en: text_en.to_owned(), ..Default::default() }.into(),
        emoji: emoji.to_owned(),
        ..Default::default()
    }
}

impl QuestionRow {
    pub(crate) fn to_proto(&self) -> PollQuestion {
        poll_question(self.id, &self.text_ja, &self.text_en, &self.emoji)
    }

    pub(crate) fn to_entry(&self) -> PollQuestionEntry {
        PollQuestionEntry {
            question: self.to_proto().into(),
            enabled: self.enabled,
            built_in: self.key.is_some(),
            use_count: self.use_count,
            created_at: ts(self.created_at),
            ..Default::default()
        }
    }
}

impl Ask {
    /// Copies the built-in questions into the database, once per process.
    /// Safe to run from several processes at once.
    pub(crate) async fn ensure_bank(&self) -> AppResult<()> {
        self.bank_ready.get_or_try_init(|| self.seed_bank()).await.map(|_| ())
    }

    async fn seed_bank(&self) -> AppResult<()> {
        let questions = built_in();
        let keys: Vec<&str> = questions.iter().map(|q| q.key).collect();
        let emoji: Vec<&str> = questions.iter().map(|q| q.emoji).collect();
        let ja: Vec<&str> = questions.iter().map(|q| q.ja).collect();
        let en: Vec<&str> = questions.iter().map(|q| q.en).collect();
        let mut tx = self.core.db.begin().await?;
        // New questions arrive enabled; known ones get their current wording
        // and keep whatever `enabled` staff left them with.
        sqlx::query(
            "INSERT INTO ask_poll_questions (key, emoji, text_ja, text_en)
             SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[], $4::text[])
             ON CONFLICT (key) DO UPDATE SET emoji = EXCLUDED.emoji, text_ja = EXCLUDED.text_ja, text_en = EXCLUDED.text_en
             WHERE (ask_poll_questions.emoji, ask_poll_questions.text_ja, ask_poll_questions.text_en)
                   IS DISTINCT FROM (EXCLUDED.emoji, EXCLUDED.text_ja, EXCLUDED.text_en)",
        )
        .bind(&keys)
        .bind(&emoji)
        .bind(&ja)
        .bind(&en)
        .execute(&mut *tx)
        .await?;
        // A built-in question that left the file is retired.
        let retired = sqlx::query(
            "UPDATE ask_poll_questions SET enabled = false WHERE key IS NOT NULL AND enabled AND NOT (key = ANY($1))",
        )
        .bind(&keys)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        if retired > 0 {
            tracing::info!(retired, "retired built-in poll questions that are no longer shipped");
        }
        Ok(())
    }

    /// The whole bank for staff, oldest first.
    pub(crate) async fn bank_page(&self, page: &Page) -> AppResult<(Vec<QuestionRow>, PageInfo)> {
        self.ensure_bank().await?;
        let cursor: Option<Uuid> = page.decode()?;
        let mut rows: Vec<QuestionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM ask_poll_questions WHERE ($1::uuid IS NULL OR id > $1) ORDER BY id LIMIT $2"
        )))
        .bind(cursor)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }

    /// Adds a question written by staff.
    pub(crate) async fn add_bank_question(
        &self,
        staff: Uuid,
        ja: &str,
        en: &str,
        emoji: &str,
    ) -> AppResult<QuestionRow> {
        let ja = text::single_line(&text::required(
            ja,
            text::POLL_QUESTION_MAX,
            "POLL_QUESTION_INVALID",
            "the Japanese text",
        )?);
        let en = text::single_line(&text::required(
            en,
            text::POLL_QUESTION_MAX,
            "POLL_QUESTION_INVALID",
            "the English text",
        )?);
        let emoji = text::emoji(emoji, "POLL_QUESTION_INVALID")?;
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO ask_poll_questions (text_ja, text_en, emoji, created_by) VALUES ($1, $2, $3, $4) RETURNING {COLUMNS}"
        )))
        .bind(ja)
        .bind(en)
        .bind(emoji)
        .bind(staff)
        .fetch_one(&self.core.db)
        .await?)
    }

    /// Switches a question on or off. Polls already drawn keep theirs.
    pub(crate) async fn set_bank_question_enabled(&self, question_id: Uuid, enabled: bool) -> AppResult<QuestionRow> {
        let row: Option<QuestionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE ask_poll_questions SET enabled = $2 WHERE id = $1 RETURNING {COLUMNS}"
        )))
        .bind(question_id)
        .bind(enabled)
        .fetch_optional(&self.core.db)
        .await?;
        row.or_not_found("POLL_QUESTION_NOT_FOUND", "poll question not found")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::filter::{WordFilter, prepare};

    /// Phrases a compliment poll must never contain: ranking looks, romance,
    /// and anything unkind. This catches a careless addition; it is not what
    /// makes the bank kind. Every line was also read by a person, and has to
    /// be again when one is added.
    const NEVER: &[&str] = &[
        // looks
        "かわい",
        "カワイイ",
        "可愛",
        "イケメン",
        "美人",
        "美男",
        "美女",
        "顔がいい",
        "顔が",
        "見た目",
        "ルックス",
        "スタイルがいい",
        "太って",
        "痩せ",
        "身長",
        "cute",
        "cuter",
        "cutest",
        "hot",
        "hotter",
        "hottest",
        "handsome",
        "pretty",
        "prettier",
        "prettiest",
        "beautiful",
        "sexy",
        "sexiest",
        "good looking",
        "best looking",
        "body",
        "skinny",
        "skinniest",
        // romance
        "好きな人",
        "付き合",
        "恋",
        "彼氏",
        "彼女",
        "デート",
        "告白",
        "キス",
        "結婚",
        "モテ",
        "crush",
        "date",
        "dating",
        "kiss",
        "boyfriend",
        "girlfriend",
        "marry",
        "love",
        // unkind
        "嫌い",
        "うるさ",
        "ダサ",
        "きらい",
        "苦手",
        "変な",
        "バカ",
        "アホ",
        "怖い",
        "暗い",
        "ugly",
        "uglier",
        "ugliest",
        "worst",
        "annoying",
        "fat",
        "fatter",
        "fattest",
        "loud",
        "louder",
        "loudest",
        "weird",
        "weirdest",
        "stupid",
        "stupidest",
        "dumb",
        "dumbest",
        "boring",
        "least",
        "hate",
    ];

    #[test]
    fn every_line_of_the_file_is_a_question() {
        let lines = DATA.lines().filter(|line| !line.trim().is_empty() && !line.starts_with('#')).count();
        assert_eq!(
            built_in().len(),
            lines,
            "a line of poll_questions.tsv does not have exactly four tab-separated fields"
        );
    }

    #[test]
    fn the_bank_is_large_and_its_keys_are_unique() {
        let bank = built_in();
        assert!(bank.len() >= 150, "the bank has {} questions; at least 150 are required", bank.len());
        let keys: HashSet<&str> = bank.iter().map(|q| q.key).collect();
        assert_eq!(keys.len(), bank.len(), "duplicate key");
        for question in &bank {
            assert!(
                !question.key.is_empty()
                    && question.key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "key {:?} is not lower_snake_case",
                question.key
            );
        }
    }

    #[test]
    fn every_question_has_both_languages_and_fits() {
        for question in built_in() {
            for (language, text) in [("ja", question.ja), ("en", question.en)] {
                assert!(!text.trim().is_empty(), "{}: {language} text is empty", question.key);
                assert_eq!(text, text.trim(), "{}: {language} text has stray whitespace", question.key);
                assert!(
                    text.chars().count() <= text::POLL_QUESTION_MAX,
                    "{}: {language} text is too long",
                    question.key
                );
                assert!(text.ends_with('?'), "{}: {language} text is not a question", question.key);
            }
            assert!(question.ja.ends_with("は?"), "{}: a Japanese poll question asks 「…人は?」", question.key);
            assert!(question.en.starts_with("Who"), "{}: an English poll question asks who", question.key);
            assert!(
                text::emoji(question.emoji, "X").is_ok_and(|emoji| !emoji.is_empty()),
                "{}: emoji {:?}",
                question.key,
                question.emoji
            );
        }
    }

    #[test]
    fn no_question_ranks_looks_pushes_romance_or_is_unkind() {
        let never = WordFilter::new(NEVER);
        for question in built_in() {
            for text in [question.ja, question.en] {
                assert!(
                    !never.matches(&prepare(text)),
                    "{}: {text:?} contains a phrase a compliment poll must not use",
                    question.key
                );
            }
        }
    }

    #[test]
    fn the_lint_itself_catches_what_it_is_for() {
        let never = WordFilter::new(NEVER);
        // Poll questions ask for "the most …", so superlatives matter most.
        let bad_questions = [
            "一番かわいい人は?",
            "一番カワイイ人は?",
            "付き合いたい人は?",
            "一番うるさい人は?",
            "声がうるさくて目立つ人は?",
            "Who is the hottest?",
            "Who is the prettiest in class?",
            "Who is the most beautiful?",
            "Who would you date?",
            "Who is the most annoying?",
            "Who is the loudest?",
        ];
        for bad in bad_questions {
            assert!(never.matches(&prepare(bad)), "{bad:?} should be rejected");
        }
    }
}
