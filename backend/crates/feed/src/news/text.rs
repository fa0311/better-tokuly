//! Turning feed markup into plain text.
//!
//! Titles and summaries in feeds are HTML more often than not, sometimes
//! escaped twice. Clients show them as text, and the search index stores
//! them, so what leaves this module must be text and nothing else:
//!
//! - tags and comments are removed, whatever is inside `<script>`, `<style>`
//!   and similar elements is dropped entirely;
//! - character references are decoded once, *after* the markup is gone, so
//!   `&lt;b&gt;` comes out as the four characters `<b>` and not as a tag;
//! - control characters are removed and white space collapsed.
//!
//! The scanner is deliberately forgiving about broken markup and strict
//! about the result: anything that might still be inside a tag when the
//! input ends is dropped rather than leaked as text.

/// Elements whose content is not text for readers.
const DROPPED_ELEMENTS: &[&str] = &["script", "style", "noscript", "template", "iframe", "object", "svg", "math", "head", "title"];
/// Elements that end a line or a block: they become a space.
const BREAKING_ELEMENTS: &[&str] = &[
    "br", "p", "div", "li", "ul", "ol", "tr", "td", "th", "table", "h1", "h2", "h3", "h4", "h5", "h6", "blockquote", "pre", "hr", "section", "article",
    "header", "footer", "figure", "figcaption", "dd", "dt", "dl", "img",
];

/// Whether the text at a `<` starts markup (a tag, a comment, a declaration)
/// rather than being a stray less-than sign.
fn starts_markup(rest: &str) -> bool {
    matches!(rest.as_bytes().get(1), Some(b) if b.is_ascii_alphabetic() || matches!(b, b'/' | b'!' | b'?'))
}

/// Byte length of the tag starting at the beginning of `rest` (which starts
/// with `<`), up to and including its closing `>`. Quoted attribute values
/// may contain `>`. `None` when the tag never ends.
fn tag_len(rest: &str) -> Option<usize> {
    if rest.starts_with("<!--") {
        return rest.find("-->").map(|end| end + 3);
    }
    let mut quote: Option<u8> = None;
    for (index, byte) in rest.bytes().enumerate().skip(1) {
        match (quote, byte) {
            (Some(open), b) if b == open => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(byte),
            (None, b'>') => return Some(index + 1),
            (None, _) => {}
        }
    }
    None
}

/// The lowercased element name of a tag (`<a href>` and `</a>` both give `a`).
fn element_name(tag: &str) -> String {
    tag.trim_start_matches('<').trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

/// Byte offset just past the closing tag of the dropped element `name`,
/// searching `rest` case-insensitively. `None` when it is never closed.
fn skip_element(rest: &str, name: &str) -> Option<usize> {
    let lower = rest.to_ascii_lowercase();
    let closing = format!("</{name}");
    let mut from = 0;
    while let Some(found) = lower[from..].find(&closing) {
        let at = from + found;
        let after = at + closing.len();
        // `</scripty>` does not close `<script>`.
        let boundary = lower.as_bytes().get(after).is_none_or(|b| !b.is_ascii_alphanumeric());
        if boundary {
            return Some(at + tag_len(&rest[at..])?);
        }
        from = after;
    }
    None
}

/// Removes markup, leaving the text between the tags (still with character
/// references).
fn strip_tags(input: &str) -> String {
    let mut text = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(open) = rest.find('<') {
        text.push_str(&rest[..open]);
        rest = &rest[open..];
        if !starts_markup(rest) {
            text.push('<');
            rest = &rest[1..];
            continue;
        }
        // An unterminated tag swallows the rest: better to lose text than to leak markup.
        let Some(len) = tag_len(rest) else { return text };
        let tag = &rest[..len];
        let name = element_name(tag);
        rest = &rest[len..];
        let opening = !tag.starts_with("</") && !tag.ends_with("/>");
        if opening && DROPPED_ELEMENTS.contains(&name.as_str()) {
            match skip_element(rest, &name) {
                Some(skip) => rest = &rest[skip..],
                None => return text,
            }
            text.push(' ');
        } else if BREAKING_ELEMENTS.contains(&name.as_str()) {
            text.push(' ');
        }
    }
    text.push_str(rest);
    text
}

/// Named character references worth knowing; everything else stays as written.
const ENTITIES: &[(&str, &str)] = &[
    ("amp", "&"),
    ("lt", "<"),
    ("gt", ">"),
    ("quot", "\""),
    ("apos", "'"),
    ("nbsp", " "),
    ("ensp", " "),
    ("emsp", " "),
    ("thinsp", " "),
    ("hellip", "…"),
    ("mdash", "—"),
    ("ndash", "–"),
    ("lsquo", "‘"),
    ("rsquo", "’"),
    ("ldquo", "“"),
    ("rdquo", "”"),
    ("laquo", "«"),
    ("raquo", "»"),
    ("middot", "·"),
    ("bull", "•"),
    ("copy", "©"),
    ("reg", "®"),
    ("trade", "™"),
    ("yen", "¥"),
    ("euro", "€"),
    ("pound", "£"),
    ("times", "×"),
    ("deg", "°"),
];

/// The character a reference body (`amp`, `#12354`, `#x3042`) stands for.
fn decode_reference(body: &str) -> Option<String> {
    if let Some(number) = body.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse::<u32>().ok()?,
        };
        return char::from_u32(code).filter(|c| *c != '\0').map(String::from);
    }
    ENTITIES.iter().find(|(name, _)| *name == body).map(|(_, text)| (*text).to_owned())
}

/// Decodes character references, once.
fn decode_entities(input: &str) -> String {
    let mut text = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp) = rest.find('&') {
        text.push_str(&rest[..amp]);
        rest = &rest[amp..];
        // A reference is `&`, a short body, `;`.
        let decoded = rest[1..].find(';').filter(|end| *end > 0 && *end <= 10).and_then(|end| decode_reference(&rest[1..=end]).map(|decoded| (decoded, end + 2)));
        match decoded {
            Some((decoded, consumed)) => {
                text.push_str(&decoded);
                rest = &rest[consumed..];
            }
            None => {
                text.push('&');
                rest = &rest[1..];
            }
        }
    }
    text.push_str(rest);
    text
}

/// Single spaces, no control characters, no leading or trailing space.
pub fn collapse(input: &str) -> String {
    let cleaned: String = input.chars().map(|c| if c.is_control() || c == '\u{a0}' || c == '\u{3000}' { ' ' } else { c }).collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn looks_like_markup(text: &str) -> bool {
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        rest = &rest[open..];
        if starts_markup(rest) && tag_len(rest).is_some() {
            return true;
        }
        rest = &rest[1..];
    }
    false
}

/// The plain text of a piece of feed markup.
pub fn html_to_text(input: &str) -> String {
    let once = decode_entities(&strip_tags(input));
    // Feeds that escape their HTML twice leave tags behind after one round.
    let text = if looks_like_markup(&once) { decode_entities(&strip_tags(&once)) } else { once };
    collapse(&text)
}

/// At most `max` characters, cut at a character boundary with an ellipsis.
pub fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", text[..cut].trim_end()),
        None => text.to_owned(),
    }
}

/// The first `<img src>` of a piece of markup, as written.
pub fn first_image(html: &str) -> Option<String> {
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        rest = &rest[open..];
        if !starts_markup(rest) {
            rest = &rest[1..];
            continue;
        }
        let len = tag_len(rest)?;
        let tag = &rest[..len];
        if element_name(tag) == "img"
            && let Some(src) = attribute(tag, "src")
        {
            return Some(src);
        }
        rest = &rest[len..];
    }
    None
}

/// The value of attribute `name` in a tag, with character references decoded.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        let after = at + name.len();
        let starts_word = lower.as_bytes().get(at.wrapping_sub(1)).is_some_and(|b| b.is_ascii_whitespace());
        let value = tag[after..].trim_start();
        if starts_word && let Some(value) = value.strip_prefix('=') {
            let value = value.trim_start();
            let raw = match value.as_bytes().first() {
                Some(quote @ (b'"' | b'\'')) => value[1..].split(*quote as char).next().unwrap_or_default(),
                _ => value.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or_default(),
            };
            let decoded = decode_entities(raw.trim());
            return (!decoded.is_empty()).then_some(decoded);
        }
        from = after;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_removed_and_blocks_become_spaces() {
        assert_eq!(html_to_text("<p>Hello <b>world</b></p><p>Second&nbsp;line</p>"), "Hello world Second line");
        assert_eq!(html_to_text("一行目<br/>二行目<br>三行目"), "一行目 二行目 三行目");
        assert_eq!(html_to_text("<a href=\"https://example.com/?a=1&amp;b=2\">link</a> text"), "link text");
        assert_eq!(html_to_text("no markup at all"), "no markup at all");
        assert_eq!(html_to_text(""), "");
    }

    #[test]
    fn scripts_styles_and_comments_leave_nothing_behind() {
        assert_eq!(html_to_text("before<script>alert('x < y')</script>after"), "before after");
        assert_eq!(html_to_text("a<STYLE type=\"text/css\">p { color: red }</STYLE>b"), "a b");
        assert_eq!(html_to_text("a<!-- <b>hidden</b> -->b"), "ab");
        assert_eq!(html_to_text("a<iframe src=\"https://evil.example\">fallback</iframe>b"), "a b");
        // `</scripty>` is not the end of the script.
        assert_eq!(html_to_text("a<script>1</scripty>2</script>b"), "a b");
    }

    #[test]
    fn broken_markup_never_leaks_as_text() {
        // Unterminated tags, scripts and comments swallow the rest.
        assert_eq!(html_to_text("safe <img src=\"x\" onerror=\"alert(1)"), "safe");
        assert_eq!(html_to_text("safe <script>alert(1)"), "safe");
        assert_eq!(html_to_text("safe <!-- never closed"), "safe");
        // A `>` inside a quoted attribute does not end the tag.
        assert_eq!(html_to_text("<a title=\"a > b\" href=\"#\">x</a>y"), "xy");
        assert_eq!(html_to_text("<img alt='1 > 0'>ok"), "ok");
    }

    #[test]
    fn stray_angle_brackets_are_text() {
        assert_eq!(html_to_text("1 < 2 and 3 > 2"), "1 < 2 and 3 > 2");
        assert_eq!(html_to_text("a <- b"), "a <- b");
        assert_eq!(html_to_text("x<3"), "x<3");
    }

    #[test]
    fn references_are_decoded_once_after_the_markup_is_gone() {
        assert_eq!(html_to_text("Tom &amp; Jerry &lt;3"), "Tom & Jerry <3");
        assert_eq!(html_to_text("&#12354;&#x3044;&#X3046;"), "あいう");
        assert_eq!(html_to_text("&quot;quoted&quot; &hellip; &unknown; &; & alone"), "\"quoted\" … &unknown; &; & alone");
        // An escaped ampersand followed by `lt;` is text, not a second reference.
        assert_eq!(html_to_text("&amp;lt;"), "&lt;");
        assert_eq!(html_to_text("&#0;&#xD800;&#99999999;"), "&#0;&#xD800;&#99999999;");
    }

    #[test]
    fn double_escaped_html_is_cleaned_too() {
        // What is left after one round still forms markup: the feed escaped
        // its HTML twice, so a second round strips it.
        assert_eq!(html_to_text("&lt;p&gt;Hello&lt;/p&gt;"), "Hello");
        assert_eq!(html_to_text("&lt;script&gt;alert(1)&lt;/script&gt;after"), "after");
        assert_eq!(html_to_text("&lt;p&gt;Tom &amp;amp; Jerry&lt;/p&gt;"), "Tom & Jerry");
        // No markup after one round: references are not decoded again.
        assert_eq!(html_to_text("&amp;lt;b&amp;gt;bold&amp;lt;/b&amp;gt;"), "&lt;b&gt;bold&lt;/b&gt;");
    }

    #[test]
    fn white_space_and_control_characters_are_normalized() {
        assert_eq!(html_to_text("  a\n\n b\t\tc\u{0007}d\u{3000}e\u{a0}f "), "a b c d e f");
        assert_eq!(collapse("\u{0000}x\r\ny"), "x y");
    }

    #[test]
    fn truncation_respects_characters() {
        assert_eq!(truncate("こんにちは世界", 5), "こんにちは…");
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("word then cut", 5), "word…");
        assert_eq!(truncate("", 3), "");
    }

    #[test]
    fn the_first_image_of_a_body_is_found() {
        assert_eq!(first_image("<p>text</p><img class=\"a\" src=\"https://cdn.example/a.jpg?x=1&amp;y=2\"><img src='b.png'>").as_deref(), Some("https://cdn.example/a.jpg?x=1&y=2"));
        assert_eq!(first_image("<IMG SRC='/relative/pic.png' alt=x>").as_deref(), Some("/relative/pic.png"));
        assert_eq!(first_image("<img src=unquoted.gif>").as_deref(), Some("unquoted.gif"));
        assert_eq!(first_image("<img data-src=\"lazy.jpg\" alt=\"no src\">"), None);
        assert_eq!(first_image("<p>no images</p> 1 < 2"), None);
        assert_eq!(first_image("<img src=\"\">"), None);
    }
}
