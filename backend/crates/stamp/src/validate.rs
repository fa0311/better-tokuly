//! Validation and normalization of everything creators and users type.
//!
//! Lengths are counted in grapheme clusters, i.e. what a person perceives as
//! one character, so Japanese text and emoji are measured fairly.
use tokuly_core::prelude::*;
use unicode_normalization::UnicodeNormalization as _;
use unicode_segmentation::UnicodeSegmentation as _;

pub const MAX_STAMPS_PER_PACK: usize = 120;
/// Stamps accepted by one AddStamps call.
pub const MAX_STAMPS_PER_REQUEST: usize = 30;
/// Lowest price of a paid pack. Below this the platform fee rounds to nothing.
pub const MIN_PAID_PRICE: i32 = 10;
pub const MAX_PRICE: i32 = 10_000;
pub const MAX_TAGS: usize = 10;
pub const MAX_KEYWORDS: usize = 10;

const MAX_PACK_NAME: usize = 40;
const MAX_DESCRIPTION: usize = 500;
const MAX_STAMP_NAME: usize = 40;
const MAX_WORD: usize = 24;
const MAX_QUERY: usize = 64;
const MAX_REASON: usize = 500;

fn length(text: &str) -> usize {
    text.graphemes(true).count()
}

fn has_control_chars(text: &str) -> bool {
    text.chars().any(|c| c.is_control() && c != '\n')
}

/// Pack name: 1-40 characters on one line.
pub fn pack_name(input: &str) -> AppResult<String> {
    let name = input.trim();
    ensure!(
        (1..=MAX_PACK_NAME).contains(&length(name)) && !has_control_chars(name) && !name.contains('\n'),
        invalid,
        "PACK_NAME_INVALID",
        "a pack name has 1 to {MAX_PACK_NAME} characters"
    );
    Ok(name.to_owned())
}

/// Pack description: up to 500 characters, line breaks allowed.
pub fn description(input: &str) -> AppResult<String> {
    let text = input.trim();
    ensure!(
        length(text) <= MAX_DESCRIPTION && !has_control_chars(text),
        invalid,
        "PACK_DESCRIPTION_INVALID",
        "a pack description has at most {MAX_DESCRIPTION} characters"
    );
    Ok(text.to_owned())
}

/// Price in coins: free, or within the range allowed for paid packs.
pub fn price(value: i32) -> AppResult<i32> {
    ensure!(
        value == 0 || (MIN_PAID_PRICE..=MAX_PRICE).contains(&value),
        invalid,
        "PRICE_INVALID",
        "a pack is free or costs {MIN_PAID_PRICE} to {MAX_PRICE} coins"
    );
    Ok(value)
}

/// Normalizes one tag or keyword so that different spellings of the same word
/// meet: compatibility forms are folded (full-width letters, half-width
/// kana), case is dropped and a leading `#` removed. `None` when what is left
/// is not a single word of 1-24 characters.
pub fn normalize_word(input: &str) -> Option<String> {
    let folded: String = input.nfkc().collect::<String>().to_lowercase();
    let word = folded.trim().trim_start_matches('#').trim();
    let valid = (1..=MAX_WORD).contains(&length(word))
        && !word.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '#' | ',' | '、'));
    valid.then(|| word.to_owned())
}

fn words<S: AsRef<str>>(input: &[S], max: usize, reason: &'static str, what: &str) -> AppResult<Vec<String>> {
    let mut out: Vec<String> = Vec::with_capacity(input.len());
    for raw in input {
        let Some(word) = normalize_word(raw.as_ref()) else {
            bail!(invalid, reason, "{what} are single words of at most {MAX_WORD} characters");
        };
        if !out.contains(&word) {
            out.push(word);
        }
    }
    ensure!(out.len() <= max, invalid, reason, "at most {max} {what}");
    Ok(out)
}

/// Pack tags: up to 10 distinct normalized words, in the given order.
pub fn tags<S: AsRef<str>>(input: &[S]) -> AppResult<Vec<String>> {
    words(input, MAX_TAGS, "TAGS_INVALID", "tags")
}

/// Stamp keywords: up to 10 distinct normalized words, in the given order.
pub fn keywords<S: AsRef<str>>(input: &[S]) -> AppResult<Vec<String>> {
    words(input, MAX_KEYWORDS, "KEYWORDS_INVALID", "keywords")
}

/// Stamp name (alt text): up to 40 characters on one line; may be empty.
pub fn stamp_name(input: &str) -> AppResult<String> {
    let name = input.trim();
    ensure!(
        length(name) <= MAX_STAMP_NAME && !has_control_chars(name) && !name.contains('\n'),
        invalid,
        "STAMP_NAME_INVALID",
        "a stamp name has at most {MAX_STAMP_NAME} characters"
    );
    Ok(name.to_owned())
}

/// The emoji a stamp stands for: empty, or exactly one emoji.
pub fn emoji(input: &str) -> AppResult<String> {
    let emoji = input.trim();
    ensure!(emoji.is_empty() || is_single_emoji(emoji), invalid, "STAMP_EMOJI_INVALID", "at most one emoji");
    Ok(emoji.to_owned())
}

/// One grapheme cluster that is a symbol rather than text. Letters, digits
/// and punctuation are not emoji, except as part of a keycap such as `1️⃣`.
fn is_single_emoji(text: &str) -> bool {
    const KEYCAP: char = '\u{20E3}';
    if length(text) != 1 {
        return false;
    }
    let Some(first) = text.chars().next() else { return false };
    text.contains(KEYCAP)
        || !(first.is_alphanumeric() || first.is_whitespace() || first.is_control() || first.is_ascii_punctuation())
}

/// The validated text of a stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StampText {
    pub name: String,
    pub emoji: String,
    pub keywords: Vec<String>,
}

pub fn stamp_text<S: AsRef<str>>(name: &str, emoji_input: &str, keyword_input: &[S]) -> AppResult<StampText> {
    Ok(StampText { name: stamp_name(name)?, emoji: emoji(emoji_input)?, keywords: keywords(keyword_input)? })
}

/// A search query: `None` when empty, an error when longer than 64 characters.
pub fn search_query(input: &str) -> AppResult<Option<String>> {
    let query = input.trim();
    ensure!(length(query) <= MAX_QUERY, invalid, "QUERY_TOO_LONG", "a search query has at most {MAX_QUERY} characters");
    Ok((!query.is_empty()).then(|| query.to_owned()))
}

/// The reason a moderator gives a creator: 1-500 characters.
pub fn reason(input: &str) -> AppResult<String> {
    let text = input.trim();
    ensure!(
        (1..=MAX_REASON).contains(&length(text)),
        invalid,
        "REASON_REQUIRED",
        "a reason of 1 to {MAX_REASON} characters is required"
    );
    Ok(text.to_owned())
}

/// Escapes `%`, `_` and `\` so user text can be embedded in a `LIKE` pattern.
pub fn like_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_names_are_trimmed_and_bounded() {
        assert_eq!(pack_name("  ねこスタンプ ").unwrap(), "ねこスタンプ");
        assert_eq!(pack_name("").unwrap_err().reason, "PACK_NAME_INVALID");
        assert_eq!(pack_name("   ").unwrap_err().reason, "PACK_NAME_INVALID");
        assert_eq!(pack_name("two\nlines").unwrap_err().reason, "PACK_NAME_INVALID");
        // 40 Japanese characters fit; 41 do not, although they are 123 bytes.
        assert!(pack_name(&"あ".repeat(40)).is_ok());
        assert!(pack_name(&"あ".repeat(41)).is_err());
    }

    #[test]
    fn descriptions_allow_line_breaks_but_not_other_control_characters() {
        assert_eq!(description(" 一行目\n二行目 ").unwrap(), "一行目\n二行目");
        assert_eq!(description("bell\u{7}").unwrap_err().reason, "PACK_DESCRIPTION_INVALID");
        assert!(description(&"x".repeat(501)).is_err());
    }

    #[test]
    fn price_is_free_or_within_the_paid_range() {
        assert_eq!(price(0).unwrap(), 0);
        assert_eq!(price(MIN_PAID_PRICE).unwrap(), MIN_PAID_PRICE);
        assert_eq!(price(MAX_PRICE).unwrap(), MAX_PRICE);
        for bad in [-1, 1, MIN_PAID_PRICE - 1, MAX_PRICE + 1] {
            assert_eq!(price(bad).unwrap_err().reason, "PRICE_INVALID", "{bad}");
        }
    }

    #[test]
    fn words_are_folded_to_one_spelling() {
        assert_eq!(normalize_word("#Cute").as_deref(), Some("cute"));
        assert_eq!(normalize_word(" ＣＵＴＥ ").as_deref(), Some("cute"), "full-width letters");
        assert_eq!(normalize_word("ﾈｺ").as_deref(), Some("ネコ"), "half-width kana");
        assert_eq!(normalize_word("two words"), None);
        assert_eq!(normalize_word("#"), None);
        assert_eq!(normalize_word(&"a".repeat(25)), None);
    }

    #[test]
    fn tags_are_deduplicated_and_limited() {
        assert_eq!(tags(&["Cute", "#cute", "ねこ"]).unwrap(), vec!["cute", "ねこ"]);
        assert_eq!(tags(&["ok", "not ok"]).unwrap_err().reason, "TAGS_INVALID");
        let many: Vec<String> = (0..=MAX_TAGS).map(|i| format!("tag{i}")).collect();
        assert_eq!(tags(&many).unwrap_err().reason, "TAGS_INVALID");
        assert_eq!(keywords(&["a b"]).unwrap_err().reason, "KEYWORDS_INVALID");
        assert!(tags::<&str>(&[]).unwrap().is_empty());
    }

    #[test]
    fn emoji_accepts_one_symbol_only() {
        for ok in ["", "😂", "👍🏽", "👨‍👩‍👧", "❤️", "1️⃣", "🇯🇵"] {
            assert!(emoji(ok).is_ok(), "{ok:?} should be accepted");
        }
        for bad in ["😂😂", "a", "あ", "1", "ok", "!", "😂a"] {
            assert_eq!(emoji(bad).unwrap_err().reason, "STAMP_EMOJI_INVALID", "{bad:?}");
        }
    }

    #[test]
    fn stamp_text_validates_every_part() {
        let text = stamp_text(" やったー ", "🎉", &["Yay", "わーい"]).unwrap();
        assert_eq!(
            text,
            StampText {
                name: "やったー".into(), emoji: "🎉".into(), keywords: vec!["yay".into(), "わーい".into()]
            }
        );
        assert_eq!(stamp_text::<&str>(&"x".repeat(41), "", &[]).unwrap_err().reason, "STAMP_NAME_INVALID");
    }

    #[test]
    fn queries_and_reasons() {
        assert_eq!(search_query("  ").unwrap(), None);
        assert_eq!(search_query(" neko ").unwrap().as_deref(), Some("neko"));
        assert_eq!(search_query(&"q".repeat(65)).unwrap_err().reason, "QUERY_TOO_LONG");
        assert_eq!(reason(" ").unwrap_err().reason, "REASON_REQUIRED");
        assert_eq!(reason(" 権利侵害 ").unwrap(), "権利侵害");
        assert_eq!(like_escape("50%_off\\"), "50\\%\\_off\\\\");
    }
}
