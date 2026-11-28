//! Posting a moment, and every way one is deleted.
use std::{collections::HashSet, time::Duration};

use tokuly_core::{events, prelude::*};
use tokuly_media::assets;
use tokuly_proto::{
    media::AssetPurpose,
    moments::{MomentAudience, MomentRef, PostMomentRequest, moments_event::Event as Push},
};
use tokuly_social::graph::{self, ContentAudience};

use crate::{
    Moments,
    events::{MOMENT_DELETED, MOMENT_POSTED, MomentDeleted, MomentPosted},
    model::{MOMENT_COLUMNS, MomentRow, audience, moment_holder},
    rules,
};

const PICTURE: &[AssetPurpose] = &[AssetPurpose::ASSET_PURPOSE_MOMENT];

impl Moments {
    /// Posts the caller's moment of their region's current day.
    pub(crate) async fn post(&self, user_id: Uuid, request: PostMomentRequest) -> AppResult<MomentRow> {
        let core = &self.core;
        core.cache.rate_limit(&format!("moments-post:{user_id}"), 10, Duration::from_secs(3600)).await?;
        let caption = rules::caption(&request.caption)?;
        let retakes = rules::retakes(request.retakes)?;
        let location = request.location.as_option().map(|at| rules::location(at.lat, at.lng)).transpose()?;
        let back_id = parse_id(&request.back_asset_id)?;
        let front_id = parse_id(&request.front_asset_id)?;
        ensure!(back_id != front_id, invalid, "SAME_PICTURE", "the back and the front are two pictures");
        let audience = match request.audience.as_known() {
            Some(MomentAudience::MOMENT_AUDIENCE_UNSPECIFIED) => self.settings_of(user_id).await?.audience,
            Some(chosen) => chosen as i16,
            None => bail!(invalid, "INVALID_AUDIENCE", "unknown audience"),
        };
        // Everything read through the pool is read before the transaction.
        let back = assets::require_image_json(core, back_id, user_id, PICTURE).await?;
        let front = assets::require_image_json(core, front_id, user_id, PICTURE).await?;
        let region = self.region_of(user_id).await?;

        let mut tx = core.db.begin().await?;
        let day = self.current_day(&mut *tx, &region).await?.ok_or_else(|| {
            AppError::precondition("NO_PROMPT_YET", "the first prompt of your region has not gone out yet")
        })?;
        let late_seconds = rules::late_seconds(
            rules::on_time_until(day.announced_at, self.cfg.window_seconds),
            now(),
            self.cfg.upload_grace_seconds,
        );
        let moment_id: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO moments (user_id, day_id, back, front, back_asset_id, front_asset_id, caption, lat, lng,
                                  retakes, late_seconds, audience)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             ON CONFLICT (user_id, day_id) DO NOTHING
             RETURNING id",
        )
        .bind(user_id)
        .bind(day.id)
        .bind(&back)
        .bind(&front)
        .bind(back_id)
        .bind(front_id)
        .bind(&caption)
        .bind(location.map(|(lat, _)| lat))
        .bind(location.map(|(_, lng)| lng))
        .bind(retakes)
        .bind(late_seconds)
        .bind(audience)
        .fetch_optional(&mut *tx)
        .await?;
        let moment_id =
            moment_id.ok_or_else(|| AppError::already_exists("ALREADY_POSTED", "you already posted today's moment"))?;
        for asset_id in [back_id, front_id] {
            assets::attach(&mut *tx, asset_id, &moment_holder(moment_id)).await?;
        }
        let posted = MomentPosted {
            moment_id,
            owner_id: user_id,
            day_id: day.id,
            date: day.date,
            late_seconds,
            audience: i32::from(audience),
        };
        events::emit(&mut *tx, MOMENT_POSTED, Some(user_id), &posted).await?;
        tx.commit().await?;

        let moment = self.find_moment(moment_id).await?.or_not_found("MOMENT_NOT_FOUND", "moment not found")?;
        let audience_ids = self.audience_ids(user_id, audience).await?;
        self.push(audience_ids, Push::FriendPosted(Box::new(moment_ref(&moment)))).await;
        Ok(moment)
    }

    /// The owner deletes one of their moments. Deleting one that is gone
    /// already succeeds.
    pub(crate) async fn delete(&self, user_id: Uuid, moment_id: Uuid) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("moments-delete:{user_id}"), 30, Duration::from_secs(3600)).await?;
        let owner: Option<Uuid> = sqlx::query_scalar("SELECT user_id FROM moments WHERE id = $1")
            .bind(moment_id)
            .fetch_optional(&self.core.db)
            .await?;
        ensure!(owner == Some(user_id), not_found, "MOMENT_NOT_FOUND", "moment not found");
        self.withdraw(moment_id, crate::model::reason::OWNER, Some(user_id)).await
    }

    /// Deletes a moment for `reason`. It stays on record (for reports)
    /// until the evidence period is over, and its day stays used. Whoever
    /// could see it is told (realtime). Idempotent.
    pub(crate) async fn withdraw(&self, moment_id: Uuid, reason: &str, actor: Option<Uuid>) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let withdrawn: Option<(Uuid, i16)> = sqlx::query_as(
            "UPDATE moments SET deleted_at = now(), delete_reason = $2
             WHERE id = $1 AND deleted_at IS NULL
             RETURNING user_id, audience",
        )
        .bind(moment_id)
        .bind(reason)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((owner_id, audience)) = withdrawn else { return Ok(()) };
        let deleted = MomentDeleted { moment_id, owner_id, reason: reason.to_owned() };
        events::emit(&mut *tx, MOMENT_DELETED, actor, &deleted).await?;
        tx.commit().await?;
        let mut told = self.audience_ids(owner_id, audience).await?;
        told.push(owner_id);
        let gone =
            MomentRef { moment_id: moment_id.to_string(), author_id: owner_id.to_string(), ..Default::default() };
        self.push(told, Push::Deleted(Box::new(gone))).await;
        Ok(())
    }

    /// A moment with its day, deleted or not.
    pub(crate) async fn find_moment(&self, moment_id: Uuid) -> AppResult<Option<MomentRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MOMENT_COLUMNS} FROM moments m JOIN moment_days d ON d.id = m.day_id WHERE m.id = $1"
        )))
        .bind(moment_id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// Who a moment shared with `audience` reaches: the owner's friends, or
    /// those of them among the owner's close friends; active accounts only,
    /// nobody on either side of a block with the owner.
    pub(crate) async fn audience_ids(&self, owner_id: Uuid, audience: i16) -> AppResult<Vec<Uuid>> {
        let core = &self.core;
        let friends = graph::audience_member_ids(core, owner_id, ContentAudience::Friends).await?;
        if audience != audience::CLOSE_FRIENDS {
            return Ok(friends);
        }
        let close: HashSet<Uuid> =
            graph::audience_member_ids(core, owner_id, ContentAudience::CloseFriends).await?.into_iter().collect();
        Ok(friends.into_iter().filter(|id| close.contains(id)).collect())
    }
}

pub(crate) fn moment_ref(moment: &MomentRow) -> MomentRef {
    MomentRef { moment_id: moment.id.to_string(), author_id: moment.user_id.to_string(), ..Default::default() }
}
