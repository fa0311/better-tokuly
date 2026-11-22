//! Telling people that a stream went on air.
//!
//! When a stream starts, three groups hear about it, in this order, each in
//! batches of `announce_batch_size` handled by one `live.announce` job that
//! queues the next:
//!
//! 1. **reminders**: whoever asked ("tell me when it starts") gets
//!    `live.reminder`, whatever the stream's visibility allows them to see;
//! 2. **subscribers** of the channel get `live.started`: level ALL inbox
//!    and push, PERSONALIZED inbox only, NONE nothing. For a friends-only
//!    stream only subscribers who are friends of the owner; streams that
//!    are not listed (NO_FEED, UNLISTED, PRIVATE) are not announced;
//! 3. **friends** of the owner, for friends-only streams, get
//!    `live.started` with the owner as actor (friends may know who is
//!    behind a channel; nobody else is told).
//!
//! All notifications of one stream share a collapse key, so someone in two
//! groups, or a batch that is retried, ends up with one entry.
use std::collections::HashSet;

use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_social::graph;
use tokuly_video::channels::{self, ChannelRow, NotifyLevel};

use crate::{
    Live,
    access::{FRIENDS, PRIVATE, PUBLIC},
    events::{AnnounceJob, AnnouncePhase, JOB_ANNOUNCE},
    streams::{self, StreamRow},
};

fn phase_name(phase: AnnouncePhase) -> &'static str {
    match phase {
        AnnouncePhase::Reminders => "reminders",
        AnnouncePhase::Subscribers => "subscribers",
        AnnouncePhase::Friends => "friends",
    }
}

/// The phase after `phase` for a stream of this visibility, if any.
fn next_phase(phase: AnnouncePhase, visibility: i32) -> Option<AnnouncePhase> {
    match (phase, visibility) {
        (AnnouncePhase::Reminders, PUBLIC | FRIENDS) => Some(AnnouncePhase::Subscribers),
        (AnnouncePhase::Subscribers, FRIENDS) => Some(AnnouncePhase::Friends),
        _ => None,
    }
}

impl Live {
    /// Queues the announcement of a stream that went on air (once per stream).
    pub async fn start_announcing(&self, stream_id: Uuid) -> AppResult<()> {
        let first = sqlx::query(
            "UPDATE live_streams SET announced_at = now() WHERE id = $1 AND announced_at IS NULL AND status = 2",
        )
        .bind(stream_id)
        .execute(&self.core.db)
        .await?
        .rows_affected()
            == 1;
        if first {
            let job = AnnounceJob { stream_id, phase: AnnouncePhase::Reminders, after: None };
            self.core.jobs.enqueue_unique(JOB_ANNOUNCE, &format!("{stream_id}:reminders:start"), &job).await?;
        }
        Ok(())
    }

    fn started_notification(
        &self,
        stream: &StreamRow,
        channel: &ChannelRow,
        user_id: Uuid,
        kind: &str,
    ) -> NotificationRequest {
        // No actor: the channel speaks, not the account behind it.
        let mut request = NotificationRequest::new(
            user_id,
            kind,
            Localized::same(channel.name.clone()),
            Localized::new(
                format!("ライブ配信が始まったよ!「{}」", stream.title),
                format!("Live now: {}", stream.title),
            ),
            stream.path(),
        )
        .subject(streams::SUBJECT, stream.id)
        .collapse(format!("live.started:{}", stream.id))
        .data(serde_json::json!({
            "stream_id": stream.id,
            "short_id": stream.short_id,
            "channel_id": channel.id,
            "channel_handle": channel.handle,
            "channel_name": channel.name,
            "channel_icon_url": channel.icon_url(),
        }));
        if let Some(url) = stream.thumbnail_url() {
            request = request.image(url);
        }
        request
    }

    /// Handles one `live.announce` job: notifies the next batch of the
    /// job's phase and queues what comes after. Returns how many
    /// notifications were requested.
    pub async fn announce(&self, job: AnnounceJob) -> AppResult<u64> {
        let core = &self.core;
        let Some(stream) = streams::find_by_id(core, job.stream_id).await? else { return Ok(0) };
        if !stream.is_live() || stream.suspended {
            return Ok(0);
        }
        let Some(channel) = channels::find_by_id(core, stream.channel_id).await? else { return Ok(0) };
        let batch_size = self.cfg.announce_batch_size as usize;
        let hidden: HashSet<Uuid> = graph::blocked_ids_either(core, stream.owner_id).await?.into_iter().collect();
        let friends: HashSet<Uuid> = if stream.visibility == FRIENDS {
            graph::friend_ids(core, stream.owner_id).await?.into_iter().collect()
        } else {
            HashSet::new()
        };
        // Who of a batch may be told at all.
        let allowed = |user_id: &Uuid| {
            *user_id != stream.owner_id
                && !hidden.contains(user_id)
                && match stream.visibility {
                    FRIENDS => friends.contains(user_id),
                    PRIVATE => false,
                    _ => true,
                }
        };
        let after = job.after;
        let later = |user_id: &Uuid| after.is_none_or(|after| *user_id > after);

        // The batch: (recipient, push as well as inbox).
        let (batch, full): (Vec<(Uuid, bool)>, bool) = match job.phase {
            AnnouncePhase::Reminders => {
                let ids: Vec<Uuid> = sqlx::query_scalar(
                    "SELECT user_id FROM live_reminders WHERE stream_id = $1 AND ($2::uuid IS NULL OR user_id > $2)
                     ORDER BY user_id LIMIT $3",
                )
                .bind(stream.id)
                .bind(after)
                .bind(batch_size as i64)
                .fetch_all(&core.db)
                .await?;
                let full = ids.len() == batch_size;
                (ids.into_iter().map(|id| (id, true)).collect(), full)
            }
            AnnouncePhase::Subscribers => {
                let everything: HashSet<Uuid> =
                    channels::subscriber_ids(core, channel.id, NotifyLevel::All).await?.into_iter().collect();
                let ids: Vec<Uuid> = channels::subscriber_ids(core, channel.id, NotifyLevel::Personalized)
                    .await?
                    .into_iter()
                    .filter(later)
                    .take(batch_size)
                    .collect();
                let full = ids.len() == batch_size;
                (ids.into_iter().map(|id| (id, everything.contains(&id))).collect(), full)
            }
            AnnouncePhase::Friends => {
                let mut ids: Vec<Uuid> = friends.iter().copied().filter(later).collect();
                ids.sort_unstable();
                ids.truncate(batch_size);
                let full = ids.len() == batch_size;
                // Subscribers among the friends heard in the previous phase.
                let told: HashSet<Uuid> =
                    channels::subscriber_ids(core, channel.id, NotifyLevel::Personalized).await?.into_iter().collect();
                (ids.into_iter().filter(|id| !told.contains(id)).map(|id| (id, true)).collect(), full)
            }
        };
        let last = batch.last().map(|(user_id, _)| *user_id);

        // People with a reminder were told in the first phase.
        let reminded: HashSet<Uuid> = if job.phase == AnnouncePhase::Reminders || batch.is_empty() {
            HashSet::new()
        } else {
            let ids: Vec<Uuid> = batch.iter().map(|(id, _)| *id).collect();
            sqlx::query_scalar("SELECT user_id FROM live_reminders WHERE stream_id = $1 AND user_id = ANY($2)")
                .bind(stream.id)
                .bind(&ids)
                .fetch_all(&core.db)
                .await?
                .into_iter()
                .collect()
        };

        let mut tx = core.db.begin().await?;
        let mut notified = 0;
        for (user_id, push) in &batch {
            if !allowed(user_id) || reminded.contains(user_id) {
                continue;
            }
            let kind = if job.phase == AnnouncePhase::Reminders { "live.reminder" } else { "live.started" };
            let mut request = self.started_notification(&stream, &channel, *user_id, kind);
            if job.phase == AnnouncePhase::Friends {
                request = request.actor(stream.owner_id);
            }
            if !push {
                request = request.inbox_only();
            }
            outbound::notify(&mut *tx, &request).await?;
            notified += 1;
        }
        tx.commit().await?;

        let next = match (full, last) {
            (true, Some(last)) => Some(AnnounceJob { stream_id: stream.id, phase: job.phase, after: Some(last) }),
            _ => next_phase(job.phase, stream.visibility).map(|phase| AnnounceJob {
                stream_id: stream.id,
                phase,
                after: None,
            }),
        };
        if let Some(next) = next {
            let key = format!(
                "{}:{}:{}",
                stream.id,
                phase_name(next.phase),
                next.after.map(|id| id.to_string()).unwrap_or_else(|| "start".to_owned())
            );
            core.jobs.enqueue_unique(JOB_ANNOUNCE, &key, &next).await?;
        }
        Ok(notified)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::{NO_FEED, UNLISTED};

    #[test]
    fn who_hears_about_a_stream_depends_on_its_visibility() {
        use AnnouncePhase::*;
        // Public: reminders, then subscribers.
        assert_eq!(next_phase(Reminders, PUBLIC), Some(Subscribers));
        assert_eq!(next_phase(Subscribers, PUBLIC), None);
        // Friends-only: reminders, subscribers (the friends among them), friends.
        assert_eq!(next_phase(Reminders, FRIENDS), Some(Subscribers));
        assert_eq!(next_phase(Subscribers, FRIENDS), Some(Friends));
        assert_eq!(next_phase(Friends, FRIENDS), None);
        // Unlisted streams only reach people who asked.
        for visibility in [NO_FEED, UNLISTED, PRIVATE] {
            assert_eq!(next_phase(Reminders, visibility), None);
        }
    }
}
