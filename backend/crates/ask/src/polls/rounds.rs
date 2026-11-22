//! Rounds of compliment polls: starting one, drawing each poll, voting,
//! skipping, shuffling, and the reward at the end.
//!
//! A round is a list of questions fixed when it starts. Its polls are drawn
//! one at a time, when the round reaches them, so the four friends shown are
//! always people who are the voter's friends *now*.
//!
//! # What a vote must not reveal
//!
//! A vote for someone who blocked the voter ("whoever sent this") is stored
//! as a dropped flame and otherwise treated exactly like any other vote:
//! it advances the round, counts as a vote and earns the same coins. If a
//! dropped vote earned less, or the blocker stopped appearing among the
//! choices, the voter could tell they were blocked. For the same reason
//! [`Ask::eligible_friends`] knows nothing about sender blocks.
use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use sqlx::{Postgres, Transaction};
use tokuly_core::{events, prelude::*};
use tokuly_proto::ask::{Poll, Round, RoundState};
use tokuly_social::graph;
use tokuly_wallet::ledger;

use super::{
    bank::poll_question,
    draw::{self, CHOICES},
};
use crate::{
    Ask,
    events::{FLAME_STORED, ROUND_COMPLETED, RoundCompleted, Stored},
};

/// Ledger reason of the coins a completed round pays.
pub(crate) const REWARD_REASON: &str = "ask.round";
/// Earlier rounds whose questions a new round tries not to repeat.
const RECENT_ROUNDS: i64 = 3;

const WAITING: i16 = 0;
const VOTED: i16 = 1;
const SKIPPED: i16 = 2;

#[derive(Debug, Clone, sqlx::FromRow)]
struct RoundRow {
    id: Uuid,
    question_ids: Vec<Uuid>,
    answered: i32,
    votes: i32,
    completed_at: Option<DateTime<Utc>>,
    reward_coins: i64,
    rewarded_at: Option<DateTime<Utc>>,
}

const ROUND_COLUMNS: &str = "id, question_ids, answered, votes, completed_at, reward_coins, rewarded_at";

impl RoundRow {
    fn poll_count(&self) -> i32 {
        self.question_ids.len() as i32
    }

    fn is_complete(&self) -> bool {
        self.completed_at.is_some()
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct PollRow {
    id: Uuid,
    position: i32,
    question_id: Uuid,
    choices: Vec<Uuid>,
    shuffles_used: i32,
}

const POLL_COLUMNS: &str = "id, position, question_id, choices, shuffles_used";

/// Coins a round pays: the full reward for a round of votes, proportionally
/// less for skipped polls, nothing for a round that was only skipped.
pub(crate) fn reward_for(full_reward: i64, votes: i32, polls: i32) -> i64 {
    if polls <= 0 || votes <= 0 {
        return 0;
    }
    full_reward.max(0) * i64::from(votes.min(polls)) / i64::from(polls)
}

/// The friends a shuffle shows: as many others as there are, topped up from
/// the four shown now. `None` when nobody else is available.
fn reshuffled(current: &[Uuid], others: Vec<Uuid>) -> Option<Vec<Uuid>> {
    if others.is_empty() {
        return None;
    }
    let free_seats = CHOICES.saturating_sub(others.len());
    let kept: Vec<Uuid> = current.iter().copied().filter(|id| !others.contains(id)).take(free_seats).collect();
    let mut next = others;
    next.extend(kept);
    Some(next)
}

/// Draws without holding the thread's generator across an await.
fn draw_now(eligible: &[Uuid], shown: &HashMap<Uuid, i32>, exclude: &[Uuid], count: usize) -> Vec<Uuid> {
    draw::draw(eligible, shown, exclude, count, &mut rand::rng())
}

fn not_enough_friends() -> AppError {
    AppError::precondition("NOT_ENOUGH_FRIENDS", "a poll needs four friends who take part in compliment polls")
}

fn round_not_active() -> AppError {
    AppError::precondition("ROUND_NOT_ACTIVE", "there is no round in progress")
}

fn poll_not_current() -> AppError {
    AppError::precondition("POLL_NOT_CURRENT", "this is not the poll the round is on")
}

impl Ask {
    /// The friends of `user` who can be shown in their polls: active
    /// accounts, with no block in either direction, who did not opt out of
    /// polls, and whom `user` did not mute.
    ///
    /// Deliberately not considered: whether a friend blocked `user` as an
    /// anonymous sender. Such a friend still appears, and a vote for them is
    /// dropped silently (see the module documentation).
    pub(crate) async fn eligible_friends(&self, user: Uuid) -> AppResult<Vec<Uuid>> {
        let friends = graph::friend_ids(&self.core, user).await?;
        if friends.is_empty() {
            return Ok(Vec::new());
        }
        let hidden = graph::blocked_ids_either(&self.core, user).await?;
        Ok(sqlx::query_scalar(
            "SELECT u.id FROM users u
             WHERE u.id = ANY($1) AND u.id <> $3 AND NOT (u.id = ANY($2)) AND u.deleted_at IS NULL AND u.banned_at IS NULL
               AND NOT EXISTS (SELECT 1 FROM ask_settings s WHERE s.user_id = u.id AND NOT s.in_polls)
               AND NOT EXISTS (SELECT 1 FROM ask_friend_links l WHERE l.user_id = $3 AND l.friend_id = u.id AND l.muted)",
        )
        .bind(friends)
        .bind(hidden)
        .bind(user)
        .fetch_all(&self.core.db)
        .await?)
    }

    /// How often each of `eligible` was shown to `user`. Friends without a
    /// count are given one first (see [`draw::enrollment`]), so the result
    /// covers everyone.
    async fn shown_counts(
        tx: &mut Transaction<'_, Postgres>,
        user: Uuid,
        eligible: &[Uuid],
    ) -> AppResult<HashMap<Uuid, i32>> {
        let read = "SELECT friend_id, shown_count FROM ask_poll_exposure WHERE user_id = $1 AND friend_id = ANY($2)";
        let known: Vec<(Uuid, i32)> = sqlx::query_as(read).bind(user).bind(eligible).fetch_all(&mut **tx).await?;
        let mut shown: HashMap<Uuid, i32> = known.into_iter().collect();
        let (newcomers, start) = draw::enrollment(eligible, &shown);
        if !newcomers.is_empty() {
            sqlx::query(
                "INSERT INTO ask_poll_exposure (user_id, friend_id, shown_count)
                 SELECT $1, friend, $3 FROM UNNEST($2::uuid[]) AS friend
                 ON CONFLICT (user_id, friend_id) DO NOTHING",
            )
            .bind(user)
            .bind(&newcomers)
            .bind(start)
            .execute(&mut **tx)
            .await?;
            shown.extend(newcomers.into_iter().map(|id| (id, start)));
        }
        Ok(shown)
    }

    /// Counts one appearance for each of `friends`.
    async fn record_shown(tx: &mut Transaction<'_, Postgres>, user: Uuid, friends: &[Uuid]) -> AppResult<()> {
        sqlx::query("UPDATE ask_poll_exposure SET shown_count = shown_count + 1, last_shown_at = now() WHERE user_id = $1 AND friend_id = ANY($2)")
            .bind(user)
            .bind(friends)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    /// Serializes everything one user does to their rounds.
    async fn lock_rounds(tx: &mut Transaction<'_, Postgres>, user: Uuid) -> AppResult<()> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("ask:round:{user}"))
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    async fn active_round(tx: &mut Transaction<'_, Postgres>, user: Uuid) -> AppResult<Option<RoundRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {ROUND_COLUMNS} FROM ask_rounds WHERE user_id = $1 AND completed_at IS NULL"
        )))
        .bind(user)
        .fetch_optional(&mut **tx)
        .await?)
    }

    async fn latest_round(tx: &mut Transaction<'_, Postgres>, user: Uuid) -> AppResult<Option<RoundRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {ROUND_COLUMNS} FROM ask_rounds WHERE user_id = $1 ORDER BY id DESC LIMIT 1"
        )))
        .bind(user)
        .fetch_optional(&mut **tx)
        .await?)
    }

    /// The poll the round is on, if it has been drawn.
    async fn current_poll(tx: &mut Transaction<'_, Postgres>, round: &RoundRow) -> AppResult<Option<PollRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {POLL_COLUMNS} FROM ask_polls WHERE round_id = $1 AND position = $2 AND outcome = $3"
        )))
        .bind(round.id)
        .bind(round.answered)
        .bind(WAITING)
        .fetch_optional(&mut **tx)
        .await?)
    }

    fn cooldown(&self) -> Duration {
        Duration::seconds(self.cfg.round_cooldown_seconds.min(i64::MAX as u64) as i64)
    }

    /// Starts a round: picks its questions, preferring ones the user has
    /// not had in their last few rounds.
    async fn start_round(&self, tx: &mut Transaction<'_, Postgres>, user: Uuid) -> AppResult<RoundRow> {
        let question_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT q.id FROM ask_poll_questions q
             WHERE q.enabled
             ORDER BY (q.id IN (SELECT unnest(r.question_ids) FROM
                                  (SELECT question_ids FROM ask_rounds WHERE user_id = $1 ORDER BY id DESC LIMIT $3) r)),
                      random()
             LIMIT $2",
        )
        .bind(user)
        .bind(i64::from(self.cfg.round_polls))
        .bind(RECENT_ROUNDS)
        .fetch_all(&mut **tx)
        .await?;
        ensure!(!question_ids.is_empty(), unavailable, "POLLS_UNAVAILABLE", "there are no poll questions right now");
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO ask_rounds (user_id, question_ids) VALUES ($1, $2) RETURNING {ROUND_COLUMNS}"
        )))
        .bind(user)
        .bind(question_ids)
        .fetch_one(&mut **tx)
        .await?)
    }

    /// Draws the poll at the round's position. `None` when fewer than four
    /// friends are eligible right now.
    async fn draw_poll(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user: Uuid,
        round: &RoundRow,
    ) -> AppResult<Option<PollRow>> {
        let Some(question_id) = round.question_ids.get(round.answered as usize).copied() else { return Ok(None) };
        let eligible = self.eligible_friends(user).await?;
        if eligible.len() < CHOICES {
            return Ok(None);
        }
        let shown = Self::shown_counts(tx, user, &eligible).await?;
        let choices = draw_now(&eligible, &shown, &[], CHOICES);
        Self::record_shown(tx, user, &choices).await?;
        let poll: PollRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO ask_polls (round_id, user_id, position, question_id, choices) VALUES ($1, $2, $3, $4, $5) RETURNING {POLL_COLUMNS}"
        )))
        .bind(round.id)
        .bind(user)
        .bind(round.answered)
        .bind(question_id)
        .bind(&choices)
        .fetch_one(&mut **tx)
        .await?;
        Ok(Some(poll))
    }

    /// Replaces choices of a waiting poll who stopped being eligible since
    /// it was drawn (unfriended, blocked, account gone, opted out). Not a
    /// shuffle: it costs the voter nothing.
    async fn refresh_poll(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user: Uuid,
        poll: PollRow,
    ) -> AppResult<Option<PollRow>> {
        let eligible = self.eligible_friends(user).await?;
        if poll.choices.iter().all(|id| eligible.contains(id)) {
            return Ok(Some(poll));
        }
        if eligible.len() < CHOICES {
            return Ok(None);
        }
        let kept: Vec<Uuid> = poll.choices.iter().copied().filter(|id| eligible.contains(id)).collect();
        let shown = Self::shown_counts(tx, user, &eligible).await?;
        let replacements = draw_now(&eligible, &shown, &kept, CHOICES - kept.len());
        Self::record_shown(tx, user, &replacements).await?;
        let mut replacements = replacements.into_iter();
        // Each seat keeps its friend, or gets a new one in the same place.
        let choices: Vec<Uuid> = poll
            .choices
            .iter()
            .filter_map(|id| if eligible.contains(id) { Some(*id) } else { replacements.next() })
            .collect();
        Ok(Some(
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "UPDATE ask_polls SET choices = $2 WHERE id = $1 RETURNING {POLL_COLUMNS}"
            )))
            .bind(poll.id)
            .bind(&choices)
            .fetch_one(&mut **tx)
            .await?,
        ))
    }

    /// The caller's round: the one in progress with its current poll, the
    /// cooldown after the last one, or a new round when one is due.
    pub(crate) async fn current_round(&self, user: Uuid) -> AppResult<Round> {
        self.ensure_bank().await?;
        let mut tx = self.core.db.begin().await?;
        Self::lock_rounds(&mut tx, user).await?;
        let round = match Self::active_round(&mut tx, user).await? {
            Some(round) => round,
            None => {
                if let Some(last) = Self::latest_round(&mut tx, user).await?
                    && let Some(completed_at) = last.completed_at
                    && completed_at + self.cooldown() > Utc::now()
                {
                    tx.commit().await?;
                    // A reward that could not be paid when the round ended.
                    let last = self.settle_reward(user, last).await?;
                    return Ok(self.cooldown_view(&last));
                }
                // Checked before the round exists, so a user without enough
                // friends is not left with an empty round.
                ensure!(
                    self.eligible_friends(user).await?.len() >= CHOICES,
                    precondition,
                    "NOT_ENOUGH_FRIENDS",
                    "a poll needs four friends who take part in compliment polls"
                );
                self.start_round(&mut tx, user).await?
            }
        };
        let poll = match Self::current_poll(&mut tx, &round).await? {
            Some(poll) => self.refresh_poll(&mut tx, user, poll).await?,
            None => self.draw_poll(&mut tx, user, &round).await?,
        };
        let poll = poll.ok_or_else(not_enough_friends)?;
        tx.commit().await?;
        self.active_view(&round, Some(&poll)).await
    }

    /// Votes for `picked` in the poll the round is on.
    pub(crate) async fn cast_vote(&self, user: Uuid, poll_id: Uuid, picked: Uuid) -> AppResult<Round> {
        let mut tx = self.core.db.begin().await?;
        Self::lock_rounds(&mut tx, user).await?;
        let round = Self::active_round(&mut tx, user).await?.ok_or_else(round_not_active)?;
        let poll = Self::current_poll(&mut tx, &round)
            .await?
            .filter(|poll| poll.id == poll_id)
            .ok_or_else(poll_not_current)?;
        ensure!(poll.choices.contains(&picked), invalid, "CHOICE_INVALID", "pick one of the four friends shown");
        // Someone who stopped being a friend since the poll was drawn.
        ensure!(
            self.eligible_friends(user).await?.contains(&picked),
            invalid,
            "CHOICE_INVALID",
            "this friend can no longer be picked; shuffle or skip"
        );

        // The same statements for every voter: a blocked one differs in the
        // value of `dropped` and nowhere else.
        let dropped = Self::is_sender_blocked(&mut *tx, picked, user).await?;
        let flame_id: Uuid = sqlx::query_scalar("INSERT INTO ask_flames (receiver_id, sender_id, question_id, dropped) VALUES ($1, $2, $3, $4) RETURNING id")
            .bind(picked)
            .bind(user)
            .bind(poll.question_id)
            .bind(dropped)
            .fetch_one(&mut *tx)
            .await?;
        // No actor: the event stream must not say who voted.
        events::emit(&mut *tx, FLAME_STORED, None, &Stored { id: flame_id }).await?;
        sqlx::query("UPDATE ask_poll_questions SET use_count = use_count + 1 WHERE id = $1")
            .bind(poll.question_id)
            .execute(&mut *tx)
            .await?;
        self.advance(tx, user, round, poll, VOTED).await
    }

    /// Skips the poll the round is on.
    pub(crate) async fn pass_poll(&self, user: Uuid, poll_id: Uuid) -> AppResult<Round> {
        let mut tx = self.core.db.begin().await?;
        Self::lock_rounds(&mut tx, user).await?;
        let round = Self::active_round(&mut tx, user).await?.ok_or_else(round_not_active)?;
        let poll = Self::current_poll(&mut tx, &round)
            .await?
            .filter(|poll| poll.id == poll_id)
            .ok_or_else(poll_not_current)?;
        self.advance(tx, user, round, poll, SKIPPED).await
    }

    /// Shows other friends in the poll the round is on.
    pub(crate) async fn reshuffle_poll(&self, user: Uuid, poll_id: Uuid) -> AppResult<Round> {
        let mut tx = self.core.db.begin().await?;
        Self::lock_rounds(&mut tx, user).await?;
        let round = Self::active_round(&mut tx, user).await?.ok_or_else(round_not_active)?;
        let poll = Self::current_poll(&mut tx, &round)
            .await?
            .filter(|poll| poll.id == poll_id)
            .ok_or_else(poll_not_current)?;
        ensure!(
            poll.shuffles_used < self.cfg.shuffles_per_poll as i32,
            precondition,
            "NO_SHUFFLES_LEFT",
            "no shuffles left for this poll"
        );
        let eligible = self.eligible_friends(user).await?;
        let shown = Self::shown_counts(&mut tx, user, &eligible).await?;
        let others = draw_now(&eligible, &shown, &poll.choices, CHOICES);
        Self::record_shown(&mut tx, user, &others).await?;
        let still_eligible: Vec<Uuid> = poll.choices.iter().copied().filter(|id| eligible.contains(id)).collect();
        let choices = reshuffled(&still_eligible, others)
            .filter(|choices| choices.len() == CHOICES)
            .ok_or_else(not_enough_friends)?;
        let poll: PollRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE ask_polls SET choices = $2, shuffles_used = shuffles_used + 1 WHERE id = $1 RETURNING {POLL_COLUMNS}"
        )))
        .bind(poll.id)
        .bind(&choices)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        self.active_view(&round, Some(&poll)).await
    }

    /// Closes the current poll with `outcome` and moves the round on: to
    /// its next poll, or to its end.
    async fn advance(
        &self,
        mut tx: Transaction<'_, Postgres>,
        user: Uuid,
        round: RoundRow,
        poll: PollRow,
        outcome: i16,
    ) -> AppResult<Round> {
        sqlx::query("UPDATE ask_polls SET outcome = $2, answered_at = now() WHERE id = $1")
            .bind(poll.id)
            .bind(outcome)
            .execute(&mut *tx)
            .await?;
        let voted = i32::from(outcome == VOTED);
        let finished = round.answered + 1 >= round.poll_count();
        let reward =
            if finished { reward_for(self.cfg.round_reward_coins, round.votes + voted, round.poll_count()) } else { 0 };
        let round: RoundRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE ask_rounds
             SET answered = answered + 1, votes = votes + $2,
                 completed_at = CASE WHEN $3 THEN now() END, reward_coins = $4
             WHERE id = $1 RETURNING {ROUND_COLUMNS}"
        )))
        .bind(round.id)
        .bind(voted)
        .bind(finished)
        .bind(reward)
        .fetch_one(&mut *tx)
        .await?;
        if !finished {
            // Too few friends left for another poll: the round waits, and
            // `GetRound` says why.
            let next = self.draw_poll(&mut tx, user, &round).await?;
            tx.commit().await?;
            return self.active_view(&round, next.as_ref()).await;
        }
        let completed = RoundCompleted {
            user_id: user,
            round_id: round.id,
            votes: round.votes,
            skipped: round.poll_count() - round.votes,
            coins: reward,
        };
        events::emit(&mut *tx, ROUND_COMPLETED, Some(user), &completed).await?;
        tx.commit().await?;
        let round = self.settle_reward(user, round).await?;
        Ok(self.cooldown_view(&round))
    }

    /// Pays the reward of a completed round, once. The grant is the
    /// wallet's own transaction and idempotent on the round, so this can be
    /// repeated until `rewarded_at` is set; a wallet that is down delays the
    /// coins and nothing else.
    async fn settle_reward(&self, user: Uuid, round: RoundRow) -> AppResult<RoundRow> {
        if !round.is_complete() || round.rewarded_at.is_some() {
            return Ok(round);
        }
        if round.reward_coins > 0 {
            // The memo is shown in the user's coin history: the round, and
            // nothing about who was picked in it.
            let memo = serde_json::json!({ "round_id": round.id });
            if let Err(e) = ledger::grant(
                &self.core,
                user,
                round.reward_coins,
                REWARD_REASON,
                &format!("{REWARD_REASON}:{}", round.id),
                memo,
            )
            .await
            {
                tracing::warn!(%user, round_id = %round.id, error = %e, "paying a round's reward failed; it will be retried");
                return Ok(round);
            }
        }
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE ask_rounds SET rewarded_at = COALESCE(rewarded_at, now()) WHERE id = $1 RETURNING {ROUND_COLUMNS}"
        )))
        .bind(round.id)
        .fetch_one(&self.core.db)
        .await?)
    }

    fn cooldown_view(&self, round: &RoundRow) -> Round {
        Round {
            id: round.id.to_string(),
            state: RoundState::ROUND_STATE_COOLDOWN.into(),
            poll_count: round.poll_count(),
            answered_count: round.answered,
            next_round_at: opt_ts(round.completed_at.map(|at| at + self.cooldown())),
            // Shown once the coins are in the wallet.
            reward_coins: if round.rewarded_at.is_some() { round.reward_coins } else { 0 },
            ..Default::default()
        }
    }

    async fn active_view(&self, round: &RoundRow, poll: Option<&PollRow>) -> AppResult<Round> {
        let poll = match poll {
            Some(poll) => Some(self.poll_view(poll).await?),
            None => None,
        };
        Ok(Round {
            id: round.id.to_string(),
            state: RoundState::ROUND_STATE_ACTIVE.into(),
            poll_count: round.poll_count(),
            answered_count: round.answered,
            poll: poll.into(),
            ..Default::default()
        })
    }

    async fn poll_view(&self, poll: &PollRow) -> AppResult<Poll> {
        let (text_ja, text_en, emoji): (String, String, String) =
            sqlx::query_as("SELECT text_ja, text_en, emoji FROM ask_poll_questions WHERE id = $1")
                .bind(poll.question_id)
                .fetch_one(&self.core.db)
                .await?;
        Ok(Poll {
            id: poll.id.to_string(),
            position: poll.position + 1,
            question: poll_question(poll.question_id, &text_ja, &text_en, &emoji).into(),
            choices: tokuly_identity::summaries_ordered(&self.core, &poll.choices).await?,
            shuffles_left: (self.cfg.shuffles_per_poll as i32 - poll.shuffles_used).max(0),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reward_is_proportional_to_votes() {
        assert_eq!(reward_for(12, 12, 12), 12);
        assert_eq!(reward_for(12, 9, 12), 9);
        assert_eq!(reward_for(12, 1, 12), 1);
        assert_eq!(reward_for(12, 0, 12), 0, "a round that was only skipped pays nothing");
        assert_eq!(reward_for(10, 5, 12), 4, "rounded down");
        assert_eq!(reward_for(100, 3, 4), 75);
    }

    #[test]
    fn the_reward_survives_odd_inputs() {
        assert_eq!(reward_for(12, 5, 0), 0);
        assert_eq!(reward_for(-5, 5, 12), 0);
        assert_eq!(reward_for(12, 99, 12), 12, "never more than the full reward");
        assert_eq!(reward_for(12, -1, 12), 0);
    }

    #[test]
    fn a_shuffle_shows_others_and_tops_up_from_the_current_four() {
        let ids: Vec<Uuid> = (0..8).map(|_| Uuid::now_v7()).collect();
        let current = &ids[..4];
        // Four others: a completely new four.
        assert_eq!(reshuffled(current, ids[4..].to_vec()).unwrap(), ids[4..].to_vec());
        // One other: that one plus three of the current four.
        let next = reshuffled(current, vec![ids[4]]).unwrap();
        assert_eq!(next.len(), CHOICES);
        assert!(next.contains(&ids[4]));
        assert_eq!(next.iter().filter(|id| current.contains(id)).count(), 3);
        // Nobody else: nothing to shuffle to.
        assert!(reshuffled(current, Vec::new()).is_none());
    }

    #[test]
    fn a_shuffle_never_repeats_a_friend() {
        let ids: Vec<Uuid> = (0..6).map(|_| Uuid::now_v7()).collect();
        let mut next = reshuffled(&ids[..4], ids[4..].to_vec()).unwrap();
        assert_eq!(next.len(), CHOICES);
        next.sort_unstable();
        next.dedup();
        assert_eq!(next.len(), CHOICES);
    }
}
