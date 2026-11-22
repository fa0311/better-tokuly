//! Polls on threads: 2-6 options, single or multiple choice, an optional
//! deadline, and results that show once the viewer has voted.
use std::collections::HashMap;

use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_proto::community as pb;

use crate::{Community, validate::NewPoll};

fn poll_not_found() -> AppError {
    AppError::not_found("POLL_NOT_FOUND", "this thread has no poll")
}

/// A poll with what one viewer chose.
#[derive(Debug, Clone, Default)]
pub(crate) struct PollData {
    pub multiple: bool,
    pub closes_at: Option<Time>,
    pub closed_at: Option<Time>,
    pub voter_count: i32,
    /// `(id, text, votes)` in display order.
    pub options: Vec<(Uuid, String, i32)>,
    /// The options the viewer voted for.
    pub mine: Vec<Uuid>,
}

impl PollData {
    /// Closed by hand or past its deadline.
    pub fn is_closed(&self, now: Time) -> bool {
        self.closed_at.is_some() || self.closes_at.is_some_and(|closes| closes <= now)
    }

    /// The poll as the viewer sees it. Vote counts are filled in once the
    /// viewer voted, once the poll is closed, and for the thread's author.
    pub fn to_proto(&self, viewer_is_author: bool) -> pb::Poll {
        let closed = self.is_closed(now());
        let results_visible = closed || !self.mine.is_empty() || viewer_is_author;
        pb::Poll {
            options: self
                .options
                .iter()
                .map(|(id, text, votes)| pb::PollOption {
                    id: id.to_string(),
                    text: text.clone(),
                    vote_count: if results_visible { *votes } else { 0 },
                    ..Default::default()
                })
                .collect(),
            multiple_choice: self.multiple,
            closes_at: opt_ts(self.closes_at),
            closed,
            voter_count: self.voter_count,
            results_visible,
            my_option_ids: self.mine.iter().map(Uuid::to_string).collect(),
            ..Default::default()
        }
    }
}

impl Community {
    /// The polls of some threads, with the votes of `viewer`.
    pub(crate) async fn load_polls(
        &self,
        thread_ids: &[Uuid],
        viewer: Option<Uuid>,
    ) -> AppResult<HashMap<Uuid, PollData>> {
        if thread_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let db = &self.core.db;
        let polls: Vec<(Uuid, bool, Option<Time>, Option<Time>, i32)> = sqlx::query_as(
            "SELECT thread_id, multiple, closes_at, closed_at, voter_count FROM community_polls WHERE thread_id = ANY($1)",
        )
        .bind(thread_ids)
        .fetch_all(db)
        .await?;
        if polls.is_empty() {
            return Ok(HashMap::new());
        }
        let mut out: HashMap<Uuid, PollData> = polls
            .into_iter()
            .map(|(thread_id, multiple, closes_at, closed_at, voter_count)| {
                (thread_id, PollData { multiple, closes_at, closed_at, voter_count, ..Default::default() })
            })
            .collect();
        let with_polls: Vec<Uuid> = out.keys().copied().collect();
        let options: Vec<(Uuid, Uuid, String, i32)> = sqlx::query_as(
            "SELECT thread_id, id, text, vote_count FROM community_poll_options
             WHERE thread_id = ANY($1) ORDER BY thread_id, position",
        )
        .bind(&with_polls)
        .fetch_all(db)
        .await?;
        for (thread_id, id, text, votes) in options {
            if let Some(poll) = out.get_mut(&thread_id) {
                poll.options.push((id, text, votes));
            }
        }
        if let Some(viewer) = viewer {
            let votes: Vec<(Uuid, Uuid)> = sqlx::query_as(
                "SELECT thread_id, option_id FROM community_poll_votes WHERE thread_id = ANY($1) AND user_id = $2",
            )
            .bind(&with_polls)
            .bind(viewer)
            .fetch_all(db)
            .await?;
            for (thread_id, option_id) in votes {
                if let Some(poll) = out.get_mut(&thread_id) {
                    poll.mine.push(option_id);
                }
            }
        }
        Ok(out)
    }

    /// Stores the poll of a new thread.
    pub(crate) async fn insert_poll(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        thread_id: Uuid,
        poll: &NewPoll,
    ) -> AppResult<()> {
        sqlx::query("INSERT INTO community_polls (thread_id, multiple, closes_at) VALUES ($1, $2, $3)")
            .bind(thread_id)
            .bind(poll.multiple)
            .bind(poll.closes_at)
            .execute(&mut **tx)
            .await?;
        let positions: Vec<i32> = (0..poll.options.len() as i32).collect();
        sqlx::query(
            "INSERT INTO community_poll_options (thread_id, position, text)
             SELECT $1, * FROM unnest($2::int[], $3::text[])",
        )
        .bind(thread_id)
        .bind(&positions)
        .bind(&poll.options)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Replaces the vote of `user_id` in a thread's poll; no options takes
    /// the vote back.
    pub(crate) async fn vote(&self, thread_id: Uuid, user_id: Uuid, option_ids: &[Uuid]) -> AppResult<()> {
        let mut chosen: Vec<Uuid> = Vec::with_capacity(option_ids.len());
        for id in option_ids {
            if !chosen.contains(id) {
                chosen.push(*id);
            }
        }
        let mut tx = self.core.db.begin().await?;
        // Locking the poll serializes votes: the counters stay exact.
        let poll: Option<(bool, Option<Time>, Option<Time>)> = sqlx::query_as(
            "SELECT multiple, closes_at, closed_at FROM community_polls WHERE thread_id = $1 FOR UPDATE",
        )
        .bind(thread_id)
        .fetch_optional(&mut *tx)
        .await?;
        let (multiple, closes_at, closed_at) = poll.ok_or_else(poll_not_found)?;
        let closed = closed_at.is_some() || closes_at.is_some_and(|closes| closes <= now());
        ensure!(!closed, precondition, "POLL_CLOSED", "this poll is closed");
        ensure!(multiple || chosen.len() <= 1, invalid, "SINGLE_CHOICE", "choose one option in this poll");
        let known: i64 =
            sqlx::query_scalar("SELECT count(*) FROM community_poll_options WHERE thread_id = $1 AND id = ANY($2)")
                .bind(thread_id)
                .bind(&chosen)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(known == chosen.len() as i64, not_found, "OPTION_NOT_FOUND", "this poll has no such option");

        let previous: Vec<Uuid> = sqlx::query_scalar(
            "DELETE FROM community_poll_votes WHERE thread_id = $1 AND user_id = $2 RETURNING option_id",
        )
        .bind(thread_id)
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query("UPDATE community_poll_options SET vote_count = greatest(vote_count - 1, 0) WHERE id = ANY($1)")
            .bind(&previous)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO community_poll_votes (thread_id, option_id, user_id)
             SELECT $1, option_id, $3 FROM unnest($2::uuid[]) AS chosen(option_id)",
        )
        .bind(thread_id)
        .bind(&chosen)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE community_poll_options SET vote_count = vote_count + 1 WHERE id = ANY($1)")
            .bind(&chosen)
            .execute(&mut *tx)
            .await?;
        let delta = i32::from(!chosen.is_empty()) - i32::from(!previous.is_empty());
        sqlx::query("UPDATE community_polls SET voter_count = greatest(voter_count + $2, 0) WHERE thread_id = $1")
            .bind(thread_id)
            .bind(delta)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Ends a poll now. Closing a closed poll changes nothing.
    pub(crate) async fn close_poll(&self, thread_id: Uuid) -> AppResult<()> {
        let closed =
            sqlx::query("UPDATE community_polls SET closed_at = coalesce(closed_at, now()) WHERE thread_id = $1")
                .bind(thread_id)
                .execute(&self.core.db)
                .await?;
        ensure!(closed.rows_affected() > 0, not_found, "POLL_NOT_FOUND", "this thread has no poll");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn poll() -> PollData {
        PollData {
            multiple: false,
            closes_at: None,
            closed_at: None,
            voter_count: 3,
            options: vec![(Uuid::from_u128(1), "はい".into(), 2), (Uuid::from_u128(2), "いいえ".into(), 1)],
            mine: Vec::new(),
        }
    }

    #[test]
    fn results_stay_hidden_until_the_viewer_votes() {
        let hidden = poll().to_proto(false);
        assert!(!hidden.results_visible);
        assert!(hidden.options.iter().all(|option| option.vote_count == 0));
        // How many voted is always shown.
        assert_eq!(hidden.voter_count, 3);

        let voted = PollData { mine: vec![Uuid::from_u128(1)], ..poll() }.to_proto(false);
        assert!(voted.results_visible);
        assert_eq!(voted.options.iter().map(|option| option.vote_count).collect::<Vec<_>>(), vec![2, 1]);
        assert_eq!(voted.my_option_ids, vec![Uuid::from_u128(1).to_string()]);
    }

    #[test]
    fn the_author_and_everyone_after_closing_see_results() {
        assert!(poll().to_proto(true).results_visible);
        let by_hand = PollData { closed_at: Some(now()), ..poll() }.to_proto(false);
        assert!(by_hand.closed && by_hand.results_visible);
        let past_deadline = PollData { closes_at: Some(now() - Duration::seconds(1)), ..poll() };
        assert!(past_deadline.is_closed(now()) && past_deadline.to_proto(false).results_visible);
        let running = PollData { closes_at: Some(now() + Duration::hours(1)), ..poll() };
        assert!(!running.is_closed(now()) && !running.to_proto(false).results_visible);
    }
}
