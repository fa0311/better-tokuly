//! Reactions to other domains' events (see `docs/events/geo.md`). Every
//! handler is idempotent: events are delivered at least once.
use chrono::{DateTime, Utc};
use tokuly_core::{events::Event, geoindex, prelude::*};
use tokuly_identity::maintenance::UserRef;
use tokuly_media::assets;
use tokuly_proto::{common::SubjectType, geo::geo_event::Event as Push, media::AssetPurpose};
use tokuly_social::events::{BlockChanged, BumpCreated, FriendAccepted, FriendRemoved};

use crate::{
    Geo,
    geocode::Gazetteer,
    places::{PLACE_COLUMNS, PlaceRow},
};

pub(crate) const USER_DELETED: &str = "identity.user.deleted";
pub(crate) const USER_BANNED: &str = "identity.user.banned";
pub(crate) const USER_UNBANNED: &str = "identity.user.unbanned";
pub(crate) const USER_UPDATED: &str = "identity.user.updated";
pub(crate) const PRIVACY_CHANGED: &str = "identity.user.privacy_changed";
pub(crate) const STATUS_CHANGED: &str = "identity.user.status_changed";
pub(crate) const FRIEND_ACCEPTED: &str = tokuly_social::events::FRIEND_ACCEPTED;
pub(crate) const FRIEND_REMOVED: &str = tokuly_social::events::FRIEND_REMOVED;
pub(crate) const USER_BLOCKED: &str = tokuly_social::events::USER_BLOCKED;
pub(crate) const BUMP_CREATED: &str = tokuly_social::events::BUMP_CREATED;
pub(crate) const CONTENT_REMOVED: &str = "admin.content.removed";
pub(crate) const ASSET_DELETED: &str = "media.asset.deleted";
pub(crate) const REINDEX_REQUESTED: &str = "search.reindex.requested";
/// The kind of our search documents (`SearchDoc::kind`).
const SEARCH_KIND: &str = "place";

/// Every topic the geo domain consumes.
pub(crate) const TOPICS: &[&str] = &[
    USER_DELETED,
    USER_BANNED,
    USER_UNBANNED,
    USER_UPDATED,
    PRIVACY_CHANGED,
    STATUS_CHANGED,
    FRIEND_ACCEPTED,
    FRIEND_REMOVED,
    USER_BLOCKED,
    BUMP_CREATED,
    CONTENT_REMOVED,
    ASSET_DELETED,
    REINDEX_REQUESTED,
];

/// `admin.content.removed` (the moderation contract).
#[derive(serde::Deserialize)]
struct ContentRemoved {
    subject_type: i32,
    subject_id: Uuid,
}

/// `search.reindex.requested`: an administrator wants one kind of search
/// document rebuilt.
#[derive(serde::Deserialize)]
struct ReindexRequested {
    kind: String,
}

/// `media.asset.deleted`.
#[derive(serde::Deserialize)]
struct AssetDeleted {
    asset_id: Uuid,
    purpose: i32,
}

impl Geo {
    /// Reacts to one event from another domain. Public so tests can deliver
    /// events directly (no relay runs in tests).
    pub async fn handle_event(&self, event: &Event) -> AppResult<()> {
        match event.topic.as_str() {
            USER_DELETED => self.forget_user(event.data::<UserRef>()?.user_id).await,
            USER_BANNED => self.hide_user(event.data::<UserRef>()?.user_id).await,
            USER_UNBANNED | STATUS_CHANGED | USER_UPDATED => self.user_changed(event.data::<UserRef>()?.user_id).await,
            PRIVACY_CHANGED => self.privacy_changed(event.data::<UserRef>()?.user_id).await,
            FRIEND_ACCEPTED => {
                let accepted: FriendAccepted = event.data()?;
                self.became_friends(accepted.user_id, accepted.friend_id).await
            }
            FRIEND_REMOVED => {
                let removed: FriendRemoved = event.data()?;
                self.stopped_being_friends(removed.user_id, removed.friend_id).await
            }
            USER_BLOCKED => {
                let blocked: BlockChanged = event.data()?;
                self.stopped_being_friends(blocked.user_id, blocked.blocked_id).await
            }
            BUMP_CREATED => self.bump_happened(event.data()?, event.occurred_at).await,
            CONTENT_REMOVED => {
                let removed: ContentRemoved = event.data()?;
                if removed.subject_type == SubjectType::SUBJECT_TYPE_PLACE as i32 {
                    self.take_down_place(removed.subject_id).await
                } else {
                    Ok(())
                }
            }
            ASSET_DELETED => self.asset_deleted(event.data()?).await,
            REINDEX_REQUESTED => {
                if event.data::<ReindexRequested>()?.kind == SEARCH_KIND {
                    let published = self.reindex_places().await?;
                    tracing::info!(published, "public places republished to the search index");
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Two users became friends: each sees the other's pin at once.
    async fn became_friends(&self, a: Uuid, b: Uuid) -> AppResult<()> {
        self.refresh_viewers(a, &[b]).await?;
        self.refresh_viewers(b, &[a]).await
    }

    /// A friendship ended (unfriended or blocked): the two disappear from
    /// each other's maps, and nothing they had set up about each other stays.
    async fn stopped_being_friends(&self, a: Uuid, b: Uuid) -> AppResult<()> {
        self.drop_ghost_overrides(a, b).await?;
        self.drop_alerts_between(a, b).await?;
        self.drop_trips_between(a, b).await?;
        self.drop_sos_recipient(a, b).await?;
        self.drop_sos_recipient(b, a).await?;
        // Each other's friends-only places can no longer be kept as saved.
        sqlx::query(
            "DELETE FROM geo_saved_places s USING geo_places p
             WHERE p.id = s.place_id AND p.visibility <> 1 AND ((s.user_id = $1 AND p.owner_id = $2) OR (s.user_id = $2 AND p.owner_id = $1))",
        )
        .bind(a)
        .bind(b)
        .execute(&self.core.db)
        .await?;
        self.invalidate_ctx(&[a, b]).await;
        self.push(a, Push::LocationRemovedUserId(b.to_string())).await;
        self.push(b, Push::LocationRemovedUserId(a.to_string())).await;
        Ok(())
    }

    /// Status, language, or a lifted ban: rebuild the context and show friends the pin again.
    async fn user_changed(&self, user_id: Uuid) -> AppResult<()> {
        self.invalidate_ctx(&[user_id]).await;
        self.forget_language(user_id);
        self.refresh_friends(user_id).await
    }

    /// A banned account disappears from its friends' maps.
    async fn hide_user(&self, user_id: Uuid) -> AppResult<()> {
        self.invalidate_ctx(&[user_id]).await;
        let friends = tokuly_social::graph::friend_ids(&self.core, user_id).await?;
        self.push_many(friends, Push::LocationRemovedUserId(user_id.to_string())).await;
        geoindex::remove(&self.core.cache, user_id).await?;
        self.leave_gathering(user_id).await
    }

    /// Privacy settings changed: what friends see follows at once, and a user
    /// who switched history off stops having an open visit.
    async fn privacy_changed(&self, user_id: Uuid) -> AppResult<()> {
        self.invalidate_ctx(&[user_id]).await;
        let ctx = self.ctx_of(user_id).await?;
        sqlx::query("UPDATE geo_users SET footprints_audience = $2 WHERE user_id = $1")
            .bind(user_id)
            .bind(ctx.privacy.footprints_audience as i16)
            .execute(&self.core.db)
            .await?;
        if !ctx.privacy.record_history {
            self.with_state(user_id, async |state| {
                if let Some(visit_id) = state.visit_id.take() {
                    sqlx::query("UPDATE geo_visits SET departed_at = GREATEST(arrived_at, now()) WHERE id = $1 AND departed_at IS NULL")
                        .bind(visit_id)
                        .execute(&self.core.db)
                        .await?;
                }
                state.night = None;
                state.odometer.take();
                state.odometer.disconnect();
                Ok(())
            })
            .await?;
        }
        self.refresh_friends(user_id).await
    }

    /// Two users bumped: remember it for the statistics and tell the social
    /// domain what the spot is called.
    async fn bump_happened(&self, bump: BumpCreated, at: DateTime<Utc>) -> AppResult<()> {
        let [a, b] = bump.user_ids;
        let at_point = (bump.lat, bump.lng);
        // A place both may know by name; otherwise the area (Japanese first).
        let place_name = match self.shared_place_name(a, b, at_point).await? {
            Some(name) => name,
            None => Gazetteer::shared()
                .nearest(bump.lat, bump.lng)
                .map(|found| found.locality(true).to_owned())
                .unwrap_or_default(),
        };
        sqlx::query(
            "INSERT INTO geo_bumps (bump_id, user_a, user_b, lat, lng, place_name, created_at)
             SELECT $1, $2, $3, $4, $5, $6, $7
             WHERE EXISTS (SELECT 1 FROM users WHERE id = $2) AND EXISTS (SELECT 1 FROM users WHERE id = $3)
             ON CONFLICT (bump_id) DO NOTHING",
        )
        .bind(bump.bump_id)
        .bind(a)
        .bind(b)
        .bind(bump.lat)
        .bind(bump.lng)
        .bind(&place_name)
        .bind(at)
        .execute(&self.core.db)
        .await?;
        if !place_name.is_empty() {
            tokuly_social::bumps::set_bump_place(&self.core, bump.bump_id, &place_name).await?;
        }
        Ok(())
    }

    /// A moderator removed a place.
    async fn take_down_place(&self, place_id: Uuid) -> AppResult<()> {
        let Some(place) = self.place_row(place_id).await? else { return Ok(()) };
        let mut tx = self.core.db.begin().await?;
        self.remove_place(&mut tx, &place).await?;
        tx.commit().await?;
        self.places_changed(place.owner_id).await
    }

    /// A photo used by a place or a day memo was deleted (moderation, or the
    /// media domain cleaning up): drop the references to it.
    async fn asset_deleted(&self, asset: AssetDeleted) -> AppResult<()> {
        if asset.purpose == AssetPurpose::ASSET_PURPOSE_PLACE_PHOTO as i32 {
            sqlx::query("UPDATE geo_places SET photo = NULL, photo_asset_id = NULL WHERE photo_asset_id = $1")
                .bind(asset.asset_id)
                .execute(&self.core.db)
                .await?;
        } else if asset.purpose == AssetPurpose::ASSET_PURPOSE_DAY_MEMO as i32 {
            let memos: Vec<(Uuid, chrono::NaiveDate, String, serde_json::Value, Vec<Uuid>)> =
                sqlx::query_as("SELECT user_id, day, text, photos, photo_asset_ids FROM geo_day_memos WHERE photo_asset_ids @> ARRAY[$1]::uuid[]")
                    .bind(asset.asset_id)
                    .fetch_all(&self.core.db)
                    .await?;
            for (user_id, day, text, photos, asset_ids) in memos {
                let keep: Vec<usize> = asset_ids
                    .iter()
                    .enumerate()
                    .filter(|(_, id)| **id != asset.asset_id)
                    .map(|(index, _)| index)
                    .collect();
                let photos: Vec<serde_json::Value> =
                    keep.iter().filter_map(|index| photos.get(*index).cloned()).collect();
                let asset_ids: Vec<Uuid> = keep.iter().map(|index| asset_ids[*index]).collect();
                if text.is_empty() && asset_ids.is_empty() {
                    sqlx::query("DELETE FROM geo_day_memos WHERE user_id = $1 AND day = $2")
                        .bind(user_id)
                        .bind(day)
                        .execute(&self.core.db)
                        .await?;
                } else {
                    sqlx::query(
                        "UPDATE geo_day_memos SET photos = $3, photo_asset_ids = $4 WHERE user_id = $1 AND day = $2",
                    )
                    .bind(user_id)
                    .bind(day)
                    .bind(serde_json::Value::Array(photos))
                    .bind(asset_ids)
                    .execute(&self.core.db)
                    .await?;
                }
            }
        }
        Ok(())
    }

    /// Removes everything this domain holds about a deleted account.
    async fn forget_user(&self, user_id: Uuid) -> AppResult<()> {
        // First the cached context: from here on the pipeline finds the
        // account gone and accepts nothing more for it. Then the rest under
        // the account's pipeline lock, so a sample that was already being
        // processed has finished and cannot write anything back afterwards.
        self.invalidate_ctx(&[user_id]).await;
        let lock = self.lock_user(user_id).await?;
        let forgotten = self.forget_user_locked(user_id).await;
        self.unlock_user(lock).await;
        forgotten
    }

    async fn forget_user_locked(&self, user_id: Uuid) -> AppResult<()> {
        let db = &self.core.db;
        // Out of the live picture first.
        self.leave_gathering(user_id).await?;
        let shares: Vec<Vec<u8>> =
            sqlx::query_scalar("SELECT token_sealed FROM geo_live_shares WHERE user_id = $1 AND ended_at IS NULL")
                .bind(user_id)
                .fetch_all(db)
                .await?;
        for token in shares.iter().filter_map(|sealed| self.open_share_token(sealed)) {
            self.core
                .rt
                .notify_topic(
                    &crate::shares::topic(&token),
                    &crate::fanout::frame(Push::LocationRemovedUserId(user_id.to_string())),
                )
                .await;
            self.core.rt.close_topic(&crate::shares::topic(&token)).await;
        }
        // People whose cached context mentions this user: alert subjects, SOS senders.
        let mut affected: Vec<Uuid> = sqlx::query_scalar("SELECT subject_id FROM geo_place_alerts WHERE owner_id = $1")
            .bind(user_id)
            .fetch_all(db)
            .await?;
        affected.extend(
            sqlx::query_scalar::<_, Uuid>(
                "UPDATE geo_sos SET notify_user_ids = array_remove(notify_user_ids, $1) WHERE resolved_at IS NULL AND notify_user_ids @> ARRAY[$1]::uuid[] RETURNING user_id",
            )
            .bind(user_id)
            .fetch_all(db)
            .await?,
        );
        affected.extend(
            sqlx::query_scalar::<_, Uuid>("SELECT owner_id FROM geo_ghost_rules WHERE viewer_id = $1")
                .bind(user_id)
                .fetch_all(db)
                .await?,
        );

        let places: Vec<PlaceRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {PLACE_COLUMNS} FROM geo_places WHERE owner_id = $1")))
                .bind(user_id)
                .fetch_all(db)
                .await?;
        let memo_days: Vec<chrono::NaiveDate> =
            sqlx::query_scalar("SELECT day FROM geo_day_memos WHERE user_id = $1").bind(user_id).fetch_all(db).await?;
        let mut tx = db.begin().await?;
        for place in &places {
            self.remove_place(&mut tx, place).await?;
        }
        for day in memo_days {
            assets::detach_holder(&mut *tx, &format!("geo:day-memo:{user_id}:{day}")).await?;
        }
        // Each statement removes the rows in which the user is either party.
        const DELETES: &[&str] = &[
            "DELETE FROM geo_ghost_rules WHERE owner_id = $1 OR viewer_id = $1",
            "DELETE FROM geo_saved_places WHERE user_id = $1",
            "DELETE FROM geo_nights WHERE user_id = $1",
            "DELETE FROM geo_visits WHERE user_id = $1",
            "DELETE FROM geo_activity_hours WHERE user_id = $1",
            "DELETE FROM geo_day_memos WHERE user_id = $1",
            "DELETE FROM geo_cells WHERE user_id = $1",
            "DELETE FROM geo_region_stats WHERE user_id = $1",
            "DELETE FROM geo_trips WHERE user_id = $1 OR dest_user_id = $1",
            "DELETE FROM geo_place_alerts WHERE owner_id = $1 OR subject_id = $1",
            "DELETE FROM geo_sos WHERE user_id = $1",
            "DELETE FROM geo_live_shares WHERE user_id = $1",
            "DELETE FROM geo_gathering_members WHERE user_id = $1",
            "DELETE FROM geo_bumps WHERE user_a = $1 OR user_b = $1",
            "DELETE FROM geo_users WHERE user_id = $1",
        ];
        for delete in DELETES {
            sqlx::query(*delete).bind(user_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;

        self.delete_points(user_id, DateTime::UNIX_EPOCH, Utc::now() + chrono::Duration::days(3650)).await?;
        self.drop_state(user_id).await?;
        geoindex::remove(&self.core.cache, user_id).await?;
        self.forget_language(user_id);
        affected.push(user_id);
        self.invalidate_ctx(&affected).await;
        Ok(())
    }
}
