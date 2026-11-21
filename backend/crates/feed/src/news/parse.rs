//! From a downloaded feed document to articles.
//!
//! `feed-rs` does the format work (RSS 0.9x/1.0/2.0, Atom, JSON Feed, the
//! MediaRSS and Dublin Core extensions, character sets); this module decides
//! what an article *is* for us: a title, a link, a plain-text excerpt, an
//! image taken from the feed itself, a publication time and the feed's own
//! categories. Nothing beyond the feed document is ever fetched.
use chrono::Duration;
use feed_rs::model::{Entry, Feed as ParsedDocument, Link};
use tokuly_core::prelude::*;
use url::Url;

use super::{
    text::{collapse, first_image, html_to_text, truncate},
    thumbnails::item_thumbnails,
};

/// Entries read from one document (the newest ones).
pub const MAX_ENTRIES: usize = 200;
pub const MAX_TITLE_CHARS: usize = 300;
pub const MAX_SUMMARY_CHARS: usize = 500;
/// Longest URL kept (article links, images); also bounds the unique index.
pub const MAX_URL_LEN: usize = 2000;
/// Categories taken from one entry.
const MAX_ENTRY_CATEGORIES: usize = 8;
/// How far in the future a publication time may claim to be.
const MAX_CLOCK_SKEW_HOURS: i64 = 24;

/// Query parameters that only track where a click came from.
const TRACKING_PARAMS: &[&str] = &[
    "fbclid", "gclid", "dclid", "gbraid", "wbraid", "msclkid", "yclid", "igshid", "mc_cid", "mc_eid", "_ga", "_gl",
    "ref_src", "ref_url", "cmpid",
];

/// What a feed says about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedInfo {
    /// The feed's title as plain text; may be empty.
    pub title: String,
    /// The site the feed belongs to; may be empty.
    pub site_url: String,
    /// The site's icon: the feed's own, else `/favicon.ico` of the site.
    pub favicon_url: String,
}

/// One article of a feed.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedArticle {
    /// The link without fragment and tracking parameters: the article's identity.
    pub canonical_url: String,
    /// The link as the feed gave it (made absolute).
    pub url: String,
    pub title: String,
    pub summary: String,
    pub image_url: String,
    /// Categories of the entry as written in the feed.
    pub categories: Vec<String>,
    /// `None` when the entry carries no (usable) date.
    pub published_at: Option<Time>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedFeed {
    pub info: FeedInfo,
    pub articles: Vec<ParsedArticle>,
}

/// Parses a feed document fetched from `document_url`.
pub fn parse_feed(body: &[u8], document_url: &Url, now: Time) -> Result<ParsedFeed, String> {
    let document = feed_rs::parser::Builder::new()
        .base_uri(Some(document_url.as_str()))
        .build()
        .parse(body)
        .map_err(|e| e.to_string())?;
    // Thumbnails the parser does not report, by item position. Only usable
    // when both passes saw the same items.
    let thumbnails =
        item_thumbnails(body).filter(|thumbnails| thumbnails.len() == document.entries.len()).unwrap_or_default();
    Ok(convert(document, thumbnails, document_url, now))
}

fn convert(document: ParsedDocument, thumbnails: Vec<Option<String>>, document_url: &Url, now: Time) -> ParsedFeed {
    let site = site_link(&document.links, document_url);
    let favicon = document
        .icon
        .as_ref()
        .and_then(|icon| web_url(&icon.uri, document_url))
        .map(String::from)
        .or_else(|| {
            site.as_ref().or(Some(document_url)).and_then(|origin| origin.join("/favicon.ico").ok()).map(String::from)
        })
        .unwrap_or_default();
    let info = FeedInfo {
        title: truncate(&document.title.as_ref().map(|title| html_to_text(&title.content)).unwrap_or_default(), 100),
        site_url: site.as_ref().map(|url| url.to_string()).unwrap_or_default(),
        favicon_url: favicon,
    };

    let base = site.as_ref().unwrap_or(document_url);
    let mut articles: Vec<ParsedArticle> = Vec::new();
    for (position, entry) in document.entries.into_iter().enumerate() {
        let thumbnail = thumbnails.get(position).and_then(Option::as_deref);
        if let Some(article) = convert_entry(entry, thumbnail, base, now)
            // A feed listing the same article twice gives one article.
            && !articles.iter().any(|known| known.canonical_url == article.canonical_url)
        {
            articles.push(article);
        }
    }
    // Newest first (undated entries keep the feed's order, after dated ones
    // of the same position), then the cap.
    articles.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    articles.truncate(MAX_ENTRIES);
    ParsedFeed { info, articles }
}

/// The site a feed belongs to: its `alternate` link (never the feed itself).
fn site_link(links: &[Link], document_url: &Url) -> Option<Url> {
    let candidate = |link: &&Link| {
        link.rel.as_deref().is_none_or(|rel| rel == "alternate")
            && !link.media_type.as_deref().is_some_and(is_feed_type)
    };
    links.iter().filter(candidate).filter_map(|link| web_url(&link.href, document_url)).find(|url| url != document_url)
}

fn is_feed_type(media_type: &str) -> bool {
    let media_type = media_type.to_ascii_lowercase();
    media_type.contains("rss") || media_type.contains("atom") || media_type.contains("feed+json")
}

/// An absolute http(s) URL of bounded length, resolved against `base`.
fn web_url(raw: &str, base: &Url) -> Option<Url> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let url = Url::parse(raw).or_else(|_| base.join(raw)).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host().is_some() && url.as_str().len() <= MAX_URL_LEN)
        .then_some(url)
}

fn convert_entry(entry: Entry, thumbnail: Option<&str>, base: &Url, now: Time) -> Option<ParsedArticle> {
    let link = article_link(&entry.links, base)?;
    let title = truncate(&html_to_text(&entry.title.as_ref()?.content), MAX_TITLE_CHARS);
    if title.is_empty() {
        return None;
    }
    let summary_html =
        entry.summary.as_ref().map(|summary| summary.content.as_str()).filter(|text| !text.trim().is_empty());
    let content_html =
        entry.content.as_ref().and_then(|content| content.body.as_deref()).filter(|text| !text.trim().is_empty());
    let summary = truncate(&html_to_text(summary_html.or(content_html).unwrap_or_default()), MAX_SUMMARY_CHARS);
    let image_url =
        entry_image(&entry, thumbnail, content_html.or(summary_html), base).map(String::from).unwrap_or_default();
    let categories = entry
        .categories
        .iter()
        .map(|category| {
            collapse(category.label.as_deref().filter(|label| !label.trim().is_empty()).unwrap_or(&category.term))
        })
        .filter(|category| !category.is_empty())
        .take(MAX_ENTRY_CATEGORIES)
        .collect();
    // A date far in the future is a broken clock, not news from tomorrow.
    let published_at = entry
        .published
        .or(entry.updated)
        .map(|at| if at > now + Duration::hours(MAX_CLOCK_SKEW_HOURS) { now } else { at.min(now) });
    Some(ParsedArticle {
        canonical_url: canonical_url(&link),
        url: link.into(),
        title,
        summary,
        image_url,
        categories,
        published_at,
    })
}

/// The page an entry stands for: its `alternate` link, else its first link.
fn article_link(links: &[Link], base: &Url) -> Option<Url> {
    let usable = |link: &Link| web_url(&link.href, base);
    links.iter().filter(|link| link.rel.as_deref().is_none_or(|rel| rel == "alternate")).find_map(usable).or_else(
        || {
            links
                .iter()
                .filter(|link| !matches!(link.rel.as_deref(), Some("enclosure" | "self" | "replies" | "edit")))
                .find_map(usable)
        },
    )
}

fn is_image_type(media_type: &str) -> bool {
    media_type.trim().to_ascii_lowercase().starts_with("image/")
}

fn looks_like_image(url: &Url) -> bool {
    let path = url.path().to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".webp", ".gif", ".avif"].iter().any(|extension| path.ends_with(extension))
}

/// The lead image of an entry, from the feed alone: MediaRSS content and
/// thumbnails (`thumbnail` is the one found by the extra pass of
/// [`item_thumbnails`]), RSS enclosures, Atom enclosure links, and as a last
/// resort the first image of the entry's own markup.
fn entry_image(entry: &Entry, thumbnail: Option<&str>, html: Option<&str>, base: &Url) -> Option<Url> {
    let media_content = entry.media.iter().flat_map(|media| media.content.iter()).find_map(|content| {
        let url = content.url.as_ref()?;
        let is_image = match &content.content_type {
            Some(content_type) => is_image_type(&content_type.to_string()),
            None => looks_like_image(url),
        };
        is_image.then(|| web_url(url.as_str(), base)).flatten()
    });
    let media_thumbnail = || {
        entry
            .media
            .iter()
            .flat_map(|media| media.thumbnails.iter())
            .find_map(|thumbnail| web_url(&thumbnail.image.uri, base))
    };
    let item_thumbnail = || thumbnail.and_then(|url| web_url(url, base));
    let enclosure_link = || {
        entry
            .links
            .iter()
            .filter(|link| {
                link.rel.as_deref() == Some("enclosure") && link.media_type.as_deref().is_some_and(is_image_type)
            })
            .find_map(|link| web_url(&link.href, base))
    };
    let inline = || html.and_then(first_image).and_then(|src| web_url(&src, base));
    media_content.or_else(media_thumbnail).or_else(item_thumbnail).or_else(enclosure_link).or_else(inline)
}

/// The identity of an article: its link without fragment and tracking
/// parameters. Two feeds (or one feed, twice) carrying the same article give
/// the same canonical URL.
pub fn canonical_url(url: &Url) -> String {
    let mut canonical = url.clone();
    canonical.set_fragment(None);
    if let Some(query) = url.query() {
        // Work on the raw segments so that everything kept stays byte-identical.
        let kept: Vec<&str> =
            query.split('&').filter(|segment| !segment.is_empty() && !is_tracking_param(segment)).collect();
        let kept = kept.join("&");
        canonical.set_query((!kept.is_empty()).then_some(kept.as_str()));
    }
    canonical.into()
}

fn is_tracking_param(segment: &str) -> bool {
    let key = segment.split('=').next().unwrap_or_default().to_ascii_lowercase();
    key.starts_with("utm_") || TRACKING_PARAMS.contains(&key.as_str())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    fn at(year: i32, month: u32, day: u32, hour: u32) -> Time {
        chrono::Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
    }

    fn parse(xml: &str) -> ParsedFeed {
        parse_feed(xml.as_bytes(), &url("https://news.example/feed.xml"), at(2026, 11, 20, 12))
            .expect("the feed parses")
    }

    const RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/" xmlns:content="http://purl.org/rss/1.0/modules/content/" xmlns:dc="http://purl.org/dc/elements/1.1/">
  <channel>
    <title>Example &amp; News</title>
    <link>https://news.example/</link>
    <description>All the news</description>
    <item>
      <title>&lt;b&gt;Big&lt;/b&gt; story</title>
      <link>https://news.example/articles/1?utm_source=rss&amp;id=7#top</link>
      <description>&lt;p&gt;First &lt;a href="https://x.example"&gt;paragraph&lt;/a&gt;.&lt;/p&gt;&lt;script&gt;alert(1)&lt;/script&gt;</description>
      <category>Tech</category>
      <category>ゲーム</category>
      <pubDate>Thu, 19 Nov 2026 09:30:00 +0900</pubDate>
      <enclosure url="https://cdn.example/1.jpg" type="image/jpeg" length="1000"/>
    </item>
    <item>
      <title>Second story</title>
      <link>/articles/2</link>
      <content:encoded><![CDATA[<p>Body with <img src="/img/2.png" alt=""> picture</p>]]></content:encoded>
      <dc:date>2026-11-18T00:00:00Z</dc:date>
    </item>
    <item>
      <title>Media thumbnail</title>
      <link>https://news.example/articles/3</link>
      <media:thumbnail url="https://cdn.example/3-thumb.jpg"/>
      <enclosure url="https://cdn.example/3.mp3" type="audio/mpeg" length="5"/>
    </item>
    <item>
      <title>   </title>
      <link>https://news.example/articles/untitled</link>
    </item>
    <item>
      <title>No link</title>
    </item>
    <item>
      <title>Not on the web</title>
      <link>javascript:alert(1)</link>
    </item>
    <item>
      <title>Big story again</title>
      <link>https://news.example/articles/1?id=7&amp;utm_medium=feed</link>
    </item>
  </channel>
</rss>"#;

    #[test]
    fn rss_items_become_articles() {
        let feed = parse(RSS);
        assert_eq!(
            feed.info,
            FeedInfo {
                title: "Example & News".into(),
                site_url: "https://news.example/".into(),
                favicon_url: "https://news.example/favicon.ico".into()
            }
        );
        assert_eq!(feed.articles.len(), 3, "untitled, unlinked, non-web and duplicate items are skipped");

        let first = &feed.articles[0];
        assert_eq!(first.title, "Big story");
        assert_eq!(first.url, "https://news.example/articles/1?utm_source=rss&id=7#top");
        assert_eq!(first.canonical_url, "https://news.example/articles/1?id=7");
        assert_eq!(first.summary, "First paragraph.");
        assert_eq!(first.image_url, "https://cdn.example/1.jpg");
        assert_eq!(first.categories, vec!["Tech", "ゲーム"]);
        assert_eq!(first.published_at, Some(at(2026, 11, 19, 0) + Duration::minutes(30)));

        let second = &feed.articles[1];
        assert_eq!(second.url, "https://news.example/articles/2", "relative links are resolved");
        assert_eq!(second.summary, "Body with picture");
        assert_eq!(second.image_url, "https://news.example/img/2.png", "the first image of the body, resolved");
        assert_eq!(second.published_at, Some(at(2026, 11, 18, 0)));

        let third = &feed.articles[2];
        assert_eq!(third.image_url, "https://cdn.example/3-thumb.jpg", "an audio enclosure is not an image");
        assert_eq!((third.summary.as_str(), third.published_at), ("", None));
    }

    const ATOM: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Atom ブログ</title>
  <link rel="self" type="application/atom+xml" href="https://blog.example/atom.xml"/>
  <link rel="alternate" type="text/html" href="https://blog.example/"/>
  <icon>https://blog.example/icon.png</icon>
  <updated>2026-11-20T00:00:00Z</updated>
  <entry>
    <title type="html">新しい &lt;em&gt;記事&lt;/em&gt;</title>
    <link rel="replies" href="https://blog.example/posts/1/comments"/>
    <link rel="alternate" href="https://blog.example/posts/1"/>
    <link rel="enclosure" type="image/png" href="https://blog.example/posts/1/cover.png"/>
    <id>tag:blog.example,2026:1</id>
    <updated>2026-11-19T10:00:00Z</updated>
    <summary type="html">&lt;p&gt;要約です&lt;/p&gt;</summary>
    <category term="diary" label="日記"/>
    <category term="life"/>
  </entry>
  <entry>
    <title>From the future</title>
    <link href="https://blog.example/posts/2"/>
    <id>tag:blog.example,2026:2</id>
    <updated>2031-01-01T00:00:00Z</updated>
  </entry>
</feed>"#;

    #[test]
    fn atom_entries_become_articles() {
        let feed = parse_feed(ATOM.as_bytes(), &url("https://blog.example/atom.xml"), at(2026, 11, 20, 12)).unwrap();
        assert_eq!(
            feed.info,
            FeedInfo {
                title: "Atom ブログ".into(),
                site_url: "https://blog.example/".into(),
                favicon_url: "https://blog.example/icon.png".into()
            }
        );
        assert_eq!(feed.articles.len(), 2);

        // The entry with the broken clock is "now", which sorts first.
        let future = &feed.articles[0];
        assert_eq!((future.title.as_str(), future.published_at), ("From the future", Some(at(2026, 11, 20, 12))));

        let entry = &feed.articles[1];
        assert_eq!(entry.title, "新しい 記事");
        assert_eq!(entry.url, "https://blog.example/posts/1", "the alternate link, not the comments link");
        assert_eq!(entry.summary, "要約です");
        assert_eq!(entry.image_url, "https://blog.example/posts/1/cover.png");
        assert_eq!(entry.categories, vec!["日記", "life"]);
        assert_eq!(entry.published_at, Some(at(2026, 11, 19, 10)));
    }

    #[test]
    fn malformed_documents_are_errors() {
        let base = url("https://news.example/feed.xml");
        let now = at(2026, 11, 20, 12);
        for body in [
            "",
            "not a feed at all",
            "<html><body>404</body></html>",
            "<rss><channel><title>cut off",
            "{\"version\": 42}",
        ] {
            assert!(parse_feed(body.as_bytes(), &base, now).is_err(), "{body:?}");
        }
    }

    #[test]
    fn an_empty_feed_is_fine() {
        let feed = parse(
            r#"<rss version="2.0"><channel><title>Quiet</title><link>https://quiet.example</link></channel></rss>"#,
        );
        assert!(feed.articles.is_empty());
        assert_eq!(feed.info.title, "Quiet");
        assert_eq!(feed.info.favicon_url, "https://quiet.example/favicon.ico");
    }

    #[test]
    fn a_feed_without_a_site_link_uses_its_own_origin() {
        let feed = parse(
            r#"<rss version="2.0"><channel><title>Bare</title><item><title>One</title><link>/a</link></item></channel></rss>"#,
        );
        assert_eq!(feed.info.site_url, "");
        assert_eq!(feed.info.favicon_url, "https://news.example/favicon.ico");
        assert_eq!(feed.articles[0].url, "https://news.example/a");
    }

    #[test]
    fn long_texts_are_cut_and_entries_capped() {
        let long_title = "t".repeat(MAX_TITLE_CHARS + 50);
        let long_body = "word ".repeat(400);
        let items: String = (0..MAX_ENTRIES + 20)
            .map(|i| format!("<item><title>{long_title}</title><link>https://news.example/a/{i}</link><description>{long_body}</description></item>"))
            .collect();
        let feed = parse(&format!(r#"<rss version="2.0"><channel><title>Big</title>{items}</channel></rss>"#));
        assert_eq!(feed.articles.len(), MAX_ENTRIES);
        let article = &feed.articles[0];
        assert_eq!(article.title.chars().count(), MAX_TITLE_CHARS + 1);
        assert!(article.summary.chars().count() <= MAX_SUMMARY_CHARS + 1 && article.summary.ends_with('…'));
    }

    #[test]
    fn canonical_urls_drop_fragments_and_tracking_parameters() {
        for (given, canonical) in [
            ("https://a.example/x?utm_source=rss&utm_medium=feed", "https://a.example/x"),
            ("https://a.example/x?id=5&fbclid=abc#section", "https://a.example/x?id=5"),
            ("https://a.example/x?UTM_Campaign=1&b=2&a=1", "https://a.example/x?b=2&a=1"),
            ("https://A.Example:443/x", "https://a.example/x"),
            ("https://a.example/x?q=a%20b&gclid=1", "https://a.example/x?q=a%20b"),
            ("https://a.example/x?12345", "https://a.example/x?12345"),
            ("https://a.example/x/?&", "https://a.example/x/"),
            ("https://a.example/x#only-fragment", "https://a.example/x"),
        ] {
            assert_eq!(canonical_url(&url(given)), canonical, "{given}");
        }
    }
}
