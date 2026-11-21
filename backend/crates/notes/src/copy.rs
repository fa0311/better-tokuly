//! The words this domain sends out: notification titles and activity
//! summaries, Japanese first, English second.
//!
//! Notification kinds (all in the `notes` preference category):
//! `notes.like`, `notes.renote`, `notes.reply`, `notes.mention`,
//! `notes.quote`, `notes.story_reaction`.
use tokuly_core::{
    outbound::{ActivityAudience, ActivityEntry, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::common::{SubjectType, UserSummary};

use crate::{
    model::{NoteRow, StoryRow, note_path, story_path},
    text, visibility,
};

/// Longest note excerpt quoted in a notification body.
const BODY_CHARS: usize = 80;

/// How a person is named in a notification.
pub fn display_name(user: &UserSummary) -> String {
    if user.display_name.is_empty() { format!("@{}", user.handle) } else { user.display_name.clone() }
}

/// The note as a notification body: its text, or a stand-in for a note
/// that is only pictures or a stamp.
fn note_body(note: &NoteRow) -> Localized {
    let excerpt = text::excerpt(&note.text, BODY_CHARS);
    if !excerpt.is_empty() {
        Localized::same(excerpt)
    } else if !note.asset_ids.is_empty() {
        Localized::new("📷 写真・動画", "📷 Photo or video")
    } else {
        Localized::new("スタンプ", "Stamp")
    }
}

fn about_note(
    recipient: Uuid,
    kind: &str,
    actor: &UserSummary,
    actor_id: Uuid,
    title: Localized,
    shown: &NoteRow,
    opens: Uuid,
) -> NotificationRequest {
    let mut request = NotificationRequest::new(recipient, kind, title, note_body(shown), note_path(opens))
        .actor(actor_id)
        .subject(SubjectType::SUBJECT_TYPE_NOTE, opens)
        .data(serde_json::json!({ "note_id": opens }));
    if let Some(avatar) = actor.avatar.as_option().filter(|avatar| !avatar.url.is_empty()) {
        request = request.image(avatar.url.clone());
    }
    request
}

/// `notes.like`: someone liked the recipient's note. Likes of one note
/// collapse into a single inbox entry.
pub fn like(actor: &UserSummary, actor_id: Uuid, note: &NoteRow) -> NotificationRequest {
    let name = display_name(actor);
    let title = Localized::new(format!("{name}さんがいいねしたよ!"), format!("{name} liked your note"));
    about_note(note.author_id, "notes.like", actor, actor_id, title, note, note.id)
        .collapse(format!("notes.like:{}", note.id))
}

/// `notes.renote`: someone renoted the recipient's note.
pub fn renote(actor: &UserSummary, actor_id: Uuid, note: &NoteRow) -> NotificationRequest {
    let name = display_name(actor);
    let title = Localized::new(format!("{name}さんがリノートしたよ!"), format!("{name} renoted your note"));
    about_note(note.author_id, "notes.renote", actor, actor_id, title, note, note.id)
        .collapse(format!("notes.renote:{}:{actor_id}", note.id))
}

/// `notes.reply`: `reply` answers a note of `recipient`.
pub fn reply(actor: &UserSummary, recipient: Uuid, reply: &NoteRow) -> NotificationRequest {
    let name = display_name(actor);
    let title = Localized::new(format!("{name}さんから返信がきたよ!"), format!("{name} replied to your note"));
    about_note(recipient, "notes.reply", actor, reply.author_id, title, reply, reply.id)
}

/// `notes.quote`: `quote` quotes a note of `recipient`.
pub fn quote(actor: &UserSummary, recipient: Uuid, quote: &NoteRow) -> NotificationRequest {
    let name = display_name(actor);
    let title = Localized::new(format!("{name}さんがあなたのノートを引用したよ!"), format!("{name} quoted your note"));
    about_note(recipient, "notes.quote", actor, quote.author_id, title, quote, quote.id)
}

/// `notes.mention`: `note` mentions `recipient`.
pub fn mention(actor: &UserSummary, recipient: Uuid, note: &NoteRow) -> NotificationRequest {
    let name = display_name(actor);
    let title = Localized::new(format!("{name}さんがあなたをメンションしたよ!"), format!("{name} mentioned you"));
    about_note(recipient, "notes.mention", actor, note.author_id, title, note, note.id)
}

/// `notes.story_reaction`: someone reacted to the recipient's story. A
/// changed emoji replaces the earlier notification of the same person.
pub fn story_reaction(actor: &UserSummary, actor_id: Uuid, story: &StoryRow, emoji: &str) -> NotificationRequest {
    let name = display_name(actor);
    let title = Localized::new(
        format!("{name}さんが記憶にリアクションしたよ! {emoji}"),
        format!("{name} reacted {emoji} to your story"),
    );
    let body = if story.title.is_empty() {
        Localized::new("あなたの記憶", "Your story")
    } else {
        Localized::same(text::excerpt(&story.title, BODY_CHARS))
    };
    let mut request =
        NotificationRequest::new(story.owner_id, "notes.story_reaction", title, body, story_path(story.id))
            .actor(actor_id)
            .subject(SubjectType::SUBJECT_TYPE_STORY, story.id)
            .collapse(format!("notes.story_reaction:{}:{actor_id}", story.id))
            .data(serde_json::json!({ "story_id": story.id, "emoji": emoji }));
    if let Some(cover) = story.cover().filter(|cover| !cover.url.is_empty()) {
        request = request.image(cover.url);
    }
    request
}

/// The activity entry of a new note (`note.posted`) or reply
/// (`note.replied`), shown to the people who may see the note itself.
pub fn note_activity(note: &NoteRow) -> ActivityEntry {
    let audience = match note.visibility {
        visibility::PUBLIC => ActivityAudience::Public,
        visibility::FRIENDS => ActivityAudience::Friends,
        _ => ActivityAudience::Private,
    };
    let (kind, summary) = if note.is_reply() {
        ("note.replied", Localized::new("ノートに返信したよ", "Replied to a note"))
    } else {
        ("note.posted", Localized::new("ノートを投稿したよ", "Posted a note"))
    };
    let mut entry = ActivityEntry::new(note.author_id, kind, audience, summary)
        .subject(SubjectType::SUBJECT_TYPE_NOTE, note.id)
        .path(note_path(note.id))
        .with(note.reply_to_author_id.filter(|author| *author != note.author_id))
        .data(serde_json::json!({
            "note_id": note.id,
            "text": text::excerpt(&note.text, 140),
            "reply_to_id": note.reply_to_id,
            "quote_of_id": note.quote_of_id,
            "content_warning": note.content_warning,
        }));
    // Sensitive pictures are not put on feed cards.
    if let Some(url) = note.image_url().filter(|_| !note.content_warning) {
        entry = entry.image(url);
    }
    if let (Some(lat), Some(lng)) = (note.place_lat, note.place_lng) {
        entry = entry.at(lat, lng);
    }
    entry
}

/// The activity entry of a new story (`story.posted`): for the owner's
/// friends, or only the owner's own log when the story is for chosen
/// friends (the feed cannot know who they are).
pub fn story_activity(story: &StoryRow) -> ActivityEntry {
    let audience = if story.is_close_friends() { ActivityAudience::Private } else { ActivityAudience::Friends };
    let mut entry = ActivityEntry::new(
        story.owner_id,
        "story.posted",
        audience,
        Localized::new("記憶を投稿したよ", "Posted a story"),
    )
    .subject(SubjectType::SUBJECT_TYPE_STORY, story.id)
    .path(story_path(story.id))
    .data(serde_json::json!({
        "story_id": story.id,
        "title": story.title,
        "item_count": story.items().len(),
        "expires_at": story.expires_at.to_rfc3339(),
    }));
    if let Some(cover) = story.cover().filter(|cover| !cover.url.is_empty()) {
        entry = entry.image(cover.url);
    }
    entry
}
