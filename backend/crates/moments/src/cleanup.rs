//! Reactions to what happens elsewhere — sign-ups and birthdates (the
//! defaults by age group), erased accounts, moderation, uploads that are
//! gone — and the purge of what was kept for reports.
//!
//! Events are delivered at least once and not always in order: every
//! handler gives the same result when its event arrives twice, and one about
//! a row that is gone does nothing. Blocks, unfriendings and suspensions need
//! no handler: every read asks the social graph and the account's state.
use tokuly_core::{events::Event, outbound, prelude::*};
use tokuly_media::assets;
use tokuly_proto::common::SubjectType;

use crate::{
    Moments,
    events::{
        ASSET_DELETED, AgeGroupKnown, AssetDeleted, BIRTHDATE_GIVEN, CONTENT_REMOVED, ContentRemoved, MOMENT_DELETED,
        MomentDeleted, USER_CREATED, USER_DELETED, UserDeleted,
    },
    model::{moment_holder, reaction_holder, realmoji_holder, reason},
};

/// Rows purged per transaction.
const PURGE_BATCH: i64 = 200;

impl Moments {
    /// Handles one of the events the domain consumes (see `workers`).
    pub async fn handle_event(&self, event: &Event) -> AppResult<()> {
        match event.topic.as_str() {
            USER_CREATED | BIRTHDATE_GIVEN => {
                let known: AgeGroupKnown = event.data()?;
                // An event without the age group counts as a minor's.
                self.apply_age_defaults(known.user_id, known.minor.unwrap_or(true)).await
            }
            USER_DELETED => self.erase_account(event.data::<UserDeleted>()?.user_id).await,
            CONTENT_REMOVED => self.on_content_removed(event.data()?).await,
            ASSET_DELETED => self.on_asset_deleted(event.data::<AssetDeleted>()?.asset_id).await,
            _ => Ok(()),
        }
    }

    async fn on_content_removed(&self, removed: ContentRemoved) -> AppResult<()> {
        if removed.subject_type == SubjectType::SUBJECT_TYPE_MOMENT as i32 {
            self.withdraw(removed.subject_id, reason::MODERATION, removed.moderator_id).await
        } else if removed.subject_type == SubjectType::SUBJECT_TYPE_MOMENT_REACTION as i32 {
            self.take_down_reactions("r.id = $1", removed.subject_id).await
        } else {
            Ok(())
        }
    }

    /// An upload a moment, a reaction or a library entry shows is gone: the
    /// moment is deleted, the reaction taken down, the entry removed.
    async fn on_asset_deleted(&self, asset_id: Uuid) -> AppResult<()> {
        let moment_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM moments WHERE (back_asset_id = $1 OR front_asset_id = $1) AND deleted_at IS NULL",
        )
        .bind(asset_id)
        .fetch_all(&self.core.db)
        .await?;
        for moment_id in moment_ids {
            self.withdraw(moment_id, reason::MEDIA_DELETED, None).await?;
        }
        self.take_down_reactions("r.asset_id = $1", asset_id).await?;
        sqlx::query("DELETE FROM moment_realmojis WHERE asset_id = $1").bind(asset_id).execute(&self.core.db).await?;
        Ok(())
    }

    /// Marks the live reactions matching `filter` (on `r`, with `$1`) as
    /// removed and tells the owners of their moments.
    async fn take_down_reactions(&self, filter: &'static str, id: Uuid) -> AppResult<()> {
        let touched: Vec<(Uuid, Uuid)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE moment_reactions r SET removed_at = now() FROM moments m
             WHERE {filter} AND r.removed_at IS NULL AND m.id = r.moment_id
             RETURNING m.id, m.user_id"
        )))
        .bind(id)
        .fetch_all(&self.core.db)
        .await?;
        for (moment_id, owner_id) in touched {
            self.tell_reactions_changed(moment_id, owner_id).await?;
        }
        Ok(())
    }

    /// `identity.user.deleted`: everything of the account goes at once —
    /// its moments with the reactions on them, its reactions on others',
    /// its library, settings and deliveries — with the uploads released.
    pub async fn erase_account(&self, user_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let moments: Vec<(Uuid, bool)> =
            sqlx::query_as("SELECT id, deleted_at IS NULL FROM moments WHERE user_id = $1 ORDER BY id FOR UPDATE")
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?;
        let moment_ids: Vec<Uuid> = moments.iter().map(|(id, _)| *id).collect();
        let reactions: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM moment_reactions WHERE moment_id = ANY($1) OR user_id = $2 ORDER BY id")
                .bind(&moment_ids)
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?;
        for (moment_id, live) in &moments {
            assets::detach_holder(&mut *tx, &moment_holder(*moment_id)).await?;
            if *live {
                let deleted =
                    MomentDeleted { moment_id: *moment_id, owner_id: user_id, reason: reason::ACCOUNT_DELETED.into() };
                tokuly_core::events::emit(&mut *tx, MOMENT_DELETED, None, &deleted).await?;
            }
        }
        for reaction_id in &reactions {
            assets::detach_holder(&mut *tx, &reaction_holder(*reaction_id)).await?;
        }
        let emojis: Vec<i16> = sqlx::query_scalar("DELETE FROM moment_realmojis WHERE user_id = $1 RETURNING emoji")
            .bind(user_id)
            .fetch_all(&mut *tx)
            .await?;
        for emoji in emojis {
            assets::detach_holder(&mut *tx, &realmoji_holder(user_id, emoji)).await?;
        }
        sqlx::query("DELETE FROM moment_reactions WHERE id = ANY($1)").bind(&reactions).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM moments WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM moment_deliveries WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM moment_settings WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        outbound::subjects_deleted(&mut *tx, SubjectType::SUBJECT_TYPE_MOMENT, &moment_ids).await?;
        outbound::subjects_deleted(&mut *tx, SubjectType::SUBJECT_TYPE_MOMENT_REACTION, &reactions).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Removes for good what was kept for reports and whose evidence period
    /// is over: deleted moments (with every reaction on them) and replaced or
    /// taken back reactions. Returns how many moments and reactions went.
    pub async fn purge(&self) -> AppResult<(usize, usize)> {
        let days = self.cfg.evidence_days;
        let (mut moments_gone, mut reactions_gone) = (0, 0);
        loop {
            let mut tx = self.core.db.begin().await?;
            let moments: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM moments WHERE deleted_at < now() - make_interval(days => $1::int)
                 ORDER BY deleted_at LIMIT $2 FOR UPDATE SKIP LOCKED",
            )
            .bind(days as i32)
            .bind(PURGE_BATCH)
            .fetch_all(&mut *tx)
            .await?;
            let reactions: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM moment_reactions
                 WHERE moment_id = ANY($1) OR removed_at < now() - make_interval(days => $2::int)
                 ORDER BY id LIMIT $3 FOR UPDATE SKIP LOCKED",
            )
            .bind(&moments)
            .bind(days as i32)
            .bind(PURGE_BATCH * 10)
            .fetch_all(&mut *tx)
            .await?;
            if moments.is_empty() && reactions.is_empty() {
                return Ok((moments_gone, reactions_gone));
            }
            for reaction_id in &reactions {
                assets::detach_holder(&mut *tx, &reaction_holder(*reaction_id)).await?;
            }
            for moment_id in &moments {
                assets::detach_holder(&mut *tx, &moment_holder(*moment_id)).await?;
            }
            sqlx::query("DELETE FROM moment_reactions WHERE id = ANY($1)").bind(&reactions).execute(&mut *tx).await?;
            // The purged moments' reactions the batch did not reach go too,
            // announced like the others.
            let leftover: Vec<Uuid> =
                sqlx::query_scalar("DELETE FROM moment_reactions WHERE moment_id = ANY($1) RETURNING id")
                    .bind(&moments)
                    .fetch_all(&mut *tx)
                    .await?;
            for reaction_id in &leftover {
                assets::detach_holder(&mut *tx, &reaction_holder(*reaction_id)).await?;
            }
            sqlx::query("DELETE FROM moments WHERE id = ANY($1)").bind(&moments).execute(&mut *tx).await?;
            outbound::subjects_deleted(&mut *tx, SubjectType::SUBJECT_TYPE_MOMENT, &moments).await?;
            let all_reactions: Vec<Uuid> = reactions.iter().chain(&leftover).copied().collect();
            outbound::subjects_deleted(&mut *tx, SubjectType::SUBJECT_TYPE_MOMENT_REACTION, &all_reactions).await?;
            tx.commit().await?;
            moments_gone += moments.len();
            reactions_gone += all_reactions.len();
        }
    }
}
