//! What the server reads out of a caption: its length, its hashtags and its
//! `@mentions`. Pure functions, no I/O.
//!
//! Hashtags and mentions are only recognised outside of links, so the
//! fragment of `https://example.com/#top` is not a hashtag and the user part
//! of `https://example.com/@someone` is not a mention.
use unicode_normalization::UnicodeNormalization as _;
use unicode_segmentation::UnicodeSegmentation as _;

/// Longest caption, in characters.
pub const MAX_CAPTION_CHARS: usize = 500;
/// Longest hashtag, in characters.
pub const MAX_HASHTAG_CHARS: usize = 50;
/// Hashtags and mentions beyond these stay plain text: a caption is not a
/// way to sit on every hashtag page or to ping a crowd.
pub const MAX_HASHTAGS: usize = 30;
pub const MAX_MENTIONS: usize = 20;
/// Longest sound title and artist name, in characters.
pub const MAX_SOUND_CHARS: usize = 100;

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

/// Text as stored: line endings unified, control characters dropped, no
/// leading or trailing blank space.
pub fn clean(input: &str) -> String {
    let unified = input.replace("\r\n", "\n").replace('\r', "\n");
    let kept: String = unified.chars().filter(|c| *c == '\n' || !c.is_control()).collect();
    kept.trim().to_owned()
}

/// One line of text as stored (titles, names): like [`clean`], with every
/// run of blank space folded into one space.
pub fn clean_line(input: &str) -> String {
    clean(input).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A single line of at most `max` characters for notification bodies and
/// search titles; longer text ends in an ellipsis.
pub fn excerpt(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut graphemes = line.graphemes(true);
    let head: String = graphemes.by_ref().take(max).collect();
    if graphemes.next().is_some() { format!("{head}…") } else { head }
}

/// The parts of a caption the server acts on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    /// Normalized hashtags (see [`normalize_hashtag`]), in order of first appearance.
    pub hashtags: Vec<String>,
    /// Mentioned handles, lowercased, in order of first appearance. Whether
    /// they name anyone is for the caller to find out.
    pub handles: Vec<String>,
}

/// Reads the hashtags and mentions out of a caption.
pub fn parse(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let previous = i.checked_sub(1).map(|p| chars[p]);
        if let Some(end) = link_end(&chars, i, previous) {
            i = end;
            continue;
        }
        let c = chars[i];
        // A `#` glued to the end of a word (`C#`, `色#fff`) belongs to that word.
        let starts_hashtag = previous.is_none_or(|p| !is_word(p) && p != '/' && p != '&');
        // An `@` glued to Latin text is an address (`mail@example.com`).
        // Japanese is written without spaces, so `友達の@taro` does mention someone.
        let starts_mention =
            previous.is_none_or(|p| !p.is_ascii_alphanumeric() && !matches!(p, '_' | '/' | '@' | '＠'));
        if matches!(c, '#' | '＃') && starts_hashtag {
            let end = run_end(&chars, i + 1, is_word);
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
        if matches!(c, '@' | '＠') && starts_mention {
            let end = run_end(&chars, i + 1, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            let handle = chars[i + 1..end].iter().collect::<String>().to_ascii_lowercase();
            // `@name@example.com` is an address on another service.
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

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The shape of a Tokuly handle (identity owns the full rules; this only
/// rules out what cannot be one).
fn is_handle(handle: &str) -> bool {
    (3..=24).contains(&handle.len()) && handle.as_bytes()[0].is_ascii_alphanumeric()
}

fn run_end(chars: &[char], from: usize, accept: impl Fn(char) -> bool) -> usize {
    let mut end = from;
    while end < chars.len() && accept(chars[end]) {
        end += 1;
    }
    end
}

/// If an http(s) link starts at `start`, the index just past it. A link
/// runs to the first character that cannot be part of one; Japanese text
/// usually follows a link without a space, so any non-ASCII character ends it.
fn link_end(chars: &[char], start: usize, previous: Option<char>) -> Option<usize> {
    if previous.is_some_and(|p| p.is_ascii_alphanumeric()) {
        return None;
    }
    let scheme_len = ["https://", "http://"].into_iter().find_map(|scheme| {
        let matches = chars.len() - start >= scheme.len()
            && chars[start..].iter().zip(scheme.chars()).all(|(a, b)| a.eq_ignore_ascii_case(&b));
        matches.then_some(scheme.len())
    })?;
    let end = run_end(chars, start + scheme_len, |c| {
        c.is_ascii_graphic() && !matches!(c, '<' | '>' | '"' | '`' | '{' | '}' | '|' | '\\' | '^')
    });
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
        && tag.chars().all(is_word)
        && !tag.chars().all(|c| c.is_numeric() || c == '_');
    valid.then_some(tag)
}

/// The key under which shorts naming the same external track share one
/// sound: title and artist with compatibility forms folded, lowercased and
/// stripped of blank space differences.
pub fn track_key(title: &str, artist: &str) -> String {
    let fold =
        |text: &str| text.nfkc().collect::<String>().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{}\n{}", fold(title), fold(artist))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_counts_what_people_see() {
        assert_eq!(char_count("踊ってみた!"), 6);
        assert_eq!(char_count("👨‍👩‍👧‍👦🇯🇵"), 2);
        assert!(fits(&"あ".repeat(500), MAX_CAPTION_CHARS));
        assert!(!fits(&"あ".repeat(501), MAX_CAPTION_CHARS));
        // 500 "characters" that are each a pile of combining marks.
        let pile = format!("a{}", "\u{0301}".repeat(60));
        assert!(!fits(&pile.repeat(500), MAX_CAPTION_CHARS));
    }

    #[test]
    fn cleaning_keeps_line_breaks_and_drops_control_characters() {
        assert_eq!(clean("  1行目\r\n2行目\u{7}\r3行目  "), "1行目\n2行目\n3行目");
        assert_eq!(clean_line("  夜に\n 駆ける\t "), "夜に 駆ける");
        assert_eq!(excerpt("とても  長い\nキャプション", 5), "とても 長…");
        assert_eq!(excerpt("短い", 5), "短い");
    }

    #[test]
    fn hashtags_are_normalized_and_deduplicated() {
        let parsed = parse("#ダンス #Dance ＃ＤＡＮＣＥ #踊ってみた! C#は別 #123 #a_b");
        assert_eq!(parsed.hashtags, ["ダンス", "dance", "踊ってみた", "a_b"]);
        assert_eq!(normalize_hashtag("#ﾀﾞﾝｽ"), Some("ダンス".to_owned()));
        assert_eq!(normalize_hashtag("2026"), None);
        assert_eq!(normalize_hashtag("a b"), None);
        assert_eq!(normalize_hashtag(&"あ".repeat(51)), None);
    }

    #[test]
    fn mentions_need_a_plausible_handle_and_no_address_around_them() {
        let parsed = parse("@Taro と友達の@hana_chan に感謝 mail@example.com @ab @someone@example.com");
        assert_eq!(parsed.handles, ["taro", "hana_chan"]);
        assert_eq!(parse("＠taro @taro").handles, ["taro"]);
    }

    #[test]
    fn links_hide_what_looks_like_tags_and_mentions() {
        let parsed = parse("見てね https://example.com/@someone/#top と #本物");
        assert_eq!(parsed.hashtags, ["本物"]);
        assert!(parsed.handles.is_empty());
        // A link ends where Japanese text begins.
        assert_eq!(parse("https://example.com/aを見て #tag").hashtags, ["tag"]);
        assert_eq!(parse("http://").hashtags, Vec::<String>::new());
    }

    #[test]
    fn hashtags_and_mentions_are_capped() {
        let many: String = (0..40).map(|i| format!("#tag{i}x ")).collect();
        assert_eq!(parse(&many).hashtags.len(), MAX_HASHTAGS);
        let many: String = (0..30).map(|i| format!("@user{i:02} ")).collect();
        assert_eq!(parse(&many).handles.len(), MAX_MENTIONS);
    }

    #[test]
    fn the_same_track_written_differently_has_one_key() {
        assert_eq!(track_key("夜に駆ける", "YOASOBI"), track_key(" 夜に駆ける ", "ｙｏａｓｏｂｉ"));
        assert_ne!(track_key("夜に駆ける", "YOASOBI"), track_key("夜に駆ける", ""));
        assert_ne!(track_key("a b", "c"), track_key("a", "b c"));
    }
}
