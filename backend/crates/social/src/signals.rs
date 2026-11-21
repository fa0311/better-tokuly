//! Buzz and knock: fleeting signals from one friend to another. Nothing is
//! stored; they are a realtime event plus a notification.
use std::time::Duration;

use chrono::Utc;
use tokuly_core::{
    geoindex,
    outbound::{self, NotificationRequest},
    prelude::*,
};
use tokuly_proto::social::{BuzzEvent, KnockEvent, social_event::Event as Push};

use crate::{Social, copy, people};

/// Both positions must be at least this fresh for a buzz: an old position
/// does not prove the two are near each other now.
const BUZZ_POSITION_MAX_AGE: Duration = Duration::from_secs(600);
const MAX_KNOCKS: i32 = 20;

impl Social {
    /// Buzzes a friend who is physically close by.
    pub(crate) async fn buzz(&self, user_id: Uuid, friend_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        self.require_friend(user_id, friend_id).await?;
        let apart = geoindex::distance_between(&core.cache, user_id, friend_id, BUZZ_POSITION_MAX_AGE).await?;
        ensure!(
            apart.is_some_and(|meters| meters <= self.cfg.buzz_radius_meters),
            precondition,
            "NOT_NEARBY",
            "you can only buzz a friend who is right around you"
        );
        // A buzz is a burst by design, but not an endless one.
        core.cache.rate_limit(&format!("buzz:{user_id}:{friend_id}"), 10, Duration::from_secs(60)).await?;
        core.cache.rate_limit(&format!("buzz:{user_id}"), 60, Duration::from_secs(600)).await?;

        let me = tokuly_identity::summary(core, user_id).await?;
        let text = copy::buzz(&people::name_of(&me));
        outbound::notify(
            &core.db,
            &NotificationRequest::new(friend_id, "social.buzz", text.title, text.body, "/map")
                .actor(user_id)
                .collapse(format!("social.buzz:{user_id}"))
                .push_only(),
        )
        .await?;
        self.push(friend_id, Push::Buzz(Box::new(BuzzEvent { from: me.into(), created_at: ts(Utc::now()), ..Default::default() }))).await;
        Ok(())
    }

    /// Knocks on a friend's map screen `count` times.
    pub(crate) async fn knock(&self, user_id: Uuid, friend_id: Uuid, count: i32) -> AppResult<()> {
        let core = &self.core;
        ensure!((1..=MAX_KNOCKS).contains(&count), invalid, "INVALID_KNOCK_COUNT", "knock between 1 and {MAX_KNOCKS} times");
        self.require_friend(user_id, friend_id).await?;
        core.cache.rate_limit(&format!("knock:{user_id}:{friend_id}"), 20, Duration::from_secs(60)).await?;

        let me = tokuly_identity::summary(core, user_id).await?;
        let text = copy::knock(&people::name_of(&me));
        outbound::notify(
            &core.db,
            &NotificationRequest::new(friend_id, "social.knock", text.title, text.body, "/map")
                .actor(user_id)
                .collapse(format!("social.knock:{user_id}"))
                .data(serde_json::json!({ "count": count })),
        )
        .await?;
        self.push(friend_id, Push::Knock(Box::new(KnockEvent { from: me.into(), count, created_at: ts(Utc::now()), ..Default::default() }))).await;
        Ok(())
    }
}
