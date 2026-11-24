//! Rows to API messages: a [`ShortRow`] as one viewer sees it, with its
//! channel, sound, remix source and what the viewer did with it.
//!
//! Callers decide *whether* the viewer may see a row ([`crate::access`]);
//! this module decides *what* of it they see: playback only once published,
//! studio details only for the owner, a sound's or a remix's origin only
//! while that short is visible to them.
//!
//! Counts of other shorts (`remix_count`, a sound's `use_count`) are
//! counts of what the viewer could list: public shorts whose owner is in a
//! block with them are taken off, so that a number never disagrees with
//! the list behind it and gives a block away.
use std::collections::{HashMap, HashSet};

use tokuly_core::prelude::*;
use tokuly_proto::{
    common::SubjectRef,
    shorts::{Mention, RemixSource, Short, ShortStudioInfo, ShortViewerState},
};
use tokuly_video::channels;

use crate::{
    access::{self, Viewer},
    shorts::{self, COVER_FRAME, ShortRow},
    signals, sounds,
};

/// What a signed-in viewer did with the shorts on a page.
#[derive(Debug, Default)]
struct ViewerMarks {
    liked: HashSet<Uuid>,
    saved: HashSet<Uuid>,
    hidden: HashSet<Uuid>,
    seen: HashSet<Uuid>,
    /// Channels the viewer follows.
    followed: HashSet<Uuid>,
}

async fn viewer_marks(core: &Core, user_id: Uuid, rows: &[ShortRow]) -> AppResult<ViewerMarks> {
    let short_ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let channel_ids: Vec<Uuid> = rows.iter().map(|row| row.channel_id).collect();
    let found: Vec<(i32, Uuid)> = sqlx::query_as(
        "SELECT 1, short_id FROM short_likes WHERE user_id = $1 AND short_id = ANY($2) AND liked
         UNION ALL SELECT 2, short_id FROM short_saves WHERE user_id = $1 AND short_id = ANY($2)
         UNION ALL SELECT 3, short_id FROM short_hidden WHERE user_id = $1 AND short_id = ANY($2)
         UNION ALL SELECT 4, channel_id FROM short_follows WHERE user_id = $1 AND channel_id = ANY($3)",
    )
    .bind(user_id)
    .bind(&short_ids)
    .bind(&channel_ids)
    .fetch_all(&core.db)
    .await?;
    let mut marks = ViewerMarks { seen: signals::seen_among(core, user_id, &short_ids).await?, ..Default::default() };
    for (kind, id) in found {
        match kind {
            1 => marks.liked.insert(id),
            2 => marks.saved.insert(id),
            3 => marks.hidden.insert(id),
            _ => marks.followed.insert(id),
        };
    }
    Ok(marks)
}

/// The shorts in `rows`, in order, as `viewer` sees them. A row whose
/// channel no longer exists (it is being deleted) is left out.
pub async fn shorts(core: &Core, viewer: &Viewer, rows: &[ShortRow]) -> AppResult<Vec<Short>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut sound_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.sound_id).collect();
    sound_ids.sort_unstable();
    sound_ids.dedup();
    let sound_rows = sounds::find_many(core, &sound_ids).await?;
    let hidden_sound_uses = sounds::hidden_uses(core, viewer, &sound_ids).await?;
    let hidden_remixes = hidden_remixes(core, viewer, rows).await?;

    let mut original_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.remix_of_id).collect();
    original_ids.sort_unstable();
    original_ids.dedup();
    // Only originals the viewer may open are shown.
    let mut originals: Vec<ShortRow> = shorts::find_many(core, &original_ids)
        .await?
        .into_iter()
        .filter(|original| access::can_watch(viewer, &original.facts()))
        .collect();
    viewer.drop_newly_locked(core, &mut originals, ShortRow::lock_facts).await?;
    let originals: HashMap<Uuid, ShortRow> = originals.into_iter().map(|original| (original.id, original)).collect();

    let mut channel_ids: Vec<Uuid> = rows
        .iter()
        .map(|row| row.channel_id)
        .chain(sound_rows.values().filter_map(|sound| sound.channel_id_for(viewer)))
        .chain(originals.values().map(|original| original.channel_id))
        .collect();
    channel_ids.sort_unstable();
    channel_ids.dedup();
    let channel_cards = channels::summaries(core, &channel_ids).await?;

    let marks = match viewer.user_id {
        Some(user_id) => Some(viewer_marks(core, user_id, rows).await?),
        None => None,
    };

    Ok(rows
        .iter()
        .filter_map(|row| {
            let channel = channel_cards.get(&row.channel_id)?.clone();
            let is_owner = viewer.is(row.owner_id);
            let viewer_state = marks.as_ref().map(|marks| ShortViewerState {
                is_owner,
                liked: marks.liked.contains(&row.id),
                saved: marks.saved.contains(&row.id),
                subscribed: marks.followed.contains(&row.channel_id),
                seen: marks.seen.contains(&row.id),
                not_interested: marks.hidden.contains(&row.id),
                ..Default::default()
            });
            let studio = is_owner.then(|| ShortStudioInfo {
                asset_id: row.asset_id.to_string(),
                processing_error: row.processing_error.clone(),
                cover_offset_ms: row.cover_offset_ms.unwrap_or(0),
                cover_pending: row.cover_kind == COVER_FRAME && row.cover_asset_id.is_none(),
                notify_subscribers: row.notify_subscribers,
                circle_id: row.circle_id.map(|id| id.to_string()).unwrap_or_default(),
                ..Default::default()
            });
            let remix_of = (row.remix_kind != 0).then(|| {
                let original = row.remix_of_id.and_then(|id| originals.get(&id));
                RemixSource {
                    kind: row.remix_kind.into(),
                    short_id: original.map(|original| original.short_id.clone()).unwrap_or_default(),
                    channel: original.and_then(|original| channel_cards.get(&original.channel_id).cloned()).into(),
                    cover: original.and_then(ShortRow::cover).into(),
                    ..Default::default()
                }
            });
            let source = row.source_id.filter(|_| row.source_type != 0).map(|id| SubjectRef {
                r#type: row.source_type.into(),
                id: id.to_string(),
                ..Default::default()
            });
            Some(Short {
                id: row.id.to_string(),
                short_id: row.short_id.clone(),
                channel: Some(channel).into(),
                caption: row.caption.clone(),
                hashtags: row.hashtags.clone(),
                mentions: row
                    .mentions()
                    .into_iter()
                    .map(|mention| Mention {
                        handle: mention.handle,
                        user_id: mention.user_id.to_string(),
                        ..Default::default()
                    })
                    .collect(),
                cover: row.cover().into(),
                cover_kind: row.cover_kind.into(),
                playback: if row.is_published() { row.playback().into() } else { None.into() },
                duration_seconds: row.duration_seconds,
                width: row.width,
                height: row.height,
                visibility: tokuly_video::access::shown_visibility(row.visibility, is_owner).into(),
                status: row.status.into(),
                comments_enabled: row.comments_enabled,
                allow_remix: row.allow_remix,
                allow_duet: row.allow_duet,
                sound: row
                    .sound_id
                    .and_then(|id| sound_rows.get(&id))
                    .map(|sound| {
                        let hidden = hidden_sound_uses.get(&sound.id).copied().unwrap_or(0);
                        sound.to_proto(viewer, &channel_cards, hidden)
                    })
                    .into(),
                remix_of: remix_of.into(),
                source: source.into(),
                view_count: row.view_count,
                like_count: row.like_count,
                comment_count: row.comment_count,
                share_count: row.share_count,
                save_count: row.save_count,
                pinned: row.pinned_at.is_some(),
                created_at: ts(row.created_at),
                published_at: opt_ts(row.published_at),
                viewer: viewer_state.into(),
                studio: studio.into(),
                share_url: core.web_url(&row.path()),
                remix_count: (row.remix_count - hidden_remixes.get(&row.id).copied().unwrap_or(0)).max(0),
                ..Default::default()
            })
        })
        .collect())
}

/// How many of the public remixes of each short are hidden from `viewer`
/// (their owner is in a block with them).
async fn hidden_remixes(core: &Core, viewer: &Viewer, rows: &[ShortRow]) -> AppResult<HashMap<Uuid, i64>> {
    let remixed: Vec<Uuid> = rows.iter().filter(|row| row.remix_count > 0).map(|row| row.id).collect();
    if viewer.blocked.is_empty() || remixed.is_empty() {
        return Ok(HashMap::new());
    }
    let hidden: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT s.remix_of_id, count(*) FROM shorts s
         WHERE s.remix_of_id = ANY($1) AND s.listed AND s.owner_id = ANY($2) GROUP BY s.remix_of_id",
    )
    .bind(&remixed)
    .bind(&viewer.blocked)
    .fetch_all(&core.db)
    .await?;
    Ok(hidden.into_iter().collect())
}

/// One short as `viewer` sees it (`SHORT_NOT_FOUND` when its channel is gone).
pub async fn short(core: &Core, viewer: &Viewer, row: &ShortRow) -> AppResult<Short> {
    shorts(core, viewer, std::slice::from_ref(row)).await?.pop().ok_or_else(shorts::not_found)
}
