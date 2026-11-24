//! Shaping requests from other domains before they are stored or sent.
//!
//! A `NotificationRequest` is written by whichever domain wants to notify
//! someone. The inbox and the push channels rely on a few properties (a
//! well-formed kind, a title in both languages, an in-app path, bounded
//! sizes); [`sanitize`] establishes them in one place.
use sha2::{Digest as _, Sha256};
use tokuly_core::outbound::{Localized, NotificationRequest};

use crate::push::truncate_chars;

const MAX_KIND_LEN: usize = 64;
const MAX_TITLE_CHARS: usize = 200;
const MAX_BODY_CHARS: usize = 2000;
const MAX_PATH_LEN: usize = 512;
const MAX_IMAGE_URL_LEN: usize = 2048;
const MAX_COLLAPSE_KEY_LEN: usize = 200;
const MAX_DATA_BYTES: usize = 8 * 1024;

/// Brings a request from another domain into the shape the inbox and the
/// push channels rely on. `None` when it cannot be shown at all.
pub fn sanitize(mut request: NotificationRequest) -> Option<NotificationRequest> {
    let kind_ok = !request.kind.is_empty()
        && request.kind.len() <= MAX_KIND_LEN
        && request.kind.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
        && !request.kind.starts_with('.')
        && !request.kind.ends_with('.');
    if !kind_ok {
        return None;
    }
    request.title = clean_text(&request.title, MAX_TITLE_CHARS);
    if request.title.ja.is_empty() {
        return None;
    }
    request.body = clean_text(&request.body, MAX_BODY_CHARS);

    // A path we cannot trust to be a route in the app opens the app's start page.
    let path_ok =
        is_app_path(&request.path) && request.path.len() <= MAX_PATH_LEN && !request.path.chars().any(char::is_control);
    if !path_ok {
        request.path = "/".to_owned();
    }
    request.image_url = request.image_url.filter(|url| {
        let web = url.starts_with("https://") || url.starts_with("http://") || is_app_path(url);
        // Browsers drop tabs and newlines from URLs: `/\t/evil` is `//evil`.
        web && url.len() <= MAX_IMAGE_URL_LEN && !url.chars().any(char::is_control)
    });
    request.collapse_key =
        request.collapse_key.map(|key| key.trim().to_owned()).filter(|key| !key.is_empty()).map(|key| {
            // Over-long keys are replaced by their digest: still one key per group.
            if key.len() <= MAX_COLLAPSE_KEY_LEN {
                key
            } else {
                format!("sha256:{}", hex::encode(Sha256::digest(key.as_bytes())))
            }
        });
    if request.data.to_string().len() > MAX_DATA_BYTES {
        request.data = serde_json::Value::Null;
    }
    Some(request)
}

/// A body from another domain's event (an edited comment), bounded like the
/// body of a request.
pub(crate) fn clean_body(body: &Localized) -> Localized {
    clean_text(body, MAX_BODY_CHARS)
}

/// A path on this site. `//host/…` and `/\host/…` begin with a slash too, but
/// browsers read them as another site.
fn is_app_path(path: &str) -> bool {
    path.starts_with('/') && !path.starts_with("//") && !path.starts_with("/\\")
}

/// Trims and bounds both languages, filling a missing one from the other so
/// no reader ever sees an empty text when the other language has one.
fn clean_text(text: &Localized, max_chars: usize) -> Localized {
    let ja = truncate_chars(text.ja.trim(), max_chars);
    let en = truncate_chars(text.en.trim(), max_chars);
    match (ja.is_empty(), en.is_empty()) {
        (true, false) => Localized::same(en),
        (false, true) => Localized::same(ja),
        _ => Localized::new(ja, en),
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn request() -> NotificationRequest {
        NotificationRequest::new(
            Uuid::now_v7(),
            "chat.message",
            Localized::new("ありす", "Alice"),
            Localized::new("やっほー", "Hi"),
            "/chat/1",
        )
    }

    #[test]
    fn malformed_requests_are_rejected() {
        for kind in ["", "Chat.Message", "chat message", ".chat", "chat.", &"a".repeat(65)] {
            let mut bad = request();
            bad.kind = kind.to_owned();
            assert!(sanitize(bad).is_none(), "{kind:?}");
        }
        let mut untitled = request();
        untitled.title = Localized::new("  ", "");
        assert!(sanitize(untitled).is_none());
    }

    #[test]
    fn text_is_trimmed_bounded_and_filled_across_languages() {
        let mut raw = request();
        raw.title = Localized::new("", " Only English ");
        raw.body = Localized::new("あ".repeat(3000), "");
        let clean = sanitize(raw).unwrap();
        assert_eq!((clean.title.ja.as_str(), clean.title.en.as_str()), ("Only English", "Only English"));
        assert_eq!(clean.body.ja.chars().count(), MAX_BODY_CHARS);
        assert!(clean.body.ja.ends_with('…'));
        assert_eq!(clean.body.en, clean.body.ja);
    }

    #[test]
    fn paths_images_keys_and_data_are_made_safe() {
        let mut raw = request();
        raw.path = "https://evil.example/".into();
        raw.image_url = Some("javascript:alert(1)".into());
        raw.collapse_key = Some("  ".into());
        raw.data = serde_json::json!({ "blob": "x".repeat(MAX_DATA_BYTES) });
        let clean = sanitize(raw).unwrap();
        assert_eq!(clean.path, "/");
        assert_eq!(clean.image_url, None);
        assert_eq!(clean.collapse_key, None);
        assert!(clean.data.is_null());

        let mut raw = request();
        raw.image_url = Some("https://cdn.example/a.webp".into());
        raw.collapse_key = Some("k".repeat(500));
        let clean = sanitize(raw).unwrap();
        assert_eq!(clean.path, "/chat/1");
        assert_eq!(clean.image_url.as_deref(), Some("https://cdn.example/a.webp"));
        let key = clean.collapse_key.unwrap();
        assert!(key.starts_with("sha256:") && key.len() == 7 + 64);
    }

    #[test]
    fn protocol_relative_paths_and_images_do_not_leave_the_app() {
        // `//host` and `/\host` start with a slash but are another site to a
        // browser: a tap would open it, an image would be fetched from it.
        for path in ["//evil.example/login", "/\\evil.example/login", "/\t/evil.example/login", "/\n/evil.example"] {
            let mut raw = request();
            raw.path = path.into();
            raw.image_url = Some(path.into());
            let clean = sanitize(raw).unwrap();
            assert_eq!(clean.path, "/", "{path}");
            assert_eq!(clean.image_url, None, "{path}");
        }
        let mut raw = request();
        raw.image_url = Some("/media/a.webp".into());
        assert_eq!(sanitize(raw).unwrap().image_url.as_deref(), Some("/media/a.webp"));
    }
}
