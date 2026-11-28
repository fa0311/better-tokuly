//! Reactions to moments (a RealMoji or a stamp, one per person and
//! moment) and every account's RealMoji library.
//!
//! A reaction keeps the picture it was made with: the library entry's
//! asset is held by the reaction too, so changing or deleting the entry
//! later changes nothing that was sent. A replaced or taken back reaction
//! stays on record (`removed_at`) for the evidence period: it was seen,
//! and the owner may still report it.
use std::time::Duration;

use tokuly_core::{outbound, prelude::*};
use tokuly_media::assets::{self, image_from_json};
use tokuly_proto::{
    media::AssetPurpose,
    moments::{
        MomentReaction, ReactRequest, ReactionsChanged, Realmoji, RealmojiEmoji, moments_event::Event as Push,
        react_request::Reaction,
    },
};
use tokuly_social::graph;
use tokuly_stamp::SendContext;

use crate::{
    Moments, copy,
    model::{REACTION_COLUMNS, ReactionRow, reaction_holder, realmoji_holder},
    rules,
    viewing::reaction_view,
};

const SELFIE: &[AssetPurpose] = &[AssetPurpose::ASSET_PURPOSE_REALMOJI];

/// What a new reaction is made of.
enum Made {
    Realmoji { emoji: i16, asset_id: Uuid, image: serde_json::Value },
    Stamp(Uuid),
}

/// The keyset of a moment's reactions: newest first.
#[derive(serde::Serialize, serde::Deserialize)]
struct Cursor {
    created_at: Time,
    id: Uuid,
}

impl Moments {
    pub(crate) async fn react(&self, user_id: Uuid, request: ReactRequest) -> AppResult<MomentReaction> {
        let core = &self.core;
        core.cache.rate_limit(&format!("moments-react:{user_id}"), 60, Duration::from_secs(600)).await?;
        let moment = self.require_visible(user_id, parse_id(&request.moment_id)?).await?;
        ensure!(moment.user_id != user_id, precondition, "OWN_MOMENT", "you cannot react to your own moment");
        ensure!(self.unlocked(user_id).await?, precondition, "MOMENT_LOCKED", "post your own moment first");
        let made = match request.reaction {
            Some(Reaction::Realmoji(emoji)) => match emoji.as_known() {
                Some(RealmojiEmoji::REALMOJI_EMOJI_INSTANT) => {
                    let asset_id = parse_id(&request.asset_id)?;
                    let image = assets::require_image_json(core, asset_id, user_id, SELFIE).await?;
                    Made::Realmoji { emoji: RealmojiEmoji::REALMOJI_EMOJI_INSTANT as i16, asset_id, image }
                }
                face => {
                    let emoji = rules::library_emoji(face)? as i16;
                    let (asset_id, image): (Uuid, serde_json::Value) = sqlx::query_as(
                        "SELECT asset_id, image FROM moment_realmojis WHERE user_id = $1 AND emoji = $2",
                    )
                    .bind(user_id)
                    .bind(emoji)
                    .fetch_optional(&core.db)
                    .await?
                    .ok_or_else(|| AppError::precondition("NO_REALMOJI", "take a RealMoji for this face first"))?;
                    Made::Realmoji { emoji, asset_id, image }
                }
            },
            Some(Reaction::StampId(stamp_id)) => {
                let stamp_id = parse_id(&stamp_id)?;
                tokuly_stamp::ensure_usable(core, user_id, stamp_id).await?;
                Made::Stamp(stamp_id)
            }
            None => bail!(invalid, "INVALID_REACTION", "a RealMoji or a stamp is required"),
        };
        // At most a few notifications an hour from one person to another.
        let notify_owner = core
            .cache
            .rate_limit(&format!("moments-reacted:{user_id}:{}", moment.user_id), 10, Duration::from_secs(3600))
            .await
            .is_ok();
        let reactor = tokuly_identity::summary(core, user_id).await?;

        let mut tx = core.db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("moments:react:{}:{user_id}", moment.id))
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE moment_reactions SET removed_at = now() WHERE moment_id = $1 AND user_id = $2 AND removed_at IS NULL")
            .bind(moment.id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        let (realmoji, image, asset_id, stamp_id) = match &made {
            Made::Realmoji { emoji, asset_id, image } => (Some(*emoji), Some(image), Some(*asset_id), None),
            Made::Stamp(stamp_id) => (None, None, None, Some(*stamp_id)),
        };
        let row: ReactionRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO moment_reactions AS r (moment_id, user_id, realmoji, image, asset_id, stamp_id)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING {REACTION_COLUMNS}"
        )))
        .bind(moment.id)
        .bind(user_id)
        .bind(realmoji)
        .bind(image)
        .bind(asset_id)
        .bind(stamp_id)
        .fetch_one(&mut *tx)
        .await?;
        if let Some(asset_id) = asset_id {
            assets::attach(&mut *tx, asset_id, &reaction_holder(row.id)).await?;
        }
        if notify_owner {
            outbound::notify(&mut *tx, &copy::reaction(moment.user_id, user_id, &reactor.display_name, moment.id))
                .await?;
        }
        tx.commit().await?;

        if let Made::Stamp(stamp_id) = made
            && let Err(e) = tokuly_stamp::record_send(core, user_id, stamp_id, SendContext::Comment).await
        {
            tracing::warn!(error = %e, "recording a stamp sent as a moment reaction failed");
        }
        self.tell_reactions_changed(moment.id, moment.user_id).await?;
        let names = [(user_id, reactor)].into_iter().collect();
        let stamps = self.stamps_of([&row]).await?;
        Ok(reaction_view(&row, &names, &stamps))
    }

    /// Takes the caller's reaction back, whether or not they still see the
    /// moment.
    pub(crate) async fn remove_reaction(&self, user_id: Uuid, moment_id: Uuid) -> AppResult<()> {
        let owner: Option<Uuid> = sqlx::query_scalar(
            "UPDATE moment_reactions r SET removed_at = now() FROM moments m
             WHERE r.moment_id = $1 AND r.user_id = $2 AND r.removed_at IS NULL AND m.id = r.moment_id
             RETURNING m.user_id",
        )
        .bind(moment_id)
        .bind(user_id)
        .fetch_optional(&self.core.db)
        .await?;
        if let Some(owner) = owner {
            self.tell_reactions_changed(moment_id, owner).await?;
        }
        Ok(())
    }

    /// The reactions on a moment the caller may see, newest first.
    pub(crate) async fn list_reactions(
        &self,
        user_id: Uuid,
        moment_id: Uuid,
        page: Page,
    ) -> AppResult<(Vec<MomentReaction>, tokuly_proto::common::PageInfo)> {
        let core = &self.core;
        core.cache.rate_limit(&format!("moments-read:{user_id}"), 240, Duration::from_secs(60)).await?;
        let moment = self.require_visible(user_id, moment_id).await?;
        ensure!(
            moment.user_id == user_id || self.unlocked(user_id).await?,
            precondition,
            "MOMENT_LOCKED",
            "post your own moment first"
        );
        let after: Option<Cursor> = page.decode()?;
        let hidden = graph::blocked_ids_either(core, user_id).await?;
        let mut rows: Vec<ReactionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {REACTION_COLUMNS} FROM moment_reactions r
             JOIN users u ON u.id = r.user_id AND u.deleted_at IS NULL AND u.banned_at IS NULL
             WHERE r.moment_id = $1 AND r.removed_at IS NULL AND r.user_id <> ALL($2)
               AND ($3::timestamptz IS NULL OR (r.created_at, r.id) < ($3, $4))
             ORDER BY r.created_at DESC, r.id DESC
             LIMIT $5"
        )))
        .bind(moment.id)
        .bind(&hidden)
        .bind(after.as_ref().map(|cursor| cursor.created_at))
        .bind(after.as_ref().map(|cursor| cursor.id))
        .bind(page.fetch())
        .fetch_all(&core.db)
        .await?;
        let info = page.finish(&mut rows, |r| Cursor { created_at: r.created_at, id: r.id });
        let people: Vec<Uuid> = rows.iter().map(|r| r.user_id).collect();
        let names = tokuly_identity::summaries(core, &people).await?;
        let stamps = self.stamps_of(&rows).await?;
        Ok((rows.iter().map(|r| reaction_view(r, &names, &stamps)).collect(), info))
    }

    /// Pushes the owner the number of reactions they see now.
    pub(crate) async fn tell_reactions_changed(&self, moment_id: Uuid, owner_id: Uuid) -> AppResult<()> {
        let hidden = graph::blocked_ids_either(&self.core, owner_id).await?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM moment_reactions r
             JOIN users u ON u.id = r.user_id AND u.deleted_at IS NULL AND u.banned_at IS NULL
             WHERE r.moment_id = $1 AND r.removed_at IS NULL AND r.user_id <> ALL($2)",
        )
        .bind(moment_id)
        .bind(&hidden)
        .fetch_one(&self.core.db)
        .await?;
        let changed =
            ReactionsChanged { moment_id: moment_id.to_string(), reaction_count: count as i32, ..Default::default() };
        self.push([owner_id], Push::ReactionsChanged(Box::new(changed))).await;
        Ok(())
    }

    // ───────────────────────────── the library ─────────────────────────────

    pub(crate) async fn realmojis(&self, user_id: Uuid) -> AppResult<Vec<Realmoji>> {
        let rows: Vec<(i16, serde_json::Value)> =
            sqlx::query_as("SELECT emoji, image FROM moment_realmojis WHERE user_id = $1 ORDER BY emoji")
                .bind(user_id)
                .fetch_all(&self.core.db)
                .await?;
        Ok(rows.into_iter().map(|(emoji, image)| realmoji(emoji, &image)).collect())
    }

    pub(crate) async fn set_realmoji(
        &self,
        user_id: Uuid,
        emoji: Option<RealmojiEmoji>,
        asset_id: &str,
    ) -> AppResult<Realmoji> {
        let core = &self.core;
        core.cache.rate_limit(&format!("moments-realmoji:{user_id}"), 30, Duration::from_secs(3600)).await?;
        let emoji = rules::library_emoji(emoji)? as i16;
        let asset_id = parse_id(asset_id)?;
        let image = assets::require_image_json(core, asset_id, user_id, SELFIE).await?;
        let holder = realmoji_holder(user_id, emoji);
        let mut tx = core.db.begin().await?;
        // Two selfies set at once: the second sees the first, and releases it.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))").bind(&holder).execute(&mut *tx).await?;
        let before: Option<Uuid> =
            sqlx::query_scalar("SELECT asset_id FROM moment_realmojis WHERE user_id = $1 AND emoji = $2 FOR UPDATE")
                .bind(user_id)
                .bind(emoji)
                .fetch_optional(&mut *tx)
                .await?;
        sqlx::query(
            "INSERT INTO moment_realmojis (user_id, emoji, asset_id, image) VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id, emoji) DO UPDATE SET asset_id = EXCLUDED.asset_id, image = EXCLUDED.image",
        )
        .bind(user_id)
        .bind(emoji)
        .bind(asset_id)
        .bind(&image)
        .execute(&mut *tx)
        .await?;
        if let Some(before) = before.filter(|before| *before != asset_id) {
            assets::detach(&mut *tx, before, &holder).await?;
        }
        assets::attach(&mut *tx, asset_id, &holder).await?;
        tx.commit().await?;
        Ok(realmoji(emoji, &image))
    }

    pub(crate) async fn delete_realmoji(&self, user_id: Uuid, emoji: Option<RealmojiEmoji>) -> AppResult<()> {
        let emoji = rules::library_emoji(emoji)? as i16;
        let mut tx = self.core.db.begin().await?;
        sqlx::query("DELETE FROM moment_realmojis WHERE user_id = $1 AND emoji = $2")
            .bind(user_id)
            .bind(emoji)
            .execute(&mut *tx)
            .await?;
        assets::detach_holder(&mut *tx, &realmoji_holder(user_id, emoji)).await?;
        tx.commit().await?;
        Ok(())
    }
}

fn realmoji(emoji: i16, image: &serde_json::Value) -> Realmoji {
    Realmoji {
        emoji: i32::from(emoji).into(),
        image: image_from_json(Some(image)).unwrap_or_default().into(),
        ..Default::default()
    }
}
