//! What the server reads out of the text people write: its length, the
//! `@mentions` in it, and whether it contains a muted word. Pure functions,
//! no I/O.
use unicode_normalization::UnicodeNormalization as _;
use unicode_segmentation::UnicodeSegmentation as _;

/// Mentions beyond this many are left as plain text: a post is not a way
/// to ping a crowd.
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

/// A single line as stored: like [`clean`], with line breaks turned into spaces.
pub fn clean_line(input: &str) -> String {
    clean(input).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A single line of at most `max` characters for notification bodies,
/// quotes and search documents; longer text ends in an ellipsis.
pub fn excerpt(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut graphemes = line.graphemes(true);
    let head: String = graphemes.by_ref().take(max).collect();
    if graphemes.next().is_some() { format!("{head}…") } else { head }
}

/// The handles `@mentioned` in `text`, lowercased, in order of first
/// appearance. Whether they name a member is for the caller to find out.
///
/// An `@` glued to Latin text is an address (`mail@example.com`), and
/// `@name@example.com` is an account on another service; neither is a
/// mention. Japanese is written without spaces, so `友達の@taro` is one.
pub fn mentioned_handles(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut handles: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '@' && c != '＠' {
            i += 1;
            continue;
        }
        let previous = i.checked_sub(1).map(|p| chars[p]);
        let starts = previous.is_none_or(|p| !p.is_ascii_alphanumeric() && !matches!(p, '_' | '/' | '@' | '＠'));
        let mut end = i + 1;
        while end < chars.len() && is_handle_char(chars[end]) {
            end += 1;
        }
        let handle: String = chars[i + 1..end].iter().collect::<String>().to_ascii_lowercase();
        let foreign = chars.get(end).is_some_and(|next| *next == '@');
        if starts && !foreign && is_handle(&handle) && !handles.contains(&handle) && handles.len() < MAX_MENTIONS {
            handles.push(handle);
        }
        i = end.max(i + 1);
    }
    handles
}

fn is_handle_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// The shape of a Tokuly handle (identity owns the full rules; this only
/// has to rule out what cannot be one).
fn is_handle(handle: &str) -> bool {
    (3..=24).contains(&handle.len()) && handle.as_bytes()[0].is_ascii_alphanumeric()
}

/// Text as compared by the word filter and by tags: compatibility forms
/// folded (full-width `ＡＢＣ` = `abc`, half-width kana = full-width kana),
/// lowercased, and katakana read as hiragana, so `バカ`, `ﾊﾞｶ` and `ばか` are
/// the same word.
pub fn fold(input: &str) -> String {
    input
        .nfkc()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            // Katakana ァ..ヶ sit exactly 0x60 above hiragana ぁ..ゖ.
            'ァ'..='ヶ' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            other => other,
        })
        .collect()
}

/// [`fold`] with everything that is not a letter or a digit removed, so
/// `b.a-k a` cannot dodge a filter for `baka`.
fn squeeze(folded: &str) -> String {
    folded.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// The first of `muted` (already folded, see [`fold`]) that `text` contains.
///
/// A word made of ASCII letters and digits only matches whole words, so
/// muting `ass` does not hold back `class`; anything else (Japanese has no
/// word boundaries) matches anywhere, also when spaced or punctuated apart.
pub fn find_muted<'a>(text: &str, muted: &'a [String]) -> Option<&'a str> {
    if muted.is_empty() {
        return None;
    }
    let folded = fold(text);
    let squeezed = squeeze(&folded);
    muted
        .iter()
        .find(|word| {
            if word.is_empty() {
                return false;
            }
            if word.chars().all(|c| c.is_ascii_alphanumeric()) {
                folded.split(|c: char| !c.is_ascii_alphanumeric()).any(|token| token == word.as_str())
            } else {
                folded.contains(word.as_str()) || squeezed.contains(&squeeze(word))
            }
        })
        .map(String::as_str)
}

/// A tag as stored and compared: without `#`, folded like hashtags
/// elsewhere (NFKC, lowercase). `None` when it is not a usable tag: empty,
/// longer than `max_chars`, or containing anything but letters, digits and `_`.
pub fn normalize_tag(input: &str, max_chars: usize) -> Option<String> {
    let input = input.trim();
    let input = input.strip_prefix('#').or_else(|| input.strip_prefix('＃')).unwrap_or(input);
    let tag: String = input.nfkc().collect::<String>().to_lowercase();
    let valid =
        !tag.is_empty() && char_count(&tag) <= max_chars && tag.chars().all(|c| c.is_alphanumeric() || c == '_');
    valid.then_some(tag)
}

/// Escapes `%`, `_` and `\` for use inside a `LIKE` / `ILIKE` pattern.
pub fn like_pattern(query: &str) -> String {
    format!("%{}%", query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_counts_characters_as_people_do() {
        assert_eq!(char_count("とくぞう👨‍👩‍👧"), 5);
        assert!(fits("あ".repeat(100).as_str(), 100));
        assert!(!fits("あ".repeat(101).as_str(), 100));
        // One "character" made of thousands of combining marks is not one.
        let zalgo = format!("a{}", "\u{0301}".repeat(5000));
        assert_eq!(char_count(&zalgo), 1);
        assert!(!fits(&zalgo, 100));
    }

    #[test]
    fn cleaning_keeps_line_breaks_and_drops_control_characters() {
        assert_eq!(clean("  a\r\nb\rc\u{0007}  "), "a\nb\nc");
        assert_eq!(clean_line("  a\n b\t c  "), "a b c");
    }

    #[test]
    fn excerpts_are_one_line_with_an_ellipsis() {
        assert_eq!(excerpt("a\nb   c", 10), "a b c");
        assert_eq!(excerpt("あいうえお", 3), "あいう…");
    }

    #[test]
    fn mentions_are_found_and_addresses_are_not() {
        assert_eq!(mentioned_handles("@Taro と @hanako-1 へ"), vec!["taro", "hanako-1"]);
        assert_eq!(mentioned_handles("友達の@taro、＠jiro"), vec!["taro", "jiro"]);
        assert!(mentioned_handles("mail@example.com").is_empty());
        assert!(mentioned_handles("@name@example.com").is_empty());
        assert!(mentioned_handles("@ab @_abc").is_empty());
        // Each handle once, and no more than the cap.
        assert_eq!(mentioned_handles("@taro @TARO @taro"), vec!["taro"]);
        let crowd: String = (0..20).map(|i| format!("@user{i:02} ")).collect();
        assert_eq!(mentioned_handles(&crowd).len(), MAX_MENTIONS);
    }

    #[test]
    fn folding_unifies_width_case_and_kana() {
        assert_eq!(fold("ＡＢＣ"), "abc");
        assert_eq!(fold("ﾊﾞｶ"), fold("ばか"));
        assert_eq!(fold("バカ"), "ばか");
    }

    #[test]
    fn muted_words_match_whole_latin_words_and_any_japanese() {
        let muted = vec![fold("ass"), fold("バカ"), fold("go away")];
        assert_eq!(find_muted("you ASS!", &muted), Some("ass"));
        assert_eq!(find_muted("first class", &muted), None);
        assert_eq!(find_muted("おまえﾊﾞｶだろ", &muted), Some("ばか"));
        assert_eq!(find_muted("ば・か", &muted), Some("ばか"));
        assert_eq!(find_muted("please GO   AWAY now", &muted), Some("go away"));
        assert_eq!(find_muted("いい天気", &muted), None);
        assert_eq!(find_muted("anything", &[]), None);
    }

    #[test]
    fn tags_are_normalized_or_rejected() {
        assert_eq!(normalize_tag(" #ＡＰＥＸ ", 30).as_deref(), Some("apex"));
        assert_eq!(normalize_tag("推し活", 30).as_deref(), Some("推し活"));
        assert_eq!(normalize_tag("a b", 30), None);
        assert_eq!(normalize_tag("", 30), None);
        assert_eq!(normalize_tag(&"a".repeat(31), 30), None);
    }

    #[test]
    fn like_patterns_escape_wildcards() {
        assert_eq!(like_pattern("50%_off\\"), "%50\\%\\_off\\\\%");
    }
}
