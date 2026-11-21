//! What happens when things disappear: an account is erased, moderation
//! takes content down, an upload is deleted by force. These are the
//! reactions to the events the stamp domain consumes; every one is
//! idempotent.
use sqlx::{Postgres, Transaction};
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_media::assets;
use tokuly_proto::common::SubjectType;

use crate::{Stamp, events::ContentRemoved, lifecycle, model::status, packs, stamps};

impl Stamp {
    /// Deletes stamps and puts the packs they were in back in order: cover
    /// and counts are refreshed, and a published pack left without stamps
    /// leaves the store. Returns the users this took something from — the
    /// owners of deleted personal stamps and the creators of affected packs.
    async fn remove_stamps(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stamp_ids: &[Uuid],
        actor_id: Option<Uuid>,
    ) -> AppResult<Vec<Uuid>> {
        // Packs are locked first, like in every other change to a pack, and
        // in a fixed order so two removals cannot wait on each other.
        let pack_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT pack_id FROM stamps WHERE id = ANY($1) AND pack_id IS NOT NULL ORDER BY pack_id",
        )
        .bind(stamp_ids)
        .fetch_all(&mut **tx)
        .await?;
        for pack_id in &pack_ids {
            packs::lock(tx, *pack_id).await?;
        }
        let deleted = stamps::delete(tx, stamp_ids).await?;

        let mut affected: Vec<Uuid> = deleted.iter().filter_map(|stamp| stamp.owner_id).collect();
        for pack_id in pack_ids {
            let pack = packs::refresh_derived(tx, pack_id).await?;
            if pack.is_published() && pack.stamp_count == 0 {
                lifecycle::unpublish(tx, &pack, actor_id).await?;
            } else {
                packs::index(tx, &pack).await?;
            }
            affected.extend(pack.creator_id);
        }
        affected.sort();
        affected.dedup();
        Ok(affected)
    }

    /// `identity.user.deleted`: erases what the account left in the stamp
    /// domain — their packs, their personal stamps, and their own library
    /// and history.
    ///
    /// One thing stays: a pack somebody bought. Its buyers paid for it, so
    /// it leaves the store but not their libraries; the creator shows as a
    /// deleted user from then on. The same goes, without anything to do
    /// here, for copies other people saved of the user's personal stamps
    /// and for official stamps an admin uploaded: each holds its upload in
    /// the media domain, which keeps held uploads past their uploader's
    /// account.
    pub(crate) async fn on_user_deleted(&self, user_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let pack_ids: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM stamp_packs WHERE creator_id = $1 ORDER BY id FOR UPDATE")
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?;
        for pack_id in pack_ids {
            self.erase_pack_of_deleted_creator(&mut tx, pack_id).await?;
        }
        let personal: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM stamps WHERE owner_id = $1").bind(user_id).fetch_all(&mut *tx).await?;
        stamps::delete(&mut tx, &personal).await?;

        // Their own library. Other people's packs lose one install each.
        sqlx::query(
            "UPDATE stamp_packs p SET install_count = GREATEST(p.install_count - 1, 0)
             WHERE p.id IN (SELECT pack_id FROM stamp_pack_installs WHERE user_id = $1)",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        for table in [
            "stamp_pack_installs",
            "stamp_pack_purchases",
            "stamp_purchase_intents",
            "stamp_favorites",
            "stamp_recents",
            "stamp_usage",
            "stamp_creators",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE user_id = $1")))
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Deletes a pack whose creator's account was erased — unless somebody
    /// bought it. A sold pack is kept for its buyers as it was reviewed:
    /// out of the store and of review, without the stamps only its creator
    /// could see.
    async fn erase_pack_of_deleted_creator(&self, tx: &mut Transaction<'_, Postgres>, pack_id: Uuid) -> AppResult<()> {
        let pack = packs::lock(tx, pack_id).await?;
        let sold: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM stamp_pack_purchases WHERE pack_id = $1)")
            .bind(pack.id)
            .fetch_one(&mut **tx)
            .await?;
        if !sold {
            return packs::delete(tx, &pack).await;
        }
        let unreviewed: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM stamps WHERE pack_id = $1 AND approved_at IS NULL")
                .bind(pack.id)
                .fetch_all(&mut **tx)
                .await?;
        stamps::delete(tx, &unreviewed).await?;
        if matches!(pack.status, status::PUBLISHED | status::IN_REVIEW) {
            lifecycle::unpublish(tx, &pack, None).await?;
        }
        packs::refresh_derived(tx, pack.id).await?;
        Ok(())
    }

    /// `media.asset.deleted`: the files of an upload are gone, so the stamps
    /// that showed it are removed rather than left with a broken picture.
    ///
    /// This is a safety net. A stamp holds its upload, so neither its
    /// uploader nor the media janitor can delete it; the event only reaches
    /// a stamp when some domain deleted the upload by force (a takedown).
    /// That is also why a pack somebody bought may lose a stamp here, and
    /// nowhere else.
    pub(crate) async fn on_asset_deleted(&self, asset_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let stamp_ids: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM stamps WHERE asset_id = $1").bind(asset_id).fetch_all(&mut *tx).await?;
        if stamp_ids.is_empty() {
            return Ok(());
        }
        self.remove_stamps(&mut tx, &stamp_ids, None).await?;
        tx.commit().await?;
        Ok(())
    }

    /// `admin.content.removed`: applies a moderator's takedown to a pack or
    /// to a single stamp. Other kinds of content are not ours.
    pub(crate) async fn on_content_removed(&self, removed: ContentRemoved) -> AppResult<()> {
        if removed.subject_type == SubjectType::SUBJECT_TYPE_STAMP_PACK as i32 {
            self.take_down_pack(&removed).await
        } else if removed.subject_type == SubjectType::SUBJECT_TYPE_STAMP as i32 {
            self.take_down_stamp(&removed).await
        } else {
            Ok(())
        }
    }

    async fn take_down_pack(&self, removed: &ContentRemoved) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        // The event may arrive (again) after the pack is gone: nothing to do then.
        if packs::find_by_id(&mut *tx, removed.subject_id).await?.is_none() {
            return Ok(());
        }
        let pack = packs::lock(&mut tx, removed.subject_id).await?;
        lifecycle::take_down(&mut tx, &pack, Some(removed.moderator_id), &removed.reason).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Deletes a stamp for breaking the rules, together with every other
    /// stamp showing the same upload (the copies people saved of it), tells
    /// the people they belonged to, and has the upload itself deleted.
    async fn take_down_stamp(&self, removed: &ContentRemoved) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let target: Option<Option<Uuid>> = sqlx::query_scalar("SELECT asset_id FROM stamps WHERE id = $1")
            .bind(removed.subject_id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(asset_id) = target else { return Ok(()) };
        let stamp_ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM stamps WHERE id = $1 OR asset_id = $2")
            .bind(removed.subject_id)
            .bind(asset_id)
            .fetch_all(&mut *tx)
            .await?;
        let affected = self.remove_stamps(&mut tx, &stamp_ids, Some(removed.moderator_id)).await?;
        for user_id in affected {
            let notice = NotificationRequest::new(
                user_id,
                "stamp.stamp.removed",
                Localized::new("スタンプが削除されたよ", "A stamp was removed"),
                Localized::new(
                    format!("ルール違反のため、スタンプが1つ削除されたよ。理由: {}", removed.reason),
                    format!("One of your stamps was removed for breaking the rules. Reason: {}", removed.reason),
                ),
                "/stamps",
            );
            outbound::notify(&mut *tx, &notice).await?;
        }
        tx.commit().await?;

        // The picture must not stay online, nor be turned into a stamp again.
        // If this fails nothing is lost: no stamp holds the upload any more,
        // so the media janitor collects it.
        if let Some(asset_id) = asset_id
            && let Err(e) = assets::delete(&self.core, asset_id, None).await
        {
            tracing::warn!(%asset_id, error = %e, "upload of a removed stamp was not deleted; left to the media janitor");
        }
        Ok(())
    }
}
