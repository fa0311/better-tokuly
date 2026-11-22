//! Validation and shortening of user-written text.
use tokuly_core::prelude::*;

pub(crate) const QUESTION_MAX: usize = 300;
pub(crate) const ANSWER_MAX: usize = 1000;
pub(crate) const RESPONSE_MAX: usize = 300;
pub(crate) const PROMPT_MAX: usize = 100;
pub(crate) const BOX_PROMPT_MAX: usize = 80;
pub(crate) const POLL_QUESTION_MAX: usize = 80;
pub(crate) const MUTED_WORD_MAX: usize = 30;
pub(crate) const MUTED_WORDS_MAX: usize = 100;
/// Characters of a text kept as a reminder or a notification line.
pub(crate) const PREVIEW_CHARS: usize = 80;

/// Trims a text and drops control characters other than line breaks. Line
/// endings become `\n`.
fn tidy(input: &str) -> String {
    input
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .collect::<String>()
        .trim()
        .to_owned()
}

/// A required text of at most `max` characters, or `reason`.
pub(crate) fn required(input: &str, max: usize, reason: &'static str, what: &str) -> AppResult<String> {
    let text = tidy(input);
    ensure!(!text.is_empty(), invalid, reason, "{what} cannot be empty");
    ensure!(text.chars().count() <= max, invalid, reason, "{what} can have at most {max} characters");
    Ok(text)
}

/// An optional text of at most `max` characters, or `reason`.
pub(crate) fn optional(input: &str, max: usize, reason: &'static str, what: &str) -> AppResult<String> {
    let text = tidy(input);
    ensure!(text.chars().count() <= max, invalid, reason, "{what} can have at most {max} characters");
    Ok(text)
}

/// A single-line text: line breaks become spaces.
pub(crate) fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A single emoji standing in for an icon: short, no letters or digits.
pub(crate) fn emoji(input: &str, reason: &'static str) -> AppResult<String> {
    let emoji = tidy(input);
    ensure!(
        emoji.chars().count() <= 8 && !emoji.chars().any(|c| c.is_alphanumeric() || c.is_whitespace()),
        invalid,
        reason,
        "an emoji is expected"
    );
    Ok(emoji)
}

/// The first `max` characters of a text on one line, with an ellipsis when
/// something was cut.
pub(crate) fn excerpt(text: &str, max: usize) -> String {
    let line = single_line(text);
    if line.chars().count() <= max {
        return line;
    }
    let mut cut: String = line.chars().take(max).collect();
    cut.push('…');
    cut
}

/// The owner's list of muted words: trimmed, without duplicates.
pub(crate) fn muted_words(words: &[String]) -> AppResult<Vec<String>> {
    ensure!(words.len() <= MUTED_WORDS_MAX, invalid, "MUTED_WORDS_INVALID", "at most {MUTED_WORDS_MAX} muted words");
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    for word in words {
        let word = single_line(&tidy(word));
        ensure!(
            !word.is_empty() && word.chars().count() <= MUTED_WORD_MAX,
            invalid,
            "MUTED_WORDS_INVALID",
            "a muted word has 1 to {MUTED_WORD_MAX} characters"
        );
        if !out.iter().any(|known| known.eq_ignore_ascii_case(&word)) {
            out.push(word);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_text_is_trimmed_and_bounded() {
        assert_eq!(required("  やっほー\r\n元気? \u{7}", 10, "X", "text").unwrap(), "やっほー\n元気?");
        assert_eq!(required("   ", 10, "TEXT_INVALID", "text").unwrap_err().reason, "TEXT_INVALID");
        assert!(required(&"あ".repeat(300), QUESTION_MAX, "X", "text").is_ok());
        assert!(required(&"あ".repeat(301), QUESTION_MAX, "X", "text").is_err());
        assert_eq!(optional("  ", 5, "X", "text").unwrap(), "");
    }

    #[test]
    fn excerpts_are_one_line_and_marked_when_cut() {
        assert_eq!(excerpt("短い\n質問", 10), "短い 質問");
        assert_eq!(excerpt("あいうえおかきくけこ", 5), "あいうえお…");
    }

    #[test]
    fn emoji_refuses_words() {
        assert_eq!(emoji(" 🔥 ", "X").unwrap(), "🔥");
        assert_eq!(emoji("", "X").unwrap(), "");
        assert!(emoji("fire", "X").is_err());
        assert!(emoji("🔥 🔥", "X").is_err());
    }

    #[test]
    fn muted_words_are_cleaned_and_deduplicated() {
        let words = muted_words(&[" Ugly ".into(), "ugly".into(), "うざ  い".into()]).unwrap();
        assert_eq!(words, vec!["Ugly", "うざ い"]);
        assert_eq!(muted_words(&["".into()]).unwrap_err().reason, "MUTED_WORDS_INVALID");
        assert!(muted_words(&vec!["x".to_owned(); MUTED_WORDS_MAX + 1]).is_err());
        assert!(muted_words(&["あ".repeat(MUTED_WORD_MAX + 1)]).is_err());
    }
}
