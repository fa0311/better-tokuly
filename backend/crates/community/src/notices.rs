//! The words of every notification and activity entry this domain sends,
//! Japanese first (casual, friendly), English second.
//!
//! Notification kinds: `community.announcement`, `community.mention`,
//! `community.reply`, `community.event_reminder`, `community.event_update`,
//! `community.join_request`, `community.join_approved`,
//! `community.membership`, `community.moderation`, `community.review`,
//! `community.author_revealed`.
use tokuly_core::{
    outbound::{ActivityAudience, ActivityEntry, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::common::{SubjectType, UserSummary};

use crate::{
    model::{CommunityRow, EventRow, ReplyRow, ThreadRow},
    permissions::Role,
    pseudonym, text,
};

/// Longest post excerpt in a notification body.
const EXCERPT_CHARS: usize = 80;

/// Who wrote a post, as a notification may name them: the account, or in
/// an anonymous channel the pseudonym — and then no actor, whose avatar
/// would give the author away.
#[derive(Debug, Clone)]
pub struct Speaker {
    pub actor_id: Option<Uuid>,
    pub name: Localized,
}

impl Speaker {
    pub fn user(user: &UserSummary, user_id: Uuid) -> Self {
        Self { actor_id: Some(user_id), name: Localized::same(display_name(user)) }
    }

    pub fn anonymous(keys: &tokuly_core::keys::Keys, thread_id: Uuid, author_id: Uuid) -> Self {
        let (ja, en) = pseudonym::label(&pseudonym::pseudonym(keys, thread_id, author_id));
        Self { actor_id: None, name: Localized::new(ja, en) }
    }
}

fn display_name(user: &UserSummary) -> String {
    if user.display_name.is_empty() { format!("@{}", user.handle) } else { user.display_name.clone() }
}

/// What a post says, in one line: its text, or what it is made of.
fn post_excerpt(body: &str, fallback: &str) -> String {
    let excerpt = text::excerpt(body, EXCERPT_CHARS);
    if excerpt.is_empty() { fallback.to_owned() } else { excerpt }
}

fn with_actor(request: NotificationRequest, actor_id: Option<Uuid>) -> NotificationRequest {
    match actor_id {
        Some(actor_id) => request.actor(actor_id),
        None => request,
    }
}

fn with_icon(request: NotificationRequest, community: &CommunityRow) -> NotificationRequest {
    match community.icon_url() {
        Some(url) => request.image(url),
        None => request,
    }
}

/// Someone was `@mentioned` in a thread (`reply` unset) or a reply.
pub fn mention(
    recipient: Uuid,
    speaker: &Speaker,
    community: &CommunityRow,
    thread: &ThreadRow,
    reply: Option<&ReplyRow>,
) -> NotificationRequest {
    let (body, path, subject) = match reply {
        Some(reply) => (
            post_excerpt(&reply.body, &thread.title),
            reply.path(community),
            (SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY, reply.id),
        ),
        None => (
            post_excerpt(&format!("{} {}", thread.title, thread.body), &thread.title),
            thread.path(community),
            (SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, thread.id),
        ),
    };
    let request = NotificationRequest::new(
        recipient,
        "community.mention",
        Localized::new(
            format!("{}さんが{}であなたをメンションしたよ!", speaker.name.ja, community.name),
            format!("{} mentioned you in {}", speaker.name.en, community.name),
        ),
        Localized::same(body),
        path,
    )
    .subject(subject.0, subject.1)
    .data(serde_json::json!({ "community_id": community.id, "thread_id": thread.id, "reply_id": reply.map(|r| r.id) }));
    with_actor(request, speaker.actor_id)
}

/// A reply was written in a thread the recipient started or follows. One
/// inbox entry per thread: a newer reply replaces the older notice.
pub fn reply(
    recipient: Uuid,
    speaker: &Speaker,
    community: &CommunityRow,
    thread: &ThreadRow,
    reply: &ReplyRow,
) -> NotificationRequest {
    let said = post_excerpt(&reply.body, "📎");
    let request = NotificationRequest::new(
        recipient,
        "community.reply",
        Localized::new(
            format!("「{}」に返信がきたよ!", text::excerpt(&thread.title, 40)),
            format!("New reply in \"{}\"", text::excerpt(&thread.title, 40)),
        ),
        Localized::new(format!("{}: {said}", speaker.name.ja), format!("{}: {said}", speaker.name.en)),
        reply.path(community),
    )
    .subject(SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, thread.id)
    .collapse(format!("community.reply:{}", thread.id))
    .data(serde_json::json!({ "community_id": community.id, "thread_id": thread.id, "reply_id": reply.id }));
    with_actor(request, speaker.actor_id)
}

/// A thread was marked as an announcement.
pub fn announcement(recipient: Uuid, community: &CommunityRow, thread: &ThreadRow) -> NotificationRequest {
    let request = NotificationRequest::new(
        recipient,
        "community.announcement",
        Localized::new(
            format!("📢 {}からのお知らせだよ!", community.name),
            format!("📢 Announcement from {}", community.name),
        ),
        Localized::same(thread.title.clone()),
        thread.path(community),
    )
    .subject(SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, thread.id)
    .collapse(format!("community.announcement:{}", thread.id))
    .data(serde_json::json!({ "community_id": community.id, "thread_id": thread.id }));
    with_icon(request, community)
}

/// Which reminder of an event is due.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reminder {
    /// About a day before the start.
    Day,
    /// About an hour before the start.
    Hour,
}

pub fn event_reminder(
    recipient: Uuid,
    community: &CommunityRow,
    event: &EventRow,
    reminder: Reminder,
) -> NotificationRequest {
    let title = match reminder {
        Reminder::Day => {
            Localized::new(format!("明日は「{}」だよ!", event.title), format!("\"{}\" is tomorrow", event.title))
        }
        Reminder::Hour => Localized::new(
            format!("もうすぐ「{}」が始まるよ!", event.title),
            format!("\"{}\" starts soon", event.title),
        ),
    };
    let place = if event.online { "オンライン".to_owned() } else { event.place_name.clone() };
    let place_en = if event.online { "Online".to_owned() } else { event.place_name.clone() };
    let request = NotificationRequest::new(
        recipient,
        "community.event_reminder",
        title,
        Localized::new(format!("{} ・ {place}", community.name), format!("{} · {place_en}", community.name)),
        event.path(community),
    )
    .subject(SubjectType::SUBJECT_TYPE_COMMUNITY_EVENT, event.id)
    .collapse(format!("community.event_reminder:{}", event.id))
    .data(serde_json::json!({
        "community_id": community.id, "event_id": event.id, "starts_at": event.starts_at.to_rfc3339(),
        "reminder": if reminder == Reminder::Day { "day" } else { "hour" },
    }));
    with_icon(request, community)
}

/// What happened to an event the recipient answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventUpdate {
    /// A place freed and the recipient moved up from the waitlist.
    Promoted,
    Cancelled {
        reason: String,
    },
    /// The start moved.
    Rescheduled,
}

pub fn event_update(
    recipient: Uuid,
    community: &CommunityRow,
    event: &EventRow,
    update: &EventUpdate,
) -> NotificationRequest {
    let (title, body, what) = match update {
        EventUpdate::Promoted => (
            Localized::new(
                format!("「{}」に参加できるようになったよ!", event.title),
                format!("You're in: a place opened up for \"{}\"", event.title),
            ),
            Localized::new("キャンセル待ちから繰り上がったよ 🎉", "You moved up from the waitlist 🎉"),
            "promoted",
        ),
        EventUpdate::Cancelled { reason } => (
            Localized::new(
                format!("「{}」は中止になりました", event.title),
                format!("\"{}\" was cancelled", event.title),
            ),
            Localized::same(reason.clone()),
            "cancelled",
        ),
        EventUpdate::Rescheduled => (
            Localized::new(
                format!("「{}」の日時が変わったよ!", event.title),
                format!("\"{}\" was rescheduled", event.title),
            ),
            Localized::new("新しい日時を確認してね", "Check the new time"),
            "rescheduled",
        ),
    };
    let request = NotificationRequest::new(recipient, "community.event_update", title, body, event.path(community))
        .subject(SubjectType::SUBJECT_TYPE_COMMUNITY_EVENT, event.id)
        .data(serde_json::json!({ "community_id": community.id, "event_id": event.id, "update": what }));
    with_icon(request, community)
}

/// To the staff: someone asks to join. One inbox entry per community.
pub fn join_request(
    recipient: Uuid,
    requester: &UserSummary,
    requester_id: Uuid,
    community: &CommunityRow,
) -> NotificationRequest {
    NotificationRequest::new(
        recipient,
        "community.join_request",
        Localized::new(
            format!("{}に参加リクエストがきたよ!", community.name),
            format!("New request to join {}", community.name),
        ),
        Localized::new(
            format!("{}さんが参加したがっています", display_name(requester)),
            format!("{} wants to join", display_name(requester)),
        ),
        format!("{}/requests", community.path()),
    )
    .actor(requester_id)
    .subject(SubjectType::SUBJECT_TYPE_COMMUNITY, community.id)
    .collapse(format!("community.join_request:{}", community.id))
    .data(serde_json::json!({ "community_id": community.id }))
}

pub fn join_approved(recipient: Uuid, community: &CommunityRow) -> NotificationRequest {
    let request = NotificationRequest::new(
        recipient,
        "community.join_approved",
        Localized::new(
            format!("{}に参加できたよ!", community.name),
            format!("You're in: welcome to {}", community.name),
        ),
        Localized::new("さっそくのぞいてみよう 👀", "Come and have a look 👀"),
        community.path(),
    )
    .subject(SubjectType::SUBJECT_TYPE_COMMUNITY, community.id)
    .data(serde_json::json!({ "community_id": community.id }));
    with_icon(request, community)
}

/// A change to the recipient's place in a community, made by the staff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MembershipNotice {
    Role(Role),
    /// The recipient now owns the community.
    Ownership,
    Removed {
        reason: String,
    },
    Banned {
        reason: String,
        expires_at: Option<Time>,
    },
    TimedOut {
        reason: String,
        until: Time,
    },
}

pub fn membership(recipient: Uuid, community: &CommunityRow, notice: &MembershipNotice) -> NotificationRequest {
    let name = &community.name;
    let (title, body, what) = match notice {
        MembershipNotice::Role(role) => {
            let (ja, en) = match role {
                Role::Owner => ("オーナー", "owner"),
                Role::Admin => ("管理者", "admin"),
                Role::Moderator => ("モデレーター", "moderator"),
                Role::Member => ("メンバー", "member"),
            };
            (
                Localized::new(format!("{name}での役割が変わったよ"), format!("Your role in {name} changed")),
                Localized::new(format!("{ja}になりました"), format!("You are now a {en}")),
                "role",
            )
        }
        MembershipNotice::Ownership => (
            Localized::new(format!("{name}のオーナーになったよ!"), format!("You now own {name}")),
            Localized::new("コミュニティをよろしくね 👑", "Take good care of it 👑"),
            "ownership",
        ),
        MembershipNotice::Removed { reason } => (
            Localized::new(format!("{name}から退出になりました"), format!("You were removed from {name}")),
            Localized::same(reason.clone()),
            "removed",
        ),
        MembershipNotice::Banned { reason, .. } => (
            Localized::new(format!("{name}に参加できなくなりました"), format!("You were banned from {name}")),
            Localized::same(reason.clone()),
            "banned",
        ),
        MembershipNotice::TimedOut { reason, .. } => (
            Localized::new(format!("{name}でしばらく投稿できなくなりました"), format!("You were timed out in {name}")),
            Localized::same(reason.clone()),
            "timed_out",
        ),
    };
    let until = match notice {
        MembershipNotice::Banned { expires_at, .. } => *expires_at,
        MembershipNotice::TimedOut { until, .. } => Some(*until),
        _ => None,
    };
    let request = NotificationRequest::new(recipient, "community.membership", title, body, community.path())
        .subject(SubjectType::SUBJECT_TYPE_COMMUNITY, community.id)
        .data(serde_json::json!({
            "community_id": community.id, "change": what, "until": until.map(|time| time.to_rfc3339()),
        }));
    with_icon(request, community)
}

/// What the staff (or the automatic tools) did with a post of the recipient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostNotice {
    /// Hidden until the staff reviewed it.
    Held,
    /// Back in public view after review.
    Restored,
    /// Deleted by the staff.
    Removed { reason: String },
}

/// `title` names the post: the thread's title (for a reply: of its thread).
pub fn moderation(
    recipient: Uuid,
    community: &CommunityRow,
    title: &str,
    path: String,
    notice: &PostNotice,
) -> NotificationRequest {
    let post = text::excerpt(title, 40);
    let (heading, body, what) = match notice {
        PostNotice::Held => (
            Localized::new(
                format!("「{post}」の投稿は確認待ちです"),
                format!("Your post in \"{post}\" is waiting for review"),
            ),
            Localized::new(
                "モデレーターが確認するまで、ほかの人には表示されません",
                "It is hidden from others until a moderator has looked at it",
            ),
            "held",
        ),
        PostNotice::Restored => (
            Localized::new(
                format!("「{post}」の投稿が公開されたよ"),
                format!("Your post in \"{post}\" is visible again"),
            ),
            Localized::new("確認が終わりました", "The review is done"),
            "restored",
        ),
        PostNotice::Removed { reason } => (
            Localized::new(
                format!("「{post}」の投稿が削除されました"),
                format!("Your post in \"{post}\" was removed by the moderators"),
            ),
            Localized::same(reason.clone()),
            "removed",
        ),
    };
    let request = NotificationRequest::new(recipient, "community.moderation", heading, body, path)
        .subject(SubjectType::SUBJECT_TYPE_COMMUNITY, community.id)
        .data(serde_json::json!({ "community_id": community.id, "change": what }));
    with_icon(request, community)
}

/// To the staff: a post was hidden and waits for review. One inbox entry
/// per community.
pub fn review(recipient: Uuid, community: &CommunityRow) -> NotificationRequest {
    let request = NotificationRequest::new(
        recipient,
        "community.review",
        Localized::new(
            format!("{}で確認待ちの投稿があります", community.name),
            format!("A post in {} is waiting for review", community.name),
        ),
        Localized::new("非表示になった投稿を確認してね", "Have a look at the hidden post"),
        format!("{}/review", community.path()),
    )
    .subject(SubjectType::SUBJECT_TYPE_COMMUNITY, community.id)
    .collapse(format!("community.review:{}", community.id))
    .data(serde_json::json!({ "community_id": community.id }));
    with_icon(request, community)
}

/// To the rest of the staff: someone looked up the author of an anonymous
/// post. The author is not named.
pub fn author_revealed(
    recipient: Uuid,
    actor: &UserSummary,
    actor_id: Uuid,
    community: &CommunityRow,
    thread_title: &str,
    reason: &str,
) -> NotificationRequest {
    NotificationRequest::new(
        recipient,
        "community.author_revealed",
        Localized::new(
            format!("{}さんが匿名投稿の投稿者を確認しました", display_name(actor)),
            format!("{} looked up the author of an anonymous post", display_name(actor)),
        ),
        Localized::new(
            format!("「{}」・理由: {reason}", text::excerpt(thread_title, 40)),
            format!("\"{}\" · reason: {reason}", text::excerpt(thread_title, 40)),
        ),
        format!("{}/log", community.path()),
    )
    .actor(actor_id)
    .subject(SubjectType::SUBJECT_TYPE_COMMUNITY, community.id)
    .data(serde_json::json!({ "community_id": community.id }))
}

/// The activity entry of someone joining (or creating) a community. Shown
/// to friends only for a public community the member shows on their profile.
pub fn joined_activity(user_id: Uuid, community: &CommunityRow, created: bool, shown: bool) -> ActivityEntry {
    let audience = if shown && community.is_public() { ActivityAudience::Friends } else { ActivityAudience::Private };
    let (kind, summary) = if created {
        (
            "community.created",
            Localized::new(
                format!("{}をつくったよ!", community.name),
                format!("Created the community {}", community.name),
            ),
        )
    } else {
        (
            "community.joined",
            Localized::new(
                format!("{}に参加したよ!", community.name),
                format!("Joined the community {}", community.name),
            ),
        )
    };
    let entry = ActivityEntry::new(user_id, kind, audience, summary)
        .subject(SubjectType::SUBJECT_TYPE_COMMUNITY, community.id)
        .path(community.path())
        .data(serde_json::json!({ "community_id": community.id, "handle": community.handle }));
    match community.icon_url() {
        Some(url) => entry.image(url),
        None => entry,
    }
}

/// The activity entry of a check-in at an event: the member's own log only.
pub fn attended_activity(user_id: Uuid, community: &CommunityRow, event: &EventRow) -> ActivityEntry {
    let entry = ActivityEntry::new(
        user_id,
        "community.event_attended",
        ActivityAudience::Private,
        Localized::new(format!("「{}」に参加したよ!", event.title), format!("Attended \"{}\"", event.title)),
    )
    .subject(SubjectType::SUBJECT_TYPE_COMMUNITY_EVENT, event.id)
    .path(event.path(community))
    .data(serde_json::json!({ "community_id": community.id, "event_id": event.id }));
    match (event.place_lat, event.place_lng) {
        (Some(lat), Some(lng)) => entry.at(lat, lng),
        _ => entry,
    }
}
