//! The small markup of note text (MFM-like), parsed on the server into the
//! typed tree clients render ([`RichText`]). Pure functions, no I/O.
//!
//! The tree is stored with the note and returned as it is, so no client
//! implements the grammar again and nothing outside the tree can be
//! expressed: there is no HTML, no style, no position — only the node kinds
//! of `tokuly.notes.v1.RichText`. What the server acts on (hashtags,
//! mentions, the first link, custom emoji) is read off the same tree
//! ([`facts`]), so a `#tag` inside inline code is neither a hashtag in the
//! text nor one in the hashtag timeline.
//!
//! Grammar (documented for people in `docs/events/notes.md`):
//!
//! - **Blocks**, line by line: a line ```` ```lang ```` up to a line
//!   ```` ``` ```` is a code block (no closing line: plain text); lines
//!   starting with `>` are a quote; a `<center>` line up to a `</center>`
//!   line is centred; everything else forms paragraphs, separated by blank
//!   lines, with line breaks kept as `\n`.
//! - **Inline**: `**bold**` / `<b>`, `*italic*` / `<i>`, `~~strike~~` /
//!   `<s>`, `<small>`, `` `code` ``, `[label](https://…)`, bare http(s)
//!   links, `@handle`, `#hashtag`, `:emoji:`, `$[effect content]`. A
//!   backslash before a markup character writes it as it is. Markup that is
//!   not closed is plain text.
//! - **Effects** (Misskey's animated functions, the small safe set):
//!   `tada jelly jump bounce shake spin rainbow sparkle x2`. Clients play
//!   them only when the reader has not asked for reduced motion. Left out
//!   on purpose: position, rotation, scale beyond twice, blur, colours and
//!   fonts — they can cover the rest of the screen, hide text from some
//!   readers or imitate the app's own interface.
//! - Nesting stops at depth [`MAX_DEPTH`]: deeper markup stays plain text.
use tokuly_proto::notes::{
    RichText, TextBlock, TextCodeBlock, TextEffect, TextEffectKind, TextEmoji, TextHashtag, TextInline, TextLink,
    TextMention, TextParagraph, TextParagraphs, TextSpan, TextStyle, text_block::Block, text_inline::Inline,
};

use crate::text;

/// How deeply spans and effects may nest.
pub const MAX_DEPTH: usize = 8;
/// Longest `:shortcode:` taken as a custom emoji.
pub const MAX_EMOJI_NAME: usize = 64;
/// Custom emoji resolved per note.
pub const MAX_EMOJI: usize = 50;
/// Longest language name after an opening fence.
const MAX_FENCE_LANGUAGE: usize = 20;

/// The effects that exist, by their name in `$[name …]`.
const EFFECTS: &[(&str, TextEffectKind)] = &[
    ("tada", TextEffectKind::TEXT_EFFECT_KIND_TADA),
    ("jelly", TextEffectKind::TEXT_EFFECT_KIND_JELLY),
    ("jump", TextEffectKind::TEXT_EFFECT_KIND_JUMP),
    ("bounce", TextEffectKind::TEXT_EFFECT_KIND_BOUNCE),
    ("shake", TextEffectKind::TEXT_EFFECT_KIND_SHAKE),
    ("spin", TextEffectKind::TEXT_EFFECT_KIND_SPIN),
    ("rainbow", TextEffectKind::TEXT_EFFECT_KIND_RAINBOW),
    ("sparkle", TextEffectKind::TEXT_EFFECT_KIND_SPARKLE),
    ("x2", TextEffectKind::TEXT_EFFECT_KIND_X2),
];

/// Delimited spans: opening marker, closing marker, style.
const SPANS: &[(&str, &str, TextStyle)] = &[
    ("**", "**", TextStyle::TEXT_STYLE_BOLD),
    ("~~", "~~", TextStyle::TEXT_STYLE_STRIKE),
    ("<b>", "</b>", TextStyle::TEXT_STYLE_BOLD),
    ("<i>", "</i>", TextStyle::TEXT_STYLE_ITALIC),
    ("<s>", "</s>", TextStyle::TEXT_STYLE_STRIKE),
    ("<small>", "</small>", TextStyle::TEXT_STYLE_SMALL),
];

/// Parses note text (as cleaned by [`text::clean`]) into its tree.
pub fn parse(input: &str) -> RichText {
    let lines: Vec<&str> = input.split('\n').collect();
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(language) = fence_language(line)
            && let Some(close) = (i + 1..lines.len()).find(|j| lines[*j].trim_end() == "```")
        {
            flush_paragraphs(&mut paragraph, &mut blocks);
            let code = TextCodeBlock {
                language: language.to_owned(),
                code: lines[i + 1..close].join("\n"),
                ..Default::default()
            };
            blocks.push(block(Block::Code(Box::new(code))));
            i = close + 1;
            continue;
        }
        if line.trim() == "<center>"
            && let Some(close) = (i + 1..lines.len()).find(|j| lines[*j].trim() == "</center>")
        {
            flush_paragraphs(&mut paragraph, &mut blocks);
            let paragraphs = paragraphs(&lines[i + 1..close]);
            blocks.push(block(Block::Center(Box::new(TextParagraphs { paragraphs, ..Default::default() }))));
            i = close + 1;
            continue;
        }
        if line.starts_with('>') {
            flush_paragraphs(&mut paragraph, &mut blocks);
            let mut quoted = Vec::new();
            while i < lines.len() && lines[i].starts_with('>') {
                let rest = &lines[i][1..];
                quoted.push(rest.strip_prefix(' ').unwrap_or(rest));
                i += 1;
            }
            let paragraphs = paragraphs(&quoted);
            if !paragraphs.is_empty() {
                blocks.push(block(Block::Quote(Box::new(TextParagraphs { paragraphs, ..Default::default() }))));
            }
            continue;
        }
        paragraph.push(line);
        i += 1;
    }
    flush_paragraphs(&mut paragraph, &mut blocks);
    RichText { blocks, ..Default::default() }
}

fn block(block: Block) -> TextBlock {
    TextBlock { block: Some(block), ..Default::default() }
}

/// The language after an opening fence: "```" plus an optional word.
fn fence_language(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("```")?.trim();
    let plain = rest.len() <= MAX_FENCE_LANGUAGE && !rest.contains('`') && !rest.contains(char::is_whitespace);
    plain.then_some(rest)
}

/// Lines into paragraphs: blank lines separate them.
fn paragraphs(lines: &[&str]) -> Vec<TextParagraph> {
    let mut out = Vec::new();
    let mut run: Vec<&str> = Vec::new();
    for line in lines.iter().copied().chain(std::iter::once("")) {
        if line.trim().is_empty() {
            if !run.is_empty() {
                let chars: Vec<char> = run.join("\n").chars().collect();
                out.push(TextParagraph { inlines: inlines(&chars, 0, true), ..Default::default() });
                run.clear();
            }
        } else {
            run.push(line);
        }
    }
    out
}

fn flush_paragraphs(lines: &mut Vec<&str>, blocks: &mut Vec<TextBlock>) {
    for paragraph in paragraphs(lines) {
        blocks.push(block(Block::Paragraph(Box::new(paragraph))));
    }
    lines.clear();
}

fn node(inline: Inline) -> TextInline {
    TextInline { inline: Some(inline), ..Default::default() }
}

fn is_markup_char(c: char) -> bool {
    matches!(c, '\\' | '*' | '~' | '`' | '[' | ']' | '(' | ')' | '$' | ':' | '#' | '＃' | '@' | '＠' | '<' | '>' | '_')
}

fn is_emoji_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '+'
}

fn starts_with(chars: &[char], at: usize, marker: &str) -> bool {
    (at..).zip(marker.chars()).all(|(index, expected)| chars.get(index) == Some(&expected))
}

/// Where `marker` next starts at or after `from`, skipping escaped characters.
fn find_marker(chars: &[char], from: usize, marker: &str) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if starts_with(chars, i, marker) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Inline markup of `chars`. `links`: links may be made (not inside a link label).
fn inlines(chars: &[char], depth: usize, links: bool) -> Vec<TextInline> {
    let mut out: Vec<TextInline> = Vec::new();
    let mut plain = String::new();
    let flush = |plain: &mut String, out: &mut Vec<TextInline>| {
        if !plain.is_empty() {
            out.push(node(Inline::Text(std::mem::take(plain))));
        }
    };
    let mut i = 0;
    'scan: while i < chars.len() {
        let c = chars[i];
        let previous = i.checked_sub(1).map(|p| chars[p]);

        if c == '\\' && chars.get(i + 1).is_some_and(|next| is_markup_char(*next)) {
            plain.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if links && let Some(end) = text::url_end(chars, i, previous) {
            flush(&mut plain, &mut out);
            let url: String = chars[i..end].iter().collect();
            out.push(node(Inline::Link(Box::new(TextLink { url, ..Default::default() }))));
            i = end;
            continue;
        }
        if c == '`'
            && let Some(close) = (i + 1..chars.len()).find(|j| chars[*j] == '`' || chars[*j] == '\n')
            && chars[close] == '`'
            && close > i + 1
        {
            flush(&mut plain, &mut out);
            out.push(node(Inline::Code(chars[i + 1..close].iter().collect())));
            i = close + 1;
            continue;
        }
        if links
            && c == '['
            && let Some((label_end, url_end)) = markdown_link(chars, i)
        {
            flush(&mut plain, &mut out);
            let label =
                if depth < MAX_DEPTH { inlines(&chars[i + 1..label_end], depth + 1, false) } else { Vec::new() };
            let url: String = chars[label_end + 2..url_end].iter().collect();
            out.push(node(Inline::Link(Box::new(TextLink { url, label, ..Default::default() }))));
            i = url_end + 1;
            continue;
        }
        if depth < MAX_DEPTH {
            for (open, close, style) in SPANS {
                let inner = i + open.chars().count();
                if starts_with(chars, i, open)
                    && let Some(end) = find_marker(chars, inner, close)
                    && end > inner
                {
                    flush(&mut plain, &mut out);
                    let children = inlines(&chars[inner..end], depth + 1, links);
                    out.push(node(Inline::Span(Box::new(TextSpan {
                        style: (*style).into(),
                        children,
                        ..Default::default()
                    }))));
                    i = end + close.chars().count();
                    continue 'scan;
                }
            }
            if c == '*'
                && let Some(end) = italic_end(chars, i)
            {
                flush(&mut plain, &mut out);
                let children = inlines(&chars[i + 1..end], depth + 1, links);
                let style = TextStyle::TEXT_STYLE_ITALIC.into();
                out.push(node(Inline::Span(Box::new(TextSpan { style, children, ..Default::default() }))));
                i = end + 1;
                continue;
            }
            if starts_with(chars, i, "$[")
                && let Some((kind, content_start, end)) = effect(chars, i)
            {
                flush(&mut plain, &mut out);
                let children = inlines(&chars[content_start..end], depth + 1, links);
                out.push(node(Inline::Effect(Box::new(TextEffect {
                    kind: kind.into(),
                    children,
                    ..Default::default()
                }))));
                i = end + 1;
                continue;
            }
        }
        if c == ':' {
            let end = text::run_end(chars, i + 1, is_emoji_char);
            if end > i + 1 && end - i - 1 <= MAX_EMOJI_NAME && chars.get(end) == Some(&':') {
                flush(&mut plain, &mut out);
                let name: String = chars[i + 1..end].iter().collect::<String>().to_ascii_lowercase();
                out.push(node(Inline::Emoji(Box::new(TextEmoji { name, ..Default::default() }))));
                i = end + 1;
                continue;
            }
        }
        // Hashtags and mentions follow the same rules as `text::parse`.
        if (c == '#' || c == '＃') && previous.is_none_or(|p| !text::is_word(p) && p != '/' && p != '&') {
            let end = text::run_end(chars, i + 1, text::is_hashtag_char);
            let written: String = chars[i + 1..end].iter().collect();
            if let Some(tag) = text::normalize_hashtag(&written) {
                flush(&mut plain, &mut out);
                out.push(node(Inline::Hashtag(Box::new(TextHashtag { tag, text: written, ..Default::default() }))));
                i = end;
                continue;
            }
        }
        if (c == '@' || c == '＠')
            && previous.is_none_or(|p| !p.is_ascii_alphanumeric() && !matches!(p, '_' | '/' | '@' | '＠'))
        {
            let end = text::run_end(chars, i + 1, text::is_handle_char);
            let handle = chars[i + 1..end].iter().collect::<String>().to_ascii_lowercase();
            let foreign = chars.get(end).is_some_and(|next| *next == '@');
            if text::is_handle(&handle) && !foreign {
                flush(&mut plain, &mut out);
                out.push(node(Inline::Mention(Box::new(TextMention { handle, ..Default::default() }))));
                i = end;
                continue;
            }
        }
        plain.push(c);
        i += 1;
    }
    flush(&mut plain, &mut out);
    out
}

/// `[label](url)` starting at `start`: the index of `]` and of the closing `)`.
fn markdown_link(chars: &[char], start: usize) -> Option<(usize, usize)> {
    let close = (start + 1..chars.len()).find(|j| matches!(chars[*j], ']' | '[' | '\n'))?;
    if chars[close] != ']' || close == start + 1 || chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let url_end = text::url_end(chars, close + 2, None)?;
    (chars.get(url_end) == Some(&')')).then_some((close, url_end))
}

/// `*italic*` starting at `start` (not `**`): the index of the closing `*`.
/// The text inside starts and ends with something other than a space and
/// stays on one line, so `2 * 3 * 4` is no markup.
fn italic_end(chars: &[char], start: usize) -> Option<usize> {
    let first = *chars.get(start + 1)?;
    if first == '*' || first.is_whitespace() {
        return None;
    }
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '\n' => return None,
            '*' if chars.get(i + 1) != Some(&'*') && !chars[i - 1].is_whitespace() => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// `$[name content]` starting at `start`: the effect, where its content
/// starts and the index of the matching `]`.
fn effect(chars: &[char], start: usize) -> Option<(TextEffectKind, usize, usize)> {
    let name_end = text::run_end(chars, start + 2, |c| c.is_ascii_alphanumeric());
    let name: String = chars[start + 2..name_end].iter().collect();
    let kind = EFFECTS.iter().find(|(known, _)| *known == name).map(|(_, kind)| *kind)?;
    if chars.get(name_end) != Some(&' ') {
        return None;
    }
    let mut depth = 0usize;
    let mut i = name_end + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '[' => depth += 1,
            ']' if depth == 0 => return (i > name_end + 1).then_some((kind, name_end + 1, i)),
            ']' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

/// What the server acts on in a note, read off its tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// Normalized hashtags in order of first appearance (at most `text::MAX_HASHTAGS`).
    pub hashtags: Vec<String>,
    /// Mentioned handles in order of first appearance (at most `text::MAX_MENTIONS`).
    pub handles: Vec<String>,
    /// The first link.
    pub first_url: Option<String>,
    /// Custom emoji names (at most [`MAX_EMOJI`]).
    pub emoji: Vec<String>,
}

pub fn facts(body: &RichText) -> Facts {
    let mut facts = Facts::default();
    for block in &body.blocks {
        match &block.block {
            Some(Block::Paragraph(paragraph)) => collect(&paragraph.inlines, &mut facts),
            Some(Block::Quote(paragraphs)) | Some(Block::Center(paragraphs)) => {
                for paragraph in &paragraphs.paragraphs {
                    collect(&paragraph.inlines, &mut facts);
                }
            }
            Some(Block::Code(_)) | None => {}
        }
    }
    facts
}

fn collect(inlines: &[TextInline], facts: &mut Facts) {
    for inline in inlines {
        match &inline.inline {
            Some(Inline::Span(span)) => collect(&span.children, facts),
            Some(Inline::Effect(effect)) => collect(&effect.children, facts),
            Some(Inline::Link(link)) => {
                if facts.first_url.is_none() {
                    facts.first_url = Some(link.url.clone());
                }
                collect(&link.label, facts);
            }
            Some(Inline::Hashtag(tag)) => {
                if !facts.hashtags.contains(&tag.tag) && facts.hashtags.len() < text::MAX_HASHTAGS {
                    facts.hashtags.push(tag.tag.clone());
                }
            }
            Some(Inline::Mention(mention)) => {
                if !facts.handles.contains(&mention.handle) && facts.handles.len() < text::MAX_MENTIONS {
                    facts.handles.push(mention.handle.clone());
                }
            }
            Some(Inline::Emoji(emoji)) => {
                if !facts.emoji.contains(&emoji.name) && facts.emoji.len() < MAX_EMOJI {
                    facts.emoji.push(emoji.name.clone());
                }
            }
            Some(Inline::Text(_)) | Some(Inline::Code(_)) | None => {}
        }
    }
}

/// Visits every inline node of a tree mutably (hydration fills in mention
/// users and emoji pictures).
pub fn for_each_inline(body: &mut RichText, visit: &mut impl FnMut(&mut TextInline)) {
    fn walk(inlines: &mut [TextInline], visit: &mut impl FnMut(&mut TextInline)) {
        for inline in inlines {
            visit(inline);
            match &mut inline.inline {
                Some(Inline::Span(span)) => walk(&mut span.children, visit),
                Some(Inline::Effect(effect)) => walk(&mut effect.children, visit),
                Some(Inline::Link(link)) => walk(&mut link.label, visit),
                _ => {}
            }
        }
    }
    for block in &mut body.blocks {
        match &mut block.block {
            Some(Block::Paragraph(paragraph)) => walk(&mut paragraph.inlines, visit),
            Some(Block::Quote(paragraphs)) | Some(Block::Center(paragraphs)) => {
                for paragraph in &mut paragraphs.paragraphs {
                    walk(&mut paragraph.inlines, visit);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A compact rendering of a tree, for readable assertions.
    fn show(body: &RichText) -> String {
        fn inline(node: &TextInline) -> String {
            match &node.inline {
                Some(Inline::Text(text)) => text.replace('\n', "⏎"),
                Some(Inline::Code(code)) => format!("`{code}`"),
                Some(Inline::Span(span)) => {
                    let tag = match span.style.as_known() {
                        Some(TextStyle::TEXT_STYLE_BOLD) => "b",
                        Some(TextStyle::TEXT_STYLE_ITALIC) => "i",
                        Some(TextStyle::TEXT_STYLE_STRIKE) => "s",
                        Some(TextStyle::TEXT_STYLE_SMALL) => "small",
                        _ => "?",
                    };
                    format!("<{tag}>{}</{tag}>", all(&span.children))
                }
                Some(Inline::Link(link)) if link.label.is_empty() => format!("<{}>", link.url),
                Some(Inline::Link(link)) => format!("[{}]({})", all(&link.label), link.url),
                Some(Inline::Mention(mention)) => format!("{{@{}}}", mention.handle),
                Some(Inline::Hashtag(tag)) => format!("{{#{}|{}}}", tag.tag, tag.text),
                Some(Inline::Emoji(emoji)) => format!("{{:{}:}}", emoji.name),
                Some(Inline::Effect(effect)) => format!("$[{} {}]", effect.kind.to_i32(), all(&effect.children)),
                None => "∅".into(),
            }
        }
        fn all(nodes: &[TextInline]) -> String {
            nodes.iter().map(inline).collect()
        }
        fn paragraphs(paragraphs: &[TextParagraph]) -> String {
            paragraphs.iter().map(|p| all(&p.inlines)).collect::<Vec<_>>().join(" ¶ ")
        }
        body.blocks
            .iter()
            .map(|block| match &block.block {
                Some(Block::Paragraph(p)) => format!("P({})", all(&p.inlines)),
                Some(Block::Quote(q)) => format!("Q({})", paragraphs(&q.paragraphs)),
                Some(Block::Center(c)) => format!("C({})", paragraphs(&c.paragraphs)),
                Some(Block::Code(code)) => format!("CODE[{}]({})", code.language, code.code.replace('\n', "⏎")),
                None => "∅".into(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn p(text: &str) -> String {
        show(&parse(text))
    }

    #[test]
    fn plain_text_is_one_paragraph_with_its_line_breaks() {
        assert_eq!(p("今何してる?"), "P(今何してる?)");
        assert_eq!(p("一行目\n二行目"), "P(一行目⏎二行目)");
        assert_eq!(p("one\n\n\ntwo"), "P(one) P(two)");
        assert_eq!(p(""), "");
    }

    #[test]
    fn styles_nest_and_unclosed_markup_stays_text() {
        assert_eq!(p("**太字** と *斜体* と ~~取り消し~~"), "P(<b>太字</b> と <i>斜体</i> と <s>取り消し</s>)");
        assert_eq!(p("<b>a <i>b</i></b><small>c</small>"), "P(<b>a <i>b</i></b><small>c</small>)");
        assert_eq!(p("**open"), "P(**open)");
        assert_eq!(p("2 * 3 * 4"), "P(2 * 3 * 4)");
        assert_eq!(p("a*b*c"), "P(a<i>b</i>c)");
        assert_eq!(p("****"), "P(****)");
        assert_eq!(p("<script>alert(1)</script>"), "P(<script>alert(1)</script>)", "unknown tags are text");
    }

    #[test]
    fn escapes_write_markup_characters_plainly() {
        assert_eq!(p("\\*not italic\\*"), "P(*not italic*)");
        assert_eq!(p("\\#nottag \\@nobody"), "P(#nottag @nobody)");
        assert_eq!(p("a \\ b"), "P(a \\ b)", "a backslash before other characters stays");
    }

    #[test]
    fn code_is_taken_literally() {
        assert_eq!(p("use `#tag` and `**x**`"), "P(use `#tag` and `**x**`)");
        assert_eq!(p("```rust\nfn main() {}\n  **x**\n```\nafter"), "CODE[rust](fn main() {}⏎  **x**) P(after)");
        assert_eq!(p("```\nno end"), "P(```⏎no end)");
        assert_eq!(p("``"), "P(``)");
        assert_eq!(p("`a\nb`"), "P(`a⏎b`)", "inline code stays on one line");
    }

    #[test]
    fn quotes_and_centre_are_blocks() {
        assert_eq!(p("> 引用です\n> 二行目\nふつう"), "Q(引用です⏎二行目) P(ふつう)");
        assert_eq!(p(">no space"), "Q(no space)");
        assert_eq!(p("<center>\n**真ん中**\n</center>"), "C(<b>真ん中</b>)");
        assert_eq!(p("<center>\nunclosed"), "P(<center>⏎unclosed)");
    }

    #[test]
    fn links_are_http_only_and_labels_hold_no_links() {
        assert_eq!(p("見て https://example.com/a?b=1。"), "P(見て <https://example.com/a?b=1>。)");
        assert_eq!(p("[サイト](https://example.com)"), "P([サイト](https://example.com))");
        assert_eq!(p("[**太**](https://e.com/x)"), "P([<b>太</b>](https://e.com/x))");
        assert_eq!(p("[x](javascript:alert(1))"), "P([x](javascript:alert(1)))", "only http(s) links");
        assert_eq!(p("[https://a.com](https://b.com)"), "P([https://a.com](https://b.com))", "no link in a label");
        assert_eq!(p("https://example.com/#top"), "P(<https://example.com/#top>)", "no hashtag inside a link");
    }

    #[test]
    fn mentions_hashtags_and_emoji_follow_the_text_rules() {
        assert_eq!(p("@taro_1 さん #春 :blobcat:"), "P({@taro_1} さん {#春|春} {:blobcat:})");
        assert_eq!(p("mail@example.com C#"), "P(mail@example.com C#)");
        assert_eq!(p("友達の@hanako"), "P(友達の{@hanako})");
        assert_eq!(p("#ＡＢＣ"), "P({#abc|ＡＢＣ})");
        assert_eq!(
            p("10:30:00"),
            "P(10{:30:}00)",
            "digits make a shortcode too; it renders as text when no emoji exists"
        );
        assert_eq!(p("a :: b"), "P(a :: b)");
    }

    #[test]
    fn effects_are_the_safe_set_only() {
        assert_eq!(p("$[tada やった!]"), "P($[1 やった!])");
        assert_eq!(p("$[x2 $[rainbow 大きい]]"), "P($[9 $[7 大きい]])");
        assert_eq!(p("$[position.x=100 ずれる]"), "P($[position.x=100 ずれる])");
        assert_eq!(p("$[blur 見えない]"), "P($[blur 見えない])");
        assert_eq!(p("$[spin]"), "P($[spin])");
        assert_eq!(p("$[shake [括弧] 入り]"), "P($[5 [括弧] 入り])");
    }

    #[test]
    fn nesting_stops_at_the_limit() {
        let deep = format!("{}x{}", "$[tada ".repeat(MAX_DEPTH + 2), "]".repeat(MAX_DEPTH + 2));
        let shown = p(&deep);
        assert_eq!(shown.matches("$[1 ").count(), MAX_DEPTH);
        let mut depth = 0;
        let body = parse(&deep);
        for_each_inline(&mut body.clone(), &mut |node| {
            if matches!(node.inline, Some(Inline::Effect(_))) {
                depth += 1;
            }
        });
        assert_eq!(depth, MAX_DEPTH);
    }

    #[test]
    fn facts_come_from_the_tree() {
        let body = parse("**#一** @alice `#code` [見る](https://a.example/x) https://b.example :wave: #一 #二 @alice");
        let facts = facts(&body);
        assert_eq!(facts.hashtags, vec!["一", "二"]);
        assert_eq!(facts.handles, vec!["alice"]);
        assert_eq!(facts.first_url.as_deref(), Some("https://a.example/x"));
        assert_eq!(facts.emoji, vec!["wave"]);
        let many: String = (0..20).map(|i| format!("#tag{i} ")).collect();
        assert_eq!(super::facts(&parse(&many)).hashtags.len(), text::MAX_HASHTAGS);
    }

    #[test]
    fn any_input_parses_without_panicking() {
        for input in [
            "\\",
            "*",
            "**",
            "~~~",
            "[",
            "[](",
            "[a](",
            "$[",
            "$[tada",
            "$[tada ",
            ":",
            "#",
            "@",
            "`",
            "```",
            ">",
            "<center>",
            "</center>",
            "<b>",
            "<b></b>",
            "a\\",
            "\\\\",
            "*a\\*",
            "$[tada \\]",
            "＃",
            "＠",
        ] {
            let _ = parse(input);
            let _ = parse(&input.repeat(50));
        }
    }
}
