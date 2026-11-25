//! What a push shows on the lock screen (「通知に内容を表示」).
//!
//! The recipient chooses one of three [`PushPreview`] levels. Anything a
//! level leaves out is taken out of the [`PushMessage`] before any channel
//! builds its payload, so it never reaches Web Push, FCM or APNs at all,
//! instead of being hidden by the app. The inbox keeps the full entry.
use tokuly_core::outbound::Localized;
use tokuly_proto::notify::PushPreview;

use crate::{categories, push::PushMessage};

/// Where a push that shows nothing leads: the inbox, which has the details.
pub const INBOX_PATH: &str = "/notifications";
/// The title of a push that shows nothing.
const APP_NAME: &str = "Tokuly";
/// What a push that shows nothing says it is: not which feature (its kind
/// and category would), and the one Android channel such pushes share.
pub const HIDDEN_KIND: &str = "notification";
pub const HIDDEN_CATEGORY: &str = "general";
/// Pushes that show nothing replace one another, and their tag names no
/// one (a producer's key can hold a person's ID: `social.buzz:<peer>`).
pub const HIDDEN_COLLAPSE_KEY: &str = "notification";

/// Rewrites `message` (already written in its device's language) to show
/// only what `preview` allows. `actor_name` is the display name of who
/// caused the notification, when it has one the recipient may see (producers
/// leave the actor out of anonymous notifications). `inbox_url` is the web
/// URL of [`INBOX_PATH`].
pub fn apply(message: &mut PushMessage, preview: PushPreview, actor_name: Option<&str>, inbox_url: &str) {
    match preview {
        PushPreview::PUSH_PREVIEW_UNSPECIFIED | PushPreview::PUSH_PREVIEW_FULL => {}
        PushPreview::PUSH_PREVIEW_SENDER_ONLY => {
            let (title, body) = sender_only(&message.category, actor_name);
            message.title = title.pick(&message.locale).to_owned();
            message.body = body.pick(&message.locale).to_owned();
            strip_content(message);
        }
        PushPreview::PUSH_PREVIEW_HIDDEN => {
            message.title = APP_NAME.to_owned();
            message.body = Localized::new("新しいお知らせがあるよ", "You have a new notification").pick(&message.locale).to_owned();
            strip_content(message);
            // The path can name a person or a conversation (`/@aoi`, `/chat/42`).
            message.path = INBOX_PATH.to_owned();
            message.url = inbox_url.to_owned();
            message.kind = HIDDEN_KIND.to_owned();
            message.category = HIDDEN_CATEGORY.to_owned();
            message.collapse_key = Some(HIDDEN_COLLAPSE_KEY.to_owned());
            message.notification_id = None;
        }
    }
}

/// Drops what may quote the content: the picture (a photo, a thumbnail of
/// the video) and the producer's extra data (excerpts, titles).
fn strip_content(message: &mut PushMessage) {
    message.image_url = None;
    message.data = serde_json::Value::Null;
}

/// Title and text of a sender-only push: the feature as the title, who it is
/// from as the text.
fn sender_only(category_id: &str, actor_name: Option<&str>) -> (Localized, Localized) {
    let category = categories::find(category_id).unwrap_or_else(|| categories::of_kind(category_id));
    let label = category.label();
    let chat = category.id == "chat";
    let body = match (actor_name.map(str::trim).filter(|name| !name.is_empty()), chat) {
        (Some(name), true) => Localized::new(format!("{name}さんからメッセージが届いたよ"), format!("New message from {name}")),
        (Some(name), false) => Localized::new(
            format!("{name}さんから{}のお知らせが届いたよ", label.ja),
            format!("New notification from {name} ({})", label.en),
        ),
        (None, true) => Localized::new("新しいメッセージが届いたよ", "You have a new message"),
        (None, false) => Localized::new(format!("{}のお知らせが届いたよ", label.ja), format!("New notification ({})", label.en)),
    };
    (label, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push::sample_message;

    fn shown(preview: PushPreview, actor: Option<&str>, locale: &str) -> PushMessage {
        let mut message = PushMessage { locale: locale.into(), ..sample_message() };
        apply(&mut message, preview, actor, "https://tokuly.example/notifications");
        message
    }

    #[test]
    fn full_keeps_everything() {
        let message = shown(PushPreview::PUSH_PREVIEW_FULL, Some("あおい"), "ja");
        let sample = sample_message();
        assert_eq!((message.title, message.body, message.image_url), (sample.title, sample.body, sample.image_url));
        assert_eq!(message.data, sample.data);
    }

    #[test]
    fn sender_only_names_the_sender_and_the_feature_but_not_the_text() {
        let message = shown(PushPreview::PUSH_PREVIEW_SENDER_ONLY, Some("あおい"), "ja");
        assert_eq!(message.title, "チャット・通話");
        assert_eq!(message.body, "あおいさんからメッセージが届いたよ");
        assert_eq!((message.image_url, message.data), (None, serde_json::Value::Null));
        assert_eq!(message.path, "/chat/42", "a tap still opens the chat");

        let message = shown(PushPreview::PUSH_PREVIEW_SENDER_ONLY, Some("Aoi"), "en");
        assert_eq!(message.body, "New message from Aoi");

        let mut video = PushMessage { category: "video".into(), ..sample_message() };
        apply(&mut video, PushPreview::PUSH_PREVIEW_SENDER_ONLY, Some("あおい"), "");
        assert_eq!((video.title.as_str(), video.body.as_str()), ("動画", "あおいさんから動画のお知らせが届いたよ"));

        // No one to name (a system notice, an anonymous question).
        let message = shown(PushPreview::PUSH_PREVIEW_SENDER_ONLY, None, "ja");
        assert_eq!(message.body, "新しいメッセージが届いたよ");
    }

    #[test]
    fn hidden_says_nothing_about_who_or_what() {
        let message = shown(PushPreview::PUSH_PREVIEW_HIDDEN, Some("あおい"), "ja");
        assert_eq!((message.title.as_str(), message.body.as_str()), ("Tokuly", "新しいお知らせがあるよ"));
        assert_eq!((message.path.as_str(), message.url.as_str()), (INBOX_PATH, "https://tokuly.example/notifications"));
        assert_eq!((message.image_url, message.data), (None, serde_json::Value::Null));
        assert_eq!(shown(PushPreview::PUSH_PREVIEW_HIDDEN, None, "en").body, "You have a new notification");
        // Not even which feature, nor anything with an ID in it.
        assert_eq!((message.kind.as_str(), message.category.as_str()), (HIDDEN_KIND, HIDDEN_CATEGORY));
        assert_eq!(message.collapse_key.as_deref(), Some(HIDDEN_COLLAPSE_KEY));
        assert_eq!(message.notification_id, None);
    }

    #[test]
    fn native_payloads_carry_none_of_the_hidden_text() {
        use crate::push::{Trim, apns, fcm};
        let secret_text = "やっほー"; // sample_message's body
        for preview in [PushPreview::PUSH_PREVIEW_SENDER_ONLY, PushPreview::PUSH_PREVIEW_HIDDEN] {
            let message = shown(preview, Some("あおい"), "ja");
            for payload in [fcm::request_body("token", &message, Trim::NONE), apns::payload(&message, Trim::NONE)] {
                let text = payload.to_string();
                assert!(!text.contains(secret_text), "{preview:?}: {text}");
                assert!(!text.contains("cdn.example"), "no picture: {text}");
                assert!(!text.contains("conversation_id"), "no extra data: {text}");
                assert_eq!(text.contains("あおい"), preview == PushPreview::PUSH_PREVIEW_SENDER_ONLY, "{text}");
                if preview == PushPreview::PUSH_PREVIEW_HIDDEN {
                    for revealing in ["chat", "00000000-0000-0000-0000-000000000000", &crate::push::collapse_id("chat:42")] {
                        assert!(!text.contains(revealing), "{revealing:?} in {text}");
                    }
                }
            }
        }
    }
}
