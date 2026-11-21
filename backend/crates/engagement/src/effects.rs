//! What a committed change still owes the outside world.
//!
//! Code that changes comments, reactions or bookmarks inside a transaction
//! records the consequences in an [`Effects`] value instead of acting on
//! them: the owning domain's counter callbacks, realtime pushes, the owner's
//! review-queue badge. [`Engagement::apply`] carries them out once the
//! transaction has committed — never before, so nobody is told about a
//! change that was rolled back, and never inside it, so a slow subscriber
//! cannot hold locks.
use tokuly_core::prelude::*;
use tokuly_proto::{
    engagement::{CommentAdded, CommentPinned, CommentRemoved, CommentUpdated, EngagementEvent, ReviewQueueChanged},
    realtime::server_frame::Body,
};
use tokuly_social::graph;

use crate::{
    Engagement, comments,
    stats::Counter,
    subjects::SubjectKey,
};

/// A realtime event about a comment, as far as it is known inside the
/// transaction. Events carrying the comment itself are filled in from the
/// committed row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Push {
    /// The comment entered public view.
    Added { key: SubjectKey, comment_id: Uuid },
    /// A comment in public view changed.
    Updated { key: SubjectKey, comment_id: Uuid },
    /// The comment left public view, with `removed_count` comments in all.
    Removed { key: SubjectKey, comment_id: Uuid, parent_id: Option<Uuid>, removed_count: i32 },
    Pinned { key: SubjectKey, comment_id: Uuid, pinned: bool },
}

#[derive(Debug, Default)]
pub(crate) struct Effects {
    counters: Vec<(SubjectKey, Counter, i64)>,
    pushes: Vec<Push>,
    review_queues: Vec<Uuid>,
}

impl Effects {
    /// A counter of `key` moved by `delta` in this transaction.
    pub(crate) fn count(&mut self, key: SubjectKey, counter: Counter, delta: i64) {
        match self.counters.iter_mut().find(|(k, c, _)| *k == key && *c == counter) {
            Some((_, _, total)) => *total += delta,
            None => self.counters.push((key, counter, delta)),
        }
    }

    pub(crate) fn push(&mut self, push: Push) {
        self.pushes.push(push);
    }

    /// The number of comments waiting for `owner_id`'s review may have changed.
    pub(crate) fn review_queue(&mut self, owner_id: Uuid) {
        if !owner_id.is_nil() && !self.review_queues.contains(&owner_id) {
            self.review_queues.push(owner_id);
        }
    }

    #[cfg(test)]
    pub(crate) fn counted(&self, key: SubjectKey, counter: Counter) -> i64 {
        self.counters.iter().filter(|(k, c, _)| *k == key && *c == counter).map(|(_, _, delta)| delta).sum()
    }
}

impl Engagement {
    /// Carries out the effects of a committed transaction. Nothing here can
    /// fail the request: what could not be delivered is logged.
    pub(crate) async fn apply(&self, effects: Effects) {
        let mut dirty: Vec<SubjectKey> = Vec::new();
        for (key, counter, delta) in effects.counters {
            self.counter_changed(key, counter, delta).await;
            if !dirty.contains(&key) {
                dirty.push(key);
            }
        }
        for push in effects.pushes {
            if let Err(error) = self.push_comment_event(push).await {
                tracing::warn!(%error, "comment event was not pushed");
            }
        }
        for key in dirty {
            self.counters_dirty(key).await;
        }
        for owner_id in effects.review_queues {
            if let Err(error) = self.push_review_queue(owner_id).await {
                tracing::warn!(%error, "review queue count was not pushed");
            }
        }
    }

    async fn push_comment_event(&self, push: Push) -> AppResult<()> {
        let (key, event) = match push {
            Push::Added { key, comment_id } => {
                let Some((parent_id, comment)) = self.public_comment(comment_id).await? else { return Ok(()) };
                let event = CommentAdded {
                    comment_id: comment_id.to_string(),
                    parent_id: parent_id.map(|id| id.to_string()).unwrap_or_default(),
                    comment: comment.into(),
                    ..Default::default()
                };
                (key, event.into())
            }
            Push::Updated { key, comment_id } => {
                let Some((parent_id, comment)) = self.public_comment(comment_id).await? else { return Ok(()) };
                let event = CommentUpdated {
                    comment_id: comment_id.to_string(),
                    parent_id: parent_id.map(|id| id.to_string()).unwrap_or_default(),
                    comment: comment.into(),
                    ..Default::default()
                };
                (key, event.into())
            }
            Push::Removed { key, comment_id, parent_id, removed_count } => {
                let event = CommentRemoved {
                    comment_id: comment_id.to_string(),
                    parent_id: parent_id.map(|id| id.to_string()).unwrap_or_default(),
                    removed_count,
                    ..Default::default()
                };
                (key, event.into())
            }
            Push::Pinned { key, comment_id, pinned } => {
                (key, CommentPinned { comment_id: comment_id.to_string(), pinned, ..Default::default() }.into())
            }
        };
        let frame = EngagementEvent { subject: key.to_ref().into(), event: Some(event), ..Default::default() };
        self.core.rt.notify_topic(&key.topic(), Body::from(frame)).await;
        Ok(())
    }

    /// The comment as the public sees it now, for an event — `None` when it
    /// is not (or no longer) in public view. The comment itself is left out
    /// (`Some((_, None))`) when its author has blocks in place: a topic
    /// cannot tell its subscribers apart, so they fetch it and the blocked
    /// ones are refused.
    async fn public_comment(&self, comment_id: Uuid) -> AppResult<Option<(Option<Uuid>, Option<tokuly_proto::engagement::Comment>)>> {
        let Some(row) = comments::find(&self.core.db, comment_id).await? else { return Ok(None) };
        let parent = match row.parent_id {
            Some(parent_id) => comments::find(&self.core.db, parent_id).await?,
            None => None,
        };
        if !row.in_public_view(parent.as_ref()) {
            return Ok(None);
        }
        if !graph::blocked_ids_either(&self.core, row.author_id).await?.is_empty() {
            return Ok(Some((row.parent_id, None)));
        }
        let comment = self.render_comments(std::slice::from_ref(&row), None, &Default::default()).await?.pop();
        Ok(Some((row.parent_id, comment)))
    }

    async fn push_review_queue(&self, owner_id: Uuid) -> AppResult<()> {
        let held_count = comments::held_count(&self.core.db, owner_id).await?;
        let frame = EngagementEvent {
            event: Some(ReviewQueueChanged { held_count, ..Default::default() }.into()),
            ..Default::default()
        };
        self.core.rt.notify_user(owner_id, Body::from(frame)).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tokuly_proto::common::SubjectType;

    use super::*;

    #[test]
    fn counter_deltas_of_one_subject_add_up() {
        let (a, b) =
            (SubjectKey::new(SubjectType::SUBJECT_TYPE_VIDEO, Uuid::now_v7()), SubjectKey::new(SubjectType::SUBJECT_TYPE_NOTE, Uuid::now_v7()));
        let mut effects = Effects::default();
        effects.count(a, Counter::Comment, 3);
        effects.count(a, Counter::Comment, -1);
        effects.count(a, Counter::Like, 1);
        effects.count(b, Counter::Comment, 1);
        assert_eq!(effects.counted(a, Counter::Comment), 2);
        assert_eq!(effects.counted(a, Counter::Like), 1);
        assert_eq!(effects.counted(b, Counter::Comment), 1);
        assert_eq!(effects.counters.len(), 3);
    }

    #[test]
    fn each_owner_is_told_once_and_nobody_owns_nothing() {
        let owner = Uuid::now_v7();
        let mut effects = Effects::default();
        effects.review_queue(owner);
        effects.review_queue(owner);
        effects.review_queue(Uuid::nil());
        assert_eq!(effects.review_queues, [owner]);
    }
}
