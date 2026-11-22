//! Checks on the text people type: names, notes, colours, emoji.
use tokuly_core::prelude::*;
use unicode_segmentation::UnicodeSegmentation as _;

pub(crate) const MAX_NAME: usize = 40;
pub(crate) const MAX_MEMO: usize = 500;

fn chars(text: &str) -> usize {
    text.graphemes(true).count()
}

/// A required one-line name: trimmed, 1 to `max` characters.
pub(crate) fn name(input: &str, max: usize, reason: &'static str, what: &str) -> AppResult<String> {
    let name = input.trim();
    ensure!(
        (1..=max).contains(&chars(name)) && !name.chars().any(char::is_control),
        invalid,
        reason,
        "{what} must be 1-{max} characters on one line"
    );
    Ok(name.to_owned())
}

/// An optional one-line field (teacher, room, label): trimmed, up to `max` characters.
pub(crate) fn line(input: &str, max: usize, what: &str) -> AppResult<String> {
    let line = input.trim();
    ensure!(
        chars(line) <= max && !line.chars().any(char::is_control),
        invalid,
        "TEXT_TOO_LONG",
        "{what} must be at most {max} characters on one line"
    );
    Ok(line.to_owned())
}

/// An optional free-form note; line breaks are fine.
pub(crate) fn memo(input: &str, what: &str) -> AppResult<String> {
    let memo = input.trim();
    ensure!(
        chars(memo) <= MAX_MEMO && !memo.chars().any(|c| c.is_control() && c != '\n'),
        invalid,
        "TEXT_TOO_LONG",
        "{what} must be at most {MAX_MEMO} characters"
    );
    Ok(memo.to_owned())
}

/// `#rrggbb` (stored lowercase), or empty for "none given".
pub(crate) fn color(input: &str) -> AppResult<String> {
    let color = input.trim().to_ascii_lowercase();
    let ok = color.is_empty()
        || (color.len() == 7 && color.starts_with('#') && color[1..].bytes().all(|b| b.is_ascii_hexdigit()));
    ensure!(ok, invalid, "COLOR_INVALID", "colours are written #rrggbb");
    Ok(color)
}

/// At most one emoji (one grapheme cluster), or empty.
pub(crate) fn emoji(input: &str) -> AppResult<String> {
    let emoji = input.trim();
    ensure!(
        chars(emoji) <= 1 && emoji.len() <= 64 && !emoji.chars().any(|c| c.is_control() || c.is_ascii_alphanumeric()),
        invalid,
        "TEXT_TOO_LONG",
        "emoji must be a single emoji"
    );
    Ok(emoji.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_counted_in_characters() {
        assert_eq!(name("  数学Ⅱ  ", 40, "SUBJECT_NAME_INVALID", "name").unwrap(), "数学Ⅱ");
        assert!(name(&"あ".repeat(40), 40, "X", "name").is_ok());
        for bad in ["", "   ", &"あ".repeat(41), "two\nlines"] {
            assert_eq!(name(bad, 40, "SUBJECT_NAME_INVALID", "name").unwrap_err().reason, "SUBJECT_NAME_INVALID");
        }
        // A family emoji is one character.
        assert!(name("👨‍👩‍👧", 1, "X", "name").is_ok());
    }

    #[test]
    fn optional_fields_may_be_empty_but_not_long() {
        assert_eq!(line("  2-A ", 40, "room").unwrap(), "2-A");
        assert_eq!(line("", 40, "room").unwrap(), "");
        assert_eq!(line(&"x".repeat(41), 40, "room").unwrap_err().reason, "TEXT_TOO_LONG");
        assert_eq!(memo("p.42\n練習問題", "memo").unwrap(), "p.42\n練習問題");
        assert_eq!(memo(&"x".repeat(501), "memo").unwrap_err().reason, "TEXT_TOO_LONG");
        assert_eq!(memo("bell\u{7}", "memo").unwrap_err().reason, "TEXT_TOO_LONG");
    }

    #[test]
    fn colours_and_emoji_are_checked() {
        assert_eq!(color(" #FF8800 ").unwrap(), "#ff8800");
        assert_eq!(color("").unwrap(), "");
        for bad in ["red", "#fff", "#gggggg", "ff8800f"] {
            assert_eq!(color(bad).unwrap_err().reason, "COLOR_INVALID", "{bad}");
        }
        assert_eq!(emoji("📐").unwrap(), "📐");
        assert_eq!(emoji("").unwrap(), "");
        assert!(emoji("📐📐").is_err());
        assert!(emoji("a").is_err());
    }
}
