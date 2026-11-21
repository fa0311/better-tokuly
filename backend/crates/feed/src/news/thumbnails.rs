//! Thumbnails of feed items that `feed-rs` does not report.
//!
//! Many feeds give each item just `<media:thumbnail url="..."/>`, with no
//! `<media:content>` next to it. `feed-rs` (3.0) only keeps MediaRSS data
//! that hangs off a `media:content`, so those thumbnails are lost and the
//! articles would have no image. This module makes one cheap extra pass over
//! the same document and returns, for every item in document order, the URL
//! of its first thumbnail; the caller pairs them with the parsed entries by
//! position.
use quick_xml::{Reader, encoding::DecodingReader, events::Event};

fn is_item(local_name: &str) -> bool {
    // RSS (every version) calls them items, Atom entries.
    local_name == "item" || local_name == "entry"
}

/// For each item or entry of an XML feed, in document order: the `url` of
/// its first `thumbnail` element, if it has one. `None` when the document is
/// not XML or cannot be read to the end (the caller then goes without).
pub fn item_thumbnails(body: &[u8]) -> Option<Vec<Option<String>>> {
    if body.iter().find(|byte| !byte.is_ascii_whitespace() && **byte != 0xEF && **byte != 0xBB && **byte != 0xBF)
        != Some(&b'<')
    {
        return None;
    }
    let mut reader = Reader::from_reader(DecodingReader::new(body));
    reader.config_mut().expand_empty_elements = true;

    let mut buffer = Vec::new();
    let mut items: Vec<Option<String>> = Vec::new();
    // How deep inside the current item we are; `None` outside of items.
    let mut depth: Option<usize> = None;
    loop {
        match reader.read_event_into(&mut buffer).ok()? {
            Event::Eof => break,
            Event::Decl(declaration) => {
                if let Some(encoding) = declaration.encoder() {
                    reader.get_mut().set_encoding(encoding);
                }
            }
            Event::Start(start) => {
                let name = start.local_name();
                match depth.as_mut() {
                    None if is_item(name.as_ref()) => {
                        items.push(None);
                        depth = Some(0);
                    }
                    None => {}
                    Some(depth) => {
                        *depth += 1;
                        if name.as_ref() == "thumbnail"
                            && let Some(slot) = items.last_mut()
                            && slot.is_none()
                        {
                            *slot = start
                                .attributes()
                                .flatten()
                                .find(|attribute| attribute.key.local_name().as_ref() == "url")
                                .and_then(|attribute| {
                                    let value = quick_xml::escape::unescape(&attribute.value)
                                        .map(|value| value.into_owned())
                                        .unwrap_or_else(|_| attribute.value.to_string());
                                    let value = value.trim().to_owned();
                                    (!value.is_empty()).then_some(value)
                                });
                        }
                    }
                }
            }
            Event::End(_) => match depth.as_mut() {
                Some(0) => depth = None,
                Some(depth) => *depth -= 1,
                None => {}
            },
            _ => {}
        }
        buffer.clear();
    }
    Some(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_are_listed_per_item_in_document_order() {
        let rss = r#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/">
  <channel>
    <title>T</title>
    <image><url>https://news.example/logo.png</url></image>
    <item><title>One</title><media:thumbnail width="240" url="https://cdn.example/1.jpg?a=1&amp;b=2"/></item>
    <item><title>Two</title></item>
    <item>
      <title>Three</title>
      <media:group><media:thumbnail url=" https://cdn.example/3a.jpg "></media:thumbnail><media:thumbnail url="https://cdn.example/3b.jpg"/></media:group>
    </item>
    <item><title>Four</title><media:thumbnail/></item>
  </channel>
</rss>"#;
        assert_eq!(
            item_thumbnails(rss.as_bytes()),
            Some(vec![
                Some("https://cdn.example/1.jpg?a=1&b=2".to_owned()),
                None,
                Some("https://cdn.example/3a.jpg".to_owned()),
                None
            ])
        );
    }

    #[test]
    fn atom_entries_count_as_items() {
        let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:media="http://search.yahoo.com/mrss/">
  <media:thumbnail url="https://cdn.example/feed-level.jpg"/>
  <entry><title>A</title><media:thumbnail url="https://cdn.example/a.jpg"/></entry>
  <entry><title>B</title></entry>
</feed>"#;
        assert_eq!(
            item_thumbnails(atom.as_bytes()),
            Some(vec![Some("https://cdn.example/a.jpg".to_owned()), None]),
            "a thumbnail outside of entries belongs to nobody"
        );
    }

    #[test]
    fn documents_that_are_not_readable_xml_give_nothing() {
        assert_eq!(item_thumbnails(b""), None);
        assert_eq!(item_thumbnails(b"{\"version\": \"https://jsonfeed.org/version/1.1\"}"), None);
        assert_eq!(item_thumbnails(b"plain text"), None);
        assert_eq!(item_thumbnails(b"<rss><channel></channel></rss>"), Some(Vec::new()));
    }

    #[test]
    fn other_encodings_are_decoded() {
        // A Shift_JIS document with a non-ASCII thumbnail URL ("日本" is 0x93 0xFA 0x96 0x7B).
        let mut document = b"<?xml version=\"1.0\" encoding=\"Shift_JIS\"?><rss xmlns:media=\"http://search.yahoo.com/mrss/\"><channel><item><media:thumbnail url=\"https://cdn.example/".to_vec();
        document.extend_from_slice(&[0x93, 0xFA, 0x96, 0x7B]);
        document.extend_from_slice(b".jpg\"/></item></channel></rss>");
        assert_eq!(item_thumbnails(&document), Some(vec![Some("https://cdn.example/日本.jpg".to_owned())]));
    }
}
