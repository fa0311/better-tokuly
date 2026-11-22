//! Validation and normalization of everything people type into the emoji
//! domain: names, aliases, licence notes, category names, reasons, search
//! queries and deck items. Pure functions; the limits live here.
use tokuly_core::prelude::*;
use unicode_normalization::UnicodeNormalization as _;
use unicode_segmentation::UnicodeSegmentation as _;

pub const NAME_MIN_CHARS: usize = 2;
pub const NAME_MAX_CHARS: usize = 32;
pub const MAX_ALIASES: usize = 16;
pub const ALIAS_MAX_CHARS: usize = 32;
pub const LICENSE_MAX_CHARS: usize = 1024;
pub const CATEGORY_MAX_CHARS: usize = 32;
pub const MAX_CATEGORIES: i64 = 200;
pub const REASON_MAX_CHARS: usize = 500;
pub const QUERY_MAX_CHARS: usize = 64;
pub const DECK_MAX_ITEMS: usize = 100;
/// Emoji a user's "recently used" list remembers.
pub const RECENTS_KEPT: i64 = 64;
/// Emoji one `RecordUse` call may report.
pub const MAX_USES_PER_CALL: usize = 50;
/// Emoji one bulk change may touch.
pub const MAX_BULK_IDS: usize = 200;
/// The longer side of an emoji picture must have at least this many pixels.
pub const MIN_IMAGE_EDGE: i32 = 16;

/// Names only moderators may give: words that would pass for something
/// official, and words clients and markup treat specially.
const RESERVED_NAMES: &[&str] = &[
    "admin",
    "administrator",
    "all",
    "announcement",
    "bot",
    "channel",
    "custom",
    "default",
    "emoji",
    "everyone",
    "false",
    "here",
    "mod",
    "moderator",
    "none",
    "null",
    "official",
    "staff",
    "support",
    "system",
    "tokuly",
    "true",
    "undefined",
    "unknown",
    "verified",
];

/// The name inside `:name:`, or the input when it has no colons around it.
pub fn strip_colons(input: &str) -> &str {
    let trimmed = input.trim();
    trimmed.strip_prefix(':').and_then(|rest| rest.strip_suffix(':')).unwrap_or(trimmed)
}

fn is_name(candidate: &str) -> bool {
    (NAME_MIN_CHARS..=NAME_MAX_CHARS).contains(&candidate.len())
        && candidate.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && candidate.bytes().any(|b| b != b'_')
}

/// The stored form of a name someone typed: colons dropped, capitals
/// lowered. `None` when it is not a valid name.
pub fn normalize_name(input: &str) -> Option<String> {
    let name = strip_colons(input).to_ascii_lowercase();
    is_name(&name).then_some(name)
}

/// Validates an emoji name (`NAME_INVALID`).
pub fn name(input: &str) -> AppResult<String> {
    normalize_name(input).ok_or_else(|| {
        AppError::invalid(
            "NAME_INVALID",
            "an emoji name is 2-32 characters of a-z, 0-9 and _ with at least one letter or digit",
        )
    })
}

/// Whether users may not submit `name` (moderators may add it).
pub fn is_reserved(name: &str, extra: &[String]) -> bool {
    RESERVED_NAMES.contains(&name) || extra.iter().any(|reserved| reserved.trim().eq_ignore_ascii_case(name))
}

/// The form in which names, aliases and queries are compared: compatibility
/// characters unified (full-width letters, half-width kana), lowercase, and
/// katakana read as hiragana, so that 「ネコ」, 「ﾈｺ」 and 「ねこ」 find each other.
pub fn fold(text: &str) -> String {
    text.nfkc()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'ァ'..='ヶ' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

fn invalid_aliases() -> AppError {
    AppError::invalid("ALIASES_INVALID", "at most 16 aliases of 1-32 characters, without spaces or colons")
}

/// One alias in stored form: trimmed, compatibility characters unified,
/// lowercase. `None` when it cannot be an alias.
pub fn normalize_alias(input: &str) -> Option<String> {
    let alias: String = input.trim().nfkc().flat_map(char::to_lowercase).collect();
    let length = alias.chars().count();
    let clean = alias.chars().all(|c| !c.is_whitespace() && !c.is_control() && c != ':');
    ((1..=ALIAS_MAX_CHARS).contains(&length) && clean).then_some(alias)
}

/// Validates a list of aliases (`ALIASES_INVALID`). Duplicates are dropped,
/// the order is kept.
pub fn aliases<S: AsRef<str>>(input: &[S]) -> AppResult<Vec<String>> {
    let mut out: Vec<String> = Vec::with_capacity(input.len());
    for alias in input {
        let alias = normalize_alias(alias.as_ref()).ok_or_else(invalid_aliases)?;
        if !out.contains(&alias) {
            out.push(alias);
        }
    }
    if out.len() > MAX_ALIASES {
        return Err(invalid_aliases());
    }
    Ok(out)
}

/// The aliases of an imported entry: whatever is usable of them, cut to the
/// limit. A pack is never refused over a strange alias.
pub fn lenient_aliases<S: AsRef<str>>(input: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for alias in input.iter().filter_map(|alias| normalize_alias(alias.as_ref())) {
        if out.len() < MAX_ALIASES && !out.contains(&alias) {
            out.push(alias);
        }
    }
    out
}

/// Drops control characters other than line breaks and trims.
fn clean_text(input: &str) -> String {
    input.chars().filter(|c| !c.is_control() || *c == '\n').collect::<String>().trim().to_owned()
}

/// Validates a licence note (`LICENSE_TOO_LONG`).
pub fn license(input: &str) -> AppResult<String> {
    let license = clean_text(input);
    ensure!(
        license.chars().count() <= LICENSE_MAX_CHARS,
        invalid,
        "LICENSE_TOO_LONG",
        "the licence note is at most {LICENSE_MAX_CHARS} characters"
    );
    Ok(license)
}

/// The licence note of an imported entry, cut to the limit.
pub fn lenient_license(input: &str) -> String {
    clean_text(input).chars().take(LICENSE_MAX_CHARS).collect()
}

/// A category name in stored form: compatibility characters unified, runs
/// of whitespace collapsed. `None` when it cannot be a category name.
pub fn normalize_category(input: &str) -> Option<String> {
    let unified: String = input.nfkc().filter(|c| !c.is_control()).collect();
    let name = unified.split_whitespace().collect::<Vec<_>>().join(" ");
    let length = name.chars().count();
    ((1..=CATEGORY_MAX_CHARS).contains(&length) && !name.starts_with('/') && !name.ends_with('/')).then_some(name)
}

/// Validates the name of a category (`CATEGORY_NAME_INVALID`).
pub fn category_name(input: &str) -> AppResult<String> {
    normalize_category(input).ok_or_else(|| {
        AppError::invalid(
            "CATEGORY_NAME_INVALID",
            "a category name is 1-32 characters and does not start or end with /",
        )
    })
}

/// Validates the reason a moderator gives (`REASON_INVALID`).
pub fn reason(input: &str) -> AppResult<String> {
    let reason = clean_text(input);
    ensure!(
        (1..=REASON_MAX_CHARS).contains(&reason.chars().count()),
        invalid,
        "REASON_INVALID",
        "give a reason of 1-{REASON_MAX_CHARS} characters"
    );
    Ok(reason)
}

/// Validates a search query (`QUERY_TOO_LONG`) and returns it folded for
/// comparison, without surrounding colons. May be empty.
pub fn query(input: &str) -> AppResult<String> {
    ensure!(
        input.chars().count() <= QUERY_MAX_CHARS,
        invalid,
        "QUERY_TOO_LONG",
        "the query is at most {QUERY_MAX_CHARS} characters"
    );
    Ok(fold(input.trim().trim_matches(':').trim()))
}

/// The name an emoji from a pack gets here. Packs made elsewhere allow
/// capitals and a few more characters: capitals are lowered and `-`, `+`,
/// `.` and spaces become `_`. `None` when that does not give a valid name.
pub fn import_name(source: &str) -> Option<String> {
    let name: String = strip_colons(source)
        .nfkc()
        .flat_map(char::to_lowercase)
        .map(|c| if matches!(c, '-' | '+' | '.' | ' ') { '_' } else { c })
        .collect();
    is_name(&name).then_some(name)
}

/// `base` with a number appended (`name_2`), shortened to fit the limit.
pub fn numbered(base: &str, number: u32) -> String {
    let suffix = format!("_{number}");
    let keep = NAME_MAX_CHARS.saturating_sub(suffix.len()).min(base.len());
    format!("{}{suffix}", &base[..keep])
}

fn is_emoji_scalar(c: char) -> bool {
    matches!(
        c as u32,
        // Pictographs, emoticons, transport, flags' regional indicators, skin tones.
        0x1F000..=0x1FAFF
        // Miscellaneous symbols and dingbats (☀, ✂, ❤…).
        | 0x2600..=0x27BF
        // Arrows, technical symbols, geometric shapes, stars and more arrows.
        | 0x2190..=0x21FF | 0x2300..=0x23FF | 0x25A0..=0x25FF | 0x2900..=0x297F | 0x2B00..=0x2BFF
        // Singles: ©, ®, ‼, ⁉, ™, ℹ, Ⓜ, 〰, 〽, ㊗, ㊙.
        | 0x00A9 | 0x00AE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x24C2 | 0x3030 | 0x303D | 0x3297 | 0x3299
    )
}

/// Characters that only modify or join the emoji around them: zero-width
/// joiner, variation selectors, and the tag characters of subdivision flags.
fn is_emoji_modifier(c: char) -> bool {
    matches!(c as u32, 0x200D | 0xFE0E | 0xFE0F | 0xE0020..=0xE007F)
}

/// Whether `text` is one Unicode emoji as a person sees it: a single
/// grapheme (including joined sequences, flags, skin tones and keycaps) made
/// of emoji characters. This is a plausibility check, not the Unicode emoji
/// list: it keeps words and markup out of decks.
pub fn is_unicode_emoji(text: &str) -> bool {
    if text.is_empty() || text.len() > 64 {
        return false;
    }
    let mut graphemes = text.graphemes(true);
    let (Some(grapheme), None) = (graphemes.next(), graphemes.next()) else { return false };
    let chars: Vec<char> = grapheme.chars().collect();
    // Keycaps: a digit, # or * with the enclosing keycap mark (1️⃣).
    if chars.last() == Some(&'\u{20E3}') {
        return matches!(chars[0], '0'..='9' | '#' | '*') && chars[1..chars.len() - 1].iter().all(|c| *c == '\u{FE0F}');
    }
    chars.iter().any(|c| is_emoji_scalar(*c)) && chars.iter().all(|c| is_emoji_scalar(*c) || is_emoji_modifier(*c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_lowered_and_checked() {
        assert_eq!(name("BlobCat").unwrap(), "blobcat");
        assert_eq!(name(":neko_2:").unwrap(), "neko_2");
        assert_eq!(name("ab").unwrap(), "ab");
        assert_eq!(name(&"a".repeat(32)).unwrap().len(), 32);
        for bad in ["", "a", "__", "blob-cat", "ねこ", "blob cat", &"a".repeat(33), "a:b", "blob.cat"] {
            assert_eq!(name(bad).unwrap_err().reason, "NAME_INVALID", "{bad:?}");
        }
    }

    #[test]
    fn reserved_names_include_the_configured_ones() {
        assert!(is_reserved("tokuly", &[]));
        assert!(is_reserved("admin", &[]));
        assert!(!is_reserved("blobcat", &[]));
        assert!(is_reserved("mascot", &[" Mascot ".to_owned()]));
        assert!(!is_reserved("mascot", &[String::new()]));
    }

    #[test]
    fn folding_unifies_width_case_and_kana() {
        assert_eq!(fold("ネコ"), "ねこ");
        assert_eq!(fold("ﾈｺ"), "ねこ");
        assert_eq!(fold("ＣＡＴ"), "cat");
        assert_eq!(fold("ヴァー"), "ゔぁー");
        assert_eq!(fold("猫Cat"), "猫cat");
    }

    #[test]
    fn aliases_are_normalized_and_bounded() {
        assert_eq!(aliases(&["Cat", " ねこ ", "cat", "ＣＡＴ"]).unwrap(), ["cat", "ねこ"]);
        assert!(aliases::<&str>(&[]).unwrap().is_empty());
        let many: Vec<String> = (0..17).map(|i| format!("a{i}")).collect();
        assert_eq!(aliases(&many).unwrap_err().reason, "ALIASES_INVALID");
        for bad in ["", "two words", "a:b", &"あ".repeat(33)] {
            assert_eq!(aliases(&[bad]).unwrap_err().reason, "ALIASES_INVALID", "{bad:?}");
        }
        assert_eq!(aliases(&["あ".repeat(32)]).unwrap().len(), 1);
    }

    #[test]
    fn imported_aliases_keep_what_is_usable() {
        let many: Vec<String> = (0..40).map(|i| format!("a{i}")).collect();
        assert_eq!(lenient_aliases(&many).len(), MAX_ALIASES);
        assert_eq!(lenient_aliases(&["ok", "not ok", "", "OK"]), ["ok"]);
    }

    #[test]
    fn licence_notes_and_reasons_are_bounded() {
        assert_eq!(license("  CC0\u{7}  ").unwrap(), "CC0");
        assert_eq!(license("line one\nline two").unwrap(), "line one\nline two");
        assert_eq!(license(&"x".repeat(1025)).unwrap_err().reason, "LICENSE_TOO_LONG");
        assert_eq!(lenient_license(&"x".repeat(2000)).len(), LICENSE_MAX_CHARS);
        assert_eq!(reason(" 権利の確認ができないよ ").unwrap(), "権利の確認ができないよ");
        assert_eq!(reason("   ").unwrap_err().reason, "REASON_INVALID");
        assert_eq!(reason(&"x".repeat(501)).unwrap_err().reason, "REASON_INVALID");
    }

    #[test]
    fn category_names_are_tidied() {
        assert_eq!(category_name("  どうぶつ   /  ねこ ").unwrap(), "どうぶつ / ねこ");
        assert_eq!(category_name("Ａnimals").unwrap(), "Animals");
        for bad in ["", "   ", "/cats", "cats/", &"c".repeat(33)] {
            assert_eq!(category_name(bad).unwrap_err().reason, "CATEGORY_NAME_INVALID", "{bad:?}");
        }
    }

    #[test]
    fn queries_are_folded_and_bounded() {
        assert_eq!(query(" :BlobCat: ").unwrap(), "blobcat");
        assert_eq!(query("ネコ").unwrap(), "ねこ");
        assert_eq!(query("").unwrap(), "");
        assert_eq!(query(&"q".repeat(65)).unwrap_err().reason, "QUERY_TOO_LONG");
    }

    #[test]
    fn pack_names_are_brought_into_our_alphabet() {
        assert_eq!(import_name("BlobCat").as_deref(), Some("blobcat"));
        assert_eq!(import_name("blob-cat+1").as_deref(), Some("blob_cat_1"));
        assert_eq!(import_name("ＡＢ").as_deref(), Some("ab"));
        assert_eq!(import_name("ねこ"), None);
        assert_eq!(import_name("x"), None);
        assert_eq!(import_name("---"), None);
        assert_eq!(import_name(&"a".repeat(33)), None);
    }

    #[test]
    fn numbered_names_fit_the_limit() {
        assert_eq!(numbered("blobcat", 2), "blobcat_2");
        let long = "a".repeat(32);
        let renamed = numbered(&long, 10);
        assert_eq!(renamed.len(), 32);
        assert!(renamed.ends_with("_10"));
        assert!(is_name(&renamed));
    }

    #[test]
    fn unicode_emoji_are_single_graphemes_of_emoji_characters() {
        for emoji in ["👍", "❤️", "👍🏽", "👨‍👩‍👧", "🇯🇵", "1️⃣", "#️⃣", "🏴󠁧󠁢󠁥󠁮󠁧󠁿", "©️", "☺", "🫠"]
        {
            assert!(is_unicode_emoji(emoji), "{emoji}");
        }
        for other in ["", "a", "1", "ab", "👍👍", "👍 ", ":blobcat:", "猫", "<b>", "#"] {
            assert!(!is_unicode_emoji(other), "{other:?}");
        }
    }
}
