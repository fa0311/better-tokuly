//! Sounds: what a short is set to, and the page of shorts using it.
//!
//! A sound is attribution, not audio (the client mixed the audio into its
//! upload). There are two kinds:
//!
//! - an **original sound** is the audio of one short. Every short whose
//!   video has audio and that names no other sound gets one of its own.
//!   Other creators may use it while that short is visible to them and
//!   allows remixes; it outlives its short for as long as anything uses it;
//! - an **external track** is a title and an artist given as text. Shorts
//!   naming the same track (see [`text::track_key`]) share one sound.
//!
//! `use_count` counts public shorts only; it is maintained by
//! [`crate::listing`].
//!
//! A sound never tells a viewer more about its short than the short
//! itself would. Whether that short was deleted, made private, belongs to
//! a suspended account or to someone in a block with the viewer, the sound
//! looks the same: a page without an origin, not reusable. (If a block
//! made the sound itself vanish, "still there" versus "gone" would tell a
//! blocked viewer which of the two happened.)
use std::collections::HashMap;

use tokuly_core::prelude::*;
use tokuly_media::assets::image_from_json;
use tokuly_proto::{
    common::ChannelSummary,
    shorts::{Sound, SoundKind},
};

use crate::{
    access::{self, ShortFacts, Viewer},
    shorts::{ShortRow, Tx, short_columns},
    text,
};

pub const ORIGINAL: i32 = SoundKind::SOUND_KIND_ORIGINAL as i32;
pub const EXTERNAL: i32 = SoundKind::SOUND_KIND_EXTERNAL as i32;

/// What a creator sets a new short to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoundChoice {
    /// The short's own original sound, with the name the creator gave it
    /// (may be empty).
    Own { title: String },
    /// A sound that already exists.
    Existing(Uuid),
    /// A named track from outside.
    External { title: String, artist: String },
}

/// A [`SoundChoice`] that passed validation.
pub(crate) enum SoundPlan {
    Own { title: String },
    Existing(Uuid),
    External { title: String, artist: String, key: String },
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SoundRow {
    pub id: Uuid,
    pub kind: i32,
    pub title: String,
    pub artist: String,
    pub use_count: i64,
    /// Original sounds: the short the audio comes from, while it exists.
    pub origin_short_id: Option<Uuid>,
    // The origin short, as far as the sound needs it.
    pub origin_public_id: Option<String>,
    pub origin_channel_id: Option<Uuid>,
    pub origin_owner_id: Option<Uuid>,
    pub origin_status: Option<i32>,
    pub origin_visibility: Option<i32>,
    pub origin_suspended: Option<bool>,
    pub origin_allow_remix: Option<bool>,
    pub origin_cover: Option<serde_json::Value>,
}

macro_rules! sound_select {
    () => {
        "SELECT d.id, d.kind, d.title, d.artist, d.use_count, d.origin_short_id, o.short_id AS origin_public_id, \
         o.channel_id AS origin_channel_id, o.owner_id AS origin_owner_id, o.status AS origin_status, \
         o.visibility AS origin_visibility, o.suspended AS origin_suspended, o.allow_remix AS origin_allow_remix, \
         o.cover AS origin_cover \
         FROM short_sounds d LEFT JOIN shorts o ON o.id = d.origin_short_id"
    };
}

impl SoundRow {
    fn origin(&self) -> Option<ShortFacts> {
        Some(ShortFacts {
            owner_id: self.origin_owner_id?,
            status: self.origin_status?,
            visibility: self.origin_visibility?,
            suspended: self.origin_suspended?,
        })
    }

    /// Whether the viewer may open the short the sound comes from.
    fn origin_visible_to(&self, viewer: &Viewer) -> bool {
        self.origin().is_some_and(|origin| access::can_watch(viewer, &origin))
    }

    /// Whether the viewer may set a new short to this sound: any external
    /// track; an original sound while its short is visible to them and
    /// allows remixes (its own creator always may).
    pub fn reusable_by(&self, viewer: &Viewer) -> bool {
        if self.kind == EXTERNAL {
            return true;
        }
        self.origin_visible_to(viewer)
            && (self.origin_allow_remix == Some(true) || self.origin_owner_id.is_some_and(|owner| viewer.is(owner)))
    }

    /// The channel to show next to an original sound, if its short is visible.
    pub fn channel_id_for(&self, viewer: &Viewer) -> Option<Uuid> {
        self.origin_channel_id.filter(|_| self.origin_visible_to(viewer))
    }

    /// The sound as `viewer` sees it. `hidden_uses` is how many of its
    /// public shorts they do not get to see ([`hidden_uses`]).
    pub fn to_proto(&self, viewer: &Viewer, channels: &HashMap<Uuid, ChannelSummary>, hidden_uses: i64) -> Sound {
        let origin_visible = self.origin_visible_to(viewer);
        Sound {
            id: self.id.to_string(),
            kind: self.kind.into(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            channel: self.channel_id_for(viewer).and_then(|id| channels.get(&id).cloned()).into(),
            origin_short_id: self.origin_public_id.clone().filter(|_| origin_visible).unwrap_or_default(),
            use_count: (self.use_count - hidden_uses).max(0),
            cover: image_from_json(self.origin_cover.as_ref()).filter(|_| origin_visible).into(),
            reusable: self.reusable_by(viewer),
            ..Default::default()
        }
    }
}

fn not_found() -> AppError {
    AppError::not_found("SOUND_NOT_FOUND", "sound not found")
}

pub async fn find(core: &Core, sound_id: Uuid) -> AppResult<Option<SoundRow>> {
    Ok(sqlx::query_as(concat!(sound_select!(), " WHERE d.id = $1")).bind(sound_id).fetch_optional(&core.db).await?)
}

/// The sound a client named (`SOUND_NOT_FOUND` when there is none).
pub async fn find_named(core: &Core, reference: &str) -> AppResult<SoundRow> {
    let Ok(sound_id) = Uuid::parse_str(reference.trim()) else { return Err(not_found()) };
    find(core, sound_id).await?.ok_or_else(not_found)
}

/// How many of the public shorts using each sound are hidden from `viewer`
/// (their owner is in a block with them): what to take off `use_count` so
/// that the number matches the list the viewer gets.
pub async fn hidden_uses(core: &Core, viewer: &Viewer, sound_ids: &[Uuid]) -> AppResult<HashMap<Uuid, i64>> {
    if viewer.blocked.is_empty() || sound_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT s.sound_id, count(*) FROM shorts s
         WHERE s.sound_id = ANY($1) AND s.listed AND s.owner_id = ANY($2) GROUP BY s.sound_id",
    )
    .bind(sound_ids)
    .bind(&viewer.blocked)
    .fetch_all(&core.db)
    .await?;
    Ok(rows.into_iter().collect())
}

/// The sounds with the given IDs, keyed by ID. Unknown IDs are absent.
pub async fn find_many(core: &Core, ids: &[Uuid]) -> AppResult<HashMap<Uuid, SoundRow>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<SoundRow> =
        sqlx::query_as(concat!(sound_select!(), " WHERE d.id = ANY($1)")).bind(ids).fetch_all(&core.db).await?;
    Ok(rows.into_iter().map(|row| (row.id, row)).collect())
}

fn title_invalid(message: &'static str) -> AppError {
    AppError::invalid("SOUND_TITLE_INVALID", message)
}

/// Validates what a creator wants a new short set to.
pub(crate) async fn plan(core: &Core, creator: &Viewer, choice: &SoundChoice) -> AppResult<SoundPlan> {
    match choice {
        SoundChoice::Own { title } => {
            let title = text::clean_line(title);
            if !text::fits(&title, text::MAX_SOUND_CHARS) {
                return Err(title_invalid("a sound name has at most 100 characters"));
            }
            Ok(SoundPlan::Own { title })
        }
        SoundChoice::External { title, artist } => {
            let (title, artist) = (text::clean_line(title), text::clean_line(artist));
            if title.is_empty() || !text::fits(&title, text::MAX_SOUND_CHARS) {
                return Err(title_invalid("a track title has 1 to 100 characters"));
            }
            if !text::fits(&artist, text::MAX_SOUND_CHARS) {
                return Err(title_invalid("an artist name has at most 100 characters"));
            }
            let key = text::track_key(&title, &artist);
            Ok(SoundPlan::External { title, artist, key })
        }
        SoundChoice::Existing(sound_id) => {
            let sound = find(core, *sound_id).await?.ok_or_else(not_found)?;
            ensure!(
                sound.reusable_by(creator),
                denied,
                "SOUND_NOT_REUSABLE",
                "this sound cannot be used: its short is gone or does not allow it"
            );
            Ok(SoundPlan::Existing(sound.id))
        }
    }
}

/// Gives the freshly inserted short `short_id` its sound, creating the
/// sound row where the plan calls for one.
pub(crate) async fn materialize_in(tx: &mut Tx<'_>, plan: &SoundPlan, short_id: Uuid) -> AppResult<()> {
    let sound_id: Uuid = match plan {
        SoundPlan::Own { title } => {
            sqlx::query_scalar(
                "INSERT INTO short_sounds (kind, title, origin_short_id) VALUES (1, $1, $2) RETURNING id",
            )
            .bind(title)
            .bind(short_id)
            .fetch_one(&mut **tx)
            .await?
        }
        SoundPlan::Existing(sound_id) => *sound_id,
        SoundPlan::External { title, artist, key } => {
            // The first short to name a track decides how it is written.
            sqlx::query_scalar(
                "INSERT INTO short_sounds (kind, title, artist, track_key) VALUES (2, $1, $2, $3)
                 ON CONFLICT (track_key) DO UPDATE SET track_key = EXCLUDED.track_key
                 RETURNING id",
            )
            .bind(title)
            .bind(artist)
            .bind(key)
            .fetch_one(&mut **tx)
            .await?
        }
    };
    sqlx::query("UPDATE shorts SET sound_id = $2 WHERE id = $1")
        .bind(short_id)
        .bind(sound_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| {
            let error = AppError::from(e);
            if error.is_not_found() { not_found() } else { error }
        })?;
    Ok(())
}

/// Removes the original sound of `short_id` (its video turned out silent),
/// unless another short already uses it. The short's `sound_id` clears with it.
pub(crate) async fn drop_own_original_in(tx: &mut Tx<'_>, short_id: Uuid) -> AppResult<()> {
    sqlx::query(
        "DELETE FROM short_sounds d WHERE d.origin_short_id = $1
           AND NOT EXISTS (SELECT 1 FROM shorts s WHERE s.sound_id = d.id AND s.id <> $1)",
    )
    .bind(short_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Removes a sound that nothing refers to any more: no short uses it and
/// the short it came from (if it had one) is gone.
pub(crate) async fn drop_if_orphaned_in(tx: &mut Tx<'_>, sound_id: Uuid) -> AppResult<()> {
    sqlx::query(
        "DELETE FROM short_sounds d WHERE d.id = $1 AND d.origin_short_id IS NULL
           AND NOT EXISTS (SELECT 1 FROM shorts s WHERE s.sound_id = d.id)",
    )
    .bind(sound_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Escapes `%`, `_` and `\` so user text matches literally in a `LIKE`.
pub(crate) fn like_literal(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Sounds a creator could set a short to: those whose title or artist
/// contains `query` (all of them when it is empty), the most used first.
pub async fn suggest(core: &Core, viewer: &Viewer, query: &str, limit: i64) -> AppResult<Vec<SoundRow>> {
    let query = text::clean_line(query);
    let pattern = format!("%{}%", like_literal(&query));
    // Over-fetch: sounds the viewer may not use are dropped afterwards.
    let rows: Vec<SoundRow> = sqlx::query_as(concat!(
        sound_select!(),
        " WHERE d.use_count > 0 AND ($1 = '' OR d.title ILIKE $2 OR d.artist ILIKE $2)
          ORDER BY d.use_count DESC, d.id DESC LIMIT $3"
    ))
    .bind(&query)
    .bind(&pattern)
    .bind(limit * 3)
    .fetch_all(&core.db)
    .await?;
    Ok(rows.into_iter().filter(|sound| sound.reusable_by(viewer)).take(limit as usize).collect())
}

/// Public shorts using a sound, newest first.
pub async fn shorts_page(
    core: &Core,
    viewer: &Viewer,
    sound_id: Uuid,
    page: &Page,
) -> AppResult<(Vec<ShortRow>, tokuly_proto::common::PageInfo)> {
    let before: Option<(Time, Uuid)> = page.decode()?;
    let mut rows: Vec<ShortRow> = sqlx::query_as(concat!(
        "SELECT ",
        short_columns!(),
        " FROM shorts s WHERE s.sound_id = $1 AND ",
        access::public!(2),
        " AND ($3::timestamptz IS NULL OR (s.published_at, s.id) < ($3, $4))
          ORDER BY s.published_at DESC, s.id DESC LIMIT $5"
    ))
    .bind(sound_id)
    .bind(&viewer.blocked)
    .bind(before.map(|(at, _)| at))
    .bind(before.map(|(_, id)| id))
    .bind(page.fetch())
    .fetch_all(&core.db)
    .await?;
    let info = page.finish(&mut rows, |row| (row.published_at.unwrap_or(row.created_at), row.id));
    Ok((rows, info))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn original(owner: Uuid, visibility: i32, allow_remix: bool) -> SoundRow {
        SoundRow {
            id: Uuid::now_v7(),
            kind: ORIGINAL,
            title: String::new(),
            artist: String::new(),
            use_count: 3,
            origin_short_id: Some(Uuid::now_v7()),
            origin_public_id: Some("abc123DEF45".to_owned()),
            origin_channel_id: Some(Uuid::now_v7()),
            origin_owner_id: Some(owner),
            origin_status: Some(access::PUBLISHED),
            origin_visibility: Some(visibility),
            origin_suspended: Some(false),
            origin_allow_remix: Some(allow_remix),
            origin_cover: None,
        }
    }

    #[test]
    fn original_sounds_follow_the_short_they_come_from() {
        let owner = Uuid::now_v7();
        let stranger = Viewer { user_id: Some(Uuid::now_v7()), ..Default::default() };
        let me = Viewer { user_id: Some(owner), ..Default::default() };

        let open = original(owner, access::PUBLIC, true);
        assert!(open.reusable_by(&stranger));
        assert_eq!(open.to_proto(&stranger, &HashMap::new(), 0).origin_short_id, "abc123DEF45");

        let closed = original(owner, access::PUBLIC, false);
        assert!(!closed.reusable_by(&stranger));
        assert!(closed.reusable_by(&me), "creators may always reuse their own sound");

        // A short that went private keeps its sound page, without a way back to it.
        let private = original(owner, access::PRIVATE, true);
        assert!(!private.reusable_by(&stranger));
        let shown = private.to_proto(&stranger, &HashMap::new(), 0);
        assert!(shown.origin_short_id.is_empty() && !shown.reusable && shown.channel.as_option().is_none());
    }

    #[test]
    fn a_hidden_origin_looks_the_same_whatever_hid_it() {
        let owner = Uuid::now_v7();
        let blocked = Viewer { user_id: Some(Uuid::now_v7()), blocked: vec![owner], ..Default::default() };
        let anyone = Viewer::anonymous();
        let show = |sound: &SoundRow, viewer: &Viewer, hidden: i64| {
            let shown = sound.to_proto(viewer, &HashMap::new(), hidden);
            (
                shown.origin_short_id,
                shown.channel.as_option().is_some(),
                shown.cover.as_option().is_some(),
                shown.reusable,
                shown.use_count,
            )
        };

        // Deleted: the sound has no origin left.
        let mut deleted = original(owner, access::PUBLIC, true);
        (deleted.origin_short_id, deleted.origin_public_id, deleted.origin_owner_id, deleted.origin_status) =
            (None, None, None, None);
        deleted.use_count = 2;
        // Blocked: the origin is there and public, and counted as a use.
        let of_blocker = original(owner, access::PUBLIC, true);
        // Private, suspended, withdrawn.
        let private = original(owner, access::PRIVATE, true);
        let mut suspended = original(owner, access::PUBLIC, true);
        suspended.origin_suspended = Some(true);
        let mut withdrawn = original(owner, access::PUBLIC, true);
        withdrawn.origin_status = Some(access::WITHDRAWN);
        (suspended.use_count, withdrawn.use_count) = (2, 2);
        let mut private = private;
        private.use_count = 2;

        let gone = show(&deleted, &anyone, 0);
        assert_eq!(gone, (String::new(), false, false, false, 2));
        assert_eq!(show(&of_blocker, &blocked, 1), gone, "a block reads like a deletion");
        assert_eq!(show(&private, &anyone, 0), gone);
        assert_eq!(show(&suspended, &anyone, 0), gone);
        assert_eq!(show(&withdrawn, &anyone, 0), gone);
        // The owner of a withdrawn short sees it the same way.
        assert_eq!(show(&withdrawn, &Viewer { user_id: Some(owner), ..Default::default() }, 0), gone);

        let mut track = original(owner, access::PUBLIC, false);
        track.kind = EXTERNAL;
        (track.origin_short_id, track.origin_owner_id) = (None, None);
        assert!(track.reusable_by(&blocked));
    }

    #[test]
    fn like_patterns_match_user_text_literally() {
        assert_eq!(like_literal("100%_達成\\"), "100\\%\\_達成\\\\");
        assert_eq!(like_literal("夜に駆ける"), "夜に駆ける");
    }
}
