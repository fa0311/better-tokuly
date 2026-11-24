//! The event consumer: where notifications come into being.
//!
//! Every domain asks for a notification by emitting `notify.requested`
//! (`tokuly_core::outbound::notify`). For each request this module
//!
//! 1. decides what the recipient wants ([`plan`]): mutes drop the request,
//!    category toggles gate the inbox and the push, and the push pause and
//!    quiet hours gate only the push — unless the request is urgent;
//! 2. stores the inbox entry, collapsing by key;
//! 3. queues the push job;
//! 4. tells the recipient's connected clients.
//!
//! Events arrive at least once. The event ID is recorded in the same
//! transaction as the inbox entry, so a redelivered event changes nothing.
use connectrpc::ErrorCode;
use sqlx::{Postgres, Transaction};
use tokuly_core::{
    events::Event,
    outbound::{NOTIFY_TOPIC, NotificationRequest},
    prelude::*,
};
use tokuly_proto::notify::{NotificationReceived, UnreadCount, notify_event::Event as NotifyEventKind};

use crate::{
    Notify, blocks,
    categories::{self, Category},
    delivery::PushJob,
    events::{
        FRIEND_PREFS_CHANGED_TOPIC, FriendPrefsChanged, USER_BLOCKED_TOPIC, USER_DELETED_TOPIC, USER_UNBLOCKED_TOPIC,
        UserDeleted,
    },
    inbox::{self, NewNotification, NotificationRow},
    locks,
    preferences::{self, DeliveryContext},
    preferences_svc::set_mute_state,
    sanitize::sanitize,
    withdraw::{self, CONTENT_REMOVED_TOPIC, UNWITHDRAWABLE_CATEGORY},
};

/// Durable consumer name on the event stream.
pub const CONSUMER_NAME: &str = "notify";
/// Topics the consumer subscribes to.
pub const TOPICS: &[&str] = &[
    NOTIFY_TOPIC,
    FRIEND_PREFS_CHANGED_TOPIC,
    USER_DELETED_TOPIC,
    USER_BLOCKED_TOPIC,
    USER_UNBLOCKED_TOPIC,
    CONTENT_REMOVED_TOPIC,
    // `withdraw::RULES`
    "chat.message.deleted",
    "engagement.comment.deleted",
    "notes.note.deleted",
    "notes.story.deleted",
    "community.thread.deleted",
    "video.playlist.deleted",
    "video.video.deleted",
    "shorts.short.deleted",
    "live.stream.deleted",
    "ask.answer.deleted",
    "video.video.unpublished",
    "notes.note.visibility_changed",
];

/// Why a notification was not pushed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushSkip {
    /// The producer asked for an inbox entry only.
    NotRequested,
    /// The recipient turned push off for the category.
    CategoryOff,
    /// The recipient paused all pushes.
    Paused,
    QuietHours,
    /// The recipient has no push registration.
    NoDevices,
    /// The recipient already got the per-minute maximum of pushes.
    RateLimited,
}

#[derive(Debug)]
pub enum PushDecision {
    Queued(PushJob),
    Skipped(PushSkip),
}

/// Why a request left no trace at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    /// Malformed request (no usable kind or title).
    Invalid,
    /// The recipient does not exist anymore.
    RecipientGone,
    /// Nobody is notified about their own actions.
    OwnAction,
    /// The recipient muted the person who caused it.
    Muted,
    /// The recipient and the person who caused it are in a block.
    Blocked,
    /// A newer event with the same collapse key was handled first.
    Superseded,
    /// What it is about was deleted or taken down before it was handled.
    Withdrawn,
}

/// What became of one `notify.requested` event.
#[derive(Debug)]
pub enum RequestOutcome {
    /// The event had been handled before (a redelivery).
    Duplicate,
    Dropped(DropReason),
    Handled {
        /// The inbox entry that was created or replaced.
        notification_id: Option<Uuid>,
        push: PushDecision,
    },
}

/// What to do with a request, given the recipient's preferences.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub inbox: bool,
    pub push: Result<(), PushSkip>,
}

/// Applies the recipient's preferences to a request. Mutes are handled
/// before this (they drop the request entirely).
///
/// The push pause and quiet hours mean "not now", and a request the producer
/// marked urgent (an incoming call, an SOS) is exactly what cannot wait: it
/// is pushed regardless. A category switched off and a mute mean "never from
/// this", so they hold for urgent requests too.
pub fn plan(request: &NotificationRequest, category: &Category, recipient: &DeliveryContext, now: Time) -> Plan {
    let inbox = request.inbox && (recipient.inbox || category.inbox_locked);
    let push = if !request.push {
        Err(PushSkip::NotRequested)
    } else if !recipient.push {
        Err(PushSkip::CategoryOff)
    } else if !request.urgent && recipient.push_paused_at(now) {
        Err(PushSkip::Paused)
    } else if !request.urgent && recipient.quiet_at(now) {
        Err(PushSkip::QuietHours)
    } else if !recipient.has_devices {
        Err(PushSkip::NoDevices)
    } else {
        Ok(())
    };
    Plan { inbox, push }
}

/// Records that an event is being handled. `false` when it already was.
async fn claim_event(tx: &mut Transaction<'_, Postgres>, event_id: Uuid) -> AppResult<bool> {
    let done = sqlx::query("INSERT INTO notify_processed_events (event_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(event_id)
        .execute(&mut **tx)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// The inbox entry a request asks for.
fn inbox_entry<'a>(
    request: &'a NotificationRequest,
    category: &'a Category,
    refs: &'a [Uuid],
    occurred_at: Time,
) -> NewNotification<'a> {
    NewNotification {
        user_id: request.user_id,
        kind: &request.kind,
        category: category.id,
        actor_id: request.actor_id,
        title: &request.title,
        body: &request.body,
        image_url: request.image_url.as_deref(),
        path: &request.path,
        subject: request.subject,
        collapse_key: request.collapse_key.as_deref(),
        data: &request.data,
        refs,
        urgent: request.urgent,
        occurred_at,
    }
}

/// The push job for a request. `stored` is its inbox entry, if it has one;
/// the badge is the recipient's unread count including that entry.
fn push_job(
    event: &Event,
    request: &NotificationRequest,
    category: &Category,
    stored: Option<&NotificationRow>,
    unread: &UnreadCount,
    withdrawable_refs: &[Uuid],
    blockable_actor: Option<Uuid>,
) -> PushJob {
    PushJob {
        id: event.id,
        user_id: request.user_id,
        actor_id: blockable_actor,
        notification_id: stored.map(|row| row.id),
        kind: request.kind.clone(),
        category: category.id.to_owned(),
        title: request.title.clone(),
        body: request.body.clone(),
        image_url: request.image_url.clone(),
        path: request.path.clone(),
        collapse_key: request.collapse_key.clone(),
        badge: i64::from(unread.total),
        data: request.data.clone(),
        refs: withdrawable_refs.to_vec(),
        urgent: request.urgent,
        created_at: event.occurred_at,
    }
}

impl Notify {
    /// Entry point of the durable consumer.
    pub async fn handle_event(&self, event: Event) -> AppResult<()> {
        match event.topic.as_str() {
            NOTIFY_TOPIC => self.handle_request(&event).await.map(|_| ()),
            FRIEND_PREFS_CHANGED_TOPIC => self.handle_friend_prefs_changed(&event).await,
            USER_BLOCKED_TOPIC => self.handle_block(&event, true).await,
            USER_UNBLOCKED_TOPIC => self.handle_block(&event, false).await,
            USER_DELETED_TOPIC => {
                let deleted: UserDeleted = event.data()?;
                self.purge_user(deleted.user_id).await
            }
            _ => match withdraw::withdrawal_of(&event)? {
                Some(withdrawal) => self.withdraw(&withdrawal).await,
                None => Ok(()),
            },
        }
    }

    /// Turns one `notify.requested` event into an inbox entry, a realtime
    /// event and a push job, as far as the recipient's preferences allow.
    pub async fn handle_request(&self, event: &Event) -> AppResult<RequestOutcome> {
        let request: NotificationRequest = event.data()?;
        let Some(request) = sanitize(request) else {
            tracing::warn!(event = %event.id, "dropping a malformed notification request");
            return Ok(RequestOutcome::Dropped(DropReason::Invalid));
        };
        if request.actor_id == Some(request.user_id) {
            return Ok(RequestOutcome::Dropped(DropReason::OwnAction));
        }
        let category = categories::of_kind(&request.kind);
        let Some(recipient) =
            preferences::delivery_context(&self.core, request.user_id, category.id, request.actor_id).await?
        else {
            return Ok(RequestOutcome::Dropped(DropReason::RecipientGone));
        };
        if recipient.muted {
            return Ok(RequestOutcome::Dropped(DropReason::Muted));
        }
        let plan = plan(&request, category, &recipient, now());
        let refs = withdraw::refs_of(&request);
        // System notices (account and security, moderation) are never
        // withdrawn, nor held back by a block.
        let system = category.id == UNWITHDRAWABLE_CATEGORY;
        let withdrawable_refs: &[Uuid] = if system { &[] } else { &refs };
        let blockable_actor = request.actor_id.filter(|_| !system);

        // Nothing above changed anything, so a redelivery may safely repeat
        // it. From here on the event ID guards against doing the work twice.
        let mut tx = self.core.db.begin().await?;
        if !claim_event(&mut tx, event.id).await? {
            return Ok(RequestOutcome::Duplicate);
        }
        // Checked under the locks a withdrawal and a block take, so that an
        // entry is either refused here or seen and removed by them.
        let mut keys: Vec<i64> = withdrawable_refs.iter().map(|id| locks::key(*id)).collect();
        keys.extend(blockable_actor.map(|actor_id| locks::pair_key(request.user_id, actor_id)));
        locks::lock_shared(&mut tx, keys).await?;
        if withdraw::any_withdrawn(&mut *tx, withdrawable_refs).await? {
            tx.commit().await?;
            return Ok(RequestOutcome::Dropped(DropReason::Withdrawn));
        }
        if let Some(actor_id) = blockable_actor
            && blocks::either_blocked(&mut *tx, request.user_id, actor_id).await?
        {
            tx.commit().await?;
            return Ok(RequestOutcome::Dropped(DropReason::Blocked));
        }
        let stored = if plan.inbox {
            let entry = inbox_entry(&request, category, &refs, event.occurred_at);
            match inbox::upsert(&mut tx, &entry).await? {
                Some(row) => Some(row),
                None => {
                    // The user already has a newer version of this entry (and
                    // its push); this older event has nothing to add.
                    tx.commit().await?;
                    return Ok(RequestOutcome::Dropped(DropReason::Superseded));
                }
            }
        } else {
            None
        };
        // Read inside the transaction so the count includes the new entry.
        let unread = inbox::unread_count(&mut *tx, request.user_id).await?;

        let push = match plan.push {
            Err(skip) => PushDecision::Skipped(skip),
            Ok(()) if !self.push_budget_left(&request).await? => PushDecision::Skipped(PushSkip::RateLimited),
            Ok(()) => {
                let job =
                    push_job(event, &request, category, stored.as_ref(), &unread, withdrawable_refs, blockable_actor);
                // Queued before the commit: if queuing fails nothing is
                // recorded and the event is retried as a whole. The job is
                // unique per event, so a retry after a failed commit does not
                // push twice.
                self.enqueue_push(&job).await?;
                PushDecision::Queued(job)
            }
        };
        tx.commit().await?;

        if let Some(row) = &stored {
            self.announce(row, &recipient.locale, unread).await;
        }
        Ok(RequestOutcome::Handled { notification_id: stored.map(|row| row.id), push })
    }

    /// Tells the recipient's connected clients about a new or replaced inbox
    /// entry. Best effort: the entry is committed, and a client that misses
    /// the event finds it the next time it lists the inbox.
    async fn announce(&self, row: &NotificationRow, locale: &str, unread: UnreadCount) {
        let notification = match inbox::to_protos(&self.core, std::slice::from_ref(row), locale).await {
            Ok(mut notifications) => notifications.pop(),
            Err(e) => {
                tracing::warn!(notification = %row.id, error = %e, "could not announce a notification");
                return;
            }
        };
        let received =
            NotificationReceived { notification: notification.into(), unread: unread.into(), ..Default::default() };
        self.publish(row.user_id, NotifyEventKind::from(received)).await;
    }

    /// The push caps: a burst of events (a busy group chat, a viral post)
    /// must not turn into a buzzing phone, and one person must not use up
    /// the recipient's budget so that nobody else's pushes get through.
    /// Returns whether one more push fits; notifications over a cap still
    /// reach the inbox.
    ///
    /// Each sender has a per-recipient budget, checked first so that a
    /// flood only spends its own. An urgent push is never the one a storm of
    /// ordinary ones crowds out: it has a sender budget of its own (so
    /// nobody can ring through quiet hours without end) and neither checks
    /// nor uses up the recipient's overall budget.
    async fn push_budget_left(&self, request: &NotificationRequest) -> AppResult<bool> {
        let user_id = request.user_id;
        if let Some(actor_id) = request.actor_id {
            let kind = if request.urgent { "urgent" } else { "push" };
            let key = format!("notify-{kind}:{user_id}:{actor_id}");
            if !self.within_rate(&key, self.cfg.push_rate_per_sender_per_minute).await? {
                return Ok(false);
            }
        }
        if request.urgent {
            return Ok(true);
        }
        self.within_rate(&format!("notify-push:{user_id}"), self.cfg.push_rate_per_minute).await
    }

    /// Counts one push against a per-minute budget of `max` (0 = no cap).
    async fn within_rate(&self, key: &str, max: u32) -> AppResult<bool> {
        if max == 0 {
            return Ok(true);
        }
        match self.core.cache.rate_limit(key, max, std::time::Duration::from_secs(60)).await {
            Ok(()) => Ok(true),
            Err(e) if e.code == ErrorCode::ResourceExhausted => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Mirrors the "mute this friend" preference of the social domain.
    pub async fn handle_friend_prefs_changed(&self, event: &Event) -> AppResult<()> {
        let change: FriendPrefsChanged = event.data()?;
        set_mute_state(&self.core.db, change.user_id, change.friend_id, change.muted, event.occurred_at).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    #[test]
    fn every_deletion_topic_is_consumed() {
        for rule in withdraw::RULES {
            assert!(TOPICS.contains(&rule.topic), "{}", rule.topic);
        }
    }
    use tokuly_core::outbound::Localized;

    use super::*;

    fn request() -> NotificationRequest {
        NotificationRequest::new(
            Uuid::now_v7(),
            "chat.message",
            Localized::new("ありす", "Alice"),
            Localized::new("やっほー", "Hi"),
            "/chat/1",
        )
    }

    fn recipient() -> DeliveryContext {
        DeliveryContext {
            locale: "ja".into(),
            time_zone: "Asia/Tokyo".into(),
            inbox: true,
            push: true,
            push_paused: false,
            push_paused_until: None,
            quiet_enabled: false,
            quiet_start_minute: 1320,
            quiet_end_minute: 420,
            muted: false,
            has_devices: true,
        }
    }

    /// 03:00 in Tokyo: inside the default 22:00-07:00 window.
    fn night() -> Time {
        chrono::Utc.with_ymd_and_hms(2026, 11, 20, 18, 0, 0).unwrap()
    }

    fn chat() -> &'static Category {
        categories::find("chat").unwrap()
    }

    #[test]
    fn defaults_deliver_everywhere() {
        assert_eq!(plan(&request(), chat(), &recipient(), night()), Plan { inbox: true, push: Ok(()) });
    }

    #[test]
    fn the_producer_chooses_the_channels() {
        let plan_of = |request: NotificationRequest| plan(&request, chat(), &recipient(), night());
        assert_eq!(plan_of(request().push_only()), Plan { inbox: false, push: Ok(()) });
        assert_eq!(plan_of(request().inbox_only()), Plan { inbox: true, push: Err(PushSkip::NotRequested) });
    }

    #[test]
    fn category_toggles_gate_inbox_and_push_separately() {
        let no_inbox = DeliveryContext { inbox: false, ..recipient() };
        assert_eq!(plan(&request(), chat(), &no_inbox, night()), Plan { inbox: false, push: Ok(()) });
        let no_push = DeliveryContext { push: false, ..recipient() };
        assert_eq!(plan(&request(), chat(), &no_push, night()), Plan { inbox: true, push: Err(PushSkip::CategoryOff) });
        // A locked category stays in the inbox even if a stale row says otherwise.
        let system = categories::find("system").unwrap();
        assert!(plan(&request(), system, &no_inbox, night()).inbox);
    }

    #[test]
    fn pause_and_quiet_hours_silence_the_push_but_not_the_inbox() {
        let paused = DeliveryContext { push_paused: true, ..recipient() };
        assert_eq!(plan(&request(), chat(), &paused, night()), Plan { inbox: true, push: Err(PushSkip::Paused) });
        let expired = DeliveryContext {
            push_paused: true,
            push_paused_until: Some(night() - chrono::Duration::minutes(1)),
            ..recipient()
        };
        assert_eq!(plan(&request(), chat(), &expired, night()).push, Ok(()));

        let quiet = DeliveryContext { quiet_enabled: true, ..recipient() };
        assert_eq!(plan(&request(), chat(), &quiet, night()), Plan { inbox: true, push: Err(PushSkip::QuietHours) });
        let noon = night() + chrono::Duration::hours(9);
        assert_eq!(plan(&request(), chat(), &quiet, noon).push, Ok(()));
    }

    #[test]
    fn urgent_requests_break_through_not_now_but_not_through_never() {
        let urgent = || request().urgent();
        // "Not now": the pause and quiet hours, even both at once.
        let silenced = DeliveryContext { push_paused: true, quiet_enabled: true, ..recipient() };
        assert_eq!(plan(&request(), chat(), &silenced, night()).push, Err(PushSkip::Paused));
        assert_eq!(plan(&urgent(), chat(), &silenced, night()), Plan { inbox: true, push: Ok(()) });
        // "Never from this category" still holds.
        let category_off = DeliveryContext { push: false, ..silenced.clone() };
        assert_eq!(plan(&urgent(), chat(), &category_off, night()).push, Err(PushSkip::CategoryOff));
        // And urgency cannot conjure a push the producer or the user's
        // devices do not allow for.
        assert_eq!(plan(&urgent().inbox_only(), chat(), &silenced, night()).push, Err(PushSkip::NotRequested));
        let no_devices = DeliveryContext { has_devices: false, ..silenced };
        assert_eq!(plan(&urgent(), chat(), &no_devices, night()).push, Err(PushSkip::NoDevices));
    }

    #[test]
    fn no_devices_means_no_push_job() {
        let no_devices = DeliveryContext { has_devices: false, ..recipient() };
        assert_eq!(plan(&request(), chat(), &no_devices, night()).push, Err(PushSkip::NoDevices));
    }
}
