//! Database rows and their API representations.
use tokuly_core::prelude::*;
use tokuly_media::assets::AssetRow;
use tokuly_proto::{
    common::Image,
    emoji::CustomEmoji,
    media::{AssetKind, AssetPurpose},
};

use crate::validate::MIN_IMAGE_EDGE;

/// `emojis.state` (`tokuly.emoji.v1.EmojiState`).
pub mod state {
    pub const PENDING: i16 = 1;
    pub const APPROVED: i16 = 2;
    pub const REJECTED: i16 = 3;
    pub const DISABLED: i16 = 4;
}

/// `emoji_reviews.action` (`tokuly.emoji.v1.ReviewAction`).
pub mod action {
    pub const SUBMITTED: i16 = 1;
    pub const APPROVED: i16 = 2;
    pub const REJECTED: i16 = 3;
    pub const DISABLED: i16 = 4;
    pub const ENABLED: i16 = 5;
    pub const ADDED: i16 = 6;
    pub const IMPORTED: i16 = 7;
    pub const EDITED: i16 = 8;
    pub const TAKEN_DOWN: i16 = 9;
}

const _: () = {
    use tokuly_proto::emoji::{EmojiState, ReviewAction};
    // SQL in this crate compares these columns with literals.
    assert!(EmojiState::EMOJI_STATE_PENDING as i32 == state::PENDING as i32);
    assert!(EmojiState::EMOJI_STATE_APPROVED as i32 == state::APPROVED as i32);
    assert!(EmojiState::EMOJI_STATE_REJECTED as i32 == state::REJECTED as i32);
    assert!(EmojiState::EMOJI_STATE_DISABLED as i32 == state::DISABLED as i32);
    assert!(ReviewAction::REVIEW_ACTION_SUBMITTED as i32 == action::SUBMITTED as i32);
    assert!(ReviewAction::REVIEW_ACTION_APPROVED as i32 == action::APPROVED as i32);
    assert!(ReviewAction::REVIEW_ACTION_REJECTED as i32 == action::REJECTED as i32);
    assert!(ReviewAction::REVIEW_ACTION_DISABLED as i32 == action::DISABLED as i32);
    assert!(ReviewAction::REVIEW_ACTION_ENABLED as i32 == action::ENABLED as i32);
    assert!(ReviewAction::REVIEW_ACTION_ADDED as i32 == action::ADDED as i32);
    assert!(ReviewAction::REVIEW_ACTION_IMPORTED as i32 == action::IMPORTED as i32);
    assert!(ReviewAction::REVIEW_ACTION_EDITED as i32 == action::EDITED as i32);
    assert!(ReviewAction::REVIEW_ACTION_TAKEN_DOWN as i32 == action::TAKEN_DOWN as i32);
};

/// The columns of [`EmojiRow`], for `SELECT` and `RETURNING`.
macro_rules! emoji_columns {
    () => {
        "e.id, e.name, e.state, e.asset_id, e.image, e.animated, e.category, e.aliases, e.license, e.sensitive, \
         e.local_only, e.author_id, e.review_note, e.reviewed_at, e.approved_at, e.catalog_version, e.created_at, e.updated_at"
    };
}
pub(crate) use emoji_columns;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EmojiRow {
    pub id: Uuid,
    pub name: String,
    pub state: i16,
    pub asset_id: Uuid,
    pub image: serde_json::Value,
    pub animated: bool,
    pub category: Option<String>,
    pub aliases: Vec<String>,
    pub license: String,
    pub sensitive: bool,
    pub local_only: bool,
    pub author_id: Option<Uuid>,
    pub review_note: String,
    pub reviewed_at: Option<Time>,
    pub approved_at: Option<Time>,
    pub catalog_version: i64,
    pub created_at: Time,
    pub updated_at: Time,
}

impl EmojiRow {
    pub fn is_approved(&self) -> bool {
        self.state == state::APPROVED
    }

    /// Whether the emoji holds its name (every state but REJECTED does).
    pub fn holds_name(&self) -> bool {
        self.state != state::REJECTED
    }

    pub fn image(&self) -> Image {
        serde_json::from_value(self.image.clone()).unwrap_or_default()
    }

    /// The emoji as it appears in text.
    pub fn shortcode(&self) -> String {
        shortcode(&self.name)
    }

    /// Its page in the web app.
    pub fn path(&self) -> String {
        format!("/emoji/{}", self.name)
    }

    /// The compact form clients keep in their catalogue.
    pub fn to_proto(&self) -> CustomEmoji {
        CustomEmoji {
            id: self.id.to_string(),
            name: self.name.clone(),
            image: Some(self.image()).into(),
            animated: self.animated,
            category: self.category.clone().unwrap_or_default(),
            aliases: self.aliases.clone(),
            sensitive: self.sensitive,
            local_only: self.local_only,
            added_at: opt_ts(self.approved_at),
            ..Default::default()
        }
    }
}

/// `:name:`.
pub fn shortcode(name: &str) -> String {
    format!(":{name}:")
}

/// The media holder under which an emoji keeps its upload in use.
pub fn holder(emoji_id: Uuid) -> String {
    format!("emoji:{emoji_id}")
}

/// The picture of an emoji, taken from a processed upload.
#[derive(Debug, Clone)]
pub struct Picture {
    pub asset_id: Uuid,
    /// `tokuly.common.v1.Image` as proto JSON.
    pub image: serde_json::Value,
    pub animated: bool,
}

/// The purpose emoji pictures are uploaded with: small pictures, rendered
/// no larger than an emoji is shown, animations kept.
pub const PICTURE_PURPOSE: AssetPurpose = AssetPurpose::ASSET_PURPOSE_CUSTOM_EMOJI;
/// The purposes an emoji picture may have: its own, and the stamps' one
/// that emoji borrowed before (uploads made then, and clients built then).
pub const PICTURE_PURPOSES: [AssetPurpose; 2] = [PICTURE_PURPOSE, AssetPurpose::ASSET_PURPOSE_STAMP];

impl Picture {
    /// Checks that a READY upload can be an emoji and builds its `Image`:
    /// the stills the media domain rendered, with the animated rendition as
    /// the main URL when there is one.
    ///
    /// Errors: `ASSET_NOT_IMAGE`, `IMAGE_TOO_LARGE`, `IMAGE_TOO_SMALL`.
    pub fn of(asset: &AssetRow, max_bytes: i64) -> AppResult<Self> {
        let animated = asset.has_kind(AssetKind::ASSET_KIND_ANIMATION);
        let image = asset.image().filter(|_| animated || asset.has_kind(AssetKind::ASSET_KIND_IMAGE));
        let Some(mut image) = image else {
            bail!(invalid, "ASSET_NOT_IMAGE", "an emoji is made from a still or animated image");
        };
        ensure!(
            asset.size_bytes <= max_bytes,
            invalid,
            "IMAGE_TOO_LARGE",
            "an emoji picture is at most {} KB",
            max_bytes / 1024
        );
        ensure!(
            image.width.max(image.height) >= MIN_IMAGE_EDGE,
            invalid,
            "IMAGE_TOO_SMALL",
            "an emoji picture is at least {MIN_IMAGE_EDGE} pixels on its longer side"
        );
        if animated {
            image.url.clone_from(&asset.url);
        }
        let image =
            serde_json::to_value(&image).map_err(|e| AppError::internal(anyhow::anyhow!("emoji image encode: {e}")))?;
        Ok(Self { asset_id: asset.id, image, animated })
    }
}

/// Who is looking at an emoji.
#[derive(Debug, Clone, Copy, Default)]
pub struct Viewer {
    pub user_id: Option<Uuid>,
    pub moderator: bool,
}

impl Viewer {
    pub fn user(user_id: Uuid) -> Self {
        Self { user_id: Some(user_id), moderator: false }
    }

    pub fn moderator(user_id: Uuid) -> Self {
        Self { user_id: Some(user_id), moderator: true }
    }

    pub fn is_author_of(&self, emoji: &EmojiRow) -> bool {
        self.user_id.is_some() && self.user_id == emoji.author_id
    }

    /// Approved emoji are public; the rest is between the author and the moderators.
    pub fn can_see(&self, emoji: &EmojiRow) -> bool {
        emoji.is_approved() || self.moderator || self.is_author_of(emoji)
    }

    /// Whether the moderators' notes on the emoji are shown.
    pub fn sees_review(&self, emoji: &EmojiRow) -> bool {
        self.moderator || self.is_author_of(emoji)
    }

    /// The author may change or withdraw what is not (or no longer) under
    /// a moderator's decision to publish.
    pub fn can_edit(&self, emoji: &EmojiRow) -> bool {
        self.is_author_of(emoji) && matches!(emoji.state, state::PENDING | state::REJECTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emoji(state: i16, author: Option<Uuid>) -> EmojiRow {
        EmojiRow {
            id: new_id(),
            name: "blobcat".into(),
            state,
            asset_id: new_id(),
            image: serde_json::json!({ "url": "https://media.example/img/a.webp", "width": 128, "height": 128 }),
            animated: false,
            category: None,
            aliases: vec!["cat".into()],
            license: String::new(),
            sensitive: false,
            local_only: false,
            author_id: author,
            review_note: String::new(),
            reviewed_at: None,
            approved_at: None,
            catalog_version: 0,
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn unapproved_emoji_are_between_author_and_moderators() {
        let author = new_id();
        let stranger = Viewer::user(new_id());
        for hidden in [state::PENDING, state::REJECTED, state::DISABLED] {
            let row = emoji(hidden, Some(author));
            assert!(!stranger.can_see(&row) && !Viewer::default().can_see(&row));
            assert!(Viewer::user(author).can_see(&row) && Viewer::user(author).sees_review(&row));
            assert!(Viewer::moderator(new_id()).can_see(&row));
        }
        let approved = emoji(state::APPROVED, Some(author));
        assert!(stranger.can_see(&approved) && Viewer::default().can_see(&approved));
        assert!(!stranger.sees_review(&approved));
    }

    #[test]
    fn authors_edit_only_what_is_waiting_or_rejected() {
        let author = new_id();
        let me = Viewer::user(author);
        assert!(me.can_edit(&emoji(state::PENDING, Some(author))));
        assert!(me.can_edit(&emoji(state::REJECTED, Some(author))));
        assert!(!me.can_edit(&emoji(state::APPROVED, Some(author))));
        assert!(!me.can_edit(&emoji(state::DISABLED, Some(author))));
        // A moderator edits through the admin service, not as an author.
        assert!(!Viewer::moderator(new_id()).can_edit(&emoji(state::PENDING, Some(author))));
        // An emoji without author belongs to nobody, signed in or not.
        assert!(!Viewer::default().is_author_of(&emoji(state::PENDING, None)));
    }

    #[test]
    fn the_compact_form_carries_what_pickers_need() {
        let mut row = emoji(state::APPROVED, None);
        row.category = Some("どうぶつ".into());
        row.approved_at = Some(now());
        let proto = row.to_proto();
        assert_eq!((proto.name.as_str(), proto.category.as_str()), ("blobcat", "どうぶつ"));
        assert_eq!(proto.image.as_option().map(|i| i.width), Some(128));
        assert!(proto.added_at.as_option().is_some());
        assert_eq!(row.shortcode(), ":blobcat:");
        assert_eq!(holder(row.id), format!("emoji:{}", row.id));
    }
}
