//! Realtime pushes: `CommunityEvent` on the topic `community:<id>` and to
//! the users concerned. Best effort, always after the change committed.
use std::time::Duration;

use tokuly_core::prelude::*;
use tokuly_proto::{
    community::{self as pb, MemberRole, community_event::Event as Push},
    realtime::server_frame::Body,
};

use crate::{
    Community,
    access::Caller,
    model::{self, CommunityRow},
};

fn frame(event: Push) -> Body {
    Body::Community(Box::new(pb::CommunityEvent { event: Some(event), ..Default::default() }))
}

impl Community {
    /// Pushes an event to everyone following the community's topic.
    pub(crate) async fn push_topic(&self, community_id: Uuid, event: impl Into<Push>) {
        self.core.rt.notify_topic(&model::topic(community_id), frame(event.into())).await;
    }

    /// Pushes an event to every connected device of some users.
    pub(crate) async fn push_users(&self, user_ids: impl IntoIterator<Item = Uuid>, event: impl Into<Push>) {
        self.core.rt.notify_users(user_ids, frame(event.into())).await;
    }

    /// Tells `user_id` what they now are to the community (after a join,
    /// an approval, a role change, a timeout, a removal or a ban).
    pub(crate) async fn push_membership(&self, community: &CommunityRow, user_id: Uuid) {
        let access = match self.stance(&Caller::user(user_id), community.clone()).await {
            Ok(access) => access,
            Err(error) => {
                tracing::warn!(%error, "membership push skipped");
                return;
            }
        };
        let event = pb::MembershipChanged {
            community_id: community.id.to_string(),
            status: access.status().into(),
            role: access.role().map_or(MemberRole::MEMBER_ROLE_UNSPECIFIED, |role| role.to_proto()).into(),
            timeout_until: opt_ts(access.timeout()),
            ..Default::default()
        };
        self.push_users([user_id], event).await;
    }

    /// Tells the community's topic that its profile or member count changed.
    pub(crate) async fn push_community_changed(&self, community_id: Uuid) {
        let member_count: Option<i32> =
            sqlx::query_scalar("SELECT member_count FROM communities WHERE id = $1 AND deleted_at IS NULL")
                .bind(community_id)
                .fetch_optional(&self.core.db)
                .await
                .unwrap_or_default();
        let event = pb::CommunityChanged {
            community_id: community_id.to_string(),
            member_count: member_count.unwrap_or_default(),
            deleted: member_count.is_none(),
            ..Default::default()
        };
        self.push_topic(community_id, event).await;
    }

    /// Tells the staff how many join requests and hidden posts wait for them.
    pub(crate) async fn push_staff_queues(&self, community_id: Uuid) {
        let result: AppResult<()> = async {
            let staff = self.staff_ids(community_id).await?;
            let event = pb::StaffQueueChanged {
                community_id: community_id.to_string(),
                pending_request_count: self.pending_request_count(community_id).await?,
                review_queue_count: self.review_queue_count(community_id).await?,
                ..Default::default()
            };
            self.push_users(staff, event).await;
            Ok(())
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(%error, "staff queue push skipped");
        }
    }

    /// Pushes a thread's counters to the topic, at most about once a
    /// second per thread: at once when nothing was pushed during the last
    /// second, otherwise once when that second is over.
    pub(crate) async fn push_thread_counts(&self, thread_id: Uuid) {
        let lead = format!("community:push:lead:{thread_id}");
        match self.core.cache.set_nx(&lead, "1", Duration::from_secs(1)).await {
            Ok(true) => self.push_thread_counts_now(thread_id).await,
            Ok(false) => {
                let trail = format!("community:push:trail:{thread_id}");
                if matches!(self.core.cache.set_nx(&trail, "1", Duration::from_secs(3)).await, Ok(true)) {
                    let community = self.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(1100)).await;
                        let _ = community.core.cache.del(&trail).await;
                        community.push_thread_counts_now(thread_id).await;
                    });
                }
            }
            Err(error) => tracing::warn!(%error, "thread counter push skipped"),
        }
    }

    async fn push_thread_counts_now(&self, thread_id: Uuid) {
        let row: Result<Option<(Uuid, i32, i64, i32)>, _> = sqlx::query_as(
            "SELECT community_id, reply_count, like_count, comment_count FROM community_threads
             WHERE id = $1 AND deleted_at IS NULL AND state = 1",
        )
        .bind(thread_id)
        .fetch_optional(&self.core.db)
        .await;
        if let Ok(Some((community_id, reply_count, like_count, comment_count))) = row {
            let event = pb::ThreadCountsChanged {
                community_id: community_id.to_string(),
                thread_id: thread_id.to_string(),
                reply_count,
                like_count,
                comment_count,
                ..Default::default()
            };
            self.push_topic(community_id, event).await;
        }
    }
}
