//! Map views: who opened whose pin in the last 24 hours.
//!
//! Everyone's views are counted, but only viewers who allow it
//! (`privacy.share_views`) are named to the person they looked at.
use std::time::Duration;

use tokuly_core::prelude::*;
use tokuly_identity::settings;
use tokuly_proto::social::{GetMapViewsResponse, MapViewEvent, social_event::Event as Push};

use crate::{
    Social,
    graph::{self, InteractionKind},
};

/// Repeated views of the same pin within this time count once.
const VIEW_DEDUPE: Duration = Duration::from_secs(60);
/// Distinct pins one user may open per minute (a whole map of friends).
const MAX_VIEWS_PER_MINUTE: u32 = 120;
/// Named viewers returned by `GetMapViews`.
const MAX_VIEWERS: i64 = 100;

impl Social {
    /// Records that `viewer` opened the pin of their friend `viewed`.
    pub(crate) async fn record_map_view(&self, viewer: Uuid, viewed: Uuid) -> AppResult<()> {
        let core = &self.core;
        self.require_friend(viewer, viewed).await?;
        // Opening and closing a pin repeatedly is one look, not twenty.
        let first_in_a_while = core.cache.set_nx(&format!("social:map-view:{}:{}", viewer.simple(), viewed.simple()), "1", VIEW_DEDUPE).await?;
        if !first_in_a_while {
            return Ok(());
        }
        // Every new look is a row and a push to the friend looked at.
        core.cache.rate_limit(&format!("map-view:{viewer}"), MAX_VIEWS_PER_MINUTE, Duration::from_secs(60)).await?;
        sqlx::query("INSERT INTO map_views (viewed_id, viewer_id) VALUES ($1, $2)").bind(viewed).bind(viewer).execute(&core.db).await?;
        if let Err(e) = graph::record_interaction(core, viewer, viewed, InteractionKind::MapView, 1).await {
            tracing::warn!(error = %e, "map view not counted towards the best-friend ranking");
        }

        let viewer_count: i64 =
            sqlx::query_scalar("SELECT count(DISTINCT viewer_id) FROM map_views WHERE viewed_id = $1 AND viewed_at > now() - interval '24 hours'")
                .bind(viewed)
                .fetch_one(&core.db)
                .await?;
        let named = if settings::privacy_of(core, viewer).await?.share_views { Some(tokuly_identity::summary(core, viewer).await?) } else { None };
        self.push(viewed, Push::MapView(Box::new(MapViewEvent { viewer: named.into(), viewer_count_24h: viewer_count as i32, ..Default::default() }))).await;
        Ok(())
    }

    /// Who looked at `user_id`'s pin in the last 24 hours.
    pub(crate) async fn map_views(&self, user_id: Uuid) -> AppResult<GetMapViewsResponse> {
        let core = &self.core;
        let (viewer_count, view_count): (i64, i64) = sqlx::query_as(
            "SELECT count(DISTINCT viewer_id), count(*) FROM map_views WHERE viewed_id = $1 AND viewed_at > now() - interval '24 hours'",
        )
        .bind(user_id)
        .fetch_one(&core.db)
        .await?;
        // Most recent viewers first. People who are no longer friends are
        // still counted above but not named.
        let recent: Vec<Uuid> = sqlx::query_scalar(
            "SELECT v.viewer_id FROM map_views v
             WHERE v.viewed_id = $1 AND v.viewed_at > now() - interval '24 hours'
               AND EXISTS (SELECT 1 FROM friendships f WHERE f.user_id = $1 AND f.friend_id = v.viewer_id)
             GROUP BY v.viewer_id
             ORDER BY max(v.viewed_at) DESC
             LIMIT $2",
        )
        .bind(user_id)
        .bind(MAX_VIEWERS)
        .fetch_all(&core.db)
        .await?;
        let privacy = settings::privacy_of_many(core, &recent).await?;
        let sharing: Vec<Uuid> = recent.into_iter().filter(|id| privacy.get(id).is_some_and(|p| p.share_views)).collect();
        Ok(GetMapViewsResponse {
            viewer_count: viewer_count as i32,
            view_count: view_count as i32,
            viewers: tokuly_identity::summaries_ordered(core, &sharing).await?,
            ..Default::default()
        })
    }
}
