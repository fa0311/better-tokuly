//! Text handling: query normalization, sanitizing what gets indexed, and
//! turning the engine's highlight markers into typed segments.
//!
//! Highlights never travel as markup. Meilisearch is asked to wrap matches in
//! two private-use characters ([`HIGHLIGHT_START`] / [`HIGHLIGHT_END`]), and
//! [`parse_highlight`] turns the result into `TextSegment`s of plain text.
//! Those two characters (and control characters) are removed from every
//! document before it is indexed ([`clean`]), so content cannot forge a
//! highlight, and since clients receive text rather than HTML, user-written
//! `<script>` stays the eight characters it is.
use tokuly_proto::search::TextSegment;
use unicode_normalization::UnicodeNormalization as _;

/// Marks the start of a match in the engine's formatted output.
pub const HIGHLIGHT_START: char = '\u{E000}';
/// Marks the end of a match.
pub const HIGHLIGHT_END: char = '\u{E001}';

/// Longest query accepted, in characters.
pub const MAX_QUERY_CHARS: usize = 100;

fn is_marker(c: char) -> bool {
    c == HIGHLIGHT_START || c == HIGHLIGHT_END
}

/// Single-line plain text: control characters and highlight markers removed,
/// whitespace runs collapsed, at most `max_chars` characters.
pub fn clean(input: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(input.len().min(max_chars * 4));
    let mut count = 0;
    let mut pending_space = false;
    for c in input.chars() {
        if is_marker(c) {
            continue;
        }
        if c.is_whitespace() || c.is_control() {
            pending_space = count > 0;
            continue;
        }
        if count + usize::from(pending_space) >= max_chars {
            break;
        }
        if pending_space {
            out.push(' ');
            count += 1;
            pending_space = false;
        }
        out.push(c);
        count += 1;
    }
    out
}

/// The query as it is searched and shown back: NFKC (so full-width letters
/// and half-width kana match their ordinary forms), cleaned, case kept.
pub fn normalize_query(input: &str) -> String {
    let folded: String = input.nfkc().collect();
    // One over the limit survives so the caller can tell "too long" apart.
    clean(&folded, MAX_QUERY_CHARS + 1)
}

/// The identity of a query for history, trending, completions and the
/// blocklist: its normalized form in lowercase.
pub fn query_key(normalized: &str) -> String {
    normalized.to_lowercase()
}

/// Normalizes a moderator-entered word (blocked term, synonym) the same way
/// queries are keyed.
pub fn normalize_term(input: &str) -> String {
    query_key(&normalize_query(input))
}

/// Exclusive upper bound of the byte-order range holding every string that
/// starts with `prefix` (the columns scanned this way use the "C" collation).
pub fn prefix_upper_bound(prefix: &str) -> String {
    let mut bound = String::with_capacity(prefix.len() + 4);
    bound.push_str(prefix);
    bound.push(char::MAX);
    bound
}

/// Splits the engine's formatted text into matching and non-matching runs.
/// Adjacent matches merge into one segment; an unbalanced marker (a crop that
/// cut a match in half) highlights what is there.
pub fn parse_highlight(formatted: &str) -> Vec<TextSegment> {
    let mut segments: Vec<TextSegment> = Vec::new();
    let mut current = String::new();
    let mut highlighted = false;
    let flush = |text: &mut String, highlighted: bool, segments: &mut Vec<TextSegment>| {
        if text.is_empty() {
            return;
        }
        match segments.last_mut() {
            Some(last) if last.highlighted == highlighted => last.text.push_str(text),
            _ => segments.push(TextSegment { text: text.clone(), highlighted, ..Default::default() }),
        }
        text.clear();
    };
    for c in formatted.chars() {
        if c == HIGHLIGHT_START {
            flush(&mut current, highlighted, &mut segments);
            highlighted = true;
        } else if c == HIGHLIGHT_END {
            flush(&mut current, highlighted, &mut segments);
            highlighted = false;
        } else {
            current.push(c);
        }
    }
    flush(&mut current, highlighted, &mut segments);
    segments
}

/// A text without matches as its single segment (nothing for an empty text).
pub fn plain_segments(text: &str) -> Vec<TextSegment> {
    if text.is_empty() {
        Vec::new()
    } else {
        vec![TextSegment { text: text.to_owned(), highlighted: false, ..Default::default() }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(segments: &[TextSegment]) -> Vec<(&str, bool)> {
        segments.iter().map(|s| (s.text.as_str(), s.highlighted)).collect()
    }

    #[test]
    fn clean_collapses_whitespace_and_drops_control_characters() {
        assert_eq!(clean("  渋谷で\n\n遊ぶ\t日  ", 100), "渋谷で 遊ぶ 日");
        assert_eq!(clean("a\u{0}b\u{7}c", 100), "a b c");
        assert_eq!(clean("\u{3000}全角　スペース\u{3000}", 100), "全角 スペース");
        assert_eq!(clean("", 100), "");
    }

    #[test]
    fn clean_truncates_by_characters_without_a_trailing_space() {
        assert_eq!(clean("あいうえお", 3), "あいう");
        assert_eq!(clean("ab cd", 3), "ab");
        assert_eq!(clean("ab cd", 4), "ab c");
        assert_eq!(clean("👨‍👩‍👧 family", 0), "");
    }

    #[test]
    fn clean_removes_highlight_markers_from_content() {
        let forged = format!("free {HIGHLIGHT_START}coins{HIGHLIGHT_END} here");
        assert_eq!(clean(&forged, 100), "free coins here");
    }

    #[test]
    fn queries_are_width_folded_but_keep_their_case() {
        assert_eq!(normalize_query("  ＴＯＫＹＯ　タワー "), "TOKYO タワー");
        assert_eq!(normalize_query("ｼﾌﾞﾔ"), "シブヤ");
        assert_eq!(normalize_query("＃渋谷"), "#渋谷");
        assert_eq!(query_key(&normalize_query("Tokyo  Tower")), "tokyo tower");
        assert_eq!(normalize_term(" ＳＰＡＭ "), "spam");
    }

    #[test]
    fn overlong_queries_stay_detectable() {
        let long = "あ".repeat(MAX_QUERY_CHARS + 50);
        assert_eq!(normalize_query(&long).chars().count(), MAX_QUERY_CHARS + 1);
        let exact = "あ".repeat(MAX_QUERY_CHARS);
        assert_eq!(normalize_query(&exact).chars().count(), MAX_QUERY_CHARS);
    }

    #[test]
    fn prefix_bounds_cover_exactly_the_prefix() {
        let upper = prefix_upper_bound("渋");
        for inside in ["渋", "渋谷", "渋谷 カフェ", "渋\u{10FFFE}"] {
            assert!(inside >= "渋" && inside < upper.as_str(), "{inside}");
        }
        for outside in ["港", "渓", "澁谷", ""] {
            assert!(!(outside >= "渋" && outside < upper.as_str()), "{outside}");
        }
    }

    #[test]
    fn highlight_markers_become_segments() {
        let formatted = format!("東京の{HIGHLIGHT_START}渋谷{HIGHLIGHT_END}を散歩");
        assert_eq!(runs(&parse_highlight(&formatted)), vec![("東京の", false), ("渋谷", true), ("を散歩", false)]);
    }

    #[test]
    fn adjacent_matches_merge_and_plain_text_is_one_segment() {
        let formatted = format!("たのしい{HIGHLIGHT_START}お{HIGHLIGHT_END}{HIGHLIGHT_START}さんぽ{HIGHLIGHT_END}");
        assert_eq!(runs(&parse_highlight(&formatted)), vec![("たのしい", false), ("おさんぽ", true)]);
        assert_eq!(runs(&parse_highlight("no match")), vec![("no match", false)]);
        assert!(parse_highlight("").is_empty());
    }

    #[test]
    fn unbalanced_markers_do_not_lose_text() {
        let cut = format!("…{HIGHLIGHT_START}渋谷");
        assert_eq!(runs(&parse_highlight(&cut)), vec![("…", false), ("渋谷", true)]);
        let stray = format!("渋谷{HIGHLIGHT_END}の夜");
        assert_eq!(runs(&parse_highlight(&stray)), vec![("渋谷の夜", false)]);
    }

    #[test]
    fn markup_in_content_stays_text() {
        let formatted = format!("<script>alert(1)</script> {HIGHLIGHT_START}<b>{HIGHLIGHT_END}");
        let segments = parse_highlight(&formatted);
        assert_eq!(runs(&segments), vec![("<script>alert(1)</script> ", false), ("<b>", true)]);
    }

    #[test]
    fn plain_segments_skip_empty_text() {
        assert!(plain_segments("").is_empty());
        assert_eq!(runs(&plain_segments("渋谷")), vec![("渋谷", false)]);
    }
}
