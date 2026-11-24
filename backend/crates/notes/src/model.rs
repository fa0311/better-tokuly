//! Database rows and the pieces of them that map straight onto API messages.
use chrono::{DateTime, Utc};
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::{Image, LatLng},
    media::{Attachment, LinkPreview},
    notes::{Mention, NoteCounters, PlaceTag, StoryItem},
};

use crate::{text, visibility};

/// Web paths of the things this domain owns (notifications, activity
/// entries, search results and reports link to them).
pub fn note_path(note_id: Uuid) -> String {
    format!("/notes/{note_id}")
}

pub fn story_path(story_id: Uuid) -> String {
    format!("/stories/{story_id}")
}

pub fn hashtag_path(tag: &str) -> String {
    format!("/notes/tags/{tag}")
}

/// Realtime topic on which a note's live numbers are published:
/// `subject:<SUBJECT_TYPE_NOTE>:<note_id>`, the same topic the engagement
/// domain uses for a subject.
pub fn note_topic(note_id: Uuid) -> String {
    format!("subject:{}:{note_id}", tokuly_proto::common::SubjectType::SUBJECT_TYPE_NOTE as i32)
}

/// Holder names under which this domain keeps uploads alive (see
/// `tokuly_media::assets`).
pub fn note_holder(note_id: Uuid) -> String {
    format!("notes:note:{note_id}")
}

pub fn story_holder(story_id: Uuid) -> String {
    format!("notes:story:{story_id}")
}

/// Every column of `notes`, for queries that alias the table as `n`.
pub const NOTE_COLUMNS: &str = "n.id, n.author_id, n.text, n.attachments, n.asset_ids, n.stamp_id, \
     n.place_name, n.place_lat, n.place_lng, n.place_id, n.visibility, n.circle_id, n.direct_user_ids, n.listed, n.content_warning, \
     n.reply_to_id, n.reply_to_author_id, n.quote_of_id, n.hashtags, n.mentions, n.link_url, n.link_preview, \
     n.reply_count, n.renote_count, n.quote_count, n.like_count, n.view_count, n.pinned_at, n.created_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct NoteRow {
    pub id: Uuid,
    pub author_id: Uuid,
    pub text: String,
    pub attachments: serde_json::Value,
    pub asset_ids: Vec<Uuid>,
    pub stamp_id: Option<Uuid>,
    pub place_name: Option<String>,
    pub place_lat: Option<f64>,
    pub place_lng: Option<f64>,
    pub place_id: Option<Uuid>,
    pub visibility: i32,
    pub circle_id: Option<Uuid>,
    pub direct_user_ids: Vec<Uuid>,
    /// Everyone may see it: public, author not locked (see [`Self::is_listed`]).
    pub listed: bool,
    pub content_warning: bool,
    pub reply_to_id: Option<Uuid>,
    pub reply_to_author_id: Option<Uuid>,
    pub quote_of_id: Option<Uuid>,
    pub hashtags: Vec<String>,
    pub mentions: serde_json::Value,
    pub link_url: Option<String>,
    pub link_preview: Option<serde_json::Value>,
    pub reply_count: i64,
    pub renote_count: i64,
    pub quote_count: i64,
    pub like_count: i64,
    pub view_count: i64,
    pub pinned_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl NoteRow {
    /// Shared with PUBLIC (which, for a locked author, still means their
    /// followers and friends only: see [`Self::is_listed`]).
    pub fn is_public(&self) -> bool {
        self.visibility == visibility::PUBLIC
    }

    /// Everyone may see the note: public, and its author is not locked. What
    /// search, hashtag counts, trends, quotes and other people's renotes go by.
    pub fn is_listed(&self) -> bool {
        self.listed
    }

    /// A private note addressed to people.
    pub fn is_direct(&self) -> bool {
        self.visibility == visibility::PRIVATE && !self.direct_user_ids.is_empty()
    }

    /// The audience the note is shared with, as the social graph names it.
    pub fn audience(&self) -> tokuly_social::graph::ContentAudience {
        visibility::audience_of(self.visibility, self.circle_id)
    }

    pub fn is_reply(&self) -> bool {
        self.reply_to_id.is_some()
    }

    pub fn attachments(&self) -> Vec<Attachment> {
        serde_json::from_value(self.attachments.clone()).unwrap_or_default()
    }

    pub fn mentions(&self) -> Vec<Mention> {
        serde_json::from_value(self.mentions.clone()).unwrap_or_default()
    }

    pub fn link_preview(&self) -> Option<LinkPreview> {
        self.link_preview.clone().and_then(|value| serde_json::from_value(value).ok())
    }

    pub fn place(&self) -> Option<PlaceTag> {
        let (name, lat, lng) = (self.place_name.as_ref()?, self.place_lat?, self.place_lng?);
        Some(PlaceTag {
            name: name.clone(),
            location: LatLng { lat, lng, ..Default::default() }.into(),
            place_id: self.place_id.map(|id| id.to_string()).unwrap_or_default(),
            ..Default::default()
        })
    }

    pub fn counters(&self) -> NoteCounters {
        NoteCounters {
            note_id: self.id.to_string(),
            reply_count: self.reply_count,
            renote_count: self.renote_count,
            quote_count: self.quote_count,
            like_count: self.like_count,
            view_count: self.view_count,
            ..Default::default()
        }
    }

    /// A picture standing for the note: its first image, or the poster of
    /// its video.
    pub fn image_url(&self) -> Option<String> {
        self.attachments().iter().find_map(attachment_image).map(|image| image.url).filter(|url| !url.is_empty())
    }

    /// One line describing the note where only a title fits (reports,
    /// search results, notification bodies).
    pub fn title(&self) -> String {
        let line = text::excerpt(&self.text, 60);
        if !line.is_empty() {
            line
        } else if !self.asset_ids.is_empty() {
            "📷".to_owned()
        } else {
            "ノート".to_owned()
        }
    }
}

/// The still picture of an attachment: the image itself, or a video's poster.
pub fn attachment_image(attachment: &Attachment) -> Option<Image> {
    attachment
        .image
        .as_option()
        .cloned()
        .or_else(|| attachment.video.as_option().and_then(|video| video.poster.as_option().cloned()))
}

/// Every column of `stories`, for queries that alias the table as `s`.
pub const STORY_COLUMNS: &str = "s.id, s.owner_id, s.title, s.items, s.asset_ids, s.audience, s.view_count, s.reaction_count, \
     s.expires_at, s.archived_at, s.created_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StoryRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub title: String,
    pub items: serde_json::Value,
    pub asset_ids: Vec<Uuid>,
    pub audience: i32,
    pub view_count: i32,
    pub reaction_count: i32,
    pub expires_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl StoryRow {
    pub fn items(&self) -> Vec<StoryItem> {
        serde_json::from_value(self.items.clone()).unwrap_or_default()
    }

    pub fn is_expired(&self) -> bool {
        self.expires_at <= Utc::now()
    }

    pub fn is_close_friends(&self) -> bool {
        self.audience == visibility::AUDIENCE_CLOSE_FRIENDS
    }

    /// The first photo of the story, or the poster of its first video.
    pub fn cover(&self) -> Option<Image> {
        self.items().iter().find_map(|item| item.media.as_option().and_then(attachment_image))
    }

    pub fn title_or_default(&self) -> String {
        if self.title.is_empty() { "記憶".to_owned() } else { text::excerpt(&self.title, 60) }
    }
}

/// Stores an API message as proto JSON in a `jsonb` column.
pub fn to_json<T: serde::Serialize>(value: &T) -> AppResult<serde_json::Value> {
    serde_json::to_value(value).map_err(|e| AppError::internal(anyhow::anyhow!("json encode: {e}")))
}

/// The smallest UUIDv7 a row created at `time` can have. Notes, likes,
/// renotes and stories are keyed by UUIDv7, so "created after T" can be
/// asked of the primary key index instead of needing one on `created_at`.
pub fn id_floor(time: DateTime<Utc>) -> Uuid {
    let millis = time.timestamp_millis().max(0) as u128;
    Uuid::from_u128(millis << 80)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_floor_sorts_before_ids_created_later() {
        let floor = id_floor(Utc::now() - chrono::Duration::seconds(5));
        assert!(floor < Uuid::now_v7());
        assert!(id_floor(Utc::now() + chrono::Duration::seconds(5)) > Uuid::now_v7());
    }

    #[test]
    fn topics_and_holders_follow_the_conventions() {
        let id = Uuid::nil();
        assert_eq!(note_topic(id), "subject:3:00000000-0000-0000-0000-000000000000");
        assert_eq!(note_holder(id), "notes:note:00000000-0000-0000-0000-000000000000");
        assert_eq!(story_holder(id), "notes:story:00000000-0000-0000-0000-000000000000");
        assert_eq!(note_path(id), "/notes/00000000-0000-0000-0000-000000000000");
        assert_eq!(hashtag_path("春"), "/notes/tags/春");
    }
}
