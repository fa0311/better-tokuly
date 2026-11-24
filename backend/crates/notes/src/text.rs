//! What the server reads out of the text people write: length, hashtags,
//! `@mentions`, the first link, and whether a reaction is one emoji. Pure
//! functions, no I/O.
//!
//! Hashtags and mentions are only recognised outside of links, so the
//! fragment of `https://example.com/#top` is not a hashtag and the user part
//! of `https://example.com/@someone` is not a mention.
use unicode_normalization::UnicodeNormalization as _;
use unicode_segmentation::UnicodeSegmentation as _;

/// Longest hashtag, in characters.
pub const MAX_HASHTAG_CHARS: usize = 50;
/// Hashtags and mentions beyond these are left as plain text: a note is not
/// a way to ping a crowd or to sit in every timeline.
pub const MAX_HASHTAGS: usize = 10;
pub const MAX_MENTIONS: usize = 10;

/// Most bytes one user-perceived character may take on average. An emoji
/// family is 25 bytes; text stacked with hundreds of combining marks is not
/// a character anyone means to write.
const MAX_BYTES_PER_CHAR: usize = 40;

/// Length as people count it: user-perceived characters, so an emoji or a
/// kana with dakuten is one.
pub fn char_count(text: &str) -> usize {
    text.graphemes(true).count()
}

/// Whether `text` is at most `max_chars` characters long — and not, behind
/// that count, kilobytes of combining marks.
pub fn fits(text: &str, max_chars: usize) -> bool {
    text.len() <= max_chars * MAX_BYTES_PER_CHAR && char_count(text) <= max_chars
}

/// Text as stored: line endings unified, control characters dropped, and no
/// leading or trailing blank space.
pub fn clean(input: &str) -> String {
    let unified = input.replace("\r\n", "\n").replace('\r', "\n");
    let kept: String = unified.chars().filter(|c| *c == '\n' || !c.is_control()).collect();
    kept.trim().to_owned()
}

/// A single line of at most `max` characters for notification bodies,
/// search titles and reports; longer text ends in an ellipsis.
pub fn excerpt(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut graphemes = line.graphemes(true);
    let head: String = graphemes.by_ref().take(max).collect();
    if graphemes.next().is_some() { format!("{head}…") } else { head }
}

/// The parts of a note's text the server acts on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    /// Normalized hashtags (see [`normalize_hashtag`]), in order of first appearance.
    pub hashtags: Vec<String>,
    /// Mentioned handles, lowercased, in order of first appearance. Whether
    /// they name anyone is for the caller to find out.
    pub handles: Vec<String>,
    /// The first http(s) link.
    pub first_url: Option<String>,
}

/// Reads hashtags, mentions and the first link out of a note's text.
pub fn parse(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let previous = i.checked_sub(1).map(|p| chars[p]);
        if let Some(end) = url_end(&chars, i, previous) {
            if parsed.first_url.is_none() {
                parsed.first_url = Some(chars[i..end].iter().collect());
            }
            i = end;
            continue;
        }
        let c = chars[i];
        // A `#` glued to the end of a word (`a#b`, `C#`, `色#fff`) is part of that word.
        let starts_hashtag = previous.is_none_or(|p| !is_word(p) && p != '/' && p != '&');
        // An `@` glued to Latin text is an address (`mail@example.com`). Japanese
        // is written without spaces, so `友達の@taro` does mention someone.
        let starts_mention =
            previous.is_none_or(|p| !p.is_ascii_alphanumeric() && !matches!(p, '_' | '/' | '@' | '＠'));
        if (c == '#' || c == '＃') && starts_hashtag {
            let end = run_end(&chars, i + 1, is_hashtag_char);
            let tag: String = chars[i + 1..end].iter().collect();
            if let Some(tag) = normalize_hashtag(&tag)
                && !parsed.hashtags.contains(&tag)
                && parsed.hashtags.len() < MAX_HASHTAGS
            {
                parsed.hashtags.push(tag);
            }
            i = end.max(i + 1);
            continue;
        }
        if (c == '@' || c == '＠') && starts_mention {
            let end = run_end(&chars, i + 1, is_handle_char);
            let handle: String = chars[i + 1..end].iter().collect::<String>().to_ascii_lowercase();
            // `@name@example.com` is an address on another service, not one of our users.
            let foreign = chars.get(end).is_some_and(|next| *next == '@');
            if is_handle(&handle)
                && !foreign
                && !parsed.handles.contains(&handle)
                && parsed.handles.len() < MAX_MENTIONS
            {
                parsed.handles.push(handle);
            }
            i = end.max(i + 1);
            continue;
        }
        i += 1;
    }
    parsed
}

pub(crate) fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub(crate) fn is_hashtag_char(c: char) -> bool {
    is_word(c)
}

pub(crate) fn is_handle_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// The shape of a Tokuly handle (identity owns the full rules; this only
/// has to rule out what cannot be one).
pub(crate) fn is_handle(handle: &str) -> bool {
    (3..=24).contains(&handle.len()) && handle.as_bytes()[0].is_ascii_alphanumeric()
}

pub(crate) fn run_end(chars: &[char], from: usize, accept: fn(char) -> bool) -> usize {
    let mut end = from;
    while end < chars.len() && accept(chars[end]) {
        end += 1;
    }
    end
}

/// If a link starts at `start`, the index just past it.
///
/// A link runs to the first character that cannot be part of one. Japanese
/// text usually follows a link without a space, so any non-ASCII character
/// ends it. Sentence punctuation right after the link is not part of it.
pub(crate) fn url_end(chars: &[char], start: usize, previous: Option<char>) -> Option<usize> {
    if previous.is_some_and(|p| p.is_ascii_alphanumeric()) {
        return None;
    }
    let scheme_len = ["https://", "http://"].into_iter().find_map(|scheme| {
        let matches = chars[start..].iter().zip(scheme.chars()).filter(|(a, b)| a.eq_ignore_ascii_case(b)).count()
            == scheme.len();
        matches.then_some(scheme.len())
    })?;
    let mut end = run_end(chars, start + scheme_len, |c| {
        c.is_ascii_graphic() && !matches!(c, '<' | '>' | '"' | '`' | '{' | '}' | '|' | '\\' | '^')
    });
    while end > start + scheme_len {
        let last = chars[end - 1];
        let unbalanced_close = last == ')' && {
            let inside = &chars[start..end];
            inside.iter().filter(|c| **c == ')').count() > inside.iter().filter(|c| **c == '(').count()
        };
        if matches!(last, '.' | ',' | '!' | '?' | ':' | ';' | '\'' | ']') || unbalanced_close {
            end -= 1;
        } else {
            break;
        }
    }
    (end > start + scheme_len).then_some(end)
}

/// A hashtag as stored and compared: without `#`, compatibility forms
/// folded (full-width `ＡＢＣ` = `abc`, half-width kana = full-width kana)
/// and lowercased. `None` when it is not a usable hashtag: empty, too long,
/// made of digits only, or containing anything but letters, digits and `_`.
pub fn normalize_hashtag(input: &str) -> Option<String> {
    let input = input.trim();
    let input = input.strip_prefix('#').or_else(|| input.strip_prefix('＃')).unwrap_or(input);
    let tag: String = input.nfkc().collect::<String>().to_lowercase();
    let valid = !tag.is_empty()
        && char_count(&tag) <= MAX_HASHTAG_CHARS
        && tag.chars().all(is_hashtag_char)
        && !tag.chars().all(|c| c.is_numeric() || c == '_');
    valid.then_some(tag)
}

/// Whether `input` is exactly one emoji (one user-perceived character that
/// is pictographic: a face, a flag, a keycap, a family joined by ZWJ…).
pub fn is_single_emoji(input: &str) -> bool {
    let mut graphemes = input.graphemes(true);
    let (Some(grapheme), None) = (graphemes.next(), graphemes.next()) else { return false };
    grapheme.len() <= 64 && grapheme.chars().any(is_pictographic)
}

fn is_pictographic(c: char) -> bool {
    matches!(
        c as u32,
        0x00A9 | 0x00AE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x20E3 | 0x24C2 | 0x3030 | 0x303D | 0x3297 | 0x3299
            | 0x2190..=0x21FF
            | 0x2300..=0x23FF
            | 0x25A0..=0x25FF
            | 0x2600..=0x27BF
            | 0x2900..=0x297F
            | 0x2B00..=0x2BFF
            | 0x1F000..=0x1FAFF
    )
}

/// The language of a note when it can be told from its letters: `ja` when
/// it has kana, `en` when its letters are (almost) all Latin, else empty.
/// Kanji alone could be Chinese and are not enough. Used to split trends
/// by language, so a guess that is sometimes empty is better than a wrong one.
pub fn language(text: &str) -> &'static str {
    let (mut kana, mut latin, mut letters) = (0usize, 0usize, 0usize);
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        letters += 1;
        match c as u32 {
            0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9D => kana += 1,
            _ if c.is_ascii_alphabetic() => latin += 1,
            _ => {}
        }
    }
    if kana > 0 {
        "ja"
    } else if latin >= 3 && latin * 10 >= letters * 9 {
        "en"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_are_told_from_the_letters() {
        assert_eq!(language("今日はいい天気"), "ja");
        assert_eq!(language("ﾃｽﾄ"), "ja");
        assert_eq!(language("what a nice day #sunny"), "en");
        assert_eq!(language("今天天气很好"), "", "kanji alone could be Chinese");
        assert_eq!(language("ok"), "");
        assert_eq!(language("123 !!!"), "");
    }

    #[test]
    fn length_counts_what_people_see() {
        assert_eq!(char_count("今何してる?"), 6);
        assert_eq!(char_count("👨‍👩‍👧‍👦🇯🇵"), 2);
        assert_eq!(char_count("が"), 1); // か + combining dakuten
        assert_eq!(char_count(""), 0);
    }

    #[test]
    fn limits_count_characters_but_bound_bytes() {
        assert!(fits(&"あ".repeat(300), 300));
        assert!(!fits(&"あ".repeat(301), 300));
        assert!(fits(&"👨‍👩‍👧‍👦".repeat(300), 300));
        assert!(fits("", 0));
        // 300 "characters", each a letter under a pile of combining marks.
        let stacked = format!("a{}", "\u{301}".repeat(60)).repeat(300);
        assert_eq!(char_count(&stacked), 300);
        assert!(!fits(&stacked, 300));
    }

    #[test]
    fn text_is_cleaned_before_it_is_stored() {
        assert_eq!(clean("  hello\r\nworld\r\n\n"), "hello\nworld");
        assert_eq!(clean("a\u{0}b\u{7}c\td"), "abcd");
        assert_eq!(clean(" \n\t "), "");
    }

    #[test]
    fn excerpts_are_one_short_line() {
        assert_eq!(excerpt("こんにちは\n世界", 20), "こんにちは 世界");
        assert_eq!(excerpt("abcdefghij", 4), "abcd…");
        assert_eq!(excerpt("abcd", 4), "abcd");
        assert_eq!(excerpt("", 4), "");
    }

    #[test]
    fn hashtags_are_found_and_normalized() {
        let parsed = parse("今日は #Tokuly で遊んだ! ＃ランチ #tokuly #ＡＢＣ_1 #2024 #");
        assert_eq!(parsed.hashtags, vec!["tokuly", "ランチ", "abc_1"]);
        // Japanese text ends a tag only at punctuation or a space, like on other networks.
        assert_eq!(parse("#渋谷でランチ。").hashtags, vec!["渋谷でランチ"]);
        assert_eq!(parse("abc#def a&#38; x/#y").hashtags, Vec::<String>::new());
        assert_eq!(parse("(#hello)#world").hashtags, vec!["hello", "world"]);
    }

    #[test]
    fn hashtag_normalization_rejects_unusable_tags() {
        assert_eq!(normalize_hashtag("#Tokuly"), Some("tokuly".into()));
        assert_eq!(normalize_hashtag("ﾄｸﾘｰ"), Some("トクリー".into()));
        assert_eq!(normalize_hashtag("＃春"), Some("春".into()));
        assert_eq!(normalize_hashtag(""), None);
        assert_eq!(normalize_hashtag("#"), None);
        assert_eq!(normalize_hashtag("12345"), None);
        assert_eq!(normalize_hashtag("a b"), None);
        assert_eq!(normalize_hashtag("a-b"), None);
        assert_eq!(normalize_hashtag(&"a".repeat(MAX_HASHTAG_CHARS)), Some("a".repeat(MAX_HASHTAG_CHARS)));
        assert_eq!(normalize_hashtag(&"a".repeat(MAX_HASHTAG_CHARS + 1)), None);
    }

    #[test]
    fn at_most_ten_hashtags_and_mentions_count() {
        let tags: String = (0..15).map(|i| format!("#tag{i} ")).collect();
        assert_eq!(parse(&tags).hashtags.len(), MAX_HASHTAGS);
        let handles: String = (0..15).map(|i| format!("@user{i} ")).collect();
        assert_eq!(parse(&handles).handles.len(), MAX_MENTIONS);
    }

    #[test]
    fn mentions_are_found() {
        let parsed = parse("@Alice_1 と＠bob-2に会った! @alice_1 mail@example.com @ab @x@mastodon.social (@carol)");
        assert_eq!(parsed.handles, vec!["alice_1", "bob-2", "carol"]);
        assert_eq!(parse("@_nope @-nope").handles, Vec::<String>::new());
        // The handle ends where Japanese text begins.
        assert_eq!(parse("@daveさんへ").handles, vec!["dave"]);
    }

    #[test]
    fn the_first_link_is_found_and_shields_its_contents() {
        let parsed = parse("見て https://example.com/a?b=1#top。あと http://second.example @erin #tag");
        assert_eq!(parsed.first_url.as_deref(), Some("https://example.com/a?b=1#top"));
        assert_eq!(parsed.hashtags, vec!["tag"]);
        assert_eq!(parsed.handles, vec!["erin"]);
        assert_eq!(parse("https://example.com/@someone").handles, Vec::<String>::new());
        assert_eq!(
            parse("(https://en.wikipedia.org/wiki/Tokyo_(disambiguation)).").first_url.as_deref(),
            Some("https://en.wikipedia.org/wiki/Tokyo_(disambiguation)")
        );
        assert_eq!(parse("HTTPS://Example.com/x, next").first_url.as_deref(), Some("HTTPS://Example.com/x"));
        assert_eq!(parse("xhttps://example.com https:// http://").first_url, None);
        assert_eq!(parse("no link here").first_url, None);
    }

    #[test]
    fn reactions_are_one_emoji() {
        for emoji in ["❤️", "😂", "👍🏽", "🇯🇵", "👨‍👩‍👧‍👦", "1️⃣", "✨", "©️"]
        {
            assert!(is_single_emoji(emoji), "{emoji}");
        }
        for not_emoji in ["", "a", "1", "😂😂", "ab", " ", "あ", "❤️ ", "<3"] {
            assert!(!is_single_emoji(not_emoji), "{not_emoji:?}");
        }
    }
}
