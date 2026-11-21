//! Pulling preview metadata out of an HTML page.
//!
//! Only `<meta>`, `<link>` and `<title>` matter, so instead of building a
//! DOM this walks the tags of the document once. It is deliberately
//! forgiving: real pages have unquoted attributes, stray `<` characters and
//! mislabelled encodings, and a preview that is slightly wrong beats none.
use encoding_rs::{Encoding, UTF_8};

/// Longest title / description kept (characters).
const MAX_TITLE: usize = 300;
const MAX_DESCRIPTION: usize = 500;

/// Metadata found in a page, before URLs are resolved.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PageMeta {
    pub title: String,
    pub description: String,
    pub site_name: String,
    /// As written in the page; may be relative.
    pub image: String,
    pub image_width: i32,
    pub image_height: i32,
    /// As written in the page; may be relative.
    pub favicon: String,
}

/// Decodes a page to text. The charset comes from the `Content-Type` header,
/// else from a `<meta>` declaration near the top, else UTF-8 is assumed
/// (Japanese pages in Shift_JIS or EUC-JP are still common).
pub fn decode(body: &[u8], content_type: &str) -> String {
    let declared = charset_of(content_type).or_else(|| {
        // Declarations are ASCII and must appear early in the document.
        let head = String::from_utf8_lossy(&body[..body.len().min(4096)]).to_ascii_lowercase();
        head.find("charset=").and_then(|at| charset_of(&head[at..]))
    });
    let encoding = declared.and_then(|label| Encoding::for_label(label.as_bytes())).unwrap_or(UTF_8);
    encoding.decode(body).0.into_owned()
}

/// The value after `charset=` in a header value or meta attribute.
fn charset_of(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let after = &lower[lower.find("charset=")? + "charset=".len()..];
    let label: String = after.trim_start_matches(['"', '\'', ' ']).chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect();
    (!label.is_empty()).then_some(label)
}

/// One HTML start tag: its lowercase name and attributes.
struct Tag<'a> {
    name: String,
    attributes: &'a str,
}

impl Tag<'_> {
    /// Value of attribute `name` (case-insensitive), entity-decoded.
    fn attribute(&self, name: &str) -> Option<String> {
        let mut rest = self.attributes;
        loop {
            rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
            if rest.is_empty() {
                return None;
            }
            let key_end = rest.find(|c: char| c.is_whitespace() || c == '=').unwrap_or(rest.len());
            let (key, after_key) = rest.split_at(key_end);
            let after_key = after_key.trim_start();
            let (value, remainder) = match after_key.strip_prefix('=') {
                None => ("", after_key), // attribute without a value
                Some(after_equals) => {
                    let after_equals = after_equals.trim_start();
                    match after_equals.chars().next() {
                        Some(quote @ ('"' | '\'')) => match after_equals[1..].split_once(quote) {
                            Some((value, remainder)) => (value, remainder),
                            None => (&after_equals[1..], ""),
                        },
                        _ => {
                            let end = after_equals.find(char::is_whitespace).unwrap_or(after_equals.len());
                            after_equals.split_at(end)
                        }
                    }
                }
            };
            if key.eq_ignore_ascii_case(name) {
                return Some(decode_entities(value));
            }
            if key.is_empty() {
                return None; // malformed; stop rather than loop
            }
            rest = remainder;
        }
    }
}

/// Byte offset just past the `>` that closes a tag starting in `text`,
/// honouring quoted attribute values.
fn tag_end(text: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (index, byte) in text.bytes().enumerate() {
        match (quote, byte) {
            (Some(open), _) if byte == open => quote = None,
            (None, b'"' | b'\'') => quote = Some(byte),
            (None, b'>') => return Some(index + 1),
            _ => {}
        }
    }
    None
}

/// Case-insensitive `find` for an ASCII needle.
fn find_ignore_case(haystack: &str, needle: &str) -> Option<usize> {
    haystack.as_bytes().windows(needle.len()).position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Replaces character references (`&amp;`, `&#12354;`, `&#x3042;`) in an
/// attribute value or text.
pub fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = rest[1..].find(';').filter(|end| *end <= 10).and_then(|end| {
            let name = &rest[1..1 + end];
            let character = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => {
                    let code = name.strip_prefix('#')?;
                    let value = match code.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => code.parse().ok()?,
                    };
                    char::from_u32(value)
                }
            };
            character.map(|c| (c, end + 2))
        });
        match decoded {
            Some((character, consumed)) => {
                out.push(character);
                rest = &rest[consumed..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Collapses whitespace and limits the length (in characters).
fn tidy(text: &str, max_chars: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let mut cut: String = collapsed.chars().take(max_chars.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// How good a `<link rel>` is as a favicon; higher wins.
fn favicon_rank(rel: &str) -> u8 {
    let rel = rel.to_ascii_lowercase();
    if rel.split_whitespace().any(|token| token == "icon") {
        2 // "icon" and "shortcut icon"
    } else if rel.contains("apple-touch-icon") {
        1
    } else {
        0
    }
}

fn set_if_empty(slot: &mut String, value: Option<String>) {
    if slot.is_empty()
        && let Some(value) = value.filter(|v| !v.trim().is_empty())
    {
        *slot = value.trim().to_owned();
    }
}

/// Extracts preview metadata from HTML text. Open Graph wins over Twitter
/// cards, which win over plain `<title>` / `<meta name="description">`.
pub fn extract(html: &str) -> PageMeta {
    let (mut og, mut twitter, mut plain) = (PageMeta::default(), PageMeta::default(), PageMeta::default());
    let mut favicon_best = 0u8;
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        rest = &rest[open + 1..];
        let name_end = rest.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(rest.len());
        let name = rest[..name_end].to_ascii_lowercase();
        if name.is_empty() {
            continue; // closing tag, comment, doctype or a stray '<'
        }
        let Some(end) = tag_end(rest) else { break };
        let tag = Tag { name, attributes: &rest[name_end..end - 1] };
        rest = &rest[end..];
        match tag.name.as_str() {
            "meta" => {
                let key = tag.attribute("property").or_else(|| tag.attribute("name")).unwrap_or_default().to_ascii_lowercase();
                let content = tag.attribute("content");
                let number = |value: &Option<String>| value.as_deref().and_then(|v| v.trim().parse::<i32>().ok()).filter(|n| *n > 0).unwrap_or(0);
                match key.as_str() {
                    "og:title" => set_if_empty(&mut og.title, content),
                    "og:description" => set_if_empty(&mut og.description, content),
                    "og:site_name" => set_if_empty(&mut og.site_name, content),
                    "og:image" | "og:image:url" | "og:image:secure_url" => set_if_empty(&mut og.image, content),
                    "og:image:width" if og.image_width == 0 => og.image_width = number(&content),
                    "og:image:height" if og.image_height == 0 => og.image_height = number(&content),
                    "twitter:title" => set_if_empty(&mut twitter.title, content),
                    "twitter:description" => set_if_empty(&mut twitter.description, content),
                    "twitter:image" | "twitter:image:src" => set_if_empty(&mut twitter.image, content),
                    "description" => set_if_empty(&mut plain.description, content),
                    "application-name" => set_if_empty(&mut plain.site_name, content),
                    _ => {}
                }
            }
            "link" => {
                let rank = tag.attribute("rel").map(|rel| favicon_rank(&rel)).unwrap_or(0);
                if rank > favicon_best
                    && let Some(href) = tag.attribute("href").filter(|href| !href.trim().is_empty())
                {
                    plain.favicon = href.trim().to_owned();
                    favicon_best = rank;
                }
            }
            "title" if plain.title.is_empty() => {
                let close = find_ignore_case(rest, "</title").unwrap_or(rest.len());
                plain.title = decode_entities(&rest[..close]);
                rest = &rest[close..];
            }
            // Their content is not markup; skip it so a "<" in a script
            // cannot be mistaken for a tag.
            "script" | "style" => {
                let close = find_ignore_case(rest, &format!("</{}", tag.name)).unwrap_or(rest.len());
                rest = &rest[close..];
            }
            _ => {}
        }
    }

    let pick = |values: [&String; 3]| values.into_iter().find(|v| !v.is_empty()).cloned().unwrap_or_default();
    let image = pick([&og.image, &twitter.image, &plain.image]);
    // Dimensions only describe the Open Graph image.
    let (image_width, image_height) = if !og.image.is_empty() { (og.image_width, og.image_height) } else { (0, 0) };
    PageMeta {
        title: tidy(&pick([&og.title, &twitter.title, &plain.title]), MAX_TITLE),
        description: tidy(&pick([&og.description, &twitter.description, &plain.description]), MAX_DESCRIPTION),
        site_name: tidy(&pick([&og.site_name, &twitter.site_name, &plain.site_name]), MAX_TITLE),
        image,
        image_width,
        image_height,
        favicon: plain.favicon,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_graph_wins_over_twitter_and_plain_tags() {
        let meta = extract(
            r#"<!DOCTYPE html><html><head>
            <title>Plain &amp; simple</title>
            <meta name="description" content="plain description">
            <meta name="twitter:title" content="Twitter title">
            <meta name="twitter:image" content="/tw.png">
            <meta property="og:title" content="OG &quot;title&quot;">
            <meta property="og:description" content="  spread
                over lines  ">
            <meta property="og:site_name" content="Example">
            <meta property="og:image" content="https://cdn.example.com/a.jpg">
            <meta property="og:image:width" content="1200"><meta property="og:image:height" content="630">
            <link rel="stylesheet" href="/style.css">
            <link rel="apple-touch-icon" href="/touch.png">
            <link rel="shortcut icon" href="/favicon.png">
            </head><body><p>hello</p></body></html>"#,
        );
        assert_eq!(
            meta,
            PageMeta {
                title: "OG \"title\"".into(),
                description: "spread over lines".into(),
                site_name: "Example".into(),
                image: "https://cdn.example.com/a.jpg".into(),
                image_width: 1200,
                image_height: 630,
                favicon: "/favicon.png".into(),
            }
        );
    }

    #[test]
    fn falls_back_to_twitter_then_plain() {
        let meta = extract("<title>\n  Just a title </title><META NAME=twitter:image CONTENT='/tw.png'><meta content=\"desc\" name=description>");
        assert_eq!((meta.title.as_str(), meta.image.as_str(), meta.description.as_str()), ("Just a title", "/tw.png", "desc"));
        assert_eq!((meta.image_width, meta.image_height), (0, 0));
        assert_eq!(extract("<p>no metadata at all</p>"), PageMeta::default());
    }

    #[test]
    fn scripts_and_broken_markup_do_not_confuse_the_scanner() {
        let meta = extract(
            r#"<script>if (a < b) { document.write('<meta property="og:title" content="injected">'); }</script>
            <style>a > b { content: "<title>nope</title>"; }</style>
            <meta property="og:title" content="a > b">
            <title>Real</title> 1 < 2 <meta name="description" content=unquoted>
            <meta property="og:image" content="unterminated"#,
        );
        assert_eq!(meta.title, "a > b");
        assert_eq!(meta.description, "unquoted");
        assert_eq!(meta.image, "", "an unterminated tag is ignored");
    }

    #[test]
    fn entities_are_decoded() {
        assert_eq!(decode_entities("Tom &amp; Jerry &lt;3 &#12354;&#x3044; &unknown; &#xZZ; &"), "Tom & Jerry <3 あい &unknown; &#xZZ; &");
        assert_eq!(decode_entities("plain"), "plain");
    }

    #[test]
    fn long_text_is_truncated_on_character_boundaries() {
        let meta = extract(&format!("<title>{}</title>", "あ".repeat(400)));
        assert_eq!(meta.title.chars().count(), 300);
        assert!(meta.title.ends_with('…'));
    }

    #[test]
    fn charset_comes_from_header_or_meta() {
        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode("<title>日本語のページ</title>");
        assert_eq!(extract(&decode(&sjis, "text/html; charset=Shift_JIS")).title, "日本語のページ");

        let mut page = b"<meta charset=\"euc-jp\">".to_vec();
        page.extend_from_slice(&encoding_rs::EUC_JP.encode("<title>こんにちは</title>").0);
        assert_eq!(extract(&decode(&page, "text/html")).title, "こんにちは");

        assert_eq!(extract(&decode("<title>既定はUTF-8</title>".as_bytes(), "text/html")).title, "既定はUTF-8");
        assert_eq!(charset_of("text/html; charset=\"utf-8\""), Some("utf-8".into()));
        assert_eq!(charset_of("text/html"), None);
    }
}
