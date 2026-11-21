//! Recognising links that clients can show as rich inline embeds:
//! YouTube videos, Spotify content, and pages of this Tokuly instance.
use url::Url;

/// A recognised embeddable link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Embed {
    /// `youtube`, `spotify` or `tokuly`.
    pub provider: &'static str,
    /// Provider-specific ID; see `LinkPreview.embed_id` in the proto.
    pub id: String,
}

impl Embed {
    fn new(provider: &'static str, id: impl Into<String>) -> Self {
        Self { provider, id: id.into() }
    }

    /// The oEmbed endpoint that describes this embed (title, thumbnail),
    /// for providers whose pages are unreliable to scrape.
    pub fn oembed_url(&self, page: &Url) -> Option<Url> {
        let endpoint = match self.provider {
            "youtube" => "https://www.youtube.com/oembed",
            "spotify" => "https://open.spotify.com/oembed",
            _ => return None,
        };
        Url::parse_with_params(endpoint, [("url", page.as_str()), ("format", "json")]).ok()
    }

    /// A thumbnail that exists without asking the provider.
    pub fn fallback_image(&self) -> Option<String> {
        (self.provider == "youtube").then(|| format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", self.id))
    }

    pub fn site_name(&self) -> &'static str {
        match self.provider {
            "youtube" => "YouTube",
            "spotify" => "Spotify",
            _ => "Tokuly",
        }
    }
}

fn is_youtube_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn is_slug(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn youtube(url: &Url, host: &str) -> Option<Embed> {
    let mut segments = url.path_segments()?;
    let first = segments.next().unwrap_or_default();
    let id = match host {
        "youtu.be" => first.to_owned(),
        "youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtube-nocookie.com" => match first {
            "watch" => url.query_pairs().find(|(key, _)| key == "v")?.1.into_owned(),
            "shorts" | "embed" | "live" | "v" => segments.next()?.to_owned(),
            _ => return None,
        },
        _ => return None,
    };
    is_youtube_id(&id).then(|| Embed::new("youtube", id))
}

fn spotify(url: &Url) -> Option<Embed> {
    let mut segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    // Localised links carry a prefix: /intl-ja/track/…
    if segments.first().is_some_and(|s| s.starts_with("intl-")) {
        segments.remove(0);
    }
    let [kind, id, ..] = segments.as_slice() else { return None };
    let known = matches!(*kind, "track" | "album" | "playlist" | "episode" | "show" | "artist");
    (known && id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric())).then(|| Embed::new("spotify", format!("{kind}/{id}")))
}

/// Pages of this instance that have a native card. The first path segment
/// names the kind of content; the second is its ID or handle.
fn tokuly(url: &Url) -> Option<Embed> {
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let (kind, id) = match segments.as_slice() {
        [handle] if handle.starts_with('@') => ("user", &handle[1..]),
        ["video" | "watch" | "v", id, ..] => ("video", *id),
        ["shorts" | "short", id, ..] => ("short", *id),
        ["live", id, ..] => ("live", *id),
        ["clip" | "clips", id, ..] => ("clip", *id),
        ["notes" | "note", id, ..] => ("note", *id),
        ["playlist" | "playlists", id, ..] => ("playlist", *id),
        ["stamps", id, ..] => ("stamps", *id),
        ["u" | "users", handle, ..] => ("user", *handle),
        ["channel" | "c", handle, ..] => ("channel", handle.trim_start_matches('@')),
        _ => return None,
    };
    is_slug(id).then(|| Embed::new("tokuly", format!("{kind}/{id}")))
}

/// Whether `url` points at this instance (same host and port as its public URL).
pub fn is_own(url: &Url, public_url: &str) -> bool {
    Url::parse(public_url).is_ok_and(|own| own.host_str() == url.host_str() && own.port_or_known_default() == url.port_or_known_default())
}

/// The embed a URL stands for, if any. `public_url` is this instance's origin.
pub fn recognise(url: &Url, public_url: &str) -> Option<Embed> {
    if is_own(url, public_url) {
        return tokuly(url);
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    match host {
        "youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtu.be" | "youtube-nocookie.com" => youtube(url, host),
        "open.spotify.com" => spotify(url),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: &str = "https://tokuly.example";

    fn embed(url: &str) -> Option<(&'static str, String)> {
        recognise(&Url::parse(url).unwrap(), OWN).map(|e| (e.provider, e.id))
    }

    #[test]
    fn youtube_links_in_all_their_shapes() {
        let expected = Some(("youtube", "dQw4w9WgXcQ".to_owned()));
        for url in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/watch?feature=share&v=dQw4w9WgXcQ&t=42s",
            "https://m.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ?si=abc",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.youtube.com/embed/dQw4w9WgXcQ",
            "https://www.youtube.com/live/dQw4w9WgXcQ?feature=share",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
        ] {
            assert_eq!(embed(url), expected, "{url}");
        }
        for url in [
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/@channel",
            "https://www.youtube.com/playlist?list=PL123",
            "https://youtu.be/",
            "https://notyoutube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ",
        ] {
            assert_eq!(embed(url), None, "{url}");
        }
    }

    #[test]
    fn spotify_links_keep_their_type() {
        assert_eq!(embed("https://open.spotify.com/track/6rqhFgbbKwnb9MLmUQDhG6?si=x"), Some(("spotify", "track/6rqhFgbbKwnb9MLmUQDhG6".into())));
        assert_eq!(embed("https://open.spotify.com/intl-ja/album/1DFixLWuPkv3KT3TnV35m3"), Some(("spotify", "album/1DFixLWuPkv3KT3TnV35m3".into())));
        assert_eq!(embed("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M"), Some(("spotify", "playlist/37i9dQZF1DXcBWIGoYBM5M".into())));
        assert_eq!(embed("https://open.spotify.com/user/someone"), None);
        assert_eq!(embed("https://open.spotify.com/track/tooshort"), None);
        assert_eq!(embed("https://spotify.com/track/6rqhFgbbKwnb9MLmUQDhG6"), None);
    }

    #[test]
    fn own_pages_become_native_cards() {
        assert_eq!(embed("https://tokuly.example/video/dQw4w9WgXcQ?t=10"), Some(("tokuly", "video/dQw4w9WgXcQ".into())));
        assert_eq!(embed("https://tokuly.example/shorts/abc123DEF45"), Some(("tokuly", "short/abc123DEF45".into())));
        assert_eq!(embed("https://tokuly.example/live/0190a1b2-0000-7000-8000-000000000001"), Some(("tokuly", "live/0190a1b2-0000-7000-8000-000000000001".into())));
        assert_eq!(embed("https://tokuly.example/@tokuzou"), Some(("tokuly", "user/tokuzou".into())));
        assert_eq!(embed("https://tokuly.example/channel/@games"), Some(("tokuly", "channel/games".into())));
        assert_eq!(embed("https://tokuly.example/notes/abc/replies"), Some(("tokuly", "note/abc".into())));
        assert_eq!(embed("https://tokuly.example/settings"), None);
        assert_eq!(embed("https://tokuly.example/"), None);
        assert_eq!(embed("https://tokuly.example/video/%3Cscript%3E"), None);
        // Same host on another port is another site.
        assert_eq!(embed("https://tokuly.example:8443/video/abc"), None);
        assert!(is_own(&Url::parse("https://tokuly.example:443/x").unwrap(), OWN));
        assert!(!is_own(&Url::parse("http://tokuly.example/x").unwrap(), OWN), "http is port 80");
    }

    #[test]
    fn oembed_endpoints() {
        let page = Url::parse("https://www.youtube.com/watch?v=dQw4w9WgXcQ").unwrap();
        let embed = recognise(&page, OWN).unwrap();
        assert_eq!(
            embed.oembed_url(&page).unwrap().as_str(),
            "https://www.youtube.com/oembed?url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3DdQw4w9WgXcQ&format=json"
        );
        assert_eq!(embed.fallback_image().as_deref(), Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg"));
        assert_eq!(Embed { provider: "tokuly", id: "video/x".into() }.oembed_url(&page), None);
    }
}
