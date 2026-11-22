//! Realtime pushes: every change is sent as a `SpacesEvent`, to the topic
//! of the room it is about (`space:<id>`) or to the users concerned.
//!
//! Pushes go out after the transaction that made the change committed, and
//! never fail a request: what they announce can always be fetched.
use std::time::Duration;

use tokuly_core::prelude::*;
use tokuly_proto::{
    realtime::server_frame::Body,
    spaces::{self as pb, SpacesEvent, spaces_event::Event as Push},
};

use crate::{
    Spaces,
    model::{SpaceRow, space_topic},
};

/// Rooms up to this size announce every change of their head count; larger
/// ones at most once per [`COUNTS_INTERVAL`].
const COUNTS_ALWAYS_BELOW: i32 = 150;
const COUNTS_INTERVAL: Duration = Duration::from_secs(2);

pub(crate) fn frame(event: Push) -> Body {
    Body::from(SpacesEvent { event: Some(event), ..Default::default() })
}

impl Spaces {
    /// Pushes an event to everyone following a room.
    pub(crate) async fn push_topic(&self, space_id: Uuid, event: impl Into<Push>) {
        self.core.rt.notify_topic(&space_topic(space_id), frame(event.into())).await;
    }

    /// Pushes an event to every device of one user.
    pub(crate) async fn push_user(&self, user_id: Uuid, event: impl Into<Push>) {
        self.core.rt.notify_user(user_id, frame(event.into())).await;
    }

    /// Pushes the same event to several users.
    pub(crate) async fn push_users(&self, user_ids: impl IntoIterator<Item = Uuid>, event: impl Into<Push>) {
        self.core.rt.notify_users(user_ids, frame(event.into())).await;
    }

    /// Announces a room as it is now (state, host, title, pin, stage) to
    /// its topic.
    pub(crate) async fn push_space(&self, space: &SpaceRow) {
        match self.render_space(space, None).await {
            Ok(proto) => {
                self.push_topic(space.id, pb::SpaceUpdated { space: proto.into(), ..Default::default() }).await
            }
            Err(e) => tracing::warn!(space_id = %space.id, error = %e, "room push failed"),
        }
    }

    /// Announces how many people are inside. Small rooms hear of every
    /// change; in large ones the count is sent at most every two seconds,
    /// with a last one after the changes stop.
    pub(crate) async fn push_counts(&self, space: &SpaceRow) {
        if space.listener_count <= COUNTS_ALWAYS_BELOW {
            self.push_topic(space.id, space.counts()).await;
            return;
        }
        let cache = &self.core.cache;
        match cache.set_nx(&format!("spaces:counts:{}", space.id), "1", COUNTS_INTERVAL).await {
            Ok(true) => self.push_topic(space.id, space.counts()).await,
            // Something was sent a moment ago: make sure the latest numbers follow.
            Ok(false) => {
                let trailing = format!("spaces:counts:trailing:{}", space.id);
                if matches!(cache.set_nx(&trailing, "1", COUNTS_INTERVAL).await, Ok(true)) {
                    let spaces = self.clone();
                    let space_id = space.id;
                    tokio::spawn(async move {
                        tokio::time::sleep(COUNTS_INTERVAL).await;
                        if let Ok(Some(latest)) = spaces.find_space(space_id).await {
                            spaces.push_topic(space_id, latest.counts()).await;
                        }
                    });
                }
            }
            Err(e) => tracing::warn!(space_id = %space.id, error = %e, "count push could not be throttled"),
        }
    }
}
