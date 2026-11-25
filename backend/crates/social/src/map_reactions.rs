//! Map reactions ("supersends"): hearts, eyes, drinks, an emoji or a stamp
//! thrown at friends' pins.
//!
//! A reaction is pushed over realtime right away and also stored until the
//! recipient's client acknowledges having played it, so a reaction sent to
//! someone offline plays when they next open the app.
use std::{collections::HashSet, time::Duration};

use chrono::{DateTime, Utc};
use tokuly_core::{
    outbound::{self, NotificationRequest},
    prelude::*,
};
use tokuly_identity::users;
use tokuly_proto::{
    common::{StampRef, UserSummary},
    social::{MapReaction, MapReactionKind, social_event::Event as Push},
};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    Social, copy,
    graph::{self, InteractionKind},
    people,
};

/// Friends one reaction can be thrown at (a whole gathering).
const MAX_RECIPIENTS: usize = 50;
/// Reactions one friend can receive from the same sender per minute. The
/// sender's own limit counts calls, and one call can reach 50 friends.
const MAX_PER_RECIPIENT_PER_MINUTE: u32 = 60;
/// Unplayed reactions returned at once; older ones follow after an ack.
const MAX_PENDING: i64 = 200;
const MAX_ACKS: usize = 500;
/// Unplayed reactions older than this are not replayed anymore.
pub(crate) const PENDING_DAYS: i32 = 7;
const MAX_EMOJI_GRAPHEMES: usize = 3;
const MAX_EMOJI_BYTES: usize = 64;

/// A validated reaction, ready to be stored for each recipient.
#[derive(Debug, PartialEq)]
struct Reaction {
    kind: MapReactionKind,
    /// Empty unless `kind` is EMOJI.
    emoji: String,
    /// Set only when `kind` is STAMP.
    stamp_id: Option<Uuid>,
    intensity: i32,
}

impl Reaction {
    fn parse(kind: Option<MapReactionKind>, emoji: &str, stamp_id: &str, intensity: i32) -> AppResult<Self> {
        let kind = kind
            .filter(|kind| *kind != MapReactionKind::MAP_REACTION_KIND_UNSPECIFIED)
            .ok_or_else(|| AppError::invalid("INVALID_REACTION_KIND", "choose a reaction"))?;
        let emoji = match kind {
            MapReactionKind::MAP_REACTION_KIND_EMOJI => {
                let emoji = emoji.trim();
                // Not a full emoji grammar, just enough to keep words out:
                // short, not plain ASCII (keycaps like 1️⃣ are fine), no
                // letters or spaces.
                let plausible = emoji.len() <= MAX_EMOJI_BYTES
                    && (1..=MAX_EMOJI_GRAPHEMES).contains(&emoji.graphemes(true).count())
                    && !emoji.is_ascii()
                    && !emoji.chars().any(|c| c.is_control() || c.is_ascii_alphabetic() || c.is_whitespace());
                ensure!(plausible, invalid, "INVALID_EMOJI", "send one to {MAX_EMOJI_GRAPHEMES} emoji");
                emoji.to_owned()
            }
            _ => String::new(),
        };
        let stamp_id = match kind {
            MapReactionKind::MAP_REACTION_KIND_STAMP => Some(parse_id(stamp_id)?),
            _ => None,
        };
        // 0 is what a client sends when it does not care: a plain tap.
        let intensity = if intensity == 0 { 1 } else { intensity };
        ensure!((1..=5).contains(&intensity), invalid, "INVALID_INTENSITY", "intensity is between 1 and 5");
        Ok(Self { kind, emoji, stamp_id, intensity })
    }
}

/// Counts one reaction to each recipient against the per-recipient limit,
/// all or nothing: when any of them is at the limit nothing is counted, so a
/// refused throw at a gathering does not use up the others' allowance.
const COUNT_EACH_OR_NONE: &str = r"
for _, key in ipairs(KEYS) do
    if tonumber(redis.call('GET', key) or '0') >= tonumber(ARGV[1]) then
        return 0
    end
end
for _, key in ipairs(KEYS) do
    if redis.call('INCR', key) == 1 then
        redis.call('EXPIRE', key, ARGV[2])
    end
end
return 1";

async fn ensure_room_for_each(core: &Core, user_id: Uuid, recipients: &[Uuid]) -> AppResult<()> {
    if !core.cache.enforces_rate_limits() {
        return Ok(());
    }
    let script = redis::Script::new(COUNT_EACH_OR_NONE);
    let mut script = script.prepare_invoke();
    for recipient in recipients {
        script.key(format!("rl:map-reaction:{user_id}:{recipient}"));
    }
    let counted: i64 = script.arg(MAX_PER_RECIPIENT_PER_MINUTE).arg(60).invoke_async(&mut core.cache.conn()).await?;
    ensure!(counted == 1, rate_limited, "RATE_LIMITED", "too many reactions to this friend, slow down");
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
struct ReactionRow {
    id: Uuid,
    from_user_id: Uuid,
    to_user_id: Uuid,
    kind: i32,
    emoji: String,
    stamp_id: Option<Uuid>,
    intensity: i32,
    created_at: DateTime<Utc>,
}

impl ReactionRow {
    fn into_proto(self, from: UserSummary) -> MapReaction {
        MapReaction {
            id: self.id.to_string(),
            from: from.into(),
            to_user_id: self.to_user_id.to_string(),
            kind: self.kind.into(),
            emoji: self.emoji,
            // Only the ID: clients resolve the stamp through the stamp domain.
            stamp: self.stamp_id.map(|id| StampRef { id: id.to_string(), ..Default::default() }).into(),
            intensity: self.intensity,
            created_at: ts(self.created_at),
            ..Default::default()
        }
    }
}

impl Social {
    /// Throws one reaction at each of `recipients`, who must all be friends
    /// of `user_id`. Returns the reactions as stored, one per recipient.
    pub(crate) async fn send_map_reaction(
        &self,
        user_id: Uuid,
        recipients: &[Uuid],
        kind: Option<MapReactionKind>,
        emoji: &str,
        stamp_id: &str,
        intensity: i32,
    ) -> AppResult<Vec<MapReaction>> {
        let core = &self.core;
        let reaction = Reaction::parse(kind, emoji, stamp_id, intensity)?;
        let mut recipients = recipients.to_vec();
        recipients.sort_unstable();
        recipients.dedup();
        ensure!(!recipients.is_empty(), invalid, "NO_RECIPIENTS", "choose at least one friend");
        ensure!(recipients.len() <= MAX_RECIPIENTS, invalid, "TOO_MANY_RECIPIENTS", "at most {MAX_RECIPIENTS} friends per reaction");
        let friends: i64 = sqlx::query_scalar("SELECT count(*) FROM friendships WHERE user_id = $1 AND friend_id = ANY($2)")
            .bind(user_id)
            .bind(&recipients)
            .fetch_one(&core.db)
            .await?;
        ensure!(friends as usize == recipients.len(), precondition, "NOT_FRIENDS", "map reactions can only be sent to friends");
        core.cache.rate_limit(&format!("map-reaction:{user_id}"), 120, Duration::from_secs(60)).await?;
        ensure_room_for_each(core, user_id, &recipients).await?;

        let me = tokuly_identity::summary(core, user_id).await?;
        let text = copy::map_reaction(&people::name_of(&me), reaction.kind, &reaction.emoji);
        let mut tx = core.db.begin().await?;
        let rows: Vec<ReactionRow> = sqlx::query_as(
            "INSERT INTO map_reactions (from_user_id, to_user_id, kind, emoji, stamp_id, intensity)
             SELECT $1, recipient, $3, $4, $5, $6 FROM unnest($2::uuid[]) AS r(recipient)
             RETURNING id, from_user_id, to_user_id, kind, emoji, stamp_id, intensity, created_at",
        )
        .bind(user_id)
        .bind(&recipients)
        .bind(reaction.kind as i32)
        .bind(&reaction.emoji)
        .bind(reaction.stamp_id)
        .bind(reaction.intensity)
        .fetch_all(&mut *tx)
        .await?;
        for recipient in &recipients {
            outbound::notify(
                &mut *tx,
                &NotificationRequest::new(*recipient, "social.map_reaction", text.title.clone(), text.body.clone(), "/map")
                    .actor(user_id)
                    .collapse(format!("social.map_reaction:{user_id}")),
            )
            .await?;
        }
        tx.commit().await?;

        let mut reactions = Vec::with_capacity(rows.len());
        for row in rows {
            let recipient = row.to_user_id;
            let reaction = row.into_proto(me.clone());
            self.push(recipient, Push::MapReaction(Box::new(reaction.clone()))).await;
            if let Err(e) = graph::record_interaction(core, user_id, recipient, InteractionKind::MapReaction, 1).await {
                tracing::warn!(error = %e, "map reaction not counted towards the best-friend ranking");
            }
            reactions.push(reaction);
        }
        Ok(reactions)
    }

    /// Reactions sent to `user_id` that their client has not played yet,
    /// oldest first.
    pub(crate) async fn pending_map_reactions(&self, user_id: Uuid) -> AppResult<Vec<MapReaction>> {
        let core = &self.core;
        // Someone blocked since they sent a reaction should not pop up now.
        let hidden = graph::blocked_ids_either(core, user_id).await?;
        let rows: Vec<ReactionRow> = sqlx::query_as(
            "SELECT id, from_user_id, to_user_id, kind, emoji, stamp_id, intensity, created_at FROM map_reactions
             WHERE to_user_id = $1 AND acked_at IS NULL AND created_at > now() - make_interval(days => $2)
               AND NOT (from_user_id = ANY($3))
             ORDER BY id
             LIMIT $4",
        )
        .bind(user_id)
        .bind(PENDING_DAYS)
        .bind(&hidden)
        .bind(MAX_PENDING)
        .fetch_all(&core.db)
        .await?;
        let senders: Vec<Uuid> = rows.iter().map(|r| r.from_user_id).collect::<HashSet<_>>().into_iter().collect();
        let summaries = tokuly_identity::summaries(core, &senders).await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let from = users::pick(&summaries, row.from_user_id);
                row.into_proto(from)
            })
            .collect())
    }

    /// Marks reactions addressed to `user_id` as played. IDs of other
    /// people's reactions (or unknown ones) are ignored.
    pub(crate) async fn ack_map_reactions(&self, user_id: Uuid, reaction_ids: &[Uuid]) -> AppResult<()> {
        ensure!(reaction_ids.len() <= MAX_ACKS, invalid, "TOO_MANY_IDS", "at most {MAX_ACKS} reactions per request");
        sqlx::query("UPDATE map_reactions SET acked_at = now() WHERE to_user_id = $1 AND id = ANY($2) AND acked_at IS NULL")
            .bind(user_id)
            .bind(reaction_ids)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use MapReactionKind::*;

    #[test]
    fn plain_kinds_ignore_emoji_and_stamp() {
        let reaction = Reaction::parse(Some(MAP_REACTION_KIND_HEARTS), "🍜", "not-an-id", 0).unwrap();
        assert_eq!(reaction, Reaction { kind: MAP_REACTION_KIND_HEARTS, emoji: String::new(), stamp_id: None, intensity: 1 });
    }

    #[test]
    fn emoji_reactions_need_a_real_emoji() {
        assert_eq!(Reaction::parse(Some(MAP_REACTION_KIND_EMOJI), " 🍜 ", "", 3).unwrap().emoji, "🍜");
        assert!(Reaction::parse(Some(MAP_REACTION_KIND_EMOJI), "👨‍👩‍👧🎉🔥", "", 1).is_ok(), "three grapheme clusters");
        assert!(Reaction::parse(Some(MAP_REACTION_KIND_EMOJI), "1️⃣", "", 1).is_ok(), "keycap emoji start with an ASCII digit");
        for bad in ["", "   ", "hello", "🍜🍜🍜🍜", "🍜 🍜", "a", "123", "a🍜"] {
            let err = Reaction::parse(Some(MAP_REACTION_KIND_EMOJI), bad, "", 1).unwrap_err();
            assert_eq!(err.reason, "INVALID_EMOJI", "{bad:?}");
        }
    }

    #[test]
    fn stamp_reactions_need_a_stamp_id() {
        let stamp = Uuid::now_v7();
        assert_eq!(Reaction::parse(Some(MAP_REACTION_KIND_STAMP), "", &stamp.to_string(), 2).unwrap().stamp_id, Some(stamp));
        assert_eq!(Reaction::parse(Some(MAP_REACTION_KIND_STAMP), "", "", 2).unwrap_err().reason, "INVALID_ID");
    }

    #[test]
    fn kind_and_intensity_are_checked() {
        assert_eq!(Reaction::parse(None, "", "", 1).unwrap_err().reason, "INVALID_REACTION_KIND");
        assert_eq!(Reaction::parse(Some(MAP_REACTION_KIND_UNSPECIFIED), "", "", 1).unwrap_err().reason, "INVALID_REACTION_KIND");
        assert_eq!(Reaction::parse(Some(MAP_REACTION_KIND_EYES), "", "", 6).unwrap_err().reason, "INVALID_INTENSITY");
        assert_eq!(Reaction::parse(Some(MAP_REACTION_KIND_EYES), "", "", -1).unwrap_err().reason, "INVALID_INTENSITY");
        assert_eq!(Reaction::parse(Some(MAP_REACTION_KIND_DRINKS), "", "", 5).unwrap().intensity, 5);
    }
}
