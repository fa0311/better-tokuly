//! Polls on notes: 2–10 choices, one or several picks, an end time and who
//! sees the counts when.
//!
//! Votes are private. `note_poll_votes` keeps who picked what only so a
//! vote is final and voters can be told when the poll ends; nothing reads it
//! back for anyone but the voter themself, and the counts are the only
//! thing ever shown (to the people the results visibility allows).
use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    common::SubjectType,
    notes::{Poll, PollChoice, PollTally, notes_event::Event as Push},
};
use tokuly_social::graph;

use crate::{
    Notes,
    content::NewPoll,
    model::{NoteRow, note_path, note_topic},
    visibility::{self, Viewer},
};

/// Votes one user may cast per minute.
const VOTES_PER_MINUTE: u32 = 60;
/// Polls closed per pass of the closing worker.
const CLOSE_BATCH: i64 = 100;

const ALWAYS: i32 = 1;
const AFTER_VOTING: i32 = 2;
const AFTER_END: i32 = 3;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct PollRow {
    pub note_id: Uuid,
    pub choices: Vec<String>,
    pub votes: Vec<i64>,
    pub multiple: bool,
    pub results_visibility: i32,
    pub ends_at: DateTime<Utc>,
    pub voter_count: i64,
}

const POLL_COLUMNS: &str = "note_id, choices, votes, multiple, results_visibility, ends_at, voter_count";

impl PollRow {
    fn closed(&self) -> bool {
        self.ends_at <= Utc::now()
    }

    /// The poll as `viewer` sees it, with their own choices.
    pub fn view(&self, viewer: &Viewer, author: Uuid, mine: Option<&Vec<i32>>) -> Poll {
        let closed = self.closed();
        let voted = mine.is_some();
        let results_visible = match self.results_visibility {
            AFTER_VOTING => closed || voted || viewer.is(author),
            AFTER_END => closed,
            ALWAYS => true,
            // Unknown: counts are not secret, voters are.
            _ => true,
        };
        Poll {
            choices: self
                .choices
                .iter()
                .zip(self.votes.iter().copied().chain(std::iter::repeat(0)))
                .map(|(text, votes)| PollChoice {
                    text: text.clone(),
                    votes: if results_visible { votes } else { 0 },
                    ..Default::default()
                })
                .collect(),
            multiple: self.multiple,
            ends_at: ts(self.ends_at),
            results_visibility: self.results_visibility.into(),
            closed,
            results_visible,
            voter_count: if results_visible { self.voter_count } else { 0 },
            my_choices: mine.cloned().unwrap_or_default(),
            ..Default::default()
        }
    }
}

/// Stores the poll of a new note.
pub(crate) async fn insert(tx: &mut Transaction<'_, Postgres>, note_id: Uuid, poll: &NewPoll) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO note_polls (note_id, choices, votes, multiple, results_visibility, ends_at)
         VALUES ($1, $2, array_fill(0::bigint, ARRAY[cardinality($2::text[])]), $3, $4, now() + make_interval(secs => $5))",
    )
    .bind(note_id)
    .bind(&poll.choices)
    .bind(poll.multiple)
    .bind(poll.results_visibility)
    .bind(poll.duration_seconds as f64)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Checks the picks of a vote: indexes in range, no repeats, exactly one
/// unless the poll allows several.
fn check_choices(choices: &[i32], available: usize, multiple: bool) -> AppResult<Vec<i32>> {
    let mut picked = choices.to_vec();
    picked.sort_unstable();
    picked.dedup();
    let valid = !picked.is_empty()
        && picked.len() == choices.len()
        && (multiple || picked.len() == 1)
        && picked.iter().all(|index| (0..available as i32).contains(index));
    ensure!(valid, invalid, "INVALID_CHOICE", "pick one of the poll's choices (several only where the poll allows it)");
    Ok(picked)
}

impl Notes {
    /// The polls of these notes as `viewer` sees them.
    pub(crate) async fn poll_views(&self, viewer: &Viewer, notes: &[&NoteRow]) -> AppResult<HashMap<Uuid, Poll>> {
        let ids: Vec<Uuid> = notes.iter().map(|note| note.id).collect();
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let sql = format!("SELECT {POLL_COLUMNS} FROM note_polls WHERE note_id = ANY($1)");
        let polls: Vec<PollRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(&ids).fetch_all(&self.core.db).await?;
        if polls.is_empty() {
            return Ok(HashMap::new());
        }
        let mine: HashMap<Uuid, Vec<i32>> = match viewer.id {
            Some(user_id) => {
                let rows: Vec<(Uuid, Vec<i32>)> = sqlx::query_as(
                    "SELECT note_id, choices FROM note_poll_votes WHERE user_id = $1 AND note_id = ANY($2)",
                )
                .bind(user_id)
                .bind(&ids)
                .fetch_all(&self.core.db)
                .await?;
                rows.into_iter().collect()
            }
            None => HashMap::new(),
        };
        let authors: HashMap<Uuid, Uuid> = notes.iter().map(|note| (note.id, note.author_id)).collect();
        Ok(polls
            .iter()
            .map(|poll| (poll.note_id, poll.view(viewer, authors[&poll.note_id], mine.get(&poll.note_id))))
            .collect())
    }

    /// Records `user_id`'s vote in the poll of a note they can see.
    pub(crate) async fn vote(&self, user_id: Uuid, note_id: Uuid, choices: &[i32]) -> AppResult<Poll> {
        let core = &self.core;
        core.cache.rate_limit(&format!("notes:vote:{user_id}"), VOTES_PER_MINUTE, Duration::from_secs(60)).await?;
        let viewer = Viewer::load(core, Some(user_id)).await?;
        let note = self.require_note(&viewer, note_id).await?;
        let mut tx = core.db.begin().await?;
        let sql = format!("SELECT {POLL_COLUMNS} FROM note_polls WHERE note_id = $1 FOR UPDATE");
        let poll: PollRow = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(note.id)
            .fetch_optional(&mut *tx)
            .await?
            .or_not_found("POLL_NOT_FOUND", "this note has no poll")?;
        ensure!(!poll.closed(), precondition, "POLL_CLOSED", "this poll has ended");
        let picked = check_choices(choices, poll.choices.len(), poll.multiple)?;
        let voted = sqlx::query(
            "INSERT INTO note_poll_votes (note_id, user_id, choices) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(note.id)
        .bind(user_id)
        .bind(&picked)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        ensure!(voted == 1, already_exists, "ALREADY_VOTED", "you already voted in this poll");
        let sql = format!(
            "UPDATE note_polls SET voter_count = voter_count + 1,
                    votes = (SELECT array_agg(v + CASE WHEN (i - 1) = ANY($2) THEN 1 ELSE 0 END ORDER BY i)
                             FROM unnest(votes) WITH ORDINALITY AS t(v, i))
             WHERE note_id = $1 RETURNING {POLL_COLUMNS}"
        );
        let poll: PollRow =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(note.id).bind(&picked).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        self.push_tally(&poll, false).await;
        Ok(poll.view(&viewer, note.author_id, Some(&picked)))
    }

    async fn push_tally(&self, poll: &PollRow, closed: bool) {
        let tally = PollTally {
            note_id: poll.note_id.to_string(),
            voter_count: poll.voter_count,
            closed,
            ..Default::default()
        };
        self.push_topic(&note_topic(poll.note_id), Push::PollUpdated(Box::new(tally))).await;
    }

    /// Closes polls whose end has come and tells their voters and authors,
    /// once each (the poll closing worker; public for tools and tests).
    /// Returns how many closed.
    pub async fn close_polls(&self) -> AppResult<usize> {
        let core = &self.core;
        let sql = format!(
            "SELECT {POLL_COLUMNS} FROM note_polls WHERE closed_at IS NULL AND ends_at <= now() ORDER BY ends_at LIMIT $1"
        );
        let due: Vec<PollRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(CLOSE_BATCH).fetch_all(&core.db).await?;
        let mut closed = 0;
        for poll in due {
            let sql = format!("SELECT {} FROM notes n WHERE n.id = $1", crate::model::NOTE_COLUMNS);
            let Some(note): Option<NoteRow> =
                sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(poll.note_id).fetch_optional(&core.db).await?
            else {
                continue;
            };
            // Who is told: voters who may still see the note (worked out
            // before the transaction opens), and the author.
            let voters: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM note_poll_votes WHERE note_id = $1")
                .bind(poll.note_id)
                .fetch_all(&core.db)
                .await?;
            let readers = self.still_reaching(&note, &voters).await?;
            let author = tokuly_identity::summary(core, note.author_id).await.ok();
            let mut tx = core.db.begin().await?;
            let marked =
                sqlx::query("UPDATE note_polls SET closed_at = now() WHERE note_id = $1 AND closed_at IS NULL")
                    .bind(poll.note_id)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
            if marked == 0 {
                continue; // another worker got there first
            }
            for user_id in readers.iter().copied().chain(std::iter::once(note.author_id)) {
                outbound::notify(&mut *tx, &poll_ended(user_id, &note, author.as_ref().map(|a| a.handle.as_str())))
                    .await?;
            }
            tx.commit().await?;
            closed += 1;
            self.push_tally(&poll, true).await;
        }
        Ok(closed)
    }

    /// Of `people`, those who may see `note` now (by its audience, without
    /// anyone in a block with the author).
    pub(crate) async fn still_reaching(&self, note: &NoteRow, people: &[Uuid]) -> AppResult<Vec<Uuid>> {
        if people.is_empty() {
            return Ok(Vec::new());
        }
        let core = &self.core;
        let author = note.author_id;
        let hidden = graph::blocked_ids_either(core, author).await?;
        let mut allowed: Vec<Uuid> = if note.is_listed() {
            people.to_vec()
        } else if note.visibility == visibility::PRIVATE {
            note.direct_user_ids.clone()
        } else {
            let mut members = graph::audience_member_ids(core, author, note.audience()).await?;
            if note.visibility != visibility::PUBLIC
                && let Some(answered) = note.reply_to_author_id
            {
                members.push(answered);
            }
            members
        };
        allowed.retain(|id| people.contains(id) && *id != author && !hidden.contains(id));
        allowed.sort_unstable();
        allowed.dedup();
        tokuly_identity::users::filter_active(core, &allowed).await
    }
}

/// `notes.poll_ended`: a poll the recipient voted in (or wrote) ended.
fn poll_ended(recipient: Uuid, note: &NoteRow, author_handle: Option<&str>) -> NotificationRequest {
    let (title, body) = if recipient == note.author_id {
        (Localized::new("アンケートの結果が出たよ!", "Your poll has ended"), Localized::same(note.title()))
    } else {
        let by = author_handle.map(|handle| format!("@{handle}")).unwrap_or_default();
        (
            Localized::new("投票したアンケートの結果が出たよ!", "A poll you voted in has ended"),
            Localized::same(format!("{by} {}", note.title()).trim().to_owned()),
        )
    };
    NotificationRequest::new(recipient, "notes.poll_ended", title, body, note_path(note.id))
        .subject(SubjectType::SUBJECT_TYPE_NOTE, note.id)
        .collapse(format!("notes.poll_ended:{}", note.id))
        .data(serde_json::json!({ "note_id": note.id }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll(visibility: i32, ends_in: i64) -> PollRow {
        PollRow {
            note_id: Uuid::now_v7(),
            choices: vec!["a".into(), "b".into()],
            votes: vec![3, 1],
            multiple: false,
            results_visibility: visibility,
            ends_at: Utc::now() + chrono::Duration::seconds(ends_in),
            voter_count: 4,
        }
    }

    #[test]
    fn counts_show_only_when_the_results_visibility_allows() {
        let (author, reader) = (Uuid::now_v7(), Uuid::now_v7());
        let viewer = Viewer::default();
        let open_always = poll(ALWAYS, 60).view(&viewer, author, None);
        assert!(open_always.results_visible && open_always.choices[0].votes == 3);
        let after_voting = poll(AFTER_VOTING, 60);
        assert!(!after_voting.view(&viewer, author, None).results_visible);
        let hidden = after_voting.view(&viewer, author, None);
        assert_eq!((hidden.choices[0].votes, hidden.voter_count), (0, 0), "nothing leaks while hidden");
        assert!(after_voting.view(&viewer, author, Some(&vec![0])).results_visible);
        assert!(poll(AFTER_VOTING, -1).view(&viewer, author, None).results_visible);
        assert!(!poll(AFTER_END, 60).view(&viewer, author, Some(&vec![1])).results_visible);
        assert!(poll(AFTER_END, -1).view(&viewer, reader, None).closed);
    }

    #[test]
    fn picks_are_checked() {
        assert_eq!(check_choices(&[1], 3, false).unwrap(), vec![1]);
        assert_eq!(check_choices(&[2, 0], 3, true).unwrap(), vec![0, 2]);
        for (choices, multiple) in
            [(vec![], true), (vec![0, 1], false), (vec![3], true), (vec![-1], true), (vec![1, 1], true)]
        {
            assert_eq!(check_choices(&choices, 3, multiple).unwrap_err().reason, "INVALID_CHOICE");
        }
    }
}
