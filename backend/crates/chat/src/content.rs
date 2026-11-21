//! What a message may contain, free of I/O: text normalisation and limits,
//! `@mentions`, the first link, emoji recognition and previews.
use tokuly_core::prelude::*;
use unicode_segmentation::UnicodeSegmentation as _;

/// Longest message text / media caption, in characters.
pub const MAX_TEXT_CHARS: usize = 4000;
/// Uploads one message may carry.
pub const MAX_ATTACHMENTS: usize = 10;
/// Longest group name, in characters.
pub const MAX_GROUP_NAME_CHARS: usize = 50;
/// Longest idempotency key.
pub const MAX_CLIENT_ID_LEN: usize = 64;
/// Members one message can mention by name.
pub const MAX_MENTIONS: usize = 50;
/// Longest search query, in characters.
pub const MAX_QUERY_CHARS: usize = 100;
/// Characters of text quoted above a reply.
pub const REPLY_SNIPPET_CHARS: usize = 120;
/// Characters of text shown in a notification.
pub const NOTIFICATION_SNIPPET_CHARS: usize = 100;

/// Cleans up text typed by a user: line endings become `\n`, control
/// characters other than newline and tab are dropped, surrounding whitespace
/// is trimmed. Fails with `TEXT_TOO_LONG` past [`MAX_TEXT_CHARS`].
pub fn normalize_text(input: &str) -> AppResult<String> {
    let text: String = input
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    let text = text.trim();
    ensure!(
        text.chars().count() <= MAX_TEXT_CHARS,
        invalid,
        "TEXT_TOO_LONG",
        "a message can be at most {MAX_TEXT_CHARS} characters"
    );
    Ok(text.to_owned())
}

/// Validates the idempotency key a client sends with a message.
pub fn validate_client_id(client_id: &str, max_len: usize) -> AppResult<()> {
    ensure!(
        !client_id.is_empty() && client_id.len() <= max_len && !client_id.chars().any(char::is_control),
        invalid,
        "CLIENT_ID_INVALID",
        "client_id must be 1-{max_len} characters"
    );
    Ok(())
}

pub fn validate_group_name(input: &str) -> AppResult<String> {
    let name = input.trim();
    let length = name.chars().count();
    ensure!(
        (1..=MAX_GROUP_NAME_CHARS).contains(&length) && !name.chars().any(char::is_control),
        invalid,
        "GROUP_NAME_INVALID",
        "a group name is 1-{MAX_GROUP_NAME_CHARS} characters"
    );
    Ok(name.to_owned())
}

/// The first `max_chars` characters of `text` on one line, with an ellipsis
/// when something was cut.
pub fn snippet(text: &str, max_chars: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max_chars {
        return flat;
    }
    let mut cut: String = flat.chars().take(max_chars).collect();
    cut.push('…');
    cut
}

/// `@handle` mentions found in a text.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Mentions {
    /// Lowercased handles, in order of first appearance, without duplicates.
    pub handles: Vec<String>,
    /// The text contains `@all`.
    pub everyone: bool,
}

fn is_handle_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Finds `@handle` mentions. A mention starts at the beginning of the text
/// or after a character that cannot be part of a handle or an address, so
/// `mail@example.com` mentions nobody.
pub fn parse_mentions(text: &str) -> Mentions {
    let mut found = Mentions::default();
    let mut previous: Option<char> = None;
    let mut chars = text.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let starts_mention = c == '@' && previous.is_none_or(|p| !(is_handle_char(p) || p == '@' || p == '.'));
        previous = Some(c);
        if !starts_mention {
            continue;
        }
        let rest = &text[index + 1..];
        let length = rest.find(|c: char| !is_handle_char(c)).unwrap_or(rest.len());
        let handle = rest[..length].trim_end_matches('-').to_ascii_lowercase();
        if handle == "all" {
            found.everyone = true;
        } else if (3..=24).contains(&handle.len())
            && !found.handles.contains(&handle)
            && found.handles.len() < MAX_MENTIONS
        {
            found.handles.push(handle);
        }
        // Continue after the handle.
        while chars.peek().is_some_and(|(next, _)| *next <= index + length) {
            previous = chars.next().map(|(_, c)| c);
        }
    }
    found
}

/// The first http(s) link in a text, normalised, or `None`.
///
/// A link ends at whitespace, at a character that is not ASCII (Japanese
/// text is written without spaces around links) or at an angle bracket or
/// quote; punctuation that merely ends the sentence is not part of it.
pub fn first_url(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(offset) = lower[from..].find("http") {
        let start = from + offset;
        let rest = &lower[start..];
        let scheme = if rest.starts_with("https://") {
            8
        } else if rest.starts_with("http://") {
            7
        } else {
            from = start + 4;
            continue;
        };
        from = start + scheme;
        if text[..start].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        let candidate = &text[start..];
        let end = candidate
            .find(|c: char| {
                !c.is_ascii() || c.is_ascii_whitespace() || c.is_ascii_control() || matches!(c, '<' | '>' | '"' | '`')
            })
            .unwrap_or(candidate.len());
        let mut url = &candidate[..end];
        loop {
            let trimmed = url.trim_end_matches(['.', ',', '!', '?', ':', ';', '\'', '*', '~']);
            // A closing bracket belongs to the link only when it closes one opened inside it.
            let trimmed = match trimmed.chars().next_back() {
                Some(')') if trimmed.matches(')').count() > trimmed.matches('(').count() => {
                    &trimmed[..trimmed.len() - 1]
                }
                Some(']') if trimmed.matches(']').count() > trimmed.matches('[').count() => {
                    &trimmed[..trimmed.len() - 1]
                }
                Some('}') if trimmed.matches('}').count() > trimmed.matches('{').count() => {
                    &trimmed[..trimmed.len() - 1]
                }
                _ => trimmed,
            };
            if trimmed.len() == url.len() {
                break;
            }
            url = trimmed;
        }
        if url.len() > scheme {
            if let Ok(normalized) = tokuly_media::link_preview::normalize_url(url) {
                return Some(normalized.to_string());
            }
        }
    }
    None
}

/// Whether `c` belongs to the blocks that hold nothing but emoji and
/// pictographs (including the regional indicators flags are made of).
fn is_pictograph(c: char) -> bool {
    matches!(c as u32, 0x1F000..=0x1FAFF)
}

/// Symbols and dingbats: emoji whether or not a variation selector follows.
fn is_symbol(c: char) -> bool {
    matches!(c as u32, 0x2600..=0x27BF | 0x2B00..=0x2BFF)
}

/// Characters that are text by default and emoji only with U+FE0F.
fn is_text_default_emoji(c: char) -> bool {
    matches!(
        c as u32,
        0x00A9 | 0x00AE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x2190..=0x21FF | 0x2300..=0x23FF | 0x24C2
            | 0x25A0..=0x25FF | 0x2900..=0x297F | 0x3030 | 0x303D | 0x3297 | 0x3299
    )
}

/// Emoji in the "text default" blocks that are nevertheless drawn as emoji
/// without a variation selector (⌚, ⏰, ◽…).
fn is_emoji_presentation(c: char) -> bool {
    matches!(c as u32, 0x231A | 0x231B | 0x23E9..=0x23EC | 0x23F0 | 0x23F3 | 0x25FD | 0x25FE)
}

/// Whether one grapheme cluster is an emoji: a pictograph (with any skin
/// tone, ZWJ sequence or flag), a symbol, or a keycap.
fn is_emoji_cluster(cluster: &str) -> bool {
    if cluster.len() > 64 {
        return false;
    }
    let Some(first) = cluster.chars().next() else { return false };
    let has_selector = cluster.contains('\u{FE0F}');
    if cluster.ends_with('\u{20E3}') {
        return first.is_ascii_digit() || first == '#' || first == '*';
    }
    is_pictograph(first)
        || is_symbol(first)
        || is_emoji_presentation(first)
        || (has_selector && is_text_default_emoji(first))
}

/// Whether `text` is exactly one emoji (what a reaction must be).
pub fn is_single_emoji(text: &str) -> bool {
    let mut clusters = text.graphemes(true);
    matches!((clusters.next(), clusters.next()), (Some(cluster), None) if is_emoji_cluster(cluster))
}

/// 1-3 when `text` consists of nothing but that many emoji (drawn large by
/// clients), 0 otherwise.
pub fn big_emoji_count(text: &str) -> i32 {
    let mut count = 0;
    for cluster in text.graphemes(true) {
        if !is_emoji_cluster(cluster) {
            return 0;
        }
        count += 1;
        if count > 3 {
            return 0;
        }
    }
    count
}

/// Validates a search query and turns it into an `ILIKE` pattern matching
/// it anywhere, with the pattern characters of the query itself escaped.
pub fn search_pattern(query: &str) -> AppResult<String> {
    let query = query.trim();
    let length = query.chars().count();
    ensure!(
        (1..=MAX_QUERY_CHARS).contains(&length),
        invalid,
        "QUERY_INVALID",
        "a search query is 1-{MAX_QUERY_CHARS} characters"
    );
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for c in query.chars() {
        if matches!(c, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    Ok(pattern)
}

/// Validates a BCP 47 language tag ("ja", "en-US", "zh-Hant") and returns it
/// in its conventional casing.
pub fn normalize_language_tag(tag: &str) -> AppResult<String> {
    let invalid =
        || AppError::invalid("LANGUAGE_INVALID", "the language must be a BCP 47 tag such as \"ja\" or \"en-US\"");
    let tag = tag.trim();
    if tag.is_empty() || tag.len() > 35 {
        return Err(invalid());
    }
    let mut parts = Vec::new();
    for (index, part) in tag.split('-').enumerate() {
        let alphanumeric = !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric());
        let valid = if index == 0 {
            (2..=3).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_alphabetic())
        } else {
            alphanumeric && (2..=8).contains(&part.len())
        };
        if !valid {
            return Err(invalid());
        }
        parts.push(match (index, part.len()) {
            (0, _) => part.to_ascii_lowercase(),
            // Region subtags are upper case, script subtags title case.
            (_, 2) => part.to_ascii_uppercase(),
            (_, 4) => {
                let lower = part.to_ascii_lowercase();
                format!("{}{}", lower[..1].to_ascii_uppercase(), &lower[1..])
            }
            _ => part.to_ascii_lowercase(),
        });
    }
    Ok(parts.join("-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_trimmed_and_limited() {
        assert_eq!(normalize_text("  こんにちは\r\nやっほー\u{0007}  ").unwrap(), "こんにちは\nやっほー");
        assert_eq!(normalize_text(" \n ").unwrap(), "");
        assert!(normalize_text(&"あ".repeat(MAX_TEXT_CHARS)).is_ok());
        assert_eq!(normalize_text(&"あ".repeat(MAX_TEXT_CHARS + 1)).unwrap_err().reason, "TEXT_TOO_LONG");
    }

    #[test]
    fn client_ids_and_group_names_are_validated() {
        assert!(validate_client_id("3f0c8a", 64).is_ok());
        assert_eq!(validate_client_id("", 64).unwrap_err().reason, "CLIENT_ID_INVALID");
        assert_eq!(validate_client_id(&"x".repeat(65), 64).unwrap_err().reason, "CLIENT_ID_INVALID");
        assert_eq!(validate_group_name("  放課後メンバー ").unwrap(), "放課後メンバー");
        assert_eq!(validate_group_name("   ").unwrap_err().reason, "GROUP_NAME_INVALID");
        assert_eq!(validate_group_name(&"あ".repeat(51)).unwrap_err().reason, "GROUP_NAME_INVALID");
        assert_eq!(validate_group_name("a\nb").unwrap_err().reason, "GROUP_NAME_INVALID");
    }

    #[test]
    fn snippets_are_single_line_and_cut_on_characters() {
        assert_eq!(snippet("短い", 10), "短い");
        assert_eq!(snippet("line one\n  line two", 100), "line one line two");
        assert_eq!(snippet("あいうえおかきくけこ", 5), "あいうえお…");
    }

    #[test]
    fn mentions_are_found_once_and_lowercased() {
        let found = parse_mentions("@Taro_01 おはよう！ @hanako-chan と@taro_01、それと @all");
        assert_eq!(found.handles, vec!["taro_01", "hanako-chan"]);
        assert!(found.everyone);
    }

    #[test]
    fn addresses_and_short_handles_are_not_mentions() {
        let found = parse_mentions("mail me: taro@example.com or @@weird, @ab is too short, @allstars is a handle");
        assert_eq!(found.handles, vec!["allstars"]);
        assert!(!found.everyone);
        assert_eq!(parse_mentions(""), Mentions::default());
        assert_eq!(parse_mentions("@taro-").handles, vec!["taro"]);
    }

    #[test]
    fn the_first_link_is_found_without_its_punctuation() {
        assert_eq!(first_url("見て https://example.com/a?b=1。すごい").as_deref(), Some("https://example.com/a?b=1"));
        assert_eq!(
            first_url("これhttps://youtu.be/dQw4w9WgXcQ見て！").as_deref(),
            Some("https://youtu.be/dQw4w9WgXcQ")
        );
        assert_eq!(first_url("(see http://Example.com/x).").as_deref(), Some("http://example.com/x"));
        assert_eq!(
            first_url("wiki: https://en.wikipedia.org/wiki/Rust_(programming_language)!").as_deref(),
            Some("https://en.wikipedia.org/wiki/Rust_(programming_language)")
        );
        assert_eq!(
            first_url("first https://a.example/1 then https://b.example/2").as_deref(),
            Some("https://a.example/1")
        );
    }

    #[test]
    fn text_without_a_usable_link_has_none() {
        for text in [
            "no link here",
            "xhttps://example.com",
            "https://",
            "http:// broken",
            "ftp://example.com/file",
            "httpsomething",
        ] {
            assert_eq!(first_url(text), None, "{text}");
        }
    }

    #[test]
    fn single_emoji_are_recognised() {
        for emoji in ["😀", "👍", "👍🏽", "❤️", "❤", "🇯🇵", "👨‍👩‍👧‍👦", "1️⃣", "✨", "⭐", "☀️", "🫶", "🏳️‍🌈", "⌚", "©️"]
        {
            assert!(is_single_emoji(emoji), "{emoji} should be an emoji");
        }
        for text in ["", "a", "あ", "1", "😀😀", "😀a", ":)", " ", "→", "©", "#"] {
            assert!(!is_single_emoji(text), "{text:?} should not be an emoji");
        }
    }

    #[test]
    fn big_emoji_counts_up_to_three() {
        assert_eq!(big_emoji_count("😀"), 1);
        assert_eq!(big_emoji_count("🎉🎉🇯🇵"), 3);
        assert_eq!(big_emoji_count("🎉🎉🎉🎉"), 0);
        assert_eq!(big_emoji_count("やった🎉"), 0);
        assert_eq!(big_emoji_count("🎉 🎉"), 0);
        assert_eq!(big_emoji_count(""), 0);
    }

    #[test]
    fn search_patterns_escape_like_wildcards() {
        assert_eq!(search_pattern(" 駅 ").unwrap(), "%駅%");
        assert_eq!(search_pattern("100%_off\\").unwrap(), "%100\\%\\_off\\\\%");
        assert_eq!(search_pattern("  ").unwrap_err().reason, "QUERY_INVALID");
        assert_eq!(search_pattern(&"a".repeat(101)).unwrap_err().reason, "QUERY_INVALID");
    }

    #[test]
    fn language_tags_are_validated_and_cased() {
        assert_eq!(normalize_language_tag("JA").unwrap(), "ja");
        assert_eq!(normalize_language_tag("en-us").unwrap(), "en-US");
        assert_eq!(normalize_language_tag("zh-hant-tw").unwrap(), "zh-Hant-TW");
        for bad in ["", "j", "japanese", "en_US", "en-", "-en", "en-!!", "e1"] {
            assert_eq!(normalize_language_tag(bad).unwrap_err().reason, "LANGUAGE_INVALID", "{bad}");
        }
    }
}
