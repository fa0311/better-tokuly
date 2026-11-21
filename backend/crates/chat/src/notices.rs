//! Notification requests of the chat domain and their wording.
//!
//! | Kind | When | Delivery |
//! |---|---|---|
//! | `chat.message` | a message arrived | push only, collapsed per conversation |
//! | `chat.mention` | a message mentioned the recipient | push only, collapsed per conversation |
//! | `chat.request` | a non-friend sent a first message | inbox + push |
//! | `chat.reaction` | someone reacted to the recipient's message | push only, collapsed per message |
//! | `chat.group` | the recipient was added to a group | inbox + push |
//! | `chat.call` | an incoming call (urgent), a missed call | push only while ringing; inbox + push when missed |
//! | `chat.pov` | a POV request, its answer, the answer being saved | inbox + push |
//!
//! Messages are not kept in the notification inbox: the conversation list
//! is their inbox.
use tokuly_core::{
    outbound::{Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    common::{SubjectType, UserSummary},
    media::AssetKind,
};

use crate::{
    content,
    model::{ConversationRow, MessageRow, msg},
};

pub(crate) const KIND_MESSAGE: &str = "chat.message";
pub(crate) const KIND_MENTION: &str = "chat.mention";
pub(crate) const KIND_REQUEST: &str = "chat.request";
pub(crate) const KIND_REACTION: &str = "chat.reaction";
pub(crate) const KIND_GROUP: &str = "chat.group";
pub(crate) const KIND_CALL: &str = "chat.call";
pub(crate) const KIND_POV: &str = "chat.pov";

/// The name a user goes by in notifications.
pub(crate) fn name_of(user: &UserSummary) -> String {
    if user.deleted {
        "退会したユーザー".to_owned()
    } else if user.display_name.is_empty() {
        format!("@{}", user.handle)
    } else {
        user.display_name.clone()
    }
}

pub(crate) fn conversation_path(conversation_id: Uuid) -> String {
    format!("/chat/{conversation_id}")
}

/// Notifications of one conversation replace each other.
pub(crate) fn conversation_collapse_key(conversation_id: Uuid) -> String {
    format!("chat:{conversation_id}")
}

/// What a message says, in one line, for people who are not looking at it.
pub(crate) fn teaser(row: &MessageRow) -> Localized {
    let caption = content::snippet(&row.text, content::NOTIFICATION_SNIPPET_CHARS);
    match row.kind {
        msg::STAMP => Localized::new("スタンプを送信しました", "Sent a stamp"),
        msg::LOCATION => Localized::new("📍 位置情報を送信しました", "📍 Shared a location"),
        msg::HELLO => Localized::new("👋 やっほー！", "👋 Hello!"),
        msg::MEDIA => {
            let attachments = row.attachments();
            let count = attachments.len();
            let first = attachments.first().and_then(|attachment| attachment.kind.as_known()).unwrap_or(AssetKind::ASSET_KIND_FILE);
            let (icon, ja, en) = match first {
                AssetKind::ASSET_KIND_IMAGE | AssetKind::ASSET_KIND_ANIMATION if count > 1 => {
                    ("📷", format!("写真を{count}枚送信しました"), format!("Sent {count} photos"))
                }
                AssetKind::ASSET_KIND_IMAGE | AssetKind::ASSET_KIND_ANIMATION => ("📷", "写真を送信しました".to_owned(), "Sent a photo".to_owned()),
                AssetKind::ASSET_KIND_VIDEO => ("🎬", "動画を送信しました".to_owned(), "Sent a video".to_owned()),
                AssetKind::ASSET_KIND_AUDIO => ("🎤", "ボイスメッセージを送信しました".to_owned(), "Sent a voice message".to_owned()),
                _ => ("📎", "ファイルを送信しました".to_owned(), "Sent a file".to_owned()),
            };
            if caption.is_empty() { Localized::new(format!("{icon} {ja}"), format!("{icon} {en}")) } else { Localized::same(format!("{icon} {caption}")) }
        }
        _ => Localized::same(caption),
    }
}

fn prefixed(prefix: &str, text: &Localized) -> Localized {
    Localized::new(format!("{prefix}{}", text.ja), format!("{prefix}{}", text.en))
}

/// The notification for a message that reached `recipient`.
///
/// `kind` is `chat.message`, `chat.mention` or `chat.request`. Direct
/// conversations are titled with the sender, groups with their name (the
/// sender then leads the body).
pub(crate) fn message(
    recipient: Uuid,
    kind: &str,
    sender: &UserSummary,
    sender_id: Uuid,
    conv: &ConversationRow,
    row: &MessageRow,
    image_url: Option<String>,
) -> NotificationRequest {
    let sender_name = name_of(sender);
    let said = teaser(row);
    let (title, body) = match kind {
        KIND_REQUEST => (
            Localized::same(sender_name.clone()),
            Localized::new(format!("メッセージリクエストが届きました！「{}」", said.ja), format!("Message request: {}", said.en)),
        ),
        KIND_MENTION => (
            Localized::same(conv.name.clone()),
            Localized::new(
                format!("{sender_name}さんがあなたをメンションしました: {}", said.ja),
                format!("{sender_name} mentioned you: {}", said.en),
            ),
        ),
        _ if conv.is_group() => (Localized::same(conv.name.clone()), prefixed(&format!("{sender_name}: "), &said)),
        _ => (Localized::same(sender_name), said),
    };
    let mut request = NotificationRequest::new(recipient, kind, title, body, conversation_path(conv.id))
        .actor(sender_id)
        .subject(SubjectType::SUBJECT_TYPE_CHAT_MESSAGE, row.id)
        .collapse(conversation_collapse_key(conv.id))
        .data(serde_json::json!({ "conversation_id": conv.id, "message_id": row.id, "seq": row.seq }));
    if kind != KIND_REQUEST {
        request = request.push_only();
    }
    if let Some(url) = image_url {
        request = request.image(url);
    }
    request
}

/// "A reacted to your message".
pub(crate) fn reaction(recipient: Uuid, reactor: &UserSummary, reactor_id: Uuid, conv: &ConversationRow, row: &MessageRow, emoji: &str) -> NotificationRequest {
    let name = name_of(reactor);
    let said = teaser(row);
    let title = if conv.is_group() { Localized::same(conv.name.clone()) } else { Localized::same(name.clone()) };
    let body = Localized::new(
        format!("{name}さんが{emoji}でリアクションしました: {}", said.ja),
        format!("{name} reacted {emoji} to: {}", said.en),
    );
    NotificationRequest::new(recipient, KIND_REACTION, title, body, conversation_path(conv.id))
        .actor(reactor_id)
        .subject(SubjectType::SUBJECT_TYPE_CHAT_MESSAGE, row.id)
        .collapse(format!("chat-reaction:{}", row.id))
        .push_only()
        .data(serde_json::json!({ "conversation_id": conv.id, "message_id": row.id, "emoji": emoji }))
}

/// "A added you to the group X".
pub(crate) fn added_to_group(recipient: Uuid, adder: &UserSummary, adder_id: Uuid, conv: &ConversationRow) -> NotificationRequest {
    let name = name_of(adder);
    NotificationRequest::new(
        recipient,
        KIND_GROUP,
        Localized::same(conv.name.clone()),
        Localized::new(
            format!("{name}さんがあなたをグループに追加しました！"),
            format!("{name} added you to the group"),
        ),
        conversation_path(conv.id),
    )
    .actor(adder_id)
    .collapse(conversation_collapse_key(conv.id))
    .data(serde_json::json!({ "conversation_id": conv.id }))
}

/// Notifications of one call replace each other: "incoming" becomes "missed".
fn call_collapse_key(call_id: Uuid) -> String {
    format!("chat-call:{call_id}")
}

fn call_title(caller: &UserSummary, conv: &ConversationRow) -> Localized {
    if conv.is_group() { Localized::same(conv.name.clone()) } else { Localized::same(name_of(caller)) }
}

/// An incoming call. `ringing` calls are urgent: they must break through
/// quiet hours. Large groups are told about the call without being rung.
pub(crate) fn incoming_call(
    recipient: Uuid,
    caller: &UserSummary,
    caller_id: Uuid,
    conv: &ConversationRow,
    call_id: Uuid,
    video: bool,
    ringing: bool,
) -> NotificationRequest {
    let name = name_of(caller);
    let body = match (conv.is_group(), video) {
        (false, false) => Localized::new("📞 音声通話の着信です！", "📞 Incoming voice call"),
        (false, true) => Localized::new("📹 ビデオ通話の着信です！", "📹 Incoming video call"),
        (true, false) => Localized::new(format!("📞 {name}さんがグループ通話を始めました！"), format!("📞 {name} started a group call")),
        (true, true) => {
            Localized::new(format!("📹 {name}さんがグループビデオ通話を始めました！"), format!("📹 {name} started a group video call"))
        }
    };
    let request = NotificationRequest::new(recipient, KIND_CALL, call_title(caller, conv), body, conversation_path(conv.id))
        .actor(caller_id)
        .collapse(call_collapse_key(call_id))
        .push_only()
        .data(serde_json::json!({ "conversation_id": conv.id, "call_id": call_id, "video": video, "ringing": ringing }));
    if ringing { request.urgent() } else { request }
}

/// A call the recipient did not answer.
pub(crate) fn missed_call(recipient: Uuid, caller: &UserSummary, caller_id: Uuid, conv: &ConversationRow, call_id: Uuid, video: bool) -> NotificationRequest {
    let name = name_of(caller);
    let body = if video {
        Localized::new(format!("{name}さんからのビデオ通話に出られませんでした"), format!("Missed video call from {name}"))
    } else {
        Localized::new(format!("{name}さんからの音声通話に出られませんでした"), format!("Missed voice call from {name}"))
    };
    NotificationRequest::new(recipient, KIND_CALL, call_title(caller, conv), body, conversation_path(conv.id))
        .actor(caller_id)
        .collapse(call_collapse_key(call_id))
        .data(serde_json::json!({ "conversation_id": conv.id, "call_id": call_id, "video": video, "missed": true }))
}

/// What a POV notification is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PovNotice {
    Requested,
    Answered,
    Saved,
}

pub(crate) fn pov(recipient: Uuid, notice: PovNotice, actor: &UserSummary, actor_id: Uuid, conversation_id: Uuid, request_id: Uuid) -> NotificationRequest {
    let name = name_of(actor);
    let (body, what) = match notice {
        PovNotice::Requested => (
            Localized::new("📸 POVリクエストが届きました！今いる場所を見せてね", "📸 wants your POV — show where you are right now!"),
            "requested",
        ),
        PovNotice::Answered => (
            Localized::new("📸 POVが届きました！24時間で消えるよ", "📸 sent their POV — it disappears in 24 hours!"),
            "answered",
        ),
        PovNotice::Saved => (Localized::new("あなたのPOVを保存しました✨", "saved your POV ✨"), "saved"),
    };
    NotificationRequest::new(recipient, KIND_POV, Localized::same(name), body, conversation_path(conversation_id))
        .actor(actor_id)
        .collapse(format!("chat-pov:{request_id}"))
        .data(serde_json::json!({ "conversation_id": conversation_id, "pov_request_id": request_id, "event": what }))
}

#[cfg(test)]
mod tests {
    use tokuly_proto::media::Attachment;

    use super::*;
    use crate::model::kind;

    fn row(kind: i16, text: &str, attachments: Vec<Attachment>) -> MessageRow {
        MessageRow {
            id: new_id(),
            conversation_id: new_id(),
            seq: 7,
            sender_id: Some(new_id()),
            kind,
            client_id: None,
            text: text.to_owned(),
            mentions: serde_json::json!([]),
            mention_all: false,
            stamp_id: None,
            attachments: serde_json::to_value(attachments).unwrap(),
            location: None,
            link_url: None,
            link_preview: None,
            link_checked_at: None,
            reply_to_id: None,
            forwarded: false,
            system: None,
            call_id: None,
            pov_request_id: None,
            edited_at: None,
            deleted_at: None,
            created_at: now(),
        }
    }

    fn conversation(kind: i16, name: &str) -> ConversationRow {
        ConversationRow {
            id: new_id(),
            kind,
            user_lo: None,
            user_hi: None,
            name: name.to_owned(),
            icon: None,
            icon_asset_id: None,
            member_count: 2,
            last_seq: 7,
            last_message_id: None,
            last_activity_at: now(),
            read_only: false,
            created_at: now(),
        }
    }

    fn user(name: &str) -> UserSummary {
        UserSummary { id: new_id().to_string(), handle: "taro".into(), display_name: name.into(), ..Default::default() }
    }

    fn attachment(kind: AssetKind) -> Attachment {
        Attachment { kind: kind.into(), ..Default::default() }
    }

    #[test]
    fn teasers_describe_every_kind_of_message() {
        assert_eq!(teaser(&row(msg::TEXT, "今どこ？\n駅前だよ", vec![])).ja, "今どこ？ 駅前だよ");
        assert_eq!(teaser(&row(msg::STAMP, "", vec![])).en, "Sent a stamp");
        assert_eq!(teaser(&row(msg::HELLO, "", vec![])).ja, "👋 やっほー！");
        let photos = vec![attachment(AssetKind::ASSET_KIND_IMAGE), attachment(AssetKind::ASSET_KIND_IMAGE)];
        assert_eq!(teaser(&row(msg::MEDIA, "", photos.clone())).ja, "📷 写真を2枚送信しました");
        assert_eq!(teaser(&row(msg::MEDIA, "見て！", photos)).en, "📷 見て！");
        assert_eq!(teaser(&row(msg::MEDIA, "", vec![attachment(AssetKind::ASSET_KIND_AUDIO)])).en, "🎤 Sent a voice message");
        assert_eq!(teaser(&row(msg::MEDIA, "", vec![attachment(AssetKind::ASSET_KIND_FILE)])).ja, "📎 ファイルを送信しました");
    }

    #[test]
    fn direct_messages_are_titled_with_the_sender_and_groups_with_their_name() {
        let sender = user("たろう");
        let sender_id = new_id();
        let recipient = new_id();
        let text = row(msg::TEXT, "やっほー", vec![]);

        let direct = message(recipient, KIND_MESSAGE, &sender, sender_id, &conversation(kind::DIRECT, ""), &text, None);
        assert_eq!((direct.title.ja.as_str(), direct.body.ja.as_str()), ("たろう", "やっほー"));
        assert!(direct.push && !direct.inbox, "messages do not fill the notification inbox");
        assert_eq!(direct.actor_id, Some(sender_id));

        let group = conversation(kind::GROUP, "放課後");
        let in_group = message(recipient, KIND_MESSAGE, &sender, sender_id, &group, &text, None);
        assert_eq!((in_group.title.ja.as_str(), in_group.body.ja.as_str()), ("放課後", "たろう: やっほー"));
        assert_eq!(in_group.collapse_key.as_deref(), Some(format!("chat:{}", group.id).as_str()));

        let request = message(recipient, KIND_REQUEST, &sender, sender_id, &conversation(kind::DIRECT, ""), &text, None);
        assert!(request.inbox && request.push);
        assert!(request.body.en.starts_with("Message request"));
    }

    #[test]
    fn only_ringing_calls_are_urgent_and_missed_calls_replace_them() {
        let caller = user("はなこ");
        let caller_id = new_id();
        let call_id = new_id();
        let direct = conversation(kind::DIRECT, "");
        let ringing = incoming_call(new_id(), &caller, caller_id, &direct, call_id, true, true);
        assert!(ringing.urgent && !ringing.inbox);
        let quiet = incoming_call(new_id(), &caller, caller_id, &conversation(kind::GROUP, "部活"), call_id, false, false);
        assert!(!quiet.urgent);
        let missed = missed_call(new_id(), &caller, caller_id, &direct, call_id, true);
        assert!(!missed.urgent && missed.inbox);
        assert_eq!(missed.collapse_key, ringing.collapse_key);
    }

    #[test]
    fn deleted_and_nameless_users_still_have_a_name() {
        assert_eq!(name_of(&UserSummary { handle: "taro".into(), ..Default::default() }), "@taro");
        assert_eq!(name_of(&UserSummary { deleted: true, ..Default::default() }), "退会したユーザー");
    }
}
