//! Everything this domain decides by looking at text: comment bodies,
//! `@mentions`, the owner's word and link filters, emoji reactions,
//! collection names and excerpts for notifications. Pure functions.
use tokuly_core::prelude::*;
use unicode_normalization::UnicodeNormalization as _;
use unicode_segmentation::UnicodeSegmentation as _;

/// Longest comment, in characters.
pub const MAX_COMMENT_CHARS: usize = 1000;
/// Users one comment may mention.
pub const MAX_MENTIONS: usize = 10;
/// Longest report text, in characters.
pub const MAX_REPORT_CHARS: usize = 1000;
/// Blocked words one owner may keep, and the longest of them in characters.
pub const MAX_BLOCKED_WORDS: usize = 200;
pub const MAX_BLOCKED_WORD_CHARS: usize = 50;
/// Longest collection name, in characters.
pub const MAX_COLLECTION_NAME_CHARS: usize = 50;
/// Longest search text of the comment manager, in characters.
pub const MAX_QUERY_CHARS: usize = 100;

/// Text as stored: line endings unified, control characters dropped,
/// surrounding whitespace trimmed.
pub fn clean(input: &str) -> String {
    let unified = input.replace("\r\n", "\n").replace('\r', "\n");
    unified.chars().filter(|c| *c == '\n' || !c.is_control()).collect::<String>().trim().to_owned()
}

/// Cleans and checks a comment body. An empty body is fine when the comment
/// carries a stamp or an image.
pub fn comment_text(input: &str, has_attachment: bool) -> AppResult<String> {
    let text = clean(input);
    ensure!(!text.is_empty() || has_attachment, invalid, "COMMENT_EMPTY", "a comment needs text, a stamp or an image");
    ensure!(
        text.chars().count() <= MAX_COMMENT_CHARS,
        invalid,
        "COMMENT_TOO_LONG",
        "a comment has at most {MAX_COMMENT_CHARS} characters"
    );
    Ok(text)
}

/// Cleans and checks the free text of a report.
pub fn report_text(input: &str) -> AppResult<String> {
    let text = clean(input);
    ensure!(
        text.chars().count() <= MAX_REPORT_CHARS,
        invalid,
        "REPORT_TEXT_TOO_LONG",
        "a report has at most {MAX_REPORT_CHARS} characters"
    );
    Ok(text)
}

fn is_handle_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// The handles a text mentions with `@handle` (or the full-width `＠`), in
/// order of appearance, lowercase, without duplicates. Only text that can be
/// a handle counts (3–24 characters of `a-z`, `0-9`, `_`, `-`, starting with
/// a letter or digit), and an `@` in the middle of a word — an email address
/// — starts no mention.
pub fn mention_handles(text: &str) -> Vec<String> {
    let mut handles: Vec<String> = Vec::new();
    let mut previous: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let starts_mention = (c == '@' || c == '＠') && !previous.is_some_and(|p| is_handle_char(p) || p == '.');
        previous = Some(c);
        if !starts_mention {
            continue;
        }
        let mut handle = String::new();
        while let Some(next) = chars.peek().copied().filter(|n| is_handle_char(*n)) {
            handle.push(next.to_ascii_lowercase());
            previous = Some(next);
            chars.next();
        }
        let valid = (3..=24).contains(&handle.len()) && handle.as_bytes()[0].is_ascii_alphanumeric();
        if valid && !handles.contains(&handle) {
            handles.push(handle);
        }
    }
    handles
}

/// Text as compared by the filters: NFKC (full-width and half-width forms
/// become one), lowercase, without the invisible characters people use to
/// break words apart.
pub fn fold(text: &str) -> String {
    text.nfkc()
        .filter(|c| !matches!(c, '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{00AD}'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// The first of `words` that occurs in `text`, ignoring case and character width.
pub fn find_blocked_word<'a>(text: &str, words: &'a [String]) -> Option<&'a str> {
    let folded = fold(text);
    words.iter().map(String::as_str).find(|word| {
        let word = fold(word);
        !word.is_empty() && folded.contains(&word)
    })
}

/// Checks an owner's blocked words and returns them as stored: trimmed, and
/// without entries that only differ in case or width.
pub fn blocked_words(input: &[String]) -> AppResult<Vec<String>> {
    let mut words: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for raw in input {
        let word = clean(raw);
        let length = word.chars().count();
        ensure!(
            (1..=MAX_BLOCKED_WORD_CHARS).contains(&length),
            invalid,
            "BLOCKED_WORDS_INVALID",
            "a blocked word has 1 to {MAX_BLOCKED_WORD_CHARS} characters"
        );
        let key = fold(&word);
        if !key.is_empty() && !seen.contains(&key) {
            seen.push(key);
            words.push(word);
        }
    }
    ensure!(words.len() <= MAX_BLOCKED_WORDS, invalid, "BLOCKED_WORDS_INVALID", "at most {MAX_BLOCKED_WORDS} blocked words");
    Ok(words)
}

/// Top-level domains common enough that `name.tld` in running text is taken
/// for a link.
const LINK_TLDS: &[&str] = &[
    "com", "net", "org", "jp", "io", "co", "me", "tv", "gg", "ly", "app", "dev", "xyz", "info", "biz", "link", "shop",
    "site", "online", "club", "top", "cc", "be", "to", "fm",
];

fn looks_like_domain(token: &str) -> bool {
    let token = token.trim_matches(|c| c == '.' || c == '-');
    match token.rsplit_once('.') {
        Some((host, tld)) => {
            LINK_TLDS.contains(&tld)
                && host.split('.').all(|label| !label.is_empty())
                && host.chars().any(|c| c.is_ascii_alphabetic())
        }
        None => false,
    }
}

/// Whether a text contains a link: a URL, or a bare domain such as
/// `example.com` (also when typed in full-width characters).
pub fn contains_link(text: &str) -> bool {
    let folded = fold(text);
    folded.contains("http://")
        || folded.contains("https://")
        || folded.contains("www.")
        || folded.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-')).any(looks_like_domain)
}

fn is_pictograph(c: char) -> bool {
    matches!(c,
        '\u{00A9}' | '\u{00AE}' | '\u{203C}' | '\u{2049}' | '\u{2122}' | '\u{2139}' | '\u{24C2}' | '\u{3030}' | '\u{303D}'
        | '\u{3297}' | '\u{3299}'
        | '\u{2190}'..='\u{21FF}'
        | '\u{2300}'..='\u{23FF}'
        | '\u{25A0}'..='\u{27BF}'
        | '\u{2900}'..='\u{297F}'
        | '\u{2B00}'..='\u{2BFF}'
        | '\u{1F000}'..='\u{1FAFF}')
}

/// Checks that `input` is exactly one emoji (one grapheme cluster: a
/// pictograph with its modifiers, a flag, a keycap or a ZWJ sequence) and
/// returns it.
pub fn single_emoji(input: &str) -> AppResult<String> {
    let emoji = input.trim();
    let mut graphemes = emoji.graphemes(true);
    let single = graphemes.next().is_some() && graphemes.next().is_none();
    let keycap = emoji.ends_with('\u{20E3}') && emoji.starts_with(|c: char| c.is_ascii_digit() || c == '#' || c == '*');
    let pictographic = emoji.chars().any(is_pictograph) && !emoji.chars().any(|c| c.is_alphanumeric());
    ensure!(single && emoji.len() <= 64 && (keycap || pictographic), invalid, "EMOJI_INVALID", "a reaction is a single emoji");
    Ok(emoji.to_owned())
}

/// A one-line preview of a text: whitespace collapsed, cut to `max_chars`.
pub fn excerpt(text: &str, max_chars: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max_chars {
        return line;
    }
    let mut cut: String = line.chars().take(max_chars.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// Checks a collection name and returns it with the key it is compared by.
pub fn collection_name(input: &str) -> AppResult<(String, String)> {
    let name = clean(input).replace('\n', " ");
    let length = name.chars().count();
    ensure!(
        (1..=MAX_COLLECTION_NAME_CHARS).contains(&length),
        invalid,
        "COLLECTION_NAME_INVALID",
        "a collection name has 1 to {MAX_COLLECTION_NAME_CHARS} characters"
    );
    let key = fold(&name);
    Ok((name, key))
}

/// A search text as an `ILIKE` pattern matching it anywhere, with the
/// pattern characters in it taken literally. `None` when there is nothing
/// to search for.
pub fn contains_pattern(query: &str) -> Option<String> {
    let query: String = clean(query).chars().take(MAX_QUERY_CHARS).collect();
    if query.is_empty() {
        return None;
    }
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for c in query.chars() {
        if matches!(c, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    Some(pattern)
}

/// A stable 64-bit fingerprint (FNV-1a) of what makes two comments "the
/// same" for the duplicate check: the folded text.
pub fn fingerprint(text: &str) -> u64 {
    fold(text).bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason<T: std::fmt::Debug>(result: AppResult<T>) -> String {
        result.unwrap_err().reason.into_owned()
    }

    #[test]
    fn comment_text_is_cleaned_and_bounded() {
        assert_eq!(comment_text("  こんにちは\r\nまたね\u{0007}  ", false).unwrap(), "こんにちは\nまたね");
        assert_eq!(reason(comment_text("   ", false)), "COMMENT_EMPTY");
        assert_eq!(comment_text("   ", true).unwrap(), "");
        assert_eq!(comment_text(&"あ".repeat(1000), false).unwrap().chars().count(), 1000);
        assert_eq!(reason(comment_text(&"あ".repeat(1001), false)), "COMMENT_TOO_LONG");
    }

    #[test]
    fn report_text_is_bounded() {
        assert_eq!(report_text(" 迷惑です ").unwrap(), "迷惑です");
        assert_eq!(report_text("").unwrap(), "");
        assert_eq!(reason(report_text(&"x".repeat(1001))), "REPORT_TEXT_TOO_LONG");
    }

    #[test]
    fn mentions_are_handles_after_an_at_sign() {
        assert_eq!(mention_handles("@Alice こんにちは @bob_2!"), ["alice", "bob_2"]);
        assert_eq!(mention_handles("やあ＠carol、元気?"), ["carol"]);
        assert_eq!(mention_handles("@alice @ALICE @alice."), ["alice"]);
        assert_eq!(mention_handles("(@dave-x)"), ["dave-x"]);
    }

    #[test]
    fn things_that_only_look_like_mentions_are_ignored() {
        assert!(mention_handles("mail me: someone@example.com").is_empty());
        assert!(mention_handles("@ab is too short, @ is nothing").is_empty());
        assert!(mention_handles(&format!("@{}", "a".repeat(25))).is_empty());
        assert!(mention_handles("@_underscore first").is_empty());
        assert!(mention_handles("no mentions here").is_empty());
    }

    #[test]
    fn folding_ignores_case_width_and_invisible_characters() {
        assert_eq!(fold("ＳＰＡＭ"), "spam");
        assert_eq!(fold("ｶﾀｶﾅ"), "カタカナ");
        assert_eq!(fold("s\u{200B}p\u{200D}am"), "spam");
    }

    #[test]
    fn blocked_words_match_inside_text() {
        let words = vec!["スパム".to_owned(), "Buy Now".to_owned()];
        assert_eq!(find_blocked_word("これはｽﾊﾟﾑです", &words), Some("スパム"));
        assert_eq!(find_blocked_word("BUY NOW!!", &words), Some("Buy Now"));
        assert_eq!(find_blocked_word("ｂｕｙ　ｎｏｗ", &words), Some("Buy Now"));
        assert_eq!(find_blocked_word("ふつうのコメント", &words), None);
        assert_eq!(find_blocked_word("anything", &[]), None);
    }

    #[test]
    fn blocked_word_lists_are_trimmed_deduplicated_and_bounded() {
        let stored = blocked_words(&[" spam ".into(), "SPAM".into(), "ｓｐａｍ".into(), "荒らし".into()]).unwrap();
        assert_eq!(stored, ["spam", "荒らし"]);
        assert_eq!(reason(blocked_words(&["ok".into(), "  ".into()])), "BLOCKED_WORDS_INVALID");
        assert_eq!(reason(blocked_words(&["x".repeat(51)])), "BLOCKED_WORDS_INVALID");
        let many: Vec<String> = (0..201).map(|i| format!("word{i}")).collect();
        assert_eq!(reason(blocked_words(&many)), "BLOCKED_WORDS_INVALID");
        assert_eq!(blocked_words(&many[..200]).unwrap().len(), 200);
    }

    #[test]
    fn links_are_urls_and_bare_domains() {
        for text in [
            "see https://example.com/x",
            "HTTP://EXAMPLE.COM",
            "www.example.jp だよ",
            "見てねexample.com",
            "ｅｘａｍｐｌｅ．ｃｏｍ",
            "join discord.gg/abc",
            "sub.domain.io.",
        ] {
            assert!(contains_link(text), "{text}");
        }
        for text in ["version 1.2.3", "e.g. this", "おはよう。元気?", "3.com", "end of sentence.next", "a..com"] {
            assert!(!contains_link(text), "{text}");
        }
    }

    #[test]
    fn a_reaction_is_exactly_one_emoji() {
        for emoji in ["🎉", "❤️", "👍🏽", "👨‍👩‍👧", "🇯🇵", "1️⃣", "☺"] {
            assert_eq!(single_emoji(emoji).unwrap(), emoji, "{emoji}");
        }
        assert_eq!(single_emoji(" 🎉 ").unwrap(), "🎉");
        for not_emoji in ["", "a", "🎉🎉", "ok👍", "あ", "1", ":)", "👍 👍"] {
            assert_eq!(reason(single_emoji(not_emoji)), "EMOJI_INVALID", "{not_emoji:?}");
        }
    }

    #[test]
    fn excerpts_are_one_line_and_cut_with_an_ellipsis() {
        assert_eq!(excerpt("  一行目\n二行目  ", 20), "一行目 二行目");
        assert_eq!(excerpt("abcdefghij", 5), "abcd…");
        assert_eq!(excerpt("", 5), "");
    }

    #[test]
    fn collection_names_are_bounded_and_keyed_without_case() {
        assert_eq!(collection_name("  Ｒｅｃｉｐｅｓ ").unwrap(), ("Ｒｅｃｉｐｅｓ".to_owned(), "recipes".to_owned()));
        assert_eq!(reason(collection_name("   ")), "COLLECTION_NAME_INVALID");
        assert_eq!(reason(collection_name(&"あ".repeat(51))), "COLLECTION_NAME_INVALID");
    }

    #[test]
    fn search_patterns_take_pattern_characters_literally() {
        assert_eq!(contains_pattern(" 100%_off\\ ").unwrap(), "%100\\%\\_off\\\\%");
        assert_eq!(contains_pattern("   "), None);
        assert_eq!(contains_pattern(&"x".repeat(300)).unwrap().len(), MAX_QUERY_CHARS + 2);
    }

    #[test]
    fn fingerprints_follow_the_folded_text() {
        assert_eq!(fingerprint("Hello"), fingerprint("ｈｅｌｌｏ"));
        assert_ne!(fingerprint("Hello"), fingerprint("Hello!"));
    }
}
