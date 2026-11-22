//! Polls: a question with 2-5 options, open for a while, one vote per
//! viewer, results pushed live.
//!
//! Vote counts are pushed at most once a second per poll: the first vote of
//! a second pushes right away, later ones mark the poll dirty and the poll
//! worker pushes the latest numbers on its next tick ([`flush_dirty`]).
use std::{collections::HashMap, time::Duration};

use tokuly_core::prelude::*;
use tokuly_proto::live::{Poll, PollOption};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    Live,
    streams::{self, StreamRow, Tx},
};

pub const MAX_QUESTION: usize = 100;
pub const MAX_OPTION: usize = 50;
pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 5;
/// A poll stays open for 15 seconds to 30 minutes.
pub const MIN_DURATION_SECONDS: i32 = 15;
pub const MAX_DURATION_SECONDS: i32 = 1800;

const DIRTY_KEY: &str = "live:polls:dirty";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PollRow {
    pub id: Uuid,
    pub stream_id: Uuid,
    pub question: String,
    pub options: Vec<String>,
    /// Votes per option, in the order of `options`.
    pub votes: Vec<i64>,
    pub started_at: Time,
    pub ends_at: Time,
    pub ended_at: Option<Time>,
}

macro_rules! poll_columns {
    () => {
        "p.id, p.stream_id, p.question, p.options, p.votes, p.started_at, p.ends_at, p.ended_at"
    };
}

impl PollRow {
    pub fn is_open(&self) -> bool {
        self.ended_at.is_none() && self.ends_at > now()
    }

    /// The poll with the caller's vote (`0` = none, and in pushes).
    pub fn to_proto(&self, viewer_choice: i32) -> Poll {
        Poll {
            id: self.id.to_string(),
            stream_id: self.stream_id.to_string(),
            question: self.question.clone(),
            options: self
                .options
                .iter()
                .enumerate()
                .map(|(i, text)| PollOption {
                    text: text.clone(),
                    vote_count: self.votes.get(i).copied().unwrap_or(0),
                    ..Default::default()
                })
                .collect(),
            total_votes: self.votes.iter().sum(),
            started_at: ts(self.started_at),
            ends_at: ts(self.ends_at),
            ended_at: opt_ts(self.ended_at),
            viewer_choice,
            ..Default::default()
        }
    }
}

fn not_found() -> AppError {
    AppError::not_found("POLL_NOT_FOUND", "poll not found")
}

/// Checks what a poll is made of (`POLL_INVALID`).
pub fn validate<S: AsRef<str>>(
    question: &str,
    options: &[S],
    duration_seconds: i32,
) -> AppResult<(String, Vec<String>)> {
    let bad = |message: String| AppError::invalid("POLL_INVALID", message);
    let clean = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let question = clean(question);
    if !(1..=MAX_QUESTION).contains(&question.graphemes(true).count()) {
        return Err(bad(format!("the question is 1 to {MAX_QUESTION} characters")));
    }
    let options: Vec<String> = options.iter().map(|option| clean(option.as_ref())).collect();
    if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&options.len()) {
        return Err(bad(format!("a poll has {MIN_OPTIONS} to {MAX_OPTIONS} options")));
    }
    if options.iter().any(|option| !(1..=MAX_OPTION).contains(&option.graphemes(true).count())) {
        return Err(bad(format!("an option is 1 to {MAX_OPTION} characters")));
    }
    if (1..options.len()).any(|i| options[..i].contains(&options[i])) {
        return Err(bad("the options must differ".to_owned()));
    }
    if !(MIN_DURATION_SECONDS..=MAX_DURATION_SECONDS).contains(&duration_seconds) {
        return Err(bad(format!("a poll runs for {MIN_DURATION_SECONDS} to {MAX_DURATION_SECONDS} seconds")));
    }
    Ok((question, options))
}

pub async fn find(core: &Core, poll_id: Uuid) -> AppResult<Option<PollRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", poll_columns!(), " FROM live_polls p WHERE p.id = $1"))
        .bind(poll_id)
        .fetch_optional(&core.db)
        .await?)
}

/// The poll of a stream that is open, if any.
pub async fn open_of(core: &Core, stream_id: Uuid) -> AppResult<Option<PollRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        poll_columns!(),
        " FROM live_polls p WHERE p.stream_id = $1 AND p.ended_at IS NULL"
    ))
    .bind(stream_id)
    .fetch_optional(&core.db)
    .await?)
}

/// The newest poll of a stream, open or not.
pub async fn latest_of(core: &Core, stream_id: Uuid) -> AppResult<Option<PollRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        poll_columns!(),
        " FROM live_polls p WHERE p.stream_id = $1 ORDER BY p.id DESC LIMIT 1"
    ))
    .bind(stream_id)
    .fetch_optional(&core.db)
    .await?)
}

/// Polls of a stream before `before`, newest first.
pub async fn list(core: &Core, stream_id: Uuid, before: Option<Uuid>, limit: i64) -> AppResult<Vec<PollRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        poll_columns!(),
        " FROM live_polls p WHERE p.stream_id = $1 AND ($2::uuid IS NULL OR p.id < $2) ORDER BY p.id DESC LIMIT $3"
    ))
    .bind(stream_id)
    .bind(before)
    .bind(limit)
    .fetch_all(&core.db)
    .await?)
}

/// What `user_id` voted for in each of `poll_ids` (absent = no vote).
pub async fn choices_of(core: &Core, poll_ids: &[Uuid], user_id: Option<Uuid>) -> AppResult<HashMap<Uuid, i32>> {
    let Some(user_id) = user_id else { return Ok(HashMap::new()) };
    if poll_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(Uuid, i32)> =
        sqlx::query_as("SELECT poll_id, option FROM live_poll_votes WHERE user_id = $1 AND poll_id = ANY($2)")
            .bind(user_id)
            .bind(poll_ids)
            .fetch_all(&core.db)
            .await?;
    Ok(rows.into_iter().collect())
}

/// A poll as `user_id` sees it.
pub async fn to_proto_for(core: &Core, poll: &PollRow, user_id: Option<Uuid>) -> AppResult<Poll> {
    let choice = choices_of(core, &[poll.id], user_id).await?.get(&poll.id).copied().unwrap_or(0);
    Ok(poll.to_proto(choice))
}

/// Closes the open poll of a stream inside `tx` (the stream is ending).
pub(crate) async fn close_open_in(tx: &mut Tx<'_>, stream_id: Uuid) -> AppResult<Vec<PollRow>> {
    Ok(sqlx::query_as(concat!(
        "UPDATE live_polls p SET ended_at = now() WHERE p.stream_id = $1 AND p.ended_at IS NULL RETURNING ",
        poll_columns!()
    ))
    .bind(stream_id)
    .fetch_all(&mut **tx)
    .await?)
}

impl Live {
    /// Opens a poll on a stream.
    pub async fn open_poll(
        &self,
        stream: &StreamRow,
        user_id: Uuid,
        question: &str,
        options: &[String],
        duration_seconds: i32,
    ) -> AppResult<PollRow> {
        let (question, options) = validate(question, options, duration_seconds)?;
        ensure!(!stream.is_ended(), precondition, "STREAM_ENDED", "the stream is over");
        let poll: PollRow = sqlx::query_as(concat!(
            "INSERT INTO live_polls AS p (stream_id, created_by, question, options, votes, ends_at)
             VALUES ($1, $2, $3, $4, array_fill(0::bigint, ARRAY[cardinality($4)]), now() + make_interval(secs => $5))
             RETURNING ",
            poll_columns!()
        ))
        .bind(stream.id)
        .bind(user_id)
        .bind(&question)
        .bind(&options)
        .bind(f64::from(duration_seconds))
        .fetch_one(&self.core.db)
        .await
        .map_err(|e| {
            let error = AppError::from(e);
            if error.is_already_exists() {
                AppError::precondition("POLL_ALREADY_OPEN", "close the open poll first")
            } else {
                error
            }
        })?;
        self.push_poll(stream, &poll).await;
        Ok(poll)
    }

    /// Casts the one vote of `user_id` (option is 1-based).
    pub async fn cast_vote(
        &self,
        stream: &StreamRow,
        poll: &PollRow,
        user_id: Uuid,
        option: i32,
    ) -> AppResult<PollRow> {
        ensure!(poll.is_open(), precondition, "POLL_CLOSED", "this poll is closed");
        ensure!(
            option >= 1 && option as usize <= poll.options.len(),
            invalid,
            "POLL_OPTION_INVALID",
            "choose one of the poll's options"
        );
        let core = &self.core;
        let mut tx = core.db.begin().await?;
        let voted = sqlx::query(
            "INSERT INTO live_poll_votes (poll_id, user_id, option) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(poll.id)
        .bind(user_id)
        .bind(option)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        ensure!(voted, already_exists, "ALREADY_VOTED", "you have already voted in this poll");
        let updated: Option<PollRow> = sqlx::query_as(concat!(
            "UPDATE live_polls p SET votes[$2] = p.votes[$2] + 1 WHERE p.id = $1 AND p.ended_at IS NULL AND p.ends_at > now() RETURNING ",
            poll_columns!()
        ))
        .bind(poll.id)
        .bind(option)
        .fetch_optional(&mut *tx)
        .await?;
        // Closed while the vote was on its way: the vote does not count.
        let updated = updated.ok_or_else(|| AppError::precondition("POLL_CLOSED", "this poll is closed"))?;
        tx.commit().await?;

        if core.cache.set_nx(&format!("live:poll-push:{}", poll.id), "1", Duration::from_secs(1)).await.unwrap_or(true)
        {
            self.push_poll(stream, &updated).await;
        } else {
            let _: Result<(), _> =
                redis::cmd("SADD").arg(DIRTY_KEY).arg(poll.id.to_string()).query_async(&mut core.cache.conn()).await;
        }
        Ok(updated)
    }

    /// Closes a poll now. Returns the poll as it ended (unchanged if it
    /// was closed already).
    pub async fn close_poll(&self, stream: &StreamRow, poll: &PollRow) -> AppResult<PollRow> {
        let closed: Option<PollRow> = sqlx::query_as(concat!(
            "UPDATE live_polls p SET ended_at = now() WHERE p.id = $1 AND p.ended_at IS NULL RETURNING ",
            poll_columns!()
        ))
        .bind(poll.id)
        .fetch_optional(&self.core.db)
        .await?;
        match closed {
            Some(closed) => {
                self.push_poll(stream, &closed).await;
                Ok(closed)
            }
            None => find(&self.core, poll.id).await?.ok_or_else(not_found),
        }
    }

    /// Closes the polls whose time is up and pushes the vote counts that
    /// were held back. Called every second by the poll worker; returns how
    /// many polls were closed.
    pub async fn tick_polls(&self) -> AppResult<u64> {
        let core = &self.core;
        let due: Vec<PollRow> = sqlx::query_as(concat!(
            "UPDATE live_polls p SET ended_at = now() WHERE p.ended_at IS NULL AND p.ends_at <= now() RETURNING ",
            poll_columns!()
        ))
        .fetch_all(&core.db)
        .await?;
        let closed = due.len() as u64;
        let mut to_push = due;

        let dirty: Vec<String> = redis::cmd("SPOP").arg(DIRTY_KEY).arg(200).query_async(&mut core.cache.conn()).await?;
        for poll_id in dirty.iter().filter_map(|id| Uuid::parse_str(id).ok()) {
            if to_push.iter().all(|poll| poll.id != poll_id)
                && let Some(poll) = find(core, poll_id).await?
            {
                to_push.push(poll);
            }
        }
        for poll in &to_push {
            if let Some(stream) = streams::find_by_id(core, poll.stream_id).await? {
                self.push_poll(&stream, poll).await;
            }
        }
        Ok(closed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polls_need_a_question_two_to_five_distinct_options_and_a_sane_duration() {
        let (question, options) = validate("  好きな  季節は? ", &["春", " 夏 ", "秋", "冬"], 60).unwrap();
        assert_eq!(question, "好きな 季節は?");
        assert_eq!(options, vec!["春", "夏", "秋", "冬"]);
        let reason = |q: &str, o: &[&str], d: i32| validate(q, o, d).unwrap_err().reason;
        assert_eq!(reason("", &["a", "b"], 60), "POLL_INVALID");
        assert_eq!(reason(&"q".repeat(101), &["a", "b"], 60), "POLL_INVALID");
        assert_eq!(reason("q", &["a"], 60), "POLL_INVALID");
        assert_eq!(reason("q", &["a", "b", "c", "d", "e", "f"], 60), "POLL_INVALID");
        assert_eq!(reason("q", &["a", ""], 60), "POLL_INVALID");
        assert_eq!(reason("q", &["a", &"o".repeat(51)], 60), "POLL_INVALID");
        assert_eq!(reason("q", &["a", "a "], 60), "POLL_INVALID");
        assert_eq!(reason("q", &["a", "b"], 14), "POLL_INVALID");
        assert_eq!(reason("q", &["a", "b"], 1801), "POLL_INVALID");
    }

    #[test]
    fn a_poll_reports_votes_per_option_and_in_total() {
        let poll = PollRow {
            id: Uuid::now_v7(),
            stream_id: Uuid::now_v7(),
            question: "q".into(),
            options: vec!["a".into(), "b".into(), "c".into()],
            votes: vec![3, 0, 5],
            started_at: now(),
            ends_at: now() + chrono::Duration::seconds(60),
            ended_at: None,
        };
        assert!(poll.is_open());
        let proto = poll.to_proto(3);
        assert_eq!(proto.total_votes, 8);
        assert_eq!(proto.options.iter().map(|o| o.vote_count).collect::<Vec<_>>(), vec![3, 0, 5]);
        assert_eq!(proto.viewer_choice, 3);
        let over = PollRow { ends_at: now() - chrono::Duration::seconds(1), ..poll };
        assert!(!over.is_open());
    }
}
