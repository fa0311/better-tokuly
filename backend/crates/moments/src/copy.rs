//! Every text the server writes: notifications, in Japanese and English.
use tokuly_core::{
    outbound::{Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::common::SubjectType;

use crate::model::moment_path;

/// Kind of the daily prompt.
pub const KIND_PROMPT: &str = "moments.prompt";
/// Kind of "someone reacted to your moment".
pub const KIND_REACTION: &str = "moments.reaction";

/// 「いまの瞬間を撮ろう!」, to one person of a region whose prompt went out.
pub fn prompt(user_id: Uuid, window_minutes: i64) -> NotificationRequest {
    NotificationRequest::new(
        user_id,
        KIND_PROMPT,
        Localized::new("⚡ いまの瞬間を撮ろう!", "⚡ Time to capture the moment!"),
        Localized::new(
            format!("{window_minutes}分以内に、いま見えているものと自分を撮ってシェアしよう!"),
            format!("You have {window_minutes} minutes to share what you see and yourself, right now!"),
        ),
        "/moments/camera",
    )
    .collapse("moments-prompt")
}

/// 「Aさんがあなたの瞬間にリアクションしたよ!」, to the owner. A changed
/// reaction replaces the earlier notification.
pub fn reaction(owner_id: Uuid, reactor_id: Uuid, reactor_name: &str, moment_id: Uuid) -> NotificationRequest {
    NotificationRequest::new(
        owner_id,
        KIND_REACTION,
        Localized::new(format!("{reactor_name}さんがリアクションしたよ!"), format!("{reactor_name} reacted!")),
        Localized::new("あなたのいまの瞬間を見てみよう", "See it on your moment"),
        moment_path(moment_id),
    )
    .actor(reactor_id)
    .subject(SubjectType::SUBJECT_TYPE_MOMENT, moment_id)
    .collapse(format!("moments-reaction:{moment_id}:{reactor_id}"))
}
