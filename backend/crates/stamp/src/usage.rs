//! The API other domains call: resolve stamps, check that a user may send
//! one, and record that they did.
//!
//! A domain that lets users send stamps (chat, live chat, comments, notes,
//! map reactions) does three things:
//!
//! 1. [`ensure_usable`] before accepting the message — it returns the
//!    [`StampRef`] to embed;
//! 2. [`record_send`] once the message is stored;
//! 3. [`stamp_refs`] when rendering stored stamp IDs later.
use std::collections::HashMap;

use tokuly_core::{analytics::AnalyticsEvent, prelude::*};
use tokuly_proto::{common::StampRef, stamp::UsageContext};

use crate::{access::Viewer, model::status, stamps};

/// Stamps a user's "recently used" list remembers.
pub const RECENTS_KEPT: i64 = 64;

/// The time zone statistics days are counted in. Tokuly is a Japanese
/// product: a creator's "today" should end at midnight in Japan.
pub const STATS_TIME_ZONE: &str = "Asia/Tokyo";

/// Where a stamp was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SendContext {
    Chat,
    LiveChat,
    Comment,
    Note,
    /// A map reaction thrown at a friend's pin.
    Map,
}

impl SendContext {
    /// The value stored in `stamp_usage.context` (`tokuly.stamp.v1.UsageContext`).
    pub fn code(self) -> i32 {
        let context = match self {
            Self::Chat => UsageContext::USAGE_CONTEXT_CHAT,
            Self::LiveChat => UsageContext::USAGE_CONTEXT_LIVE_CHAT,
            Self::Comment => UsageContext::USAGE_CONTEXT_COMMENT,
            Self::Note => UsageContext::USAGE_CONTEXT_NOTE,
            Self::Map => UsageContext::USAGE_CONTEXT_MAP,
        };
        context as i32
    }

    /// Name used in analytics events.
    pub fn name(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::LiveChat => "live_chat",
            Self::Comment => "comment",
            Self::Note => "note",
            Self::Map => "map",
        }
    }
}

#[derive(sqlx::FromRow)]
struct RefRow {
    id: Uuid,
    pack_id: Option<Uuid>,
    name: String,
    image_url: String,
    animation_url: String,
    format: i32,
    width: i32,
    height: i32,
}

/// Resolves stamp IDs stored in messages, comments, notes or reactions.
///
/// Stamps that were deleted, and stamps of a pack moderation took down, are
/// absent from the result: render those as "this stamp is no longer
/// available". No permission is checked — whoever can see the message can
/// see the stamp in it.
pub async fn stamp_refs(core: &Core, stamp_ids: &[Uuid]) -> AppResult<HashMap<Uuid, StampRef>> {
    if stamp_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<RefRow> = sqlx::query_as(
        "SELECT s.id, s.pack_id, s.name, s.image_url, s.animation_url, s.format, s.width, s.height
         FROM stamps s LEFT JOIN stamp_packs p ON p.id = s.pack_id
         WHERE s.id = ANY($1) AND (s.pack_id IS NULL OR p.status <> $2)",
    )
    .bind(stamp_ids)
    .bind(status::REMOVED)
    .fetch_all(&core.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let stamp = StampRef {
                id: row.id.to_string(),
                pack_id: row.pack_id.map(|id| id.to_string()).unwrap_or_default(),
                name: row.name,
                image_url: row.image_url,
                animation_url: row.animation_url,
                format: row.format.into(),
                width: row.width,
                height: row.height,
                ..Default::default()
            };
            (row.id, stamp)
        })
        .collect())
}

/// Checks that `user_id` may send `stamp_id` and returns the stamp to embed.
///
/// A user may send their own personal stamps, the stamps of packs they
/// made, installed or bought, and the stamps of free packs in the store
/// (official ones included). Everything else — unknown stamps too, so
/// nothing leaks — fails with `STAMP_NOT_USABLE`.
pub async fn ensure_usable(core: &Core, user_id: Uuid, stamp_id: Uuid) -> AppResult<StampRef> {
    let view = stamps::view(core, Some(user_id), stamp_id).await?;
    match view.filter(|v| v.usable_by(&Viewer::user(user_id))) {
        Some(view) => Ok(view.stamp_ref()),
        None => {
            Err(AppError::precondition("STAMP_NOT_USABLE", "you cannot send this stamp; install or buy its pack first"))
        }
    }
}

#[derive(sqlx::FromRow)]
struct Recorded {
    pack_id: Option<Uuid>,
    creator_id: Option<Uuid>,
    first_use: bool,
}

/// Records that `user_id` sent `stamp_id`: the usage log, the user's
/// recently used stamps, the stamp's send counter and the pack's numbers of
/// the day. Call it after the message is stored; a stamp that disappeared in
/// the meantime is silently ignored.
///
/// A creator sending stamps of their own pack is logged like anyone else
/// but does not move the counters: statistics and rankings measure the
/// audience, and must not be something a creator can inflate alone.
///
/// One statement does all of it. The counter rows are touched in a fixed
/// order (stamp, then the pack's day), so concurrent sends cannot deadlock.
pub async fn record_send(core: &Core, user_id: Uuid, stamp_id: Uuid, context: SendContext) -> AppResult<()> {
    let code = context.code();
    let recorded: Option<Recorded> = sqlx::query_as(
        "WITH target AS (
             -- The key lock keeps the stamp from being deleted under the rows written below.
             SELECT s.id, s.pack_id, p.creator_id
             FROM stamps s LEFT JOIN stamp_packs p ON p.id = s.pack_id
             WHERE s.id = $2
             FOR KEY SHARE OF s
         ), counted AS (
             UPDATE stamps s SET send_count = s.send_count + 1
             FROM target
             WHERE s.id = target.id AND target.creator_id IS DISTINCT FROM $1
             RETURNING s.pack_id
         ), day AS (
             INSERT INTO stamp_pack_daily (pack_id, day, chat_sends, live_chat_sends, comment_sends, note_sends, map_sends)
             SELECT pack_id, (now() AT TIME ZONE $4)::date,
                    ($3 = 1)::int, ($3 = 2)::int, ($3 = 3)::int, ($3 = 4)::int, ($3 = 5)::int
             FROM counted WHERE pack_id IS NOT NULL
             ON CONFLICT (pack_id, day) DO UPDATE SET
                 chat_sends      = stamp_pack_daily.chat_sends + EXCLUDED.chat_sends,
                 live_chat_sends = stamp_pack_daily.live_chat_sends + EXCLUDED.live_chat_sends,
                 comment_sends   = stamp_pack_daily.comment_sends + EXCLUDED.comment_sends,
                 note_sends      = stamp_pack_daily.note_sends + EXCLUDED.note_sends,
                 map_sends       = stamp_pack_daily.map_sends + EXCLUDED.map_sends
         ), logged AS (
             INSERT INTO stamp_usage (stamp_id, user_id, context) SELECT id, $1, $3 FROM target
         ), recent AS (
             INSERT INTO stamp_recents (user_id, stamp_id) SELECT $1, id FROM target
             ON CONFLICT (user_id, stamp_id) DO UPDATE SET used_at = now(), use_count = stamp_recents.use_count + 1
             RETURNING (xmax = 0) AS first_use
         )
         SELECT target.pack_id, target.creator_id, recent.first_use
         FROM target CROSS JOIN recent",
    )
    .bind(user_id)
    .bind(stamp_id)
    .bind(code)
    .bind(STATS_TIME_ZONE)
    .fetch_optional(&core.db)
    .await?;
    let Some(recorded) = recorded else { return Ok(()) };

    // The list only grows when a stamp is used for the first time, so that is
    // the only moment it may need trimming.
    if recorded.first_use {
        sqlx::query(
            "DELETE FROM stamp_recents WHERE user_id = $1 AND stamp_id IN (
                 SELECT stamp_id FROM stamp_recents WHERE user_id = $1 ORDER BY used_at DESC OFFSET $2)",
        )
        .bind(user_id)
        .bind(RECENTS_KEPT)
        .execute(&core.db)
        .await?;
    }

    let mut event = AnalyticsEvent::new("stamp.send")
        .user(Some(user_id))
        .subject("stamp", stamp_id)
        .props(serde_json::json!({ "context": context.name(), "pack_id": recorded.pack_id }));
    if let Some(creator_id) = recorded.creator_id {
        event = event.owner(creator_id);
    }
    core.analytics.track(event).await;
    Ok(())
}

const _: () = {
    // `record_send` compares the context code with literals in SQL.
    assert!(UsageContext::USAGE_CONTEXT_CHAT as i32 == 1);
    assert!(UsageContext::USAGE_CONTEXT_LIVE_CHAT as i32 == 2);
    assert!(UsageContext::USAGE_CONTEXT_COMMENT as i32 == 3);
    assert!(UsageContext::USAGE_CONTEXT_NOTE as i32 == 4);
    assert!(UsageContext::USAGE_CONTEXT_MAP as i32 == 5);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contexts_map_to_distinct_codes_and_names() {
        let all = [SendContext::Chat, SendContext::LiveChat, SendContext::Comment, SendContext::Note, SendContext::Map];
        let codes: std::collections::HashSet<i32> = all.iter().map(|c| c.code()).collect();
        let names: std::collections::HashSet<&str> = all.iter().map(|c| c.name()).collect();
        assert_eq!((codes.len(), names.len()), (5, 5));
        assert_eq!(SendContext::Chat.code(), 1);
        assert_eq!(SendContext::Map.code(), 5);
    }
}
